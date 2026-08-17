use std::collections::{BTreeMap, BTreeSet};

use super::*;

pub(super) fn build_preview_result_details(
    base: &CalculationSnapshot,
    candidate: &CalculationSnapshot,
    delta: &CalculationDelta,
    cancellation: &CancellationToken,
    detail_budget: &mut DetailBudget,
) -> Result<Vec<TransactionDetailItem>, SessionError> {
    let mut cells = BTreeSet::new();
    for change in delta.changed_cells() {
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        cells.insert(change.cell());
    }
    for cell in delta.removed_materialized_cells() {
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        cells.insert(*cell);
    }
    let mut details = Vec::new();
    for cell in cells {
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        let previous = base.materialized_cell(cell);
        let current = candidate.materialized_cell(cell);
        if previous == current {
            continue;
        }
        detail_budget.reserve(1)?;
        details.push(TransactionDetailItem::PreviewResult(
            TransactionResultChange {
                cell,
                previous_origin: previous.map(|value| value.origin()),
                previous_result: previous.map(|value| value.result().clone()),
                result_origin: current.map(|value| value.origin()),
                result: current.map(|value| value.result().clone()),
            },
        ));
    }
    Ok(details)
}

fn issue(result: Option<&CalculationCellResult>) -> Option<&CalculationIssue> {
    match result {
        Some(CalculationCellResult::Unavailable(issue)) => Some(issue),
        _ => None,
    }
}

pub(super) fn ensure_evaluated_resource_limits(
    calculation: &CalculationSnapshot,
    evaluated: &BTreeSet<CalculationCellId>,
    phase: &str,
    cancellation: &CancellationToken,
) -> Result<(), SessionError> {
    for cell in evaluated {
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        if let Some(CalculationCellResult::Unavailable(issue)) = calculation.cell(*cell)
            && issue.code() == CalculationIssueCode::ResourceLimitExceeded
        {
            return Err(SessionError::new(
                SessionErrorCode::TransactionResourceLimitExceeded,
                Some(format!(
                    "phase={phase}, cell={}:{}, calculation_detail={}",
                    cell.sheet_id().get(),
                    cell.address(),
                    issue.detail().unwrap_or("unspecified")
                )),
            ));
        }
    }
    Ok(())
}

pub(super) fn build_issue_details(
    results: &[TransactionDetailItem],
    cancellation: &CancellationToken,
    detail_budget: &mut DetailBudget,
) -> Result<Vec<TransactionDetailItem>, SessionError> {
    let mut details = Vec::new();
    for result in results {
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        let TransactionDetailItem::PreviewResult(result) = result else {
            continue;
        };
        let previous = issue(result.previous_result());
        let current = issue(result.result());
        let kind = match (previous, current) {
            (None, Some(_)) => Some(TransactionIssueChangeKind::Introduced),
            (Some(_), None) => Some(TransactionIssueChangeKind::Resolved),
            (Some(previous), Some(current)) if previous != current => {
                Some(TransactionIssueChangeKind::Changed)
            }
            _ => None,
        };
        if let Some(kind) = kind {
            detail_budget.reserve(1)?;
            details.push(TransactionDetailItem::PreviewIssue(
                TransactionIssueChange {
                    cell: result.cell,
                    kind,
                    previous: previous.cloned(),
                    current: current.cloned(),
                },
            ));
        }
    }
    Ok(details)
}

pub(super) fn build_affected_details(
    direct: BTreeSet<CalculationCellId>,
    transitive: BTreeSet<CalculationCellId>,
    conservative: BTreeSet<CalculationCellId>,
    cancellation: &CancellationToken,
    detail_budget: &mut DetailBudget,
) -> Result<Vec<TransactionDetailItem>, SessionError> {
    detail_budget.reserve(
        direct
            .len()
            .saturating_add(transitive.len())
            .saturating_add(conservative.len()),
    )?;
    let mut classified = BTreeMap::new();
    for (cells, cause) in [
        (direct, TransactionImpactCause::Direct),
        (transitive, TransactionImpactCause::Transitive),
        (conservative, TransactionImpactCause::Conservative),
    ] {
        for cell in cells {
            if cancellation.is_cancelled() {
                return Err(SessionError::new(SessionErrorCode::Cancelled, None));
            }
            classified.insert(cell, cause);
        }
    }
    let mut details = Vec::with_capacity(classified.len());
    for (cell, cause) in classified {
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        details.push(TransactionDetailItem::Affected(
            TransactionAffectedFormula { cell, cause },
        ));
    }
    Ok(details)
}

pub(super) fn build_evaluated_details(
    evaluated: &BTreeSet<CalculationCellId>,
    cancellation: &CancellationToken,
    detail_budget: &mut DetailBudget,
) -> Result<Vec<TransactionDetailItem>, SessionError> {
    detail_budget.reserve(evaluated.len())?;
    let mut details = Vec::with_capacity(evaluated.len());
    for cell in evaluated {
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        details.push(TransactionDetailItem::Evaluated(*cell));
    }
    Ok(details)
}

pub(super) fn build_install_details(
    delta: &CalculationDelta,
    cancellation: &CancellationToken,
    detail_budget: &mut DetailBudget,
) -> Result<Vec<TransactionDetailItem>, SessionError> {
    detail_budget.reserve(
        delta
            .changed_cells()
            .len()
            .saturating_add(delta.removed_materialized_cells().len()),
    )?;
    let mut details = BTreeMap::new();
    for change in delta.changed_cells() {
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        details.insert(
            change.cell(),
            TransactionInstallResultChange {
                cell: change.cell(),
                origin: Some(change.origin()),
                result: Some(change.result().clone()),
            },
        );
    }
    for cell in delta.removed_materialized_cells() {
        if cancellation.is_cancelled() {
            return Err(SessionError::new(SessionErrorCode::Cancelled, None));
        }
        details.insert(
            *cell,
            TransactionInstallResultChange {
                cell: *cell,
                origin: None,
                result: None,
            },
        );
    }
    Ok(details
        .into_values()
        .map(TransactionDetailItem::InstallResult)
        .collect())
}

pub(super) struct DetailBudget {
    used: usize,
    limit: usize,
}

impl DetailBudget {
    pub(super) const fn new(limit: usize) -> Self {
        Self { used: 0, limit }
    }

    pub(super) fn reserve(&mut self, items: usize) -> Result<(), SessionError> {
        let requested = self.used.saturating_add(items);
        if requested > self.limit {
            return Err(SessionError::new(
                SessionErrorCode::TransactionDetailLimitExceeded,
                Some(format!("items={requested}, limit={}", self.limit)),
            ));
        }
        self.used = requested;
        Ok(())
    }
}

pub(super) fn install_basis_reasons(
    installed: Option<&CalculationSnapshot>,
    installed_options: Option<CalculationOptions>,
    base_revision: u64,
    base_fingerprint: WorkbookFingerprint,
    options: CalculationOptions,
) -> Vec<InstallDeltaBasisReason> {
    let mut reasons = Vec::new();
    let Some(installed) = installed else {
        reasons.push(InstallDeltaBasisReason::NoInstalledCalculation);
        return reasons;
    };
    if installed.source_revision() != base_revision {
        reasons.push(InstallDeltaBasisReason::PriorPendingEdits);
    }
    if installed_options != Some(options) || installed.options() != options {
        reasons.push(InstallDeltaBasisReason::CalculationOptionsChanged);
    }
    if installed.source_revision() == base_revision
        && installed.source_fingerprint() != base_fingerprint
    {
        reasons.push(InstallDeltaBasisReason::InstalledCalculationIdentityMismatch);
    }
    reasons
}
