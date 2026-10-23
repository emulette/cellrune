use std::collections::BTreeSet;

use super::super::DraftCellMutationStore;
use super::BatchExecutionError;
use super::staged::{mark_upsert, sheet_by_id_mut};
use crate::calculation::formula_rewrite::{
    FormulaRewriteBudget, FormulaRewriteError, FormulaRewriteRequest, NameRename, SheetChanges,
    rewrite_formula,
};
use crate::calculation::table_conversion::TableArea;
use crate::{
    CalculationCellId, CellAddress, CellContent, DefinedName, FormulaCell, FormulaText, Sheet,
    SheetId, SheetName, TableId, ValidationError,
};

pub(super) struct FormulaEditState<'a> {
    pub(super) mutations: &'a mut DraftCellMutationStore,
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
    let number_format = sheet.cell(address).map_or_else(
        || sheet.new_cell_number_format(address),
        |cell| cell.number_format().clone(),
    );
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
    /// Sheets deleted, moved, or copied, with their tables.
    SheetChanges(SheetChanges<'a>),
    /// A defined name renamed in one scope; each formula resolves it from its own sheet.
    DefinedName(NameRename<'a>),
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
    /// Columns removed from a table, references naming them becoming `#REF!`.
    TableColumnsRemoved {
        table_id: TableId,
        target_sheet_index: usize,
        target_range: crate::CellRange,
        table_name: &'a str,
        removed: &'a [&'a str],
    },
    /// A table converted to a range, its structured references becoming A1 references.
    TableToRange {
        table_id: TableId,
        target_sheet_index: usize,
        area: &'a TableArea<'a>,
    },
}

