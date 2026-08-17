use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use super::delta::{DeltaMetadata, build_delta};
use super::impact::{affected_formula_impact, formula_cells_from_workbook};
use super::{
    ApplyChangesError, CalculationDecisionReason, CalculationDelta, CalculationExecutionMode,
    CancellationToken, PreparedEditBatch, RecalculationMode, SessionError, SessionErrorCode,
    SessionLimits, WorkbookCalculationSession,
};
use crate::calculation::eval::{CompiledWorkbook, clone_set_cancellable};
use crate::{
    CalculationCellId, CalculationCellResult, CalculationIssue, CalculationIssueCode,
    CalculationOptions, CalculationSnapshot, EditBatch, EditReceipt, WorkbookDraft,
    WorkbookFingerprint, WorkbookSnapshot,
};

mod cursor;
mod detail;
mod report;
mod run;

pub use cursor::{TransactionImpactPage, TransactionPageCursor};
pub use report::{
    InstallDeltaBasisReason, TransactionAffectedFormula, TransactionDetailItem,
    TransactionDetailSection, TransactionImpactCause, TransactionImpactCoverage,
    TransactionInstallResultChange, TransactionIssueChange, TransactionIssueChangeKind,
    TransactionResultChange, WorkbookTransactionReceipt, WorkbookTransactionReport,
};

/// An immutable off-lock job containing one captured base and validated edit candidate.
#[derive(Debug)]
pub struct PreparedWorkbookTransaction {
    base_draft: WorkbookDraft,
    candidate_draft: WorkbookDraft,
    edit_receipt: EditReceipt,
    base_revision: u64,
    base_cursor: u64,
    base_fingerprint: WorkbookFingerprint,
    requested_mode: RecalculationMode,
    options: CalculationOptions,
    cancellation: CancellationToken,
    limits: SessionLimits,
    captured_compiled: Option<Arc<CompiledWorkbook>>,
    captured_calculation: Option<Arc<CalculationSnapshot>>,
    captured_options: Option<CalculationOptions>,
    captured_dirty: BTreeSet<CalculationCellId>,
    captured_changes_pending: bool,
    captured_requires_full_rebuild: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletedTransactionState {
    Completed,
    Installed,
    Discarded,
    Stale,
}

#[derive(Debug)]
struct CompletedTransactionPayload {
    candidate_draft: WorkbookDraft,
    candidate_compiled: Arc<CompiledWorkbook>,
    candidate_calculation: Arc<CalculationSnapshot>,
    edit_receipt: EditReceipt,
    install_delta: CalculationDelta,
    captured_calculation: Option<Arc<CalculationSnapshot>>,
    captured_compiled: Option<Arc<CompiledWorkbook>>,
    captured_options: Option<CalculationOptions>,
    base_revision: u64,
    base_cursor: u64,
    base_fingerprint: WorkbookFingerprint,
    result_fingerprint: WorkbookFingerprint,
}

/// A fully calculated transaction that can be inspected, installed once, or discarded.
#[derive(Debug)]
pub struct CompletedWorkbookTransaction {
    report: WorkbookTransactionReport,
    payload: Option<CompletedTransactionPayload>,
    state: CompletedTransactionState,
}

impl CompletedWorkbookTransaction {
    /// Returns the complete summary. Detail paging is available only before terminal consumption.
    pub const fn report(&self) -> &WorkbookTransactionReport {
        &self.report
    }

    /// Returns one bounded detail page from the retained report.
    ///
    /// # Errors
    ///
    /// Returns a stable cursor or page-limit error.
    pub fn page(
        &self,
        section: TransactionDetailSection,
        cursor: Option<&TransactionPageCursor>,
        limit: usize,
    ) -> Result<TransactionImpactPage, SessionError> {
        self.ensure_pageable()?;
        self.report.page(section, cursor, limit)
    }

    /// Returns one bounded detail page with cooperative cancellation.
    ///
    /// # Errors
    ///
    /// Returns a stable cursor, page-limit, or retryable cancellation error.
    pub fn page_cancellable(
        &self,
        section: TransactionDetailSection,
        cursor: Option<&TransactionPageCursor>,
        limit: usize,
        cancellation: &CancellationToken,
    ) -> Result<TransactionImpactPage, SessionError> {
        self.ensure_pageable()?;
        self.report
            .page_cancellable(section, cursor, limit, cancellation)
    }

