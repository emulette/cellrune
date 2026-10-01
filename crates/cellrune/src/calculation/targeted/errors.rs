use std::fmt;
pub(super) const VALIDATED_COORDINATES: &str = "validated target coordinates";

/// Stable request failure for targeted calculation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TargetCalculationErrorCode {
    /// The request has no targets.
    EmptyTargets,
    /// A target names a sheet that does not exist.
    SheetNotFound,
    /// A configured limit is zero.
    InvalidLimits,
    /// The request exceeds its target or result limit.
    TargetLimitExceeded,
    /// The request exceeds its formula work limit.
    EvaluationLimitExceeded,
    /// The caller cancelled the request.
    Cancelled,
    /// The session changed before the result was returned.
    StaleResult,
}

impl TargetCalculationErrorCode {
    /// Returns the stable machine-readable code.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EmptyTargets => "calculation.target.empty_targets",
            Self::SheetNotFound => "calculation.target.sheet_not_found",
            Self::InvalidLimits => "calculation.target.invalid_limits",
            Self::TargetLimitExceeded => "calculation.target.target_limit_exceeded",
            Self::EvaluationLimitExceeded => "calculation.target.evaluation_limit_exceeded",
            Self::Cancelled => "calculation.target.cancelled",
            Self::StaleResult => "calculation.target.stale_result",
        }
    }

    /// Returns the stable human-readable message.
    pub const fn message(self) -> &'static str {
        match self {
            Self::EmptyTargets => "targeted calculation requires at least one target",
            Self::SheetNotFound => "target sheet does not exist",
            Self::InvalidLimits => "targeted calculation limits must be non-zero",
            Self::TargetLimitExceeded => "targeted calculation exceeds its target or result limit",
            Self::EvaluationLimitExceeded => "targeted calculation exceeds its formula work limit",
            Self::Cancelled => "targeted calculation was cancelled",
            Self::StaleResult => "targeted calculation no longer matches the session state",
        }
    }
}

/// A targeted-calculation request failed without changing workbook state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetCalculationError {
    code: TargetCalculationErrorCode,
    requested_cell_count: Option<u64>,
    evaluated_count: Option<usize>,
}

impl TargetCalculationError {
    pub(crate) const fn new(code: TargetCalculationErrorCode) -> Self {
        Self {
            code,
            requested_cell_count: None,
            evaluated_count: None,
        }
    }

    pub(crate) const fn result_limit(requested_cell_count: u64) -> Self {
        Self {
            requested_cell_count: Some(requested_cell_count),
            ..Self::new(TargetCalculationErrorCode::TargetLimitExceeded)
        }
    }

    pub(crate) const fn evaluation_limit(evaluated_count: usize) -> Self {
        Self {
            evaluated_count: Some(evaluated_count),
            ..Self::new(TargetCalculationErrorCode::EvaluationLimitExceeded)
        }
    }

    /// Returns the stable failure category.
    pub const fn code(&self) -> TargetCalculationErrorCode {
        self.code
    }

    /// Returns the requested cells counted when a
    /// [`TargetLimitExceeded`](TargetCalculationErrorCode::TargetLimitExceeded) request passed
    /// its result-cell limit.
    ///
    /// The count is a lower bound on the distinct cells the request names: a target larger than
    /// the limit reports its own cell count, and otherwise counting stops at the first distinct
    /// cell past the limit. `None` when the target-count limit rejected the request before its
    /// cells were counted, and for every other failure.
    pub const fn requested_cell_count(&self) -> Option<u64> {
        self.requested_cell_count
    }

    /// Returns the evaluations performed before an
    /// [`EvaluationLimitExceeded`](TargetCalculationErrorCode::EvaluationLimitExceeded) request
    /// stopped, counted like
    /// [`TargetCalculationResult::evaluated_count`](crate::TargetCalculationResult::evaluated_count),
    /// or `None` for every other failure.
    pub const fn evaluated_count(&self) -> Option<usize> {
        self.evaluated_count
    }
}

impl fmt::Display for TargetCalculationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code.message())
    }
}

impl std::error::Error for TargetCalculationError {}
