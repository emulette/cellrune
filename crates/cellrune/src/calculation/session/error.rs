use std::error::Error;
use std::fmt;

const MESSAGE_REVISION_MISMATCH: &str = "expected workbook revision does not match current state";
const MESSAGE_CALCULATION_UNINITIALIZED: &str =
    "incremental calculation requires an installed complete calculation state";
const MESSAGE_STATE_REVISION_MISMATCH: &str =
    "calculation state does not match the current workbook revision";
const MESSAGE_INCREMENTAL_UNSAFE: &str =
    "the requested workbook change cannot be recalculated incrementally";
const MESSAGE_STALE_RESULT: &str = "calculation completed for stale session state";
const MESSAGE_INVALID_LIMITS: &str = "session limits must be greater than zero";
const MESSAGE_EMPTY_BATCH: &str = "edit batch must contain at least one operation";
const MESSAGE_BATCH_LIMIT: &str = "edit batch exceeds the configured operation limit";
const MESSAGE_EVALUATION_LIMIT: &str = "calculation exceeds the configured evaluation limit";
const MESSAGE_DELTA_LIMIT: &str = "calculation delta exceeds the configured cell limit";
const MESSAGE_CURSOR_EXPIRED: &str = "calculation delta cursor is no longer retained";
const MESSAGE_PAGE_LIMIT: &str = "calculation delta page exceeds the configured limit";
const MESSAGE_CANCELLATION: &str = "calculation was cancelled";
const MESSAGE_REWRITE_LIMIT: &str = "formula rewrite exceeds the configured whole-workbook limit";
const MESSAGE_TABLE_MATERIALIZATION_LIMIT: &str =
    "table resize exceeds the configured materialization-cell limit";
const MESSAGE_TRANSACTION_CONSUMED: &str =
    "workbook transaction was already installed, discarded, or found stale";
const MESSAGE_TRANSACTION_CURSOR_INVALID: &str =
    "transaction detail cursor does not belong to this report section";
const MESSAGE_TRANSACTION_DETAIL_LIMIT: &str =
    "transaction report exceeds the configured retained-detail limit";
const MESSAGE_TRANSACTION_RESOURCE_LIMIT: &str =
    "workbook transaction exceeded the configured resource limit";

pub(super) fn stale_calculation_cursor_detail(
    calculation_cursor: u64,
    current_cursor: u64,
) -> String {
    format!("calculation_cursor={calculation_cursor}, current={current_cursor}")
}

/// Stable machine-readable failure produced by a stateful calculation session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum SessionErrorCode {
    /// The caller supplied an outdated expected revision.
    RevisionMismatch,
    /// Incremental calculation was requested before a complete calculation state existed.
    CalculationUninitialized,
    /// Retained calculation state does not match the workbook revision it claims.
    StateRevisionMismatch,
    /// Forced incremental calculation cannot prove a safe impact boundary.
    IncrementalUnsafe,
    /// A completed calculation belongs to older workbook or calculation state and was not installed.
    StaleResult,
    /// One or more session resource limits were configured as zero.
    InvalidLimits,
    /// An edit batch contained no operation.
    EmptyBatch,
    /// An edit batch exceeded its configured operation budget.
    BatchLimitExceeded,
    /// One calculation pass exceeded its configured evaluated-cell budget.
    EvaluationLimitExceeded,
    /// A result delta exceeded its configured changed-cell budget.
    DeltaLimitExceeded,
    /// A requested delta cursor predates the retained history.
    CursorExpired,
    /// A requested delta page exceeded its configured limit.
    PageLimitExceeded,
    /// Cooperative cancellation stopped a calculation before installation.
    Cancelled,
    /// A whole-workbook typed formula rewrite exceeded its configured budget.
    RewriteLimitExceeded,
    /// A table resize exceeded its configured worksheet-materialization budget.
    TableMaterializationLimitExceeded,
    /// A completed transaction was already installed, discarded, or found stale.
    TransactionConsumed,
    /// A transaction detail cursor belongs to another report or section.
    TransactionCursorInvalid,
    /// A transaction report exceeded its configured retained-detail budget.
    TransactionDetailLimitExceeded,
    /// A transaction phase exceeded its configured resource budget.
    TransactionResourceLimitExceeded,
}

