use std::collections::{BTreeMap, BTreeSet};

use super::super::eval::CompiledWorkbook;
use super::super::runtime::{CellId, Rect};
use crate::{CalculationCellId, CellContent, WorkbookSnapshot};

pub(super) fn affected_formulas(
    workbook: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    changed_cells: &[CalculationCellId],
) -> BTreeSet<CalculationCellId> {
    if changed_cells.is_empty() {
        return BTreeSet::new();
    }
    let mut changed_by_sheet = vec![BTreeSet::<(u32, u32)>::new(); workbook.sheets().len()];
    for cell in changed_cells {
        if let Some(internal) = public_to_internal(workbook, *cell) {
            changed_by_sheet[internal.0].insert((internal.1, internal.2));
        }
    }
    let mut dirty = BTreeSet::new();
    for (formula, rects) in compiled.dependency_rectangles() {
        if rects.iter().any(|span| {
            span.rects().any(|rect| {
                changed_by_sheet
                    .get(rect.sheet)
                    .is_some_and(|changed| rect_contains_any(rect, changed))
            })
        }) {
            dirty.insert(*formula);
        }
    }
    let mut dependents = BTreeMap::<CellId, Vec<CellId>>::new();
    for (formula, dependencies) in compiled.dependencies() {
        for dependency in dependencies {
            dependents.entry(*dependency).or_default().push(*formula);
        }
    }
    let mut pending = dirty.iter().copied().collect::<Vec<_>>();
    while let Some(cell) = pending.pop() {
        if let Some(children) = dependents.get(&cell) {
            for child in children {
                if dirty.insert(*child) {
                    pending.push(*child);
                }
            }
        }
    }
    dirty
        .into_iter()
        .filter_map(|cell| internal_to_public(workbook, cell))
        .collect()
}

fn rect_contains_any(rect: Rect, cells: &BTreeSet<(u32, u32)>) -> bool {
    cells
        .range((rect.row_start, 0)..=(rect.row_end, u32::MAX))
        .any(|(_, column)| (rect.col_start..=rect.col_end).contains(column))
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

pub(super) fn formula_cells(workbook: &WorkbookSnapshot) -> BTreeSet<CalculationCellId> {
    workbook
        .sheets()
        .iter()
        .flat_map(|sheet| {
            sheet.cells().filter_map(|cell| {
                matches!(cell.content(), CellContent::Formula(_))
                    .then_some(CalculationCellId::new(sheet.id(), cell.address()))
            })
        })
        .collect()
}
