//! Retained transaction-preview ownership and transport-safe paging.

use cellrune::{
    CancellationToken, CompletedWorkbookTransaction, PreparedWorkbookTransaction, SessionErrorCode,
    WorkbookSnapshot,
};

use super::WorkbookSession;
use super::edit::convert_edit_batch_v2;
use crate::convert::{
    calculation_options, preview_transaction_receipt, recalculation_mode,
    transaction_detail_section, transaction_page, transaction_receipt, transaction_report,
};
use crate::{
    EditBatchV2Dto, InteropError, PreviewChangesDto, PreviewCursorDto, RecalculationModeDto,
    TransactionDetailSectionDto, TransactionImpactPageDto, WorkbookTransactionReceiptDto,
};

/// Default item count selected by an interop preview-detail request.
pub const DEFAULT_PREVIEW_PAGE_SIZE: u32 = 100;
/// Hard interop preview-detail item limit. Core applies its own lower limit as well.
pub const MAX_PREVIEW_PAGE_SIZE: u32 = 1_000;

/// A completed preview retained by one interop workbook session.
#[derive(Debug)]
pub(super) struct PublishedPreview {
    id: u64,
    completed: CompletedWorkbookTransaction,
    base: WorkbookSnapshot,
    candidate: WorkbookSnapshot,
}

impl PublishedPreview {
    pub(super) fn discard(&mut self) {
        let _ = self.completed.discard();
    }
}

impl WorkbookSession {
    /// Starts an immutable edit-and-calculation preview without publishing it.
    ///
    /// A newer preview request cooperatively cancels an older active calculation, but leaves a
    /// previously published preview intact until [`Self::publish_preview`] succeeds.
    ///
    /// # Errors
    ///
    /// Returns typed edit validation, revision, calculation-option, or resource errors without
    /// disturbing a previously published preview.
    pub fn prepare_preview_changes(
        &mut self,
        expected_revision: u64,
        batch: EditBatchV2Dto,
        mode: RecalculationModeDto,
        options: crate::CalculationOptionsDto,
    ) -> Result<PreparedPreview, InteropError> {
        self.prepare_preview_changes_cancellable(
            expected_revision,
            batch,
            mode,
            options,
            &CancellationToken::new(),
        )
    }

    /// Starts an immutable preview while observing cancellation during edit conversion and core
    /// candidate capture.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::prepare_preview_changes`], plus `session.cancelled`
    /// without disturbing a previously published preview.
    pub fn prepare_preview_changes_cancellable(
        &mut self,
        expected_revision: u64,
        batch: EditBatchV2Dto,
        mode: RecalculationModeDto,
        options: crate::CalculationOptionsDto,
        cancellation: &CancellationToken,
    ) -> Result<PreparedPreview, InteropError> {
        let options = calculation_options(options)?;
        let batch = convert_edit_batch_v2(self.engine.workbook(), batch, &|| {
            cancellation.is_cancelled()
        })?;
        let token = cancellation.clone();
        let prepared = self.engine.prepare_transaction(
            expected_revision,
            batch,
            recalculation_mode(mode),
            options,
            token.clone(),
        )?;
        let preview_id = self.next_preview_id;
        self.next_preview_id = preview_id
            .checked_add(1)
            .ok_or_else(InteropError::preview_id_exhausted)?;
        let request_id = self.next_calculation_id;
        self.next_calculation_id = request_id
            .checked_add(1)
            .ok_or_else(InteropError::session_request_id_exhausted)?;
        if let Some((_, active_token)) = &self.active_calculation {
            active_token.cancel();
        }
        self.active_calculation = Some((request_id, token));
        Ok(PreparedPreview {
            request_id,
            preview_id,
            base: prepared.base_workbook().clone(),
            candidate: prepared.candidate_workbook().clone(),
            prepared,
        })
    }

