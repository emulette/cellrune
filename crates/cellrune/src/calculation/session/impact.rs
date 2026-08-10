use std::collections::BTreeSet;

use super::super::eval::CompiledWorkbook;
use super::super::runtime::CellId;
use crate::{
    CalculationCellId, CalculationSnapshot, CellContent, MaterializedResultOrigin, WorkbookSnapshot,
};

pub(super) fn affected_formulas(
    workbook: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    previous: Option<&CalculationSnapshot>,
    changed_cells: &[CalculationCellId],
) -> BTreeSet<CalculationCellId> {
    if changed_cells.is_empty() {
        return BTreeSet::new();
    }
    let mut changed_internal = Vec::with_capacity(changed_cells.len());
    for cell in changed_cells {
        if let Some(internal) = public_to_internal(workbook, *cell) {
            changed_internal.push(internal);
        }
    }
    let mut dirty = BTreeSet::new();
    if let Some(previous) = previous {
        for cell in changed_cells {
            let Some(materialized) = previous.materialized_cell(*cell) else {
                continue;
            };
            if let MaterializedResultOrigin::DynamicSpill { anchor, .. } = materialized.origin()
                && let Some(anchor) = public_to_internal(workbook, anchor)
            {
                dirty.insert(anchor);
            }
        }
    }
    for changed in changed_internal {
        for formula in compiled.direct_affected_formulas(changed) {
            dirty.insert(formula);
        }
    }
    let mut pending = dirty.iter().copied().collect::<Vec<_>>();
    while let Some(cell) = pending.pop() {
        for child in compiled.dependents(cell) {
            if dirty.insert(*child) {
                pending.push(*child);
            }
        }
    }
    dirty
        .into_iter()
        .filter_map(|cell| internal_to_public(workbook, cell))
        .collect()
}

fn public_to_internal(workbook: &WorkbookSnapshot, cell: CalculationCellId) -> Option<CellId> {
    let sheet = workbook
        .sheets()
        .iter()
        .position(|candidate| candidate.id() == cell.sheet_id())?;
    Some((
        sheet,
        cell.address().row().get(),
        cell.address().column().get(),
    ))
}

fn internal_to_public(workbook: &WorkbookSnapshot, cell: CellId) -> Option<CalculationCellId> {
    let sheet = workbook.sheets().get(cell.0)?;
    let address = crate::CellAddress::from_indices(cell.1, cell.2).ok()?;
    Some(CalculationCellId::new(sheet.id(), address))
}

pub(super) fn formula_cells(
    workbook: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<CalculationCellId>, ()> {
    let mut formulas = BTreeSet::new();
    for cell in compiled.formula_cells() {
        if cancelled() {
            return Err(());
        }
        if let Some(cell) = internal_to_public(workbook, cell) {
            formulas.insert(cell);
        }
    }
    Ok(formulas)
}

pub(super) fn formula_cells_from_workbook(
    workbook: &WorkbookSnapshot,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<CalculationCellId>, ()> {
    let mut formulas = BTreeSet::new();
    for sheet in workbook.sheets() {
        for cell in sheet.cells() {
            if cancelled() {
                return Err(());
            }
            if matches!(cell.content(), CellContent::Formula(_)) {
                formulas.insert(CalculationCellId::new(sheet.id(), cell.address()));
            }
        }
    }
    Ok(formulas)
}
