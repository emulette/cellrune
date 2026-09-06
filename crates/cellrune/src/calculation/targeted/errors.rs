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
}

impl TargetCalculationError {
    pub(crate) const fn new(code: TargetCalculationErrorCode) -> Self {
        Self { code }
    }
    /// Returns the stable failure category.
    pub const fn code(&self) -> TargetCalculationErrorCode {
        self.code
    }
}

impl fmt::Display for TargetCalculationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code.message())
    }
}

impl std::error::Error for TargetCalculationError {}
