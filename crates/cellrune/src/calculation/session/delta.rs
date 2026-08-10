use super::{CalculationDecisionReason, CalculationExecutionMode, SessionError, SessionErrorCode};
use crate::{
    CalculationCellId, CalculationCellResult, CalculationSnapshot, MaterializedResultOrigin,
};

/// One changed direct or materialized calculation result.
#[derive(Debug, Clone, PartialEq)]
pub struct CalculationDeltaCell {
    cell: CalculationCellId,
    origin: MaterializedResultOrigin,
    result: CalculationCellResult,
}

impl CalculationDeltaCell {
    pub(super) const fn new(
        cell: CalculationCellId,
        origin: MaterializedResultOrigin,
        result: CalculationCellResult,
    ) -> Self {
        Self {
            cell,
            origin,
            result,
        }
    }

    /// Returns the changed workbook-local cell.
    pub const fn cell(&self) -> CalculationCellId {
        self.cell
    }

    /// Returns why the result is present in the materialization view.
    pub const fn origin(&self) -> MaterializedResultOrigin {
        self.origin
    }

    /// Returns the new typed result or calculation issue.
    pub const fn result(&self) -> &CalculationCellResult {
        &self.result
    }
}

/// Bounded, deterministically ordered result changes from one installed calculation.
#[derive(Debug, Clone, PartialEq)]
pub struct CalculationDelta {
    pub(super) cursor: u64,
    pub(super) base_revision: u64,
    pub(super) result_revision: u64,
    pub(super) mode: CalculationExecutionMode,
    pub(super) reason: CalculationDecisionReason,
    pub(super) dirty_count: usize,
    pub(super) evaluated_count: usize,
    pub(super) parsed_formula_count: usize,
    pub(super) changed_cells: Vec<CalculationDeltaCell>,
    pub(super) removed_materialized_cells: Vec<CalculationCellId>,
}

impl CalculationDelta {
    pub(super) fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        let mut changed_cells = Vec::with_capacity(self.changed_cells.len());
        for cell in &self.changed_cells {
            if cancelled() {
                return Err(());
            }
            changed_cells.push(cell.clone());
        }
        let mut removed_materialized_cells =
            Vec::with_capacity(self.removed_materialized_cells.len());
        for cell in &self.removed_materialized_cells {
            if cancelled() {
                return Err(());
            }
            removed_materialized_cells.push(*cell);
        }
        Ok(Self {
            cursor: self.cursor,
            base_revision: self.base_revision,
            result_revision: self.result_revision,
            mode: self.mode,
            reason: self.reason,
            dirty_count: self.dirty_count,
            evaluated_count: self.evaluated_count,
            parsed_formula_count: self.parsed_formula_count,
            changed_cells,
            removed_materialized_cells,
        })
    }

    /// Returns the monotonically increasing installed-delta cursor.
    pub const fn cursor(&self) -> u64 {
        self.cursor
    }

    /// Returns the prior installed result revision, or the current revision for the first result.
    pub const fn base_revision(&self) -> u64 {
        self.base_revision
    }

    /// Returns the workbook revision calculated by this delta.
    pub const fn result_revision(&self) -> u64 {
        self.result_revision
    }

    /// Returns whether this pass evaluated a full or incremental schedule.
    pub const fn mode(&self) -> CalculationExecutionMode {
        self.mode
    }

    /// Returns the deterministic reason for the selected execution mode.
    pub const fn reason(&self) -> CalculationDecisionReason {
        self.reason
    }

    /// Returns the number of formulas invalidated before the pass.
    pub const fn dirty_count(&self) -> usize {
        self.dirty_count
    }

    /// Returns the number of formula cells whose evaluator ran.
    pub const fn evaluated_count(&self) -> usize {
        self.evaluated_count
    }

    /// Returns the number of formulas parsed while preparing this pass.
    pub const fn parsed_formula_count(&self) -> usize {
        self.parsed_formula_count
    }

    /// Returns cells whose typed result, issue, or materialization origin changed.
    pub fn changed_cells(&self) -> &[CalculationDeltaCell] {
        &self.changed_cells
    }

    /// Returns cells that were present in the prior materialization view but no longer exist.
    pub fn removed_materialized_cells(&self) -> &[CalculationCellId] {
        &self.removed_materialized_cells
    }
}

/// One cursor page of complete, individually bounded calculation deltas.
#[derive(Debug, Clone, PartialEq)]
pub struct CalculationDeltaPage {
    requested_cursor: u64,
    next_cursor: Option<u64>,
    deltas: Vec<CalculationDelta>,
}

impl CalculationDeltaPage {
    pub(super) const fn new(
        requested_cursor: u64,
        next_cursor: Option<u64>,
        deltas: Vec<CalculationDelta>,
    ) -> Self {
        Self {
            requested_cursor,
            next_cursor,
            deltas,
        }
    }

    /// Returns the exclusive cursor supplied by the caller.
    pub const fn requested_cursor(&self) -> u64 {
        self.requested_cursor
    }