    /// Calculates, serializes, and publishes a preview in one synchronous interop call.
    ///
    /// Bindings that need an outer response-size check should instead use the two-phase
    /// [`Self::prepare_preview_changes`] and [`Self::publish_preview`] operations.
    ///
    /// # Errors
    ///
    /// Returns a typed edit, calculation, cancellation, or resource error while preserving a
    /// prior published preview.
    pub fn preview_changes(
        &mut self,
        expected_revision: u64,
        batch: EditBatchV2Dto,
        mode: RecalculationModeDto,
        options: crate::CalculationOptionsDto,
    ) -> Result<PreviewChangesDto, InteropError> {
        let prepared = self.prepare_preview_changes(expected_revision, batch, mode, options)?;
        let request_id = prepared.request_id();
        let completed = match prepared.run() {
            Ok(completed) => completed,
            Err(error) => {
                self.abandon_recalculation(request_id);
                return Err(error);
            }
        };
        self.publish_preview(completed)
    }

    /// Publishes a fully calculated preview after the caller has accepted its summary DTO.
    ///
    /// Successful publication atomically replaces the prior retained preview. A cancelled or
    /// stale active request cannot replace the prior preview.
    ///
    /// # Errors
    ///
    /// Returns a stable cancellation error when another operation superseded this preview.
    pub fn publish_preview(
        &mut self,
        mut completed: CompletedPreview,
    ) -> Result<PreviewChangesDto, InteropError> {
        if let Err(error) = self.require_active_recalculation(completed.request_id) {
            self.abandon_recalculation(completed.request_id);
            return Err(error);
        }
        if let Err(error) = self.engine.validate_transaction(&mut completed.completed) {
            self.abandon_recalculation(completed.request_id);
            return Err(error.into());
        }
        self.active_calculation = None;
        let CompletedPreview {
            request_id: _,
            preview_id,
            base,
            candidate,
            completed,
            summary,
        } = completed;
        let replacement = PublishedPreview {
            id: preview_id,
            completed,
            base,
            candidate,
        };
        if let Some(mut previous) = self.published_preview.take() {
            previous.completed.discard()?;
        }
        self.published_preview = Some(replacement);
        Ok(summary)
    }

    /// Returns one count-bounded transaction detail page from a published preview.
    ///
    /// # Errors
    ///
    /// Returns stable preview, cursor, lifecycle, or page-limit errors.
    pub fn preview_changes_page(
        &self,
        preview_id: u64,
        section: TransactionDetailSectionDto,
        cursor: Option<PreviewCursorDto>,
        limit: u32,
    ) -> Result<TransactionImpactPageDto, InteropError> {
        let published = self.published(preview_id)?;
        let cursor_token = preview_cursor_token(preview_id, cursor)?;
        let core_section = transaction_detail_section(section);
        let limit = preview_page_limit(limit)?;
        let page = published.completed.page_from_token(
            core_section,
            cursor_token.as_deref(),
            usize::try_from(limit).unwrap_or(usize::MAX),
        )?;
        Ok(transaction_page(
            preview_id,
            section,
            published.completed.report().detail_count(core_section),
            page.items().iter().cloned(),
            page.next_cursor().map(|cursor| cursor.to_token()),
            &published.base,
            &published.candidate,
        ))
    }

