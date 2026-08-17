use std::collections::hash_map::RandomState;
use std::sync::atomic::{AtomicU64, Ordering};

use super::super::{
    CalculationDecisionReason, CalculationDelta, CalculationExecutionMode, RecalculationMode,
    SessionError, SessionErrorCode,
};
use crate::{
    CalculationCellId, CalculationCellResult, CalculationIssue, CalculationOptions, EditReceipt,
    InputHash, MaterializedResultOrigin, ProviderIdentity, WorkbookFingerprint,
};

pub(super) const TRANSACTION_REPORT_CONTRACT_VERSION: u16 = 1;
static NEXT_TRANSACTION_REPORT_ID: AtomicU64 = AtomicU64::new(1);

pub(super) fn next_report_identity() -> Result<u64, SessionError> {
    NEXT_TRANSACTION_REPORT_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| {
            SessionError::new(
                SessionErrorCode::TransactionResourceLimitExceeded,
                Some("transaction report identity space exhausted".to_owned()),
            )
        })
}

/// Completeness of the semantic affected-formula set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransactionImpactCoverage {
    /// The retained dependency graph proves a complete direct and transitive affected set.
    Exact,
    /// A topology or dynamic-dependency boundary requires a conservative full formula set.
    ConservativeFull,
}

/// Why one formula appears in the affected-formula section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransactionImpactCause {
    /// The formula directly reads a changed source or is itself changed.
    Direct,
    /// The formula depends transitively on a directly affected formula.
    Transitive,
    /// The formula is retained because the complete causal boundary cannot be proven.
    Conservative,
}

/// One formula in the bounded semantic impact report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionAffectedFormula {
    pub(super) cell: CalculationCellId,
    pub(super) cause: TransactionImpactCause,
}

impl TransactionAffectedFormula {
    /// Returns the affected formula cell.
    pub const fn cell(&self) -> CalculationCellId {
        self.cell
    }

    /// Returns the formula's impact classification.
    pub const fn cause(&self) -> TransactionImpactCause {
        self.cause
    }
}

/// One exact base-to-candidate materialized result change.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionResultChange {
    pub(super) cell: CalculationCellId,
    pub(super) previous_origin: Option<MaterializedResultOrigin>,
    pub(super) previous_result: Option<CalculationCellResult>,
    pub(super) result_origin: Option<MaterializedResultOrigin>,
    pub(super) result: Option<CalculationCellResult>,
}

impl TransactionResultChange {
    /// Returns the changed materialized cell.
    pub const fn cell(&self) -> CalculationCellId {
        self.cell
    }

    /// Returns the base materialization origin, or `None` when the cell was introduced.
    pub const fn previous_origin(&self) -> Option<MaterializedResultOrigin> {
        self.previous_origin
    }

    /// Returns the base typed result, or `None` when the cell was introduced.
    pub const fn previous_result(&self) -> Option<&CalculationCellResult> {
        self.previous_result.as_ref()
    }

    /// Returns the candidate materialization origin, or `None` when the cell was removed.
    pub const fn result_origin(&self) -> Option<MaterializedResultOrigin> {
        self.result_origin
    }

    /// Returns the candidate typed result, or `None` when the cell was removed.
    pub const fn result(&self) -> Option<&CalculationCellResult> {
        self.result.as_ref()
    }
}

/// Classification of one exact calculation issue difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransactionIssueChangeKind {
    /// The candidate introduced an issue where the base had none.
    Introduced,
    /// The candidate resolved an issue present in the base.
    Resolved,
    /// The candidate replaced one issue with a different issue.
    Changed,
}

/// One exact base-to-candidate calculation issue difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionIssueChange {
    pub(super) cell: CalculationCellId,
    pub(super) kind: TransactionIssueChangeKind,
    pub(super) previous: Option<CalculationIssue>,
    pub(super) current: Option<CalculationIssue>,
}

impl TransactionIssueChange {
    /// Returns the cell whose issue changed.
    pub const fn cell(&self) -> CalculationCellId {
        self.cell
    }

    /// Returns how the issue changed.
    pub const fn kind(&self) -> TransactionIssueChangeKind {
        self.kind
    }

    /// Returns the base issue, when present.
    pub const fn previous(&self) -> Option<&CalculationIssue> {
        self.previous.as_ref()
    }

