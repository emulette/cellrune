use std::collections::{BTreeMap, BTreeSet};

use super::super::DraftCellMutation;
use super::staged::{mark_upsert, sheet_by_id_mut};
use crate::{
    CalculationCellId, CellAddress, CellContent, DefinedName, FormulaCell, FormulaText,
    NumberFormat, Sheet, SheetId, SheetName, ValidationError,
};

pub(super) struct FormulaEditState<'a> {
    pub(super) mutations: &'a mut BTreeMap<CalculationCellId, DraftCellMutation>,
    pub(super) changed_cells: &'a mut BTreeSet<CalculationCellId>,
    pub(super) calculation_changed_cells: &'a mut BTreeSet<CalculationCellId>,
    pub(super) touched_sheets: &'a mut BTreeSet<SheetId>,
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

pub(super) fn rename_sheet(
    sheets: &mut [Sheet],
    defined_names: &mut [DefinedName],
    mutations: &mut BTreeMap<CalculationCellId, DraftCellMutation>,
    changed_cells: &mut BTreeSet<CalculationCellId>,
    calculation_changed_cells: &mut BTreeSet<CalculationCellId>,
    sheet_id: SheetId,
    name: &SheetName,
) -> Result<bool, ValidationError> {
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
        });
    }
    sheet_by_id_mut(sheets, sheet_id)?.rename(name.clone());
    for sheet in sheets {
        let formula_addresses = sheet
            .cells()
            .filter_map(|cell| {
                matches!(cell.content(), CellContent::Formula(_)).then_some(cell.address())
            })
            .collect::<Vec<_>>();
        for address in formula_addresses {
            let Some(cell) = sheet.cell(address).cloned() else {
                continue;
            };
            let CellContent::Formula(formula) = cell.content() else {
                continue;
            };
            let Some(text) = formula.text() else {
                continue;
            };
            let rewritten = super::super::formula_rewrite::rename_sheet_references(
                text.as_str(),
                old_name.as_str(),
                name.as_str(),
            );
            if rewritten == text.as_str() {
                continue;
            }
            sheet.upsert_cell_deferred(
                address,
                CellContent::Formula(
                    formula
                        .clone()
                        .with_text(FormulaText::from_xlsx(rewritten)?),
                ),
                cell.number_format().clone(),
            );
            mark_upsert(mutations, sheet.id(), address, false);
            let changed = CalculationCellId::new(sheet.id(), address);
            changed_cells.insert(changed);
            calculation_changed_cells.insert(changed);
        }
    }
    for defined_name in defined_names.iter_mut() {
        let rewritten = super::super::formula_rewrite::rename_sheet_references(
            defined_name.formula().as_str(),
            old_name.as_str(),
            name.as_str(),
        );
        *defined_name = DefinedName::new(
            defined_name.name(),
            defined_name.scope(),
            FormulaText::from_xlsx(rewritten)?,
            defined_name.hidden(),
        )?;
    }
    Ok(true)
}
