use std::collections::{BTreeMap, BTreeSet};

use super::super::DraftCellMutation;
use super::BatchExecutionError;
use super::staged::{mark_upsert, sheet_by_id_mut};
use crate::calculation::formula_rewrite::{
    FormulaRewriteBudget, FormulaRewriteRequest, rewrite_formula,
};
use crate::{
    CalculationCellId, CellAddress, CellContent, DefinedName, FormulaCell, FormulaText,
    NumberFormat, Sheet, SheetId, SheetName, TableId, ValidationError,
};

pub(super) struct FormulaEditState<'a> {
    pub(super) mutations: &'a mut BTreeMap<CalculationCellId, DraftCellMutation>,
    pub(super) changed_cells: &'a mut BTreeSet<CalculationCellId>,
    pub(super) calculation_changed_cells: &'a mut BTreeSet<CalculationCellId>,
    pub(super) touched_sheets: &'a mut BTreeSet<SheetId>,
}

pub(super) type TableFormulaLocations = Vec<(usize, usize, usize)>;

pub(super) struct WorkbookFormulaEdit<'a, 'cancel> {
    pub(super) state: FormulaEditState<'a>,
    pub(super) changed_table_ids: &'a mut BTreeSet<TableId>,
    pub(super) table_formula_locations: &'a TableFormulaLocations,
    pub(super) budget: &'a mut FormulaRewriteBudget<'cancel>,
}

pub(super) fn table_formula_locations(
    sheets: &[Sheet],
    budget: &FormulaRewriteBudget<'_>,
) -> Result<TableFormulaLocations, BatchExecutionError> {
    let mut locations = Vec::new();
    for (sheet_index, sheet) in sheets.iter().enumerate() {
        budget.check_cancelled()?;
        for (table_index, table) in sheet.tables().iter().enumerate() {
            budget.check_cancelled()?;
            for (column_index, column) in table.columns().iter().enumerate() {
                budget.check_cancelled()?;
                if column.calculated_column_formula().is_some()
                    || column.totals_row_formula().is_some()
                {
                    locations.push((sheet_index, table_index, column_index));
                }
            }
        }
    }
    Ok(locations)
}

pub(super) fn upsert_formula(
    sheets: &mut [Sheet],
    state: FormulaEditState<'_>,
    sheet_id: SheetId,
    address: CellAddress,
    formula: FormulaCell,
) -> Result<bool, ValidationError> {
    let sheet = sheet_by_id_mut(sheets, sheet_id)?;
    let number_format = sheet
        .cell(address)
        .map_or_else(NumberFormat::default, |cell| cell.number_format().clone());
    let content = CellContent::Formula(formula);
    if sheet
        .cell(address)
        .is_none_or(|cell| cell.content() != &content || cell.number_format() != &number_format)
    {
        sheet.upsert_cell_deferred(address, content, number_format);
        mark_upsert(state.mutations, sheet_id, address, false);
        state
            .changed_cells
            .insert(CalculationCellId::new(sheet_id, address));
        state
            .calculation_changed_cells
            .insert(CalculationCellId::new(sheet_id, address));
        state.touched_sheets.insert(sheet_id);
        return Ok(true);
    }
    Ok(false)
}

pub(super) enum WorkbookFormulaRename<'a> {
    Sheet {
        old_name: &'a str,
        new_name: &'a str,
    },
    Table {
        old_name: &'a str,
        new_name: &'a str,
    },
    TableColumn {
        table_id: TableId,
        target_sheet_index: usize,
        target_range: crate::CellRange,
        table_name: &'a str,
        old_name: &'a str,
        new_name: &'a str,
    },
}