    /// Returns the candidate issue, when present.
    pub const fn current(&self) -> Option<&CalculationIssue> {
        self.current.as_ref()
    }
}

/// One exact result change that installation will append to calculation history.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionInstallResultChange {
    pub(super) cell: CalculationCellId,
    pub(super) origin: Option<MaterializedResultOrigin>,
    pub(super) result: Option<CalculationCellResult>,
}

impl TransactionInstallResultChange {
    /// Returns the changed or removed cell.
    pub const fn cell(&self) -> CalculationCellId {
        self.cell
    }

    /// Returns the installed materialization origin, or `None` for a removal.
    pub const fn origin(&self) -> Option<MaterializedResultOrigin> {
        self.origin
    }

    /// Returns the installed result, or `None` for a removal.
    pub const fn result(&self) -> Option<&CalculationCellResult> {
        self.result.as_ref()
    }

    /// Returns whether installation removes this materialized cell.
    pub const fn is_removed(&self) -> bool {
        self.result.is_none()
    }
}

/// A transaction report detail section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum TransactionDetailSection {
    /// Formula cells with semantic direct, transitive, or conservative impact.
    Affected,
    /// Formula cells actually executed by the candidate evaluator.
    Evaluated,
    /// Exact base-to-candidate materialized result differences.
    PreviewResults,
    /// Exact base-to-candidate issue differences.
    PreviewIssues,
    /// Exact result differences that installation will append to history.
    InstallResults,
}

/// One item from a transaction detail page.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum TransactionDetailItem {
    /// An affected formula and its semantic cause.
    Affected(TransactionAffectedFormula),
    /// A formula cell actually executed by the candidate evaluator.
    Evaluated(CalculationCellId),
    /// An exact base-to-candidate result change.
    PreviewResult(TransactionResultChange),
    /// An exact base-to-candidate issue change.
    PreviewIssue(TransactionIssueChange),
    /// An exact install-delta result change.
    InstallResult(TransactionInstallResultChange),
}

/// Why the installed-calculation delta has a different comparison basis than the preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum InstallDeltaBasisReason {
    /// No complete calculation had previously been installed.
    NoInstalledCalculation,
    /// The transaction base contains edits not represented by the installed calculation.
    PriorPendingEdits,
    /// The transaction requested calculation options different from the installed calculation.
    CalculationOptionsChanged,
    /// Installed calculation identity did not match the captured transaction base.
    InstalledCalculationIdentityMismatch,
}

/// Complete bounded summary and pageable details for one calculated transaction.
#[derive(Debug)]
pub struct WorkbookTransactionReport {
    pub(super) identity: u64,
    pub(super) cursor_hash_builder: RandomState,
    pub(super) base_revision: u64,
    pub(super) result_revision: u64,
    pub(super) base_fingerprint: WorkbookFingerprint,
    pub(super) result_fingerprint: WorkbookFingerprint,
    pub(super) input_hash: Option<InputHash>,
    pub(super) calculator_provider: ProviderIdentity,
    pub(super) options: CalculationOptions,
    pub(super) base_calculation_reused: bool,
    pub(super) base_execution_mode: CalculationExecutionMode,
    pub(super) base_decision_reason: CalculationDecisionReason,
    pub(super) candidate_requested_mode: RecalculationMode,
    pub(super) candidate_execution_mode: CalculationExecutionMode,
    pub(super) candidate_decision_reason: CalculationDecisionReason,
    pub(super) edit_receipt: EditReceipt,
    pub(super) impact_coverage: TransactionImpactCoverage,
    pub(super) direct_affected_count: usize,
    pub(super) transitive_affected_count: usize,
    pub(super) conservative_affected_count: usize,
    pub(super) base_evaluated_count: usize,
    pub(super) candidate_evaluated_count: usize,
    pub(super) parsed_formula_count: usize,
    pub(super) function_iteration_count: u64,
    pub(super) reference_cell_count: u64,
    pub(super) preview_changed_count: usize,
    pub(super) preview_removed_count: usize,
    pub(super) introduced_issue_count: usize,
    pub(super) resolved_issue_count: usize,
    pub(super) changed_issue_count: usize,
    pub(super) install_delta: CalculationDelta,
    pub(super) installed_calculation_revision: Option<u64>,
    pub(super) installed_calculation_fingerprint: Option<WorkbookFingerprint>,
    pub(super) installed_calculation_options: Option<CalculationOptions>,
    pub(super) install_basis_reasons: Vec<InstallDeltaBasisReason>,
    pub(super) max_page_items: usize,
    pub(super) affected_detail_count: usize,
    pub(super) evaluated_detail_count: usize,
    pub(super) preview_result_detail_count: usize,
    pub(super) preview_issue_detail_count: usize,
    pub(super) install_result_detail_count: usize,
    pub(super) affected: Vec<TransactionDetailItem>,
    pub(super) evaluated: Vec<TransactionDetailItem>,
    pub(super) preview_results: Vec<TransactionDetailItem>,
    pub(super) preview_issues: Vec<TransactionDetailItem>,
    pub(super) install_results: Vec<TransactionDetailItem>,
}

