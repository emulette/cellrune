use std::collections::{BTreeMap, BTreeSet};

use super::super::eval::{CompiledWorkbook, DependencyTarget};
use super::super::runtime::{CellId, Rect};
use crate::{
    CalculationCellId, CalculationSnapshot, CellContent, FormulaMetadata, MaterializedResultOrigin,
    WorkbookSnapshot,
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
    let mut changed_by_sheet = vec![BTreeSet::<(u32, u32)>::new(); workbook.sheets().len()];
    for cell in changed_cells {
        if let Some(internal) = public_to_internal(workbook, *cell) {
            changed_by_sheet[internal.0].insert((internal.1, internal.2));
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
    for (sheet_index, sheet) in workbook.sheets().iter().enumerate() {
        let Some(changed) = changed_by_sheet.get(sheet_index) else {
            continue;
        };
        for cell in sheet.cells() {
            let CellContent::Formula(formula) = cell.content() else {
                continue;
            };
            let FormulaMetadata::DynamicArray {
                range: Some(range), ..
            } = formula.metadata()
            else {
                continue;
            };
            let rect = Rect {
                sheet: sheet_index,
                row_start: range.start().row().get(),
                col_start: range.start().column().get(),
                row_end: range.end().row().get(),
                col_end: range.end().column().get(),
                whole_rows: false,
            };
            if rect_contains_any(rect, changed) {
                dirty.insert((
                    sheet_index,
                    cell.address().row().get(),
                    cell.address().column().get(),
                ));
            }
        }
    }
    for (formula, targets) in compiled.dependency_targets() {
        if targets.iter().any(|target| match target {
            DependencyTarget::Cell((sheet, row, column))
            | DependencyTarget::SpillAnchor((sheet, row, column))
            | DependencyTarget::FormulaContent((sheet, row, column)) => changed_by_sheet
                .get(*sheet)
                .is_some_and(|changed| changed.contains(&(*row, *column))),
            DependencyTarget::Area(span) => span.rects().any(|rect| {
                changed_by_sheet
                    .get(rect.sheet)
                    .is_some_and(|changed| rect_contains_any(rect, changed))
            }),
            DependencyTarget::TableIdentity(_) => false,
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

pub(super) fn formula_cells(
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
