use super::detail::{
    DetailBudget, build_affected_details, build_evaluated_details, build_install_details,
    build_issue_details, build_preview_result_details, ensure_evaluated_resource_limits,
    install_basis_reasons,
};
use super::report::next_report_identity;
use super::*;

#[derive(Debug, Clone, Copy)]
enum TransactionRunCheckpoint {
    BaseCalculation,
    CandidatePlanning,
    CandidateCalculation,
    PreviewDifference,
    InstallDifference,
    ReportConstruction,
}

impl PreparedWorkbookTransaction {
    /// Returns the immutable transaction base workbook.
    pub fn base_workbook(&self) -> &crate::WorkbookSnapshot {
        self.base_draft.workbook()
    }

    /// Returns the validated candidate workbook.
    pub fn candidate_workbook(&self) -> &crate::WorkbookSnapshot {
        self.candidate_draft.workbook()
    }

    /// Returns the exact edit receipt produced while preparing the candidate.
    pub const fn edit_receipt(&self) -> &EditReceipt {
        &self.edit_receipt
    }

    /// Calculates the base and candidate and constructs a complete bounded report off-lock.
    ///
    /// # Errors
    ///
    /// Returns a calculation, cancellation, delta, or transaction detail resource-limit error.
    pub fn run(self) -> Result<CompletedWorkbookTransaction, SessionError> {
        self.run_inner(|_| {})
    }

    /// Runs a transaction and returns exclusive phase durations for the manual release benchmark.
    ///
    /// This method exists only behind the release-test-only `__internal-transaction-bench` compile
    /// fence. The array order is base calculation, candidate impact/planning, candidate
    /// calculation, preview difference, install difference, and report construction. Each entry
    /// covers only the work since the preceding checkpoint, so the durations neither overlap nor
    /// accumulate.
    #[cfg(feature = "__internal-transaction-bench")]
    #[doc(hidden)]
    pub fn run_with_benchmark_phases(
        self,
    ) -> Result<(CompletedWorkbookTransaction, [std::time::Duration; 6]), SessionError> {
        let mut phases = [std::time::Duration::ZERO; 6];
        let mut previous = std::time::Instant::now();
        let completed = self.run_inner(|checkpoint| {
            let now = std::time::Instant::now();
            let index = match checkpoint {
                TransactionRunCheckpoint::BaseCalculation => 0,
                TransactionRunCheckpoint::CandidatePlanning => 1,
                TransactionRunCheckpoint::CandidateCalculation => 2,
                TransactionRunCheckpoint::PreviewDifference => 3,
                TransactionRunCheckpoint::InstallDifference => 4,
                TransactionRunCheckpoint::ReportConstruction => 5,
            };
            phases[index] = now.duration_since(previous);
            previous = now;
        })?;
        Ok((completed, phases))
    }

