//! Rectangle value read and write DTOs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{CellValueDto, WritableCellValueDto};

/// Request for every value of one inclusive rectangle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeValuesRequestDto {
    /// Case-insensitive sheet name.
    pub sheet: String,
    /// Inclusive range start in unqualified A1 notation.
    pub start: String,
    /// Inclusive range end in unqualified A1 notation.
    pub end: String,
}

/// The values of one inclusive rectangle, as rows from top to bottom.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeValuesDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Resolved sheet name.
    pub sheet: String,
    /// Inclusive range start.
    pub start: String,
    /// Inclusive range end.
    pub end: String,
    /// One row per worksheet row, each holding one value per column from left to right.
    pub rows: Vec<Vec<RangeValueDto>>,
}

/// The value a cell holds: its current calculated result when the session has calculated the
/// current revision, otherwise its literal or saved value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RangeValueDto {
    /// A typed spreadsheet value.
    Value(CellValueDto),
    /// The current calculation withheld the formula's result.
    Unavailable(UnavailableValueDto),
}

/// A formula result the current calculation withheld.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum UnavailableValueDto {
    /// Calculation was deliberately withheld.
    Unavailable {
        /// Stable CellRune calculation issue code.
        code: String,
        /// Shared human-readable issue message.
        message: String,
        /// Optional source-specific context.
        detail: Option<String>,
        /// Calculation limit a `calculation.resource_limit_exceeded` issue exceeded. Omitted for
        /// every other issue.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<String>,
    },
}

/// Values written into the rectangle that starts at one cell, as one atomic edit batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeWriteRequestDto {
    /// Case-insensitive sheet name.
    pub sheet: String,
    /// Top-left cell in unqualified A1 notation.
    pub start: String,
    /// Rows from top to bottom, all of the same non-zero length; a blank value clears its cell.
    pub rows: Vec<Vec<WritableCellValueDto>>,
}