    /// Returns one bounded detail page from an opaque interop cursor token.
    ///
    /// Pass `None` for the first page. A subsequent token must come from
    /// [`TransactionPageCursor::to_token`] on this transaction and section.
    ///
    /// # Errors
    ///
    /// Returns a stable lifecycle, cursor, or page-limit error.
    pub fn page_from_token(
        &self,
        section: TransactionDetailSection,
        cursor_token: Option<&str>,
        limit: usize,
    ) -> Result<TransactionImpactPage, SessionError> {
        self.page_from_token_cancellable(section, cursor_token, limit, &CancellationToken::new())
    }

    /// Returns one bounded detail page from an opaque token with cooperative cancellation.
    ///
    /// # Errors
    ///
    /// Returns a stable lifecycle, cursor, page-limit, or retryable cancellation error.
    pub fn page_from_token_cancellable(
        &self,
        section: TransactionDetailSection,
        cursor_token: Option<&str>,
        limit: usize,
        cancellation: &CancellationToken,
    ) -> Result<TransactionImpactPage, SessionError> {
        self.ensure_pageable()?;
        let cursor = cursor_token
            .map(|token| self.report.cursor_from_token(token))
            .transpose()?;
        self.report
            .page_cancellable(section, cursor.as_ref(), limit, cancellation)
    }

    /// Releases the candidate without changing its source session.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::TransactionConsumed`] after any terminal transition.
    pub fn discard(&mut self) -> Result<(), SessionError> {
        if self.state != CompletedTransactionState::Completed {
            return Err(SessionError::new(
                SessionErrorCode::TransactionConsumed,
                None,
            ));
        }
        self.payload = None;
        self.report.release_details();
        self.state = CompletedTransactionState::Discarded;
        Ok(())
    }

