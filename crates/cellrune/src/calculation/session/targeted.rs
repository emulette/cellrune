use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::WorkbookCalculationSession;
use crate::calculation::targeted::{normalize_targets, source_provenance};
use crate::{
    CalculationCellId, CalculationCellResult, CalculationOptions, CalculationSnapshot,
    CalculationTarget, CancellationToken, CellContent, CellValue, TargetCalculationError,
    TargetCalculationErrorCode, TargetCalculationLimits, TargetCalculationResult, WorkbookSnapshot,
};

impl WorkbookCalculationSession {
    /// Captures a bounded partial request for execution without holding the session lock.
    ///
    /// # Errors
    /// Rejects invalid targets, excessive result counts, or cancellation.
    pub fn prepare_target_calculation(
        &self,
        targets: &[CalculationTarget],
        options: CalculationOptions,
        limits: TargetCalculationLimits,
        cancellation: CancellationToken,
    ) -> Result<PreparedTargetCalculation, TargetCalculationError> {
        let cells = normalize_targets(self.workbook(), targets, limits, &cancellation)?;
        let previous = self
            .calculation
            .as_ref()
            .filter(|calculation| {
                calculation.source_revision() == self.workbook().semantic_revision()
                    && self.calculation_options == Some(options)
            })
            .cloned();
        Ok(PreparedTargetCalculation {
            workbook: self.draft.shared_workbook(),
            cells,
            options,
            limits,
            cancellation,
            previous,
            base_cursor: self.next_cursor,
        })
    }

    /// Returns a completed partial request after checking that its source is still current.
    /// This never installs calculation state, advances a delta cursor, or clears dirty formulas.
    ///
    /// # Errors
    /// Rejects cancelled requests and results from an older session state.
    pub fn finish_target_calculation(
        &self,
        completed: CompletedTargetCalculation,
    ) -> Result<TargetCalculationResult, TargetCalculationError> {
        if completed.cancellation.is_cancelled() {
            return Err(TargetCalculationError::new(
                TargetCalculationErrorCode::Cancelled,
            ));
        }
        if self.next_cursor != completed.base_cursor
            || !Arc::ptr_eq(&self.draft.shared_workbook(), &completed.workbook)
        {
            return Err(TargetCalculationError::new(
                TargetCalculationErrorCode::StaleResult,
            ));
        }
        Ok(completed.result)
    }

    /// Calculates requested cells using a current complete cache when compatible.
    ///
    /// # Errors
    /// Returns a stable target, resource, cancellation, or stale-result error.
    pub fn calculate_targets(
        &self,
        targets: &[CalculationTarget],
        options: CalculationOptions,
        limits: TargetCalculationLimits,
        cancellation: CancellationToken,
    ) -> Result<TargetCalculationResult, TargetCalculationError> {
        self.finish_target_calculation(
            self.prepare_target_calculation(targets, options, limits, cancellation)?
                .run()?,
        )
    }
}

/// An immutable partial calculation job that owns its source snapshot.
#[derive(Debug)]
pub struct PreparedTargetCalculation {
    workbook: Arc<WorkbookSnapshot>,
    cells: BTreeSet<CalculationCellId>,
    options: CalculationOptions,
    limits: TargetCalculationLimits,
    cancellation: CancellationToken,
    previous: Option<Arc<CalculationSnapshot>>,
    base_cursor: u64,
}

impl PreparedTargetCalculation {
    /// Executes the captured request without changing the originating session.
    ///
    /// # Errors
    /// Returns a resource or cancellation error without publishing partial output.
    pub fn run(self) -> Result<CompletedTargetCalculation, TargetCalculationError> {
        let result = if let Some(previous) = self.previous.as_ref()
            && previous.matches_workbook(&self.workbook)
        {
            self.cached(previous)?
        } else {
            crate::calculation::eval::Engine::calculate_targets(
                &self.workbook,
                &self.cells,
                self.options,
                self.limits,
                &self.cancellation,
            )?
        };
        Ok(CompletedTargetCalculation {
            workbook: self.workbook,
            base_cursor: self.base_cursor,
            cancellation: self.cancellation,
            result,
        })
    }

    fn cached(
        &self,
        previous: &CalculationSnapshot,
    ) -> Result<TargetCalculationResult, TargetCalculationError> {
        let mut cells = BTreeMap::new();
        let mut reused_count = 0;
        for id in &self.cells {
            if self.cancellation.is_cancelled() {
                return Err(TargetCalculationError::new(
                    TargetCalculationErrorCode::Cancelled,
                ));
            }
            let result = if let Some(materialized) = previous.materialized_cell(*id) {
                reused_count += 1;
                materialized.result().clone()
            } else {
                let value = self
                    .workbook
                    .sheet_by_id(id.sheet_id())
                    .and_then(|sheet| sheet.cell(id.address()))
                    .and_then(|cell| match cell.content() {
                        CellContent::Literal(value) => Some(value.clone()),
                        CellContent::Formula(_) => None,
                    })
                    .unwrap_or(CellValue::Blank);
                CalculationCellResult::Value(value)
            };
            cells.insert(*id, result);
        }
        Ok(TargetCalculationResult {
            cells,
            source_revision: self.workbook.semantic_revision(),
            source_fingerprint: previous.source_fingerprint(),
            provenance: source_provenance(&self.workbook),
            options: self.options,
            limits: self.limits,
            evaluated_count: 0,
            parsed_formula_count: 0,
            reused_count,
        })
    }
}

/// A partial result awaiting source-state validation, never a complete calculation snapshot.
#[derive(Debug)]
pub struct CompletedTargetCalculation {
    workbook: Arc<WorkbookSnapshot>,
    base_cursor: u64,
    cancellation: CancellationToken,
    result: TargetCalculationResult,
}

impl CompletedTargetCalculation {
    /// Returns the captured source, for result conversion outside the session lock.
    pub fn workbook(&self) -> &WorkbookSnapshot {
        &self.workbook
    }
    /// Returns the immutable partial result for response-budget checks before publication.
    pub const fn result(&self) -> &TargetCalculationResult {
        &self.result
    }
}