    fn run_inner(
        self,
        mut checkpoint: impl FnMut(TransactionRunCheckpoint),
    ) -> Result<CompletedWorkbookTransaction, SessionError> {
        if self.cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        let captured_calculation_current =
            self.captured_calculation
                .as_ref()
                .is_some_and(|calculation| {
                    calculation.source_revision() == self.base_revision
                        && calculation.source_fingerprint() == self.base_fingerprint
                        && calculation.provenance().input_hash()
                            == self.base_draft.workbook().provenance().input_hash()
                        && calculation.options() == self.options
                });
        let base_calculation_reused = captured_calculation_current
            && self.captured_compiled.is_some()
            && self.captured_options == Some(self.options);

        let (
            base_compiled,
            base_calculation,
            base_execution_mode,
            base_decision_reason,
            base_dirty_count,
            base_evaluated_count,
            base_parsed_formula_count,
            base_function_iterations,
            base_reference_cells,
        ) = if base_calculation_reused {
            let compiled = self
                .captured_compiled
                .as_ref()
                .map(Arc::clone)
                .ok_or_else(|| {
                    SessionError::new(SessionErrorCode::CalculationUninitialized, None)
                })?;
            let calculation = self
                .captured_calculation
                .as_ref()
                .map(Arc::clone)
                .ok_or_else(|| {
                    SessionError::new(SessionErrorCode::CalculationUninitialized, None)
                })?;
            (
                compiled,
                calculation,
                CalculationExecutionMode::Incremental,
                CalculationDecisionReason::NoDirtyFormulas,
                0,
                0,
                0,
                0,
                0,
            )
        } else {
            let base_session = WorkbookCalculationSession {
                draft: self
                    .base_draft
                    .clone_cancellable(&|| self.cancellation.is_cancelled())
                    .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?,
                compiled: self.captured_compiled.as_ref().map(Arc::clone),
                calculation: self.captured_calculation.as_ref().map(Arc::clone),
                calculation_options: self.captured_options,
                dirty: clone_set_cancellable(&self.captured_dirty, &|| {
                    self.cancellation.is_cancelled()
                })
                .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?,
                calculation_changes_pending: self.captured_changes_pending,
                requires_full_rebuild: self.captured_requires_full_rebuild,
                limits: self.limits,
                next_cursor: self.base_cursor,
                history: VecDeque::new(),
            };
            let prepared = base_session.prepare_recalculation(
                RecalculationMode::Auto,
                self.options,
                self.cancellation.clone(),
            )?;
            let execution_mode = prepared.execution_mode();
            let decision_reason = prepared.decision_reason();
            let dirty_count = prepared.dirty_cells().len();
            let execution = prepared.execute()?;
            ensure_evaluated_resource_limits(
                &execution.calculation,
                &execution.evaluated_cells,
                "base",
                &self.cancellation,
            )?;
            (
                execution.compiled,
                execution.calculation,
                execution_mode,
                decision_reason,
                dirty_count,
                execution.evaluated_count,
                execution.parsed_formula_count,
                execution.function_iterations,
                execution.reference_cells,
            )
        };
        checkpoint(TransactionRunCheckpoint::BaseCalculation);

        let topology_or_metadata_changed = self.edit_receipt.topology_changed()
            || self.edit_receipt.calculation_metadata_changed();
        let conservative_impact = topology_or_metadata_changed
            || (!base_compiled.incremental_safe()
                && !self.edit_receipt.calculation_changed_cells().is_empty());
        let (impact_coverage, direct, transitive, conservative) = if conservative_impact {
            let formulas = formula_cells_from_workbook(self.candidate_draft.workbook(), &|| {
                self.cancellation.is_cancelled()
            })
            .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
            (
                TransactionImpactCoverage::ConservativeFull,
                BTreeSet::new(),
                BTreeSet::new(),
                formulas,
            )
        } else {
            let impact = affected_formula_impact(
                self.candidate_draft.workbook(),
                &base_compiled,
                Some(&base_calculation),
                self.edit_receipt.calculation_changed_cells(),
                &|| self.cancellation.is_cancelled(),
            )
            .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
            (
                TransactionImpactCoverage::Exact,
                impact.direct,
                impact.transitive,
                BTreeSet::new(),
            )
        };
        let mut candidate_dirty = direct.clone();
        candidate_dirty.extend(transitive.iter().copied());
        let candidate_session = WorkbookCalculationSession {
            draft: self.candidate_draft,
            compiled: Some(Arc::clone(&base_compiled)),
            calculation: Some(Arc::clone(&base_calculation)),
            calculation_options: Some(self.options),
            dirty: candidate_dirty,
            calculation_changes_pending: !self.edit_receipt.calculation_changed_cells().is_empty(),
            requires_full_rebuild: topology_or_metadata_changed,
            limits: self.limits,
            next_cursor: self.base_cursor,
            history: VecDeque::new(),
        };
        let candidate_prepared = candidate_session.prepare_recalculation(
            self.requested_mode,
            self.options,
            self.cancellation.clone(),
        )?;
        let candidate_execution_mode = candidate_prepared.execution_mode();
        let candidate_decision_reason = candidate_prepared.decision_reason();
        let candidate_dirty_count = candidate_prepared.dirty_cells().len();
        let planned_total = base_evaluated_count.saturating_add(candidate_dirty_count);
        if planned_total > self.limits.max_evaluated_cells {
            return Err(SessionError::new(
                SessionErrorCode::EvaluationLimitExceeded,
                Some(format!(
                    "transaction_planned={planned_total}, limit={}",
                    self.limits.max_evaluated_cells
                )),
            ));
        }
        checkpoint(TransactionRunCheckpoint::CandidatePlanning);
        let candidate_execution = candidate_prepared.execute()?;
        ensure_evaluated_resource_limits(
            &candidate_execution.calculation,
            &candidate_execution.evaluated_cells,
            "candidate",
            &self.cancellation,
        )?;
        let evaluated_total =
            base_evaluated_count.saturating_add(candidate_execution.evaluated_count);
        if evaluated_total > self.limits.max_evaluated_cells {
            return Err(SessionError::new(
                SessionErrorCode::EvaluationLimitExceeded,
                Some(format!(
                    "transaction_evaluated={evaluated_total}, limit={}",
                    self.limits.max_evaluated_cells
                )),
            ));
        }
        let parsed_formula_count =
            base_parsed_formula_count.saturating_add(candidate_execution.parsed_formula_count);
        if parsed_formula_count > self.limits.max_evaluated_cells {
            return Err(SessionError::new(
                SessionErrorCode::EvaluationLimitExceeded,
                Some(format!(
                    "transaction_parsed={parsed_formula_count}, limit={}",
                    self.limits.max_evaluated_cells
                )),
            ));
        }
        let function_iterations =
            base_function_iterations.saturating_add(candidate_execution.function_iterations);
        let function_limit = self.options.limits().max_function_iterations();
        if function_iterations > function_limit {
            return Err(SessionError::new(
                SessionErrorCode::TransactionResourceLimitExceeded,
                Some(format!(
                    "function_iterations={function_iterations}, limit={function_limit}"
                )),
            ));
        }
        let reference_cells =
            base_reference_cells.saturating_add(candidate_execution.reference_cells);
        let reference_limit = self.options.limits().max_array_cells();
        if reference_cells > reference_limit {
            return Err(SessionError::new(
                SessionErrorCode::TransactionResourceLimitExceeded,
                Some(format!(
                    "reference_cells={reference_cells}, limit={reference_limit}"
                )),
            ));
        }
        checkpoint(TransactionRunCheckpoint::CandidateCalculation);
        let preview_delta = build_delta(
            Some(&base_calculation),
            &candidate_execution.calculation,
            DeltaMetadata {
                base_revision: self.base_revision,
                result_revision: self.edit_receipt.result_revision(),
                mode: candidate_execution_mode,
                reason: candidate_decision_reason,
                dirty_count: candidate_dirty_count,
                evaluated_count: candidate_execution.evaluated_count,
                parsed_formula_count: candidate_execution.parsed_formula_count,
            },
            self.limits.max_delta_cells,
            &|| self.cancellation.is_cancelled(),
        )?;
        checkpoint(TransactionRunCheckpoint::PreviewDifference);
        let install_base_revision = self
            .captured_calculation
            .as_ref()
            .map_or(self.base_revision, |calculation| {
                calculation.source_revision()
            });
        let mut install_delta = build_delta(
            self.captured_calculation.as_deref(),
            &candidate_execution.calculation,
            DeltaMetadata {
                base_revision: install_base_revision,
                result_revision: self.edit_receipt.result_revision(),
                mode: candidate_execution_mode,
                reason: candidate_decision_reason,
                dirty_count: base_dirty_count.saturating_add(candidate_dirty_count),
                evaluated_count: evaluated_total,
                parsed_formula_count,
            },
            self.limits.max_delta_cells,
            &|| self.cancellation.is_cancelled(),
        )?;
        install_delta.cursor = self.base_cursor;
        let preview_delta_cells = preview_delta
            .changed_cells()
            .len()
            .saturating_add(preview_delta.removed_materialized_cells().len());
        let install_delta_cells = install_delta
            .changed_cells()
            .len()
            .saturating_add(install_delta.removed_materialized_cells().len());
        let transaction_delta_cells = preview_delta_cells.saturating_add(install_delta_cells);
        if transaction_delta_cells > self.limits.max_delta_cells {
            return Err(SessionError::new(
                SessionErrorCode::DeltaLimitExceeded,
                Some(format!(
                    "transaction_cells={transaction_delta_cells}, limit={}",
                    self.limits.max_delta_cells
                )),
            ));
        }

        checkpoint(TransactionRunCheckpoint::InstallDifference);
        let mut detail_budget = DetailBudget::new(self.limits.max_transaction_detail_items);
        let preview_results = build_preview_result_details(
            &base_calculation,
            &candidate_execution.calculation,
            &preview_delta,
            &self.cancellation,
            &mut detail_budget,
        )?;
        let preview_issues =
            build_issue_details(&preview_results, &self.cancellation, &mut detail_budget)?;
        let direct_affected_count = direct.len();
        let transitive_affected_count = transitive.len();
        let conservative_affected_count = conservative.len();
        let affected = build_affected_details(
            direct,
            transitive,
            conservative,
            &self.cancellation,
            &mut detail_budget,
        )?;
        let evaluated = build_evaluated_details(
            &candidate_execution.evaluated_cells,
            &self.cancellation,
            &mut detail_budget,
        )?;
        let install_results =
            build_install_details(&install_delta, &self.cancellation, &mut detail_budget)?;
        if self.cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        let introduced_issue_count = preview_issues
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    TransactionDetailItem::PreviewIssue(TransactionIssueChange {
                        kind: TransactionIssueChangeKind::Introduced,
                        ..
                    })
                )
            })
            .count();
        let resolved_issue_count = preview_issues
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    TransactionDetailItem::PreviewIssue(TransactionIssueChange {
                        kind: TransactionIssueChangeKind::Resolved,
                        ..
                    })
                )
            })
            .count();
        let changed_issue_count = preview_issues
            .len()
            .saturating_sub(introduced_issue_count)
            .saturating_sub(resolved_issue_count);
        let install_basis_reasons = install_basis_reasons(
            self.captured_calculation.as_deref(),
            self.captured_options,
            self.base_revision,
            self.base_fingerprint,
            self.options,
        );
        let result_fingerprint = candidate_execution.calculation.source_fingerprint();
        if candidate_execution.calculation.source_revision() != self.edit_receipt.result_revision()
            || result_fingerprint
                != fingerprint_cancellable(candidate_session.workbook(), &self.cancellation)?
        {
            return Err(SessionError::new(
                SessionErrorCode::StateRevisionMismatch,
                None,
            ));
        }
        let report_install_delta = install_delta
            .clone_cancellable(&|| self.cancellation.is_cancelled())
            .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
        let identity = next_report_identity()?;
        let report = WorkbookTransactionReport {
            identity,
            cursor_hash_builder: std::collections::hash_map::RandomState::new(),
            base_revision: self.base_revision,
            result_revision: self.edit_receipt.result_revision(),
            base_fingerprint: self.base_fingerprint,
            result_fingerprint,
            input_hash: self.base_draft.workbook().provenance().input_hash(),
            calculator_provider: candidate_execution
                .calculation
                .provenance()
                .provider()
                .clone(),
            options: self.options,
            base_calculation_reused,
            base_execution_mode,
            base_decision_reason,
            candidate_requested_mode: self.requested_mode,
            candidate_execution_mode,
            candidate_decision_reason,
            edit_receipt: self
                .edit_receipt
                .clone_cancellable(&|| self.cancellation.is_cancelled())
                .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?,
            impact_coverage,
            direct_affected_count,
            transitive_affected_count,
            conservative_affected_count,
            base_evaluated_count,
            candidate_evaluated_count: candidate_execution.evaluated_count,
            parsed_formula_count,
            function_iteration_count: function_iterations,
            reference_cell_count: reference_cells,
            preview_changed_count: preview_delta.changed_cells().len(),
            preview_removed_count: preview_delta.removed_materialized_cells().len(),
            introduced_issue_count,
            resolved_issue_count,
            changed_issue_count,
            install_delta: report_install_delta,
            installed_calculation_revision: self
                .captured_calculation
                .as_ref()
                .map(|calculation| calculation.source_revision()),
            installed_calculation_fingerprint: self
                .captured_calculation
                .as_ref()
                .map(|calculation| calculation.source_fingerprint()),
            installed_calculation_options: self.captured_options,
            install_basis_reasons,
            max_page_items: self.limits.max_transaction_page_items,
            affected_detail_count: affected.len(),
            evaluated_detail_count: evaluated.len(),
            preview_result_detail_count: preview_results.len(),
            preview_issue_detail_count: preview_issues.len(),
            install_result_detail_count: install_results.len(),
            affected,
            evaluated,
            preview_results,
            preview_issues,
            install_results,
        };
        let payload = CompletedTransactionPayload {
            candidate_draft: candidate_session.draft,
            candidate_compiled: candidate_execution.compiled,
            candidate_calculation: candidate_execution.calculation,
            edit_receipt: self.edit_receipt,
            install_delta,
            captured_calculation: self.captured_calculation,
            captured_compiled: self.captured_compiled,
            captured_options: self.captured_options,
            base_revision: self.base_revision,
            base_cursor: self.base_cursor,
            base_fingerprint: self.base_fingerprint,
            result_fingerprint,
        };
        checkpoint(TransactionRunCheckpoint::ReportConstruction);
        Ok(CompletedWorkbookTransaction {
            report,
            payload: Some(payload),
            state: CompletedTransactionState::Completed,
        })
    }
}
