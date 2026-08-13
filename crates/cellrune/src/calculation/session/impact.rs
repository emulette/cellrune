use std::collections::BTreeSet;

use super::super::eval::CompiledWorkbook;
use super::super::runtime::CellId;
use crate::{
    CalculationCellId, CalculationSnapshot, CellContent, MaterializedResultOrigin, WorkbookSnapshot,
};

const CHARGED_WORK_POLL_INTERVAL: u32 = 256;

/// Tracks charged impact-preparation work and polls cancellation at bounded intervals.
struct ImpactWorkCharger<'a> {
    charged: u32,
    cancelled: &'a dyn Fn() -> bool,
}

impl ImpactWorkCharger<'_> {
    fn charge(&mut self) -> Result<(), ()> {
        self.charged += 1;
        if self.charged >= CHARGED_WORK_POLL_INTERVAL {
            self.charged = 0;
            if (self.cancelled)() {
                return Err(());
            }
        }
        Ok(())
    }
}

pub(super) fn affected_formulas(
    workbook: &WorkbookSnapshot,
    compiled: &CompiledWorkbook,
    previous: Option<&CalculationSnapshot>,
    changed_cells: &[CalculationCellId],
    existing_dirty: &BTreeSet<CalculationCellId>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<CalculationCellId>, ()> {
    if cancelled() {
        return Err(());
    }

    let mut charger = ImpactWorkCharger {
        charged: 0,
        cancelled,
    };

    let mut changed_internal = Vec::with_capacity(changed_cells.len());
    for cell in changed_cells {
        charger.charge()?;
        if let Some(internal) = public_to_internal(workbook, *cell) {
            changed_internal.push(internal);
        }
    }
    let mut dirty = BTreeSet::new();
    if let Some(previous) = previous {
        for cell in changed_cells {
            charger.charge()?;
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
        let formulas = compiled.direct_affected_formulas(changed, &mut || charger.charge())?;
        for formula in formulas {
            charger.charge()?;
            dirty.insert(formula);
        }
    }
    let mut pending = dirty.iter().copied().collect::<Vec<_>>();
    while let Some(cell) = pending.pop() {
        for child in compiled.dependents(cell) {
            charger.charge()?;
            if dirty.insert(*child) {
                pending.push(*child);
            }
        }
    }
    let mut replacement = BTreeSet::new();
    for cell in existing_dirty {
        charger.charge()?;
        replacement.insert(*cell);
    }
    for cell in dirty {
        charger.charge()?;
        if let Some(cell) = internal_to_public(workbook, cell) {
            replacement.insert(cell);
        }
    }
    Ok(replacement)
}

fn public_to_internal(workbook: &WorkbookSnapshot, cell: CalculationCellId) -> Option<CellId> {
    let sheet = workbook.sheet_position(cell.sheet_id())?;
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