    /// Returns the longest complete transaction-detail prefix that fits `max_response_bytes`.
    ///
    /// The core owns cursor progression, validation, and the count ceiling. If the requested page
    /// is too large on the wire, this method uses bounded binary subdivision to retain the exact
    /// cursor for the longest prefix instead of repeatedly rebuilding every shorter prefix.
    ///
    /// # Errors
    ///
    /// Returns an explicit response-limit error when even an empty page or one detail item cannot
    /// be serialized within the supplied byte limit.
    pub fn preview_changes_page_bounded(
        &self,
        preview_id: u64,
        section: TransactionDetailSectionDto,
        cursor: Option<PreviewCursorDto>,
        limit: u32,
        max_response_bytes: usize,
    ) -> Result<TransactionImpactPageDto, InteropError> {
        let published = self.published(preview_id)?;
        let cursor_token = preview_cursor_token(preview_id, cursor)?;
        let core_section = transaction_detail_section(section);
        let requested = preview_page_limit(limit)?;
        let total_count = published.completed.report().detail_count(core_section);

        let page_for = |item_limit: usize| -> Result<TransactionImpactPageDto, InteropError> {
            let page = published.completed.page_from_token(
                core_section,
                cursor_token.as_deref(),
                item_limit,
            )?;
            Ok(transaction_page(
                preview_id,
                section,
                total_count,
                page.items().iter().cloned(),
                page.next_cursor().map(|value| value.to_token()),
                &published.base,
                &published.candidate,
            ))
        };

        let full = page_for(usize::try_from(requested).unwrap_or(usize::MAX))?;
        if serialized_bytes(&full)? <= max_response_bytes {
            return Ok(full);
        }

        let mut lower = 0_usize;
        let mut upper = full.items.len().saturating_sub(1);
        let mut accepted = None;
        while lower < upper {
            let candidate_count = lower + (upper - lower).div_ceil(2);
            let candidate = page_for(candidate_count)?;
            if serialized_bytes(&candidate)? <= max_response_bytes {
                lower = candidate_count;
                accepted = Some(candidate);
            } else {
                upper = candidate_count - 1;
            }
        }

        accepted.ok_or_else(InteropError::preview_response_limit)
    }

    /// Installs a published preview once and returns the exact committed receipt.
    ///
    /// Retryable pre-commit errors keep the preview published. A stale result is terminal and
    /// consumes the preview ID.
    ///
    /// # Errors
    ///
    /// Returns stable preview lifecycle, stale, cancellation, or resource errors.
    pub fn commit_preview(
        &mut self,
        preview_id: u64,
    ) -> Result<WorkbookTransactionReceiptDto, InteropError> {
        self.commit_preview_cancellable(preview_id, &CancellationToken::new())
    }

    /// Returns the exact commit receipt without consuming the published preview.
    ///
    /// This permits a transport to enforce its response limit before crossing the core commit
    /// boundary. A later concurrent mutation can still make the subsequent commit stale.
    ///
    /// # Errors
    ///
    /// Returns a stable preview-not-found error for an unknown or consumed preview.
    pub fn preview_commit_receipt(
        &self,
        preview_id: u64,
    ) -> Result<WorkbookTransactionReceiptDto, InteropError> {
        let published = self.published(preview_id)?;
        Ok(preview_transaction_receipt(
            &published.base,
            &published.candidate,
            published.completed.report(),
        ))
    }

    /// Installs a published preview once with cooperative pre-commit cancellation.
    ///
    /// A cancellation or resource error before the core commit boundary keeps the preview
    /// published for an explicit retry. A stale result consumes it.
    ///
    /// # Errors
    ///
    /// Returns stable preview lifecycle, stale, cancellation, or resource errors.
    pub fn commit_preview_cancellable(
        &mut self,
        preview_id: u64,
        cancellation: &CancellationToken,
    ) -> Result<WorkbookTransactionReceiptDto, InteropError> {
        let mut published = self.take_published(preview_id)?;
        match self
            .engine
            .install_transaction_cancellable(&mut published.completed, cancellation)
        {
            Ok(receipt) => {
                let dto = transaction_receipt(self.engine.workbook(), &receipt);
                self.invalidate_preview_for_mutation();
                Ok(dto)
            }
            Err(error) if error.code() == SessionErrorCode::StaleResult => Err(error.into()),
            Err(error) => {
                self.published_preview = Some(published);
                Err(error.into())
            }
        }
    }