impl WorkbookTransactionReport {
    /// Returns the version of this report and cursor contract.
    pub const fn contract_version(&self) -> u16 {
        TRANSACTION_REPORT_CONTRACT_VERSION
    }

    /// Returns the transaction base semantic revision.
    pub const fn base_revision(&self) -> u64 {
        self.base_revision
    }

    /// Returns the candidate semantic revision.
    pub const fn result_revision(&self) -> u64 {
        self.result_revision
    }

    /// Returns the transaction base semantic fingerprint.
    pub const fn base_fingerprint(&self) -> WorkbookFingerprint {
        self.base_fingerprint
    }

    /// Returns the candidate semantic fingerprint.
    pub const fn result_fingerprint(&self) -> WorkbookFingerprint {
        self.result_fingerprint
    }

    /// Returns the package input SHA-256 when the base is package-backed.
    pub const fn input_hash(&self) -> Option<InputHash> {
        self.input_hash
    }

    /// Returns the calculator provider identity and version.
    pub const fn calculator_provider(&self) -> &ProviderIdentity {
        &self.calculator_provider
    }

    /// Returns the complete options shared by base and candidate calculation.
    pub const fn calculation_options(&self) -> CalculationOptions {
        self.options
    }

    /// Returns whether the installed current base calculation was reused without evaluator work.
    pub const fn base_calculation_reused(&self) -> bool {
        self.base_calculation_reused
    }

    /// Returns the base calculation execution mode.
    pub const fn base_execution_mode(&self) -> CalculationExecutionMode {
        self.base_execution_mode
    }

    /// Returns the base calculation decision reason.
    pub const fn base_decision_reason(&self) -> CalculationDecisionReason {
        self.base_decision_reason
    }

    /// Returns the caller-requested candidate recalculation mode.
    pub const fn candidate_requested_mode(&self) -> RecalculationMode {
        self.candidate_requested_mode
    }

    /// Returns the selected candidate calculation execution mode.
    pub const fn candidate_execution_mode(&self) -> CalculationExecutionMode {
        self.candidate_execution_mode
    }

    /// Returns the candidate calculation decision reason.
    pub const fn candidate_decision_reason(&self) -> CalculationDecisionReason {
        self.candidate_decision_reason
    }

    /// Returns the exact edit receipt that installation will commit.
    pub const fn edit_receipt(&self) -> &EditReceipt {
        &self.edit_receipt
    }

    /// Returns whether the semantic affected set is exact or conservative.
    pub const fn impact_coverage(&self) -> TransactionImpactCoverage {
        self.impact_coverage
    }

    /// Returns the number of formulas directly affected by this edit batch.
    pub const fn direct_affected_count(&self) -> usize {
        self.direct_affected_count
    }

    /// Returns the number of formulas transitively affected by this edit batch.
    pub const fn transitive_affected_count(&self) -> usize {
        self.transitive_affected_count
    }

    /// Returns the number of formulas conservatively included when impact is not exact.
    pub const fn conservative_affected_count(&self) -> usize {
        self.conservative_affected_count
    }

    /// Returns formula executions spent calculating an uncached transaction base.
    pub const fn base_evaluated_count(&self) -> usize {
        self.base_evaluated_count
    }

    /// Returns formula cells actually executed for the candidate.
    pub const fn candidate_evaluated_count(&self) -> usize {
        self.candidate_evaluated_count
    }

    /// Returns formulas parsed across base and candidate calculation work.
    pub const fn parsed_formula_count(&self) -> usize {
        self.parsed_formula_count
    }

    /// Returns function iterations charged across uncached base and candidate calculation work.
    pub const fn function_iteration_count(&self) -> u64 {
        self.function_iteration_count
    }

