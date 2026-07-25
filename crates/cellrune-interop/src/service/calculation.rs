//! Calculation request ownership, cooperative cancellation, and guarded installation.

use cellrune::{CancellationToken, CompletedCalculation, PreparedCalculation};

use super::WorkbookSession;
use crate::convert::{
    calculation_delta, calculation_delta_page, calculation_options, calculation_report,
    recalculation_mode,
};
use crate::{
    CalculationDeltaDto, CalculationDeltaPageDto, CalculationOptionsDto, CalculationReportDto,
    InteropError, RecalculationModeDto,
};

impl WorkbookSession {
    /// Calculates the current workbook revision and retains the complete result for reads and save.
    ///
    /// # Errors
    ///
    /// Returns a validation error when a deterministic numeric input is not finite.
    pub fn calculate(
        &mut self,
        options: CalculationOptionsDto,
    ) -> Result<CalculationReportDto, InteropError> {
        let prepared = self.prepare_recalculation(RecalculationModeDto::Auto, options)?;
        let request_id = prepared.request_id();
        let completed = match prepared.run() {
            Ok(completed) => completed,
            Err(error) => {
                self.abandon_recalculation(request_id);
                return Err(error);
            }
        };
        self.install_recalculation(completed)?;
        let calculation = self
            .current_calculation()
            .ok_or_else(InteropError::calculation_required)?;
        Ok(calculation_report(self.engine.workbook(), calculation))
    }

    /// Returns counts for the currently installed current-revision calculation.
    ///
    /// # Errors
    ///
    /// Returns a stable state error when the workbook has not been calculated after its latest
    /// edit.
    pub fn calculation_report(&self) -> Result<CalculationReportDto, InteropError> {
        let calculation = self
            .current_calculation()
            .ok_or_else(InteropError::calculation_required)?;
        Ok(calculation_report(self.engine.workbook(), calculation))
    }

    /// Prepares a stateful calculation job that can run without holding the session lock.
    ///
    /// A newer request cooperatively cancels an older active request. Edits may commit while the
    /// returned job runs; installation then rejects a stale revision.
    ///
    /// # Errors
    ///
    /// Returns a stable input or session-state error for invalid deterministic inputs or unsafe
    /// forced incremental calculation.
    pub fn prepare_recalculation(
        &mut self,
        mode: RecalculationModeDto,
        options: CalculationOptionsDto,
    ) -> Result<PreparedRecalculation, InteropError> {
        let options = calculation_options(options)?;
        let request_id = self.next_calculation_id;
        let next_calculation_id = request_id
            .checked_add(1)
            .ok_or_else(InteropError::session_request_id_exhausted)?;
        let token = CancellationToken::new();
        let prepared =
            self.engine
                .prepare_recalculation(recalculation_mode(mode), options, token.clone())?;
        if let Some((_, active_token)) = &self.active_calculation {
            active_token.cancel();
        }
        self.next_calculation_id = next_calculation_id;
        self.active_calculation = Some((request_id, token));
        Ok(PreparedRecalculation {
            request_id,
            prepared,
        })
    }

    /// Installs a completed calculation if it is still the active current-revision request.
    ///
    /// # Errors
    ///
    /// Returns a stable cancellation or stale-result error without replacing current results.
    pub fn install_recalculation(
        &mut self,
        completed: CompletedRecalculation,
    ) -> Result<CalculationDeltaDto, InteropError> {
        self.require_active_recalculation(completed.request_id)?;
        self.active_calculation = None;
        let delta = self.engine.install(completed.completed)?;
        Ok(calculation_delta(self.engine.workbook(), &delta))
    }

    /// Returns the exact delta that installing a completed request would commit.
    ///
    /// # Errors
    ///
    /// Returns a stable cancellation, stale-result, or cursor-limit error without changing the
    /// session.
    pub fn preview_recalculation(
        &self,
        completed: &CompletedRecalculation,
    ) -> Result<CalculationDeltaDto, InteropError> {
        self.require_active_recalculation(completed.request_id)?;
        let delta = self.engine.preview_install(&completed.completed)?;
        Ok(calculation_delta(self.engine.workbook(), &delta))
    }

    /// Calculates and installs a stateful full or incremental result delta.
    ///
    /// # Errors
    ///
    /// Returns a stable input, cancellation, stale, or unsafe-incremental error.
    pub fn recalculate(
        &mut self,
        mode: RecalculationModeDto,
        options: CalculationOptionsDto,
    ) -> Result<CalculationDeltaDto, InteropError> {
        let prepared = self.prepare_recalculation(mode, options)?;
        let request_id = prepared.request_id();
        let completed = match prepared.run() {
            Ok(completed) => completed,
            Err(error) => {
                self.abandon_recalculation(request_id);
                return Err(error);
            }
        };
        self.install_recalculation(completed)
    }

    /// Requests cooperative cancellation of the active calculation.
    pub fn cancel_calculation(&mut self) -> bool {
        let Some((_, token)) = &self.active_calculation else {
            return false;
        };
        token.cancel();
        true
    }

    /// Requests cooperative cancellation only when `request_id` is still the active calculation.
    pub fn cancel_recalculation(&mut self, request_id: u64) -> bool {
        let Some((active_request_id, token)) = &self.active_calculation else {
            return false;
        };
        if *active_request_id != request_id {
            return false;
        }
        token.cancel();
        true
    }

    /// Returns whether a prepared calculation is running or awaiting installation.
    pub const fn calculation_active(&self) -> bool {
        self.active_calculation.is_some()
    }

    /// Clears a failed prepared request without disturbing a newer active request.
    pub fn abandon_recalculation(&mut self, request_id: u64) {
        if self
            .active_calculation
            .as_ref()
            .is_some_and(|(active, _)| *active == request_id)
        {
            self.active_calculation = None;
        }
    }

    /// Returns a cursor page of installed calculation deltas.
    ///
    /// # Errors
    ///
    /// Returns a stable cursor or page-limit error.
    pub fn changes_since(
        &self,
        cursor: u64,
        limit: u32,
    ) -> Result<CalculationDeltaPageDto, InteropError> {
        let page = self.engine.changes_since(cursor, limit as usize)?;
        Ok(calculation_delta_page(self.engine.workbook(), &page))
    }

    fn require_active_recalculation(&self, request_id: u64) -> Result<(), InteropError> {
        let active = self
            .active_calculation
            .as_ref()
            .map(|(active_request_id, _)| *active_request_id);
        if active == Some(request_id) {
            Ok(())
        } else {
            Err(InteropError::calculation_cancelled())
        }
    }
}

/// An interop calculation job that does not borrow or lock its source session while running.
#[derive(Debug)]
pub struct PreparedRecalculation {
    request_id: u64,
    prepared: PreparedCalculation,
}

impl PreparedRecalculation {
    /// Returns the session-local request identifier used for guarded cleanup and installation.
    pub const fn request_id(&self) -> u64 {
        self.request_id
    }

    /// Executes the prepared core calculation.
    ///
    /// # Errors
    ///
    /// Returns a stable cancellation or resource-limit error.
    pub fn run(self) -> Result<CompletedRecalculation, InteropError> {
        Ok(CompletedRecalculation {
            request_id: self.request_id,
            completed: self.prepared.run()?,
        })
    }
}

/// A calculated interop result awaiting current-revision installation.
#[derive(Debug)]
pub struct CompletedRecalculation {
    request_id: u64,
    completed: CompletedCalculation,
}