pub(super) fn rewrite_workbook_formulas(
    sheets: &mut [Sheet],
    defined_names: &mut [DefinedName],
    edit: WorkbookFormulaEdit<'_, '_>,
    rename: WorkbookFormulaRename<'_>,
) -> Result<bool, BatchExecutionError> {
    let WorkbookFormulaEdit {
        state,
        changed_table_ids,
        table_formula_locations,
        budget,
    } = edit;
    let target_location = match rename {
        WorkbookFormulaRename::TableColumn {
            target_sheet_index,
            target_range,
            ..
        } => Some((target_sheet_index, target_range)),
        WorkbookFormulaRename::Sheet { .. } | WorkbookFormulaRename::Table { .. } => None,
    };
    let mut changed = false;
    for (sheet_index, sheet) in sheets.iter_mut().enumerate() {
        let mut previous_address = None;
        loop {
            budget.check_cancelled()?;
            let Some(cell) = sheet.next_formula_cell_after(previous_address) else {
                break;
            };
            let address = cell.address();
            previous_address = Some(address);
            let owner_is_target_table = target_location.is_some_and(|(target_sheet, range)| {
                target_sheet == sheet_index && range.contains(address)
            });
            let request = request_for(&rename, owner_is_target_table);
            let CellContent::Formula(formula) = cell.content() else {
                unreachable!("formula traversal returned a non-formula cell");
            };
            let Some(text) = formula.text() else {
                continue;
            };
            let owner = format!("cell:sheet_id={},address={address}", sheet.id().get());
            let Some(rewritten) = rewrite_formula(text.as_str(), &request, budget)
                .map_err(|error| error.with_owner(owner))?
            else {
                continue;
            };
            let rewritten = FormulaText::from_xlsx(rewritten)?;
            sheet.upsert_cell_deferred(
                address,
                CellContent::Formula(formula.clone().with_text(rewritten)),
                cell.number_format().clone(),
            );
            mark_upsert(state.mutations, sheet.id(), address, false);
            let cell_id = CalculationCellId::new(sheet.id(), address);
            state.changed_cells.insert(cell_id);
            state.calculation_changed_cells.insert(cell_id);
            state.touched_sheets.insert(sheet.id());
            changed = true;
        }
    }

    for defined_name in defined_names.iter_mut() {
        let request = request_for(&rename, false);
        let owner = match defined_name.scope() {
            crate::DefinedNameScope::Workbook => {
                format!("defined_name:scope=workbook,name={}", defined_name.name())
            }
            crate::DefinedNameScope::Sheet(sheet_id) => format!(
                "defined_name:scope=sheet:{},name={}",
                sheet_id.get(),
                defined_name.name()
            ),
        };
        let Some(rewritten) = rewrite_formula(defined_name.formula().as_str(), &request, budget)
            .map_err(|error| error.with_owner(owner))?
        else {
            continue;
        };
        *defined_name = DefinedName::new(
            defined_name.name(),
            defined_name.scope(),
            FormulaText::from_xlsx(rewritten)?,
            defined_name.hidden(),
        )?;
        changed = true;
    }

    for &(sheet_index, table_index, column_index) in table_formula_locations {
        budget.check_cancelled()?;
        let table = &mut sheets[sheet_index].tables_mut()[table_index];
        let current_table_id = table.id();
        let owner_is_target_table = match rename {
            WorkbookFormulaRename::TableColumn { table_id, .. } => current_table_id == table_id,
            WorkbookFormulaRename::Sheet { .. } | WorkbookFormulaRename::Table { .. } => false,
        };
        let column = &mut table.columns_mut()[column_index];
        let table_id = current_table_id.get();
        let column_id = column.column_id().get();
        let request = request_for(&rename, owner_is_target_table);
        let calculated = column
            .calculated_column_formula()
            .map(|formula| {
                rewrite_formula(formula.text().as_str(), &request, budget).map_err(|error| {
                    error.with_owner(format!(
                        "table_formula:table_id={table_id},column_id={column_id},kind=calculated"
                    ))
                })
            })
            .transpose()?
            .flatten()
            .map(FormulaText::from_xlsx)
            .transpose()?;
        let totals = column
            .totals_row_formula()
            .map(|formula| {
                rewrite_formula(formula.text().as_str(), &request, budget).map_err(|error| {
                    error.with_owner(format!(
                        "table_formula:table_id={table_id},column_id={column_id},kind=totals"
                    ))
                })
            })
            .transpose()?
            .flatten()
            .map(FormulaText::from_xlsx)
            .transpose()?;
        if calculated.is_some() || totals.is_some() {
            column.rewrite_formulas(calculated, totals);
            changed_table_ids.insert(current_table_id);
            changed = true;
        }
    }
    Ok(changed)
}

fn request_for<'a>(
    rename: &'a WorkbookFormulaRename<'a>,
    owner_is_target_table: bool,
) -> FormulaRewriteRequest<'a> {
    match rename {
        WorkbookFormulaRename::Sheet { old_name, new_name } => {
            FormulaRewriteRequest::Sheet { old_name, new_name }
        }
        WorkbookFormulaRename::Table { old_name, new_name } => {
            FormulaRewriteRequest::Table { old_name, new_name }
        }
        WorkbookFormulaRename::TableColumn {
            table_name,
            old_name,
            new_name,
            ..
        } => FormulaRewriteRequest::TableColumn {
            table_name,
            old_name,
            new_name,
            owner_is_target_table,
        },
    }
}

pub(super) fn rename_sheet(
    sheets: &mut [Sheet],
    defined_names: &mut [DefinedName],
    edit: WorkbookFormulaEdit<'_, '_>,
    sheet_id: SheetId,
    name: &SheetName,
) -> Result<bool, BatchExecutionError> {
    let old_name = sheets
        .iter()
        .find(|sheet| sheet.id() == sheet_id)
        .ok_or(ValidationError::UnknownSheetId {
            value: sheet_id.get(),
        })?
        .name()
        .clone();
    if old_name == *name {
        return Ok(false);
    }
    if sheets
        .iter()
        .any(|sheet| sheet.id() != sheet_id && sheet.name().lookup_key() == name.lookup_key())
    {
        return Err(ValidationError::DuplicateSheetName {
            name: name.as_str().to_owned(),
        }
        .into());
    }
    rewrite_workbook_formulas(
        sheets,
        defined_names,
        edit,
        WorkbookFormulaRename::Sheet {
            old_name: old_name.as_str(),
            new_name: name.as_str(),
        },
    )?;
    sheet_by_id_mut(sheets, sheet_id)?.rename(name.clone());
    Ok(true)
}
