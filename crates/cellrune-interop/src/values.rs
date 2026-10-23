//! Rectangle value read and write DTOs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{CellValueDto, WritableCellValueDto};

const fn is_false(value: &bool) -> bool {
    !*value
}

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

/// Optional forms of a range page. The default reads every cell of the page as a full cell object,
/// exactly as a request without options does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeReadOptionsDto {
    /// Leaves out empty cells: those holding no value or formula and no calculated result. Offset
    /// and limit still count every position of the range; in the compact form only each row's
    /// trailing empty cells are left out.
    #[serde(default, skip_serializing_if = "is_false")]
    pub omit_empty: bool,
    /// Returns each cell's address and value, in `values`, instead of full cell objects.
    #[serde(default, skip_serializing_if = "is_false")]
    pub values_only: bool,
    /// Returns the page's values in `rows`, grouped by worksheet row and without addresses.
    #[serde(default, skip_serializing_if = "is_false")]
    pub compact: bool,
    /// Largest serialized page in bytes. The page is then the longest prefix of its positions that
    /// fits, and `next_offset` continues after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
}

/// One cell's address and value in a values-only range page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeCellValueDto {
    /// Unqualified A1 address.
    pub address: String,
    /// The cell's current calculated result once the session has calculated the current revision,
    /// otherwise its literal or saved value.
    pub value: RangeValueDto,
}