    /// Returns the cursor for the next page, or `None` when caught up.
    pub const fn next_cursor(&self) -> Option<u64> {
        self.next_cursor
    }

    /// Returns complete deltas in ascending cursor order.
    pub fn deltas(&self) -> &[CalculationDelta] {
        &self.deltas
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DeltaMetadata {
    pub(super) base_revision: u64,
    pub(super) result_revision: u64,
    pub(super) mode: CalculationExecutionMode,
    pub(super) reason: CalculationDecisionReason,
    pub(super) dirty_count: usize,
    pub(super) evaluated_count: usize,
    pub(super) parsed_formula_count: usize,
}

pub(super) fn build_delta(
    previous: Option<&CalculationSnapshot>,
    current: &CalculationSnapshot,
    metadata: DeltaMetadata,
    max_delta_cells: usize,
    cancelled: &impl Fn() -> bool,
) -> Result<CalculationDelta, SessionError> {
    let mut changed_cells = Vec::new();
    for (cell, value) in current.materialized_cells() {
        if cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        if previous
            .and_then(|previous| previous.materialized_cell(cell))
            .is_none_or(|previous| previous != value)
        {
            changed_cells.push(CalculationDeltaCell::new(
                cell,
                value.origin(),
                value.result().clone(),
            ));
        }
    }
    let mut removed_materialized_cells = Vec::new();
    if let Some(previous) = previous {
        for (cell, _) in previous.materialized_cells() {
            if cancelled() {
                return Err(SessionError::new(SessionErrorCode::Cancelled, None));
            }
            if current.materialized_cell(cell).is_none() {
                removed_materialized_cells.push(cell);
            }
        }
    }
    let delta_cells = changed_cells
        .len()
        .saturating_add(removed_materialized_cells.len());
    if delta_cells > max_delta_cells {
        return Err(SessionError::new(
            SessionErrorCode::DeltaLimitExceeded,
            Some(format!("cells={delta_cells}, limit={max_delta_cells}")),
        ));
    }
    Ok(CalculationDelta {
        cursor: 0,
        base_revision: metadata.base_revision,
        result_revision: metadata.result_revision,
        mode: metadata.mode,
        reason: metadata.reason,
        dirty_count: metadata.dirty_count,
        evaluated_count: metadata.evaluated_count,
        parsed_formula_count: metadata.parsed_formula_count,
        changed_cells,
        removed_materialized_cells,
    })
}

pub(super) fn build_incremental_delta(
    previous: &CalculationSnapshot,
    current: &CalculationSnapshot,
    dirty: &BTreeSet<CalculationCellId>,
    metadata: DeltaMetadata,
    max_delta_cells: usize,
    cancelled: &impl Fn() -> bool,
) -> Result<CalculationDelta, SessionError> {
    let mut candidates = BTreeSet::new();
    for owner in dirty {
        if cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        candidates.extend(previous.materialized_cells_owned_by(*owner));
        candidates.extend(current.materialized_cells_owned_by(*owner));
    }
    let mut changed_cells = Vec::new();
    let mut removed_materialized_cells = Vec::new();
    for cell in candidates {
        if cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        match (
            previous.materialized_cell(cell),
            current.materialized_cell(cell),
        ) {
            (_, Some(current_value))
                if previous
                    .materialized_cell(cell)
                    .is_none_or(|previous_value| previous_value != current_value) =>
            {
                changed_cells.push(CalculationDeltaCell::new(
                    cell,
                    current_value.origin(),
                    current_value.result().clone(),
                ));
            }
            (Some(_), None) => removed_materialized_cells.push(cell),
            _ => {}
        }
    }
    let delta_cells = changed_cells
        .len()
        .saturating_add(removed_materialized_cells.len());
    if delta_cells > max_delta_cells {
        return Err(SessionError::new(
            SessionErrorCode::DeltaLimitExceeded,
            Some(format!("cells={delta_cells}, limit={max_delta_cells}")),
        ));
    }
    Ok(CalculationDelta {
        cursor: 0,
        base_revision: metadata.base_revision,
        result_revision: metadata.result_revision,
        mode: metadata.mode,
        reason: metadata.reason,
        dirty_count: metadata.dirty_count,
        evaluated_count: metadata.evaluated_count,
        parsed_formula_count: metadata.parsed_formula_count,
        changed_cells,
        removed_materialized_cells,
    })
}

pub(super) fn build_empty_delta(metadata: DeltaMetadata) -> CalculationDelta {
    CalculationDelta {
        cursor: 0,
        base_revision: metadata.base_revision,
        result_revision: metadata.result_revision,
        mode: metadata.mode,
        reason: metadata.reason,
        dirty_count: metadata.dirty_count,
        evaluated_count: metadata.evaluated_count,
        parsed_formula_count: metadata.parsed_formula_count,
        changed_cells: Vec::new(),
        removed_materialized_cells: Vec::new(),
    }
}
use std::collections::BTreeSet;