    /// Discards a published preview without changing the live workbook.
    ///
    /// # Errors
    ///
    /// Returns a stable preview-not-found error for an unknown or already consumed preview.
    pub fn discard_preview(&mut self, preview_id: u64) -> Result<(), InteropError> {
        let mut published = self.take_published(preview_id)?;
        published.completed.discard()?;
        Ok(())
    }

    /// Returns whether an active calculation or preview run exists.
    pub const fn preview_or_calculation_active(&self) -> bool {
        self.active_calculation.is_some()
    }

    fn published(&self, preview_id: u64) -> Result<&PublishedPreview, InteropError> {
        let published = self
            .published_preview
            .as_ref()
            .ok_or_else(InteropError::preview_not_found)?;
        if published.id == preview_id {
            Ok(published)
        } else {
            Err(InteropError::preview_not_found())
        }
    }

    fn take_published(&mut self, preview_id: u64) -> Result<PublishedPreview, InteropError> {
        let published = self
            .published_preview
            .take()
            .ok_or_else(InteropError::preview_not_found)?;
        if published.id == preview_id {
            Ok(published)
        } else {
            self.published_preview = Some(published);
            Err(InteropError::preview_not_found())
        }
    }
}

/// An immutable core transaction that may run without holding its source interop session lock.
#[derive(Debug)]
pub struct PreparedPreview {
    request_id: u64,
    preview_id: u64,
    base: WorkbookSnapshot,
    candidate: WorkbookSnapshot,
    prepared: PreparedWorkbookTransaction,
}

impl PreparedPreview {
    /// Returns the active session operation identifier.
    pub const fn request_id(&self) -> u64 {
        self.request_id
    }

    /// Runs the immutable base/candidate transaction calculation.
    ///
    /// # Errors
    ///
    /// Returns a stable cancellation, calculation, or resource error.
    pub fn run(self) -> Result<CompletedPreview, InteropError> {
        let completed = self.prepared.run()?;
        let summary = PreviewChangesDto {
            schema_version: crate::INTEROP_SCHEMA_VERSION,
            preview_id: self.preview_id,
            report: transaction_report(&self.base, &self.candidate, completed.report()),
        };
        Ok(CompletedPreview {
            request_id: self.request_id,
            preview_id: self.preview_id,
            base: self.base,
            candidate: self.candidate,
            completed,
            summary,
        })
    }
}

/// A fully calculated preview awaiting two-phase publication into its source session.
#[derive(Debug)]
pub struct CompletedPreview {
    request_id: u64,
    preview_id: u64,
    base: WorkbookSnapshot,
    candidate: WorkbookSnapshot,
    completed: CompletedWorkbookTransaction,
    summary: PreviewChangesDto,
}

impl CompletedPreview {
    /// Returns the prepared summary that must be accepted before publication.
    pub const fn summary(&self) -> &PreviewChangesDto {
        &self.summary
    }

    /// Returns the session-local preview identifier reserved for this candidate.
    pub const fn preview_id(&self) -> u64 {
        self.preview_id
    }
}

fn preview_cursor_token(
    preview_id: u64,
    cursor: Option<PreviewCursorDto>,
) -> Result<Option<String>, InteropError> {
    match cursor {
        Some(cursor) if cursor.preview_id != preview_id => {
            Err(InteropError::preview_cursor_invalid())
        }
        Some(cursor) => Ok(Some(cursor.token)),
        None => Ok(None),
    }
}

fn preview_page_limit(limit: u32) -> Result<u32, InteropError> {
    let limit = if limit == 0 {
        DEFAULT_PREVIEW_PAGE_SIZE
    } else {
        limit
    };
    if limit > MAX_PREVIEW_PAGE_SIZE {
        Err(InteropError::page_limit())
    } else {
        Ok(limit)
    }
}

fn serialized_bytes(page: &TransactionImpactPageDto) -> Result<usize, InteropError> {
    serde_json::to_vec(page)
        .map(|value| value.len())
        .map_err(|_| InteropError::preview_response_limit())
}