/// Where one rewritten formula is: its sheet, whether it belongs to the table a change targets,
/// and its row when it is on that table's sheet.
#[derive(Clone, Copy)]
struct FormulaContext<'a> {
    owner_sheet: Option<&'a str>,
    owner_is_target_table: bool,
    row_on_target_sheet: Option<u32>,
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
        WorkbookFormulaRename::TableToRange {
            target_sheet_index,
            area,
            ..
        } => Some((target_sheet_index, area.range)),
        WorkbookFormulaRename::TableColumnsRemoved {
            target_sheet_index,
            target_range,
            ..
        } => Some((target_sheet_index, target_range)),
        WorkbookFormulaRename::Sheet { .. }
        | WorkbookFormulaRename::SheetChanges(_)
        | WorkbookFormulaRename::Table { .. }
        | WorkbookFormulaRename::DefinedName(_) => None,
    };
    let sheet_names = sheets
        .iter()
        .map(|sheet| (sheet.id(), sheet.name().clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut changed = false;
    for (sheet_index, sheet) in sheets.iter_mut().enumerate() {
        let owner = sheet.name().clone();
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
            let on_target_sheet =
                target_location.is_some_and(|(target_sheet, _)| target_sheet == sheet_index);
            let request = request_for(
                &rename,
                FormulaContext {
                    owner_sheet: Some(owner.as_str()),
                    owner_is_target_table,
                    row_on_target_sheet: on_target_sheet.then(|| address.row().get()),
                },
            );
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
        let owner = match defined_name.scope() {
            crate::DefinedNameScope::Workbook => None,
            crate::DefinedNameScope::Sheet(sheet_id) => sheet_names.get(&sheet_id),
        };
        let request = request_for(
            &rename,
            FormulaContext {
                owner_sheet: owner.map(SheetName::as_str),
                owner_is_target_table: false,
                row_on_target_sheet: None,
            },
        );
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
        let owner = sheets[sheet_index].name().clone();
        let table = &mut sheets[sheet_index].tables_mut()[table_index];
        let current_table_id = table.id();
        let owner_is_target_table = match rename {
            WorkbookFormulaRename::TableColumn { table_id, .. }
            | WorkbookFormulaRename::TableColumnsRemoved { table_id, .. }
            | WorkbookFormulaRename::TableToRange { table_id, .. } => current_table_id == table_id,
            WorkbookFormulaRename::Sheet { .. }
            | WorkbookFormulaRename::SheetChanges(_)
            | WorkbookFormulaRename::Table { .. }
            | WorkbookFormulaRename::DefinedName(_) => false,
        };
        // A table formula reads as the formula of the table's first data row.
        let first_data_row = table
            .range()
            .start()
            .row()
            .get()
            .saturating_add(table.header_row_count());
        let on_target_sheet =
            target_location.is_some_and(|(target_sheet, _)| target_sheet == sheet_index);
        let column = &mut table.columns_mut()[column_index];
        let table_id = current_table_id.get();
        let column_id = column.column_id().get();
        let request = request_for(
            &rename,
            FormulaContext {
                owner_sheet: Some(owner.as_str()),
                owner_is_target_table,
                row_on_target_sheet: on_target_sheet.then_some(first_data_row),
            },
        );
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

/// Returns the rewrite request for one formula in `context`.
fn request_for<'a>(
    rename: &'a WorkbookFormulaRename<'a>,
    context: FormulaContext<'a>,
) -> FormulaRewriteRequest<'a> {
    let FormulaContext {
        owner_sheet,
        owner_is_target_table,
        row_on_target_sheet,
    } = context;
    match rename {
        WorkbookFormulaRename::DefinedName(name) => {
            FormulaRewriteRequest::DefinedName(NameRename {
                owner_sheet,
                ..*name
            })
        }
        WorkbookFormulaRename::TableColumnsRemoved {
            table_name,
            removed,
            ..
        } => FormulaRewriteRequest::TableColumnsRemoved {
            table_name,
            removed,
            owner_is_target_table,
        },
        WorkbookFormulaRename::TableToRange { area, .. } => FormulaRewriteRequest::TableToRange {
            area,
            owner_is_target_table,
            this_row: row_on_target_sheet,
        },
        WorkbookFormulaRename::Sheet { old_name, new_name } => {
            FormulaRewriteRequest::Sheet { old_name, new_name }
        }
        WorkbookFormulaRename::SheetChanges(changes) => FormulaRewriteRequest::Sheets(*changes),
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
    rewrite_sheet_metadata_formulas(
        sheets,
        &WorkbookFormulaRename::Sheet {
            old_name: old_name.as_str(),
            new_name: name.as_str(),
        },
        true,
        &mut edit.budget.unmetered(),
    )?;
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

/// Rewrites sheet references in the data validation formulas and, with `hyperlinks`, the internal
/// hyperlink locations the workbook model carries, as the writer rewrites them in the saved
/// worksheet part. A deletion leaves hyperlink locations as they are, as Excel does.
///
/// Text that does not parse stays as it is. A sheet rename passes an unmetered budget because this
/// read-only metadata was not rewritten before it was modeled, and saving such text keeps refusing
/// the rename as it did.
pub(super) fn rewrite_sheet_metadata_formulas(
    sheets: &mut [Sheet],
    rename: &WorkbookFormulaRename<'_>,
    hyperlinks: bool,
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<(), BatchExecutionError> {
    for sheet in sheets.iter_mut() {
        let owner = sheet.name().clone();
        let request = request_for(
            rename,
            FormulaContext {
                owner_sheet: Some(owner.as_str()),
                owner_is_target_table: false,
                row_on_target_sheet: None,
            },
        );
        let request = &request;
        let mut rewrites = Vec::new();
        for (index, validation) in sheet.data_validations().iter().enumerate() {
            for (slot, formula) in [validation.formula1(), validation.formula2()]
                .into_iter()
                .enumerate()
            {
                budget.check_cancelled()?;
                let Some(formula) = formula else {
                    continue;
                };
                if let Some(rewritten) = rewrite_metadata_text(formula, request, budget)? {
                    rewrites.push((index, slot, rewritten));
                }
            }
        }
        for (index, slot, rewritten) in rewrites {
            *sheet.data_validations_mut()[index].formulas_mut()[slot] =
                Some(rewritten.into_boxed_str());
        }
        if !hyperlinks {
            continue;
        }
        let mut locations = Vec::new();
        for (index, hyperlink) in sheet.hyperlinks().iter().enumerate() {
            budget.check_cancelled()?;
            let Some(location) = hyperlink.location() else {
                continue;
            };
            if hyperlink.is_external() {
                continue;
            }
            if let Some(rewritten) = rewrite_metadata_text(location, request, budget)? {
                locations.push((index, rewritten));
            }
        }
        for (index, rewritten) in locations {
            *sheet.hyperlinks_mut()[index].location_mut() = Some(rewritten.into_boxed_str());
        }
    }
    Ok(())
}

/// Returns the rewritten text, or `None` to keep it, including when it does not parse.
fn rewrite_metadata_text(
    text: &str,
    request: &FormulaRewriteRequest<'_>,
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<Option<String>, BatchExecutionError> {
    match rewrite_formula(text, request, budget) {
        Ok(rewritten) => Ok(rewritten),
        Err(FormulaRewriteError::Cancelled) => Err(FormulaRewriteError::Cancelled.into()),
        Err(_) => Ok(None),
    }
}