impl SessionErrorCode {
    /// Returns the stable dotted identifier used across bindings.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RevisionMismatch => "session.revision_mismatch",
            Self::CalculationUninitialized => "session.calculation_uninitialized",
            Self::StateRevisionMismatch => "session.state_revision_mismatch",
            Self::IncrementalUnsafe => "session.incremental_unsafe",
            Self::StaleResult => "session.stale_result",
            Self::InvalidLimits => "session.limits_invalid",
            Self::EmptyBatch => "session.edit_batch_empty",
            Self::BatchLimitExceeded => "session.edit_batch_limit_exceeded",
            Self::EvaluationLimitExceeded => "session.evaluation_limit_exceeded",
            Self::DeltaLimitExceeded => "session.delta_limit_exceeded",
            Self::CursorExpired => "session.delta_cursor_expired",
            Self::PageLimitExceeded => "session.delta_page_limit_exceeded",
            Self::Cancelled => "session.cancelled",
            Self::RewriteLimitExceeded => "session.formula_rewrite_limit_exceeded",
            Self::TableMaterializationLimitExceeded => {
                "session.table_materialization_limit_exceeded"
            }
            Self::TransactionConsumed => "session.transaction_consumed",
            Self::TransactionCursorInvalid => "session.transaction_cursor_invalid",
            Self::TransactionDetailLimitExceeded => "session.transaction_detail_limit_exceeded",
            Self::TransactionResourceLimitExceeded => "session.transaction_resource_limit_exceeded",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::RevisionMismatch => MESSAGE_REVISION_MISMATCH,
            Self::CalculationUninitialized => MESSAGE_CALCULATION_UNINITIALIZED,
            Self::StateRevisionMismatch => MESSAGE_STATE_REVISION_MISMATCH,
            Self::IncrementalUnsafe => MESSAGE_INCREMENTAL_UNSAFE,
            Self::StaleResult => MESSAGE_STALE_RESULT,
            Self::InvalidLimits => MESSAGE_INVALID_LIMITS,
            Self::EmptyBatch => MESSAGE_EMPTY_BATCH,
            Self::BatchLimitExceeded => MESSAGE_BATCH_LIMIT,
            Self::EvaluationLimitExceeded => MESSAGE_EVALUATION_LIMIT,
            Self::DeltaLimitExceeded => MESSAGE_DELTA_LIMIT,
            Self::CursorExpired => MESSAGE_CURSOR_EXPIRED,
            Self::PageLimitExceeded => MESSAGE_PAGE_LIMIT,
            Self::Cancelled => MESSAGE_CANCELLATION,
            Self::RewriteLimitExceeded => MESSAGE_REWRITE_LIMIT,
            Self::TableMaterializationLimitExceeded => MESSAGE_TABLE_MATERIALIZATION_LIMIT,
            Self::TransactionConsumed => MESSAGE_TRANSACTION_CONSUMED,
            Self::TransactionCursorInvalid => MESSAGE_TRANSACTION_CURSOR_INVALID,
            Self::TransactionDetailLimitExceeded => MESSAGE_TRANSACTION_DETAIL_LIMIT,
            Self::TransactionResourceLimitExceeded => MESSAGE_TRANSACTION_RESOURCE_LIMIT,
        }
    }
}

/// Structured stateful-session error with a stable code and optional detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionError {
    code: SessionErrorCode,
    detail: Option<Box<str>>,
}

impl SessionError {
    pub(super) fn new(code: SessionErrorCode, detail: Option<String>) -> Self {
        Self {
            code,
            detail: detail.map(String::into_boxed_str),
        }
    }

    /// Returns the stable error code.
    pub const fn code(&self) -> SessionErrorCode {
        self.code
    }

    /// Returns the shared human-readable message.
    pub const fn message(&self) -> &'static str {
        self.code.message()
    }

    /// Returns structured source-specific context.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message())?;
        if let Some(detail) = self.detail() {
            write!(formatter, ": {detail}")?;
        }
        Ok(())
    }
}

impl Error for SessionError {}