    fn ensure_pageable(&self) -> Result<(), SessionError> {
        if self.state == CompletedTransactionState::Completed {
            Ok(())
        } else {
            Err(SessionError::new(
                SessionErrorCode::TransactionConsumed,
                None,
            ))
        }
    }
}

impl WorkbookCalculationSession {
    /// Captures and validates one edit candidate without changing the live session.
    ///
    /// # Errors
    ///
    /// Returns the same validation, cancellation, revision, and edit resource errors as
    /// [`Self::prepare_changes_cancellable`].
    pub fn prepare_transaction(
        &self,
        expected_revision: u64,
        batch: EditBatch,
        mode: RecalculationMode,
        options: CalculationOptions,
        cancellation: CancellationToken,
    ) -> Result<PreparedWorkbookTransaction, ApplyChangesError> {
        let prepared = self.prepare_changes_cancellable(expected_revision, batch, &cancellation)?;
        let base_draft = self
            .draft
            .clone_cancellable(&|| cancellation.is_cancelled())
            .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
        let PreparedEditBatch {
            draft: candidate_draft,
            receipt: edit_receipt,
            ..
        } = prepared;
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None).into());
        }
        let captured_dirty = clone_set_cancellable(&self.dirty, &|| cancellation.is_cancelled())
            .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
        let base_fingerprint = fingerprint_cancellable(base_draft.workbook(), &cancellation)
            .map_err(ApplyChangesError::from)?;
        Ok(PreparedWorkbookTransaction {
            base_fingerprint,
            base_draft,
            candidate_draft,
            edit_receipt,
            base_revision: expected_revision,
            base_cursor: self.next_cursor,
            requested_mode: mode,
            options,
            cancellation,
            limits: self.limits,
            captured_compiled: self.compiled.as_ref().map(Arc::clone),
            captured_calculation: self.calculation.as_ref().map(Arc::clone),
            captured_options: self.calculation_options,
            captured_dirty,
            captured_changes_pending: self.calculation_changes_pending,
            captured_requires_full_rebuild: self.requires_full_rebuild,
        })
    }

    /// Atomically installs the exact candidate workbook, calculation, receipt, and history delta.
    ///
    /// # Errors
    ///
    /// Returns a retryable cancellation or resource error before mutation. Stale state consumes
    /// the candidate terminally, and an already terminal transaction returns a lifecycle error.
    pub fn install_transaction(
        &mut self,
        completed: &mut CompletedWorkbookTransaction,
    ) -> Result<WorkbookTransactionReceipt, SessionError> {
        self.install_transaction_cancellable(completed, &CancellationToken::new())
    }

    /// Atomically installs a completed transaction with cooperative pre-commit cancellation.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::install_transaction`]. Cancellation before the commit
    /// boundary preserves the completed candidate for an explicit retry.
    pub fn install_transaction_cancellable(
        &mut self,
        completed: &mut CompletedWorkbookTransaction,
        cancellation: &CancellationToken,
    ) -> Result<WorkbookTransactionReceipt, SessionError> {
        if completed.state != CompletedTransactionState::Completed {
            return Err(SessionError::new(
                SessionErrorCode::TransactionConsumed,
                None,
            ));
        }
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        let payload = completed
            .payload
            .as_ref()
            .ok_or_else(|| SessionError::new(SessionErrorCode::TransactionConsumed, None))?;
        let calculation_identity_matches = option_arc_ptr_eq(
            self.calculation.as_ref(),
            payload.captured_calculation.as_ref(),
        );
        let compiled_identity_matches =
            option_arc_ptr_eq(self.compiled.as_ref(), payload.captured_compiled.as_ref());
        let live_fingerprint = fingerprint_cancellable(self.workbook(), cancellation)?;
        let stale = self.workbook().semantic_revision() != payload.base_revision
            || self.next_cursor != payload.base_cursor
            || live_fingerprint != payload.base_fingerprint
            || !calculation_identity_matches
            || !compiled_identity_matches
            || self.calculation_options != payload.captured_options;
        if stale {
            completed.payload = None;
            completed.report.release_details();
            completed.state = CompletedTransactionState::Stale;
            return Err(SessionError::new(SessionErrorCode::StaleResult, None));
        }
        let next_cursor = payload
            .base_cursor
            .checked_add(1)
            .ok_or_else(|| SessionError::new(SessionErrorCode::DeltaLimitExceeded, None))?;
        let history_delta = payload
            .install_delta
            .clone_cancellable(&|| cancellation.is_cancelled())
            .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))?;
        if self.history.len() < self.limits.max_retained_deltas {
            self.history.try_reserve(1).map_err(|error| {
                SessionError::new(
                    SessionErrorCode::TransactionResourceLimitExceeded,
                    Some(error.to_string()),
                )
            })?;
        }
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }

        let payload = completed
            .payload
            .take()
            .ok_or_else(|| SessionError::new(SessionErrorCode::TransactionConsumed, None))?;
        let receipt = WorkbookTransactionReceipt {
            edit: payload.edit_receipt,
            calculation_delta: payload.install_delta,
            base_fingerprint: payload.base_fingerprint,
            result_fingerprint: payload.result_fingerprint,
        };
        self.draft = payload.candidate_draft;
        self.compiled = Some(payload.candidate_compiled);
        self.calculation = Some(payload.candidate_calculation);
        self.calculation_options = Some(completed.report.options);
        self.dirty.clear();
        self.calculation_changes_pending = false;
        self.requires_full_rebuild = false;
        self.next_cursor = next_cursor;
        if self.history.len() >= self.limits.max_retained_deltas {
            self.history.pop_front();
        }
        self.history.push_back(history_delta);
        completed.report.release_details();
        completed.state = CompletedTransactionState::Installed;
        Ok(receipt)
    }
}

fn fingerprint_cancellable(
    workbook: &WorkbookSnapshot,
    cancellation: &CancellationToken,
) -> Result<WorkbookFingerprint, SessionError> {
    workbook
        .semantic_fingerprint_cancellable(&|| cancellation.is_cancelled())
        .map(WorkbookFingerprint::current)
        .map_err(|()| SessionError::new(SessionErrorCode::Cancelled, None))
}

fn option_arc_ptr_eq<T>(left: Option<&Arc<T>>, right: Option<&Arc<T>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        (None, None) => true,
        _ => false,
    }
}
