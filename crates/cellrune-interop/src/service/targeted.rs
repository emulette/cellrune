use super::WorkbookSession;
use crate::{
    CancellationToken, InteropError, TargetCalculationRequestDto, TargetCalculationResultDto,
};
use cellrune::{
    CalculationTarget, CellAddress, CellRange, CompletedTargetCalculation,
    PreparedTargetCalculation, TargetCalculationLimits,
};

impl WorkbookSession {
    /// Prepares a partial request and supersedes the previous active calculation.
    ///
    /// # Errors
    /// Returns a stable validation or resource error before superseding existing work.
    pub fn prepare_target_calculation(
        &mut self,
        request: &TargetCalculationRequestDto,
        cancellation: CancellationToken,
    ) -> Result<PreparedTargetRequest, InteropError> {
        let limits = TargetCalculationLimits::new(
            request.limits.max_targets as usize,
            request.limits.max_result_cells as usize,
            request.limits.max_evaluated_cells as usize,
        )?;
        if request.targets.len() > limits.max_targets() {
            return Err(InteropError::target_limit());
        }
        let targets = request
            .targets
            .iter()
            .map(|target| {
                let sheet = self
                    .engine
                    .workbook()
                    .sheet_by_name(&target.sheet)
                    .ok_or_else(InteropError::sheet_not_found)?;
                let start = CellAddress::from_a1(&target.start)?;
                let end = target
                    .end
                    .as_deref()
                    .map(CellAddress::from_a1)
                    .transpose()?
                    .unwrap_or(start);
                Ok(CalculationTarget::new(
                    sheet.id(),
                    CellRange::new(start, end)?,
                ))
            })
            .collect::<Result<Vec<_>, InteropError>>()?;
        let options = crate::convert::calculation_options(request.options)?;
        let request_id = self.next_calculation_id;
        let next = request_id
            .checked_add(1)
            .ok_or_else(InteropError::session_request_id_exhausted)?;
        let prepared = self.engine.prepare_target_calculation(
            &targets,
            options,
            limits,
            cancellation.clone(),
        )?;
        if let Some((_, active)) = &self.active_calculation {
            active.cancel();
        }
        self.next_calculation_id = next;
        self.active_calculation = Some((request_id, cancellation));
        Ok(PreparedTargetRequest {
            request_id,
            prepared,
            limits: request.limits,
        })
    }

    /// Returns a current partial response without installing a cache or changing preview/delta state.
    ///
    /// # Errors
    /// Rejects cancelled, superseded, or stale requests.
    pub fn finish_target_calculation(
        &mut self,
        completed: CompletedTargetRequest,
    ) -> Result<TargetCalculationResultDto, InteropError> {
        if let Err(error) = self.require_active_recalculation(completed.request_id) {
            self.abandon_recalculation(completed.request_id);
            return Err(error);
        }
        self.active_calculation = None;
        self.engine.finish_target_calculation(completed.completed)?;
        Ok(completed.response)
    }

    /// Calculates requested cells without requiring or installing a complete calculation.
    ///
    /// # Errors
    /// Returns a stable input, resource, or session error.
    pub fn calculate_targets(
        &mut self,
        request: &TargetCalculationRequestDto,
    ) -> Result<TargetCalculationResultDto, InteropError> {
        let prepared = self.prepare_target_calculation(request, CancellationToken::new())?;
        let request_id = prepared.request_id();
        match prepared.run() {
            Ok(completed) => self.finish_target_calculation(completed),
            Err(error) => {
                self.abandon_recalculation(request_id);
                Err(error)
            }
        }
    }
}

/// An owned partial job executable outside the session lock.
#[derive(Debug)]
pub struct PreparedTargetRequest {
    request_id: u64,
    prepared: PreparedTargetCalculation,
    limits: crate::TargetCalculationLimitsDto,
}

impl PreparedTargetRequest {
    /// Returns the session-local request identity used for cancellation and cleanup.
    pub const fn request_id(&self) -> u64 {
        self.request_id
    }
    /// Calculates and converts the response outside the session lock.
    ///
    /// # Errors
    /// Returns a stable resource or cancellation error.
    pub fn run(self) -> Result<CompletedTargetRequest, InteropError> {
        let completed = self.prepared.run()?;
        let result = completed.result();
        let provider = result.provenance().provider();
        let response = TargetCalculationResultDto {
            schema_version: crate::INTEROP_SCHEMA_VERSION,
            scope: crate::TargetCalculationScopeDto::Targets,
            semantic_revision: result.source_revision(),
            source_fingerprint: crate::convert::workbook_fingerprint(result.source_fingerprint()),
            input_sha256: result
                .provenance()
                .input_hash()
                .map(|hash| crate::convert::hex_bytes(hash.as_bytes())),
            calculator_provider: crate::ProviderIdentityDto {
                name: provider.name().to_owned(),
                version: provider.version().to_owned(),
            },
            calculation_options: crate::convert::calculation_options_report(result.options()),
            limits: self.limits,
            cells: result
                .cells()
                .map(|(id, result)| crate::TargetCalculationCellDto {
                    cell: crate::convert::cell_reference(completed.workbook(), id),
                    result: crate::convert::result_to_dto(result),
                })
                .collect(),
            evaluated_count: result.evaluated_count() as u64,
            parsed_formula_count: result.parsed_formula_count() as u64,
            reused_count: result.reused_count() as u64,
        };
        Ok(CompletedTargetRequest {
            request_id: self.request_id,
            completed,
            response,
        })
    }
}

/// A converted partial result awaiting cancellation and source-state validation.
#[derive(Debug)]
pub struct CompletedTargetRequest {
    request_id: u64,
    completed: CompletedTargetCalculation,
    response: TargetCalculationResultDto,
}

impl CompletedTargetRequest {
    /// Returns the complete response for byte-budget validation before publication.
    pub const fn response(&self) -> &TargetCalculationResultDto {
        &self.response
    }
}