    /// Returns referenced cells charged across uncached base and candidate calculation work.
    pub const fn reference_cell_count(&self) -> u64 {
        self.reference_cell_count
    }

    /// Returns the number of introduced or changed preview materialized results.
    pub const fn preview_changed_count(&self) -> usize {
        self.preview_changed_count
    }

    /// Returns the number of removed preview materialized results.
    pub const fn preview_removed_count(&self) -> usize {
        self.preview_removed_count
    }

    /// Returns the number of introduced calculation issues.
    pub const fn introduced_issue_count(&self) -> usize {
        self.introduced_issue_count
    }

    /// Returns the number of resolved calculation issues.
    pub const fn resolved_issue_count(&self) -> usize {
        self.resolved_issue_count
    }

    /// Returns the number of calculation issues replaced by a different issue.
    pub const fn changed_issue_count(&self) -> usize {
        self.changed_issue_count
    }

    /// Returns the exact delta reserved for installation and history append.
    pub const fn install_delta(&self) -> &CalculationDelta {
        &self.install_delta
    }

    /// Returns the prior installed calculation revision used by the install delta, when present.
    pub const fn installed_calculation_revision(&self) -> Option<u64> {
        self.installed_calculation_revision
    }

    /// Returns the prior installed calculation fingerprint used by the install delta, when present.
    pub const fn installed_calculation_fingerprint(&self) -> Option<WorkbookFingerprint> {
        self.installed_calculation_fingerprint
    }

    /// Returns the prior installed calculation options used by the install delta, when present.
    pub const fn installed_calculation_options(&self) -> Option<CalculationOptions> {
        self.installed_calculation_options
    }

    /// Returns whether installation compares against a different basis than preview.
    pub fn install_delta_basis_differs_from_preview_base(&self) -> bool {
        !self.install_basis_reasons.is_empty()
    }

    /// Returns ordered reasons why the install-delta basis differs from the preview base.
    pub fn install_delta_basis_reasons(&self) -> &[InstallDeltaBasisReason] {
        &self.install_basis_reasons
    }

    /// Returns the complete item count for one detail section.
    pub fn detail_count(&self, section: TransactionDetailSection) -> usize {
        match section {
            TransactionDetailSection::Affected => self.affected_detail_count,
            TransactionDetailSection::Evaluated => self.evaluated_detail_count,
            TransactionDetailSection::PreviewResults => self.preview_result_detail_count,
            TransactionDetailSection::PreviewIssues => self.preview_issue_detail_count,
            TransactionDetailSection::InstallResults => self.install_result_detail_count,
        }
    }

    pub(super) fn details(&self, section: TransactionDetailSection) -> &[TransactionDetailItem] {
        match section {
            TransactionDetailSection::Affected => &self.affected,
            TransactionDetailSection::Evaluated => &self.evaluated,
            TransactionDetailSection::PreviewResults => &self.preview_results,
            TransactionDetailSection::PreviewIssues => &self.preview_issues,
            TransactionDetailSection::InstallResults => &self.install_results,
        }
    }

    pub(super) fn release_details(&mut self) {
        drop(std::mem::take(&mut self.affected));
        drop(std::mem::take(&mut self.evaluated));
        drop(std::mem::take(&mut self.preview_results));
        drop(std::mem::take(&mut self.preview_issues));
        drop(std::mem::take(&mut self.install_results));
    }
}

/// Exact edit and calculation receipts returned by a successful transaction install.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkbookTransactionReceipt {
    pub(super) edit: EditReceipt,
    pub(super) calculation_delta: CalculationDelta,
    pub(super) base_fingerprint: WorkbookFingerprint,
    pub(super) result_fingerprint: WorkbookFingerprint,
}

impl WorkbookTransactionReceipt {
    /// Returns the exact installed edit receipt.
    pub const fn edit(&self) -> &EditReceipt {
        &self.edit
    }

    /// Returns the exact installed and history-appended calculation delta.
    pub const fn calculation_delta(&self) -> &CalculationDelta {
        &self.calculation_delta
    }

    /// Returns the semantic fingerprint checked before installation.
    pub const fn base_fingerprint(&self) -> WorkbookFingerprint {
        self.base_fingerprint
    }

    /// Returns the installed candidate semantic fingerprint.
    pub const fn result_fingerprint(&self) -> WorkbookFingerprint {
        self.result_fingerprint
    }
}
