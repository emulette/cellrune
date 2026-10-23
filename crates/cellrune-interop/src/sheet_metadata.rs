//! Read-only worksheet metadata pages.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Bounded request for one page of a sheet's metadata entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetPageRequestDto {
    /// Case-insensitive sheet name.
    pub sheet: String,
    /// Zero-based entry offset.
    #[serde(default)]
    pub offset: u64,
    /// Page size; zero selects the default, with a hard maximum of 10,000.
    #[serde(default)]
    pub limit: u32,
}

/// One page of a sheet's data validation rules, in worksheet order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DataValidationPageDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Resolved sheet name.
    pub sheet: String,
    /// Number of modeled rules on the sheet.
    pub total: u64,
    /// Offset represented by the first returned rule.
    pub offset: u64,
    /// Offset for the next page, or `None` at the end.
    pub next_offset: Option<u64>,
    /// Returned rules.
    pub entries: Vec<DataValidationDto>,
}

/// One worksheet data validation rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DataValidationDto {
    /// Ranges the rule applies to, in `sqref` order; a one-cell range is a single address.
    pub ranges: Vec<String>,
    /// OOXML type token: `none`, `whole`, `decimal`, `list`, `date`, `time`, `textLength`, or
    /// `custom`.
    pub validation_type: String,
    /// OOXML operator token: `between`, `notBetween`, `equal`, `notEqual`, `lessThan`,
    /// `lessThanOrEqual`, `greaterThan`, or `greaterThanOrEqual`.
    pub operator: String,
    /// First formula as stored, without a leading `=`: the list source, the custom condition, or
    /// the first bound.
    pub formula1: Option<String>,
    /// Second bound of a `between` or `notBetween` rule.
    pub formula2: Option<String>,
    /// Whether a blank cell passes the rule.
    pub allow_blank: bool,
    /// Whether a list rule shows its in-cell dropdown (OOXML stores the inverse in
    /// `showDropDown`).
    pub in_cell_dropdown: bool,
    /// Whether the input message is shown when a cell is selected.
    pub show_input_message: bool,
    /// Whether the error alert is shown for an invalid entry.
    pub show_error_message: bool,
    /// OOXML error style token: `stop`, `warning`, or `information`.
    pub error_style: String,
    /// Error alert title.
    pub error_title: Option<String>,
    /// Error alert text.
    pub error_message: Option<String>,
    /// Input message title.
    pub prompt_title: Option<String>,
    /// Input message text.
    pub prompt_message: Option<String>,
}

/// One page of a sheet's hyperlinks, in worksheet order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HyperlinkPageDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Resolved sheet name.
    pub sheet: String,
    /// Number of modeled hyperlinks on the sheet.
    pub total: u64,
    /// Offset represented by the first returned hyperlink.
    pub offset: u64,
    /// Offset for the next page, or `None` at the end.
    pub next_offset: Option<u64>,
    /// Returned hyperlinks.
    pub entries: Vec<HyperlinkDto>,
}

/// One worksheet hyperlink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HyperlinkDto {
    /// Cells the hyperlink covers; a one-cell range is a single address.
    pub range: String,
    /// External target, such as a URL, from the worksheet relationship.
    pub target: Option<String>,
    /// Place the hyperlink opens: a reference or defined name in this workbook, or with a target a
    /// place inside the target document.
    pub location: Option<String>,
    /// Recorded display text.
    pub display: Option<String>,
    /// Screen tip.
    pub tooltip: Option<String>,
}

/// One page of a sheet's cell comments (notes), in comments-part order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommentPageDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Resolved sheet name.
    pub sheet: String,
    /// Number of comments on the sheet.
    pub total: u64,
    /// Offset represented by the first returned comment.
    pub offset: u64,
    /// Offset for the next page, or `None` at the end.
    pub next_offset: Option<u64>,
    /// Returned comments.
    pub entries: Vec<CommentDto>,
}

/// One cell comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommentDto {
    /// Unqualified A1 address of the commented cell.
    pub address: String,
    /// Author, when the comment names a known one.
    pub author: Option<String>,
    /// Plain text of every run.
    pub text: String,
    /// The other comments of an Excel 365 comment thread in order; omitted for a note.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<CommentReplyDto>,
}

/// One reply in a comment thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommentReplyDto {
    /// Author's display name, when the workbook's persons part names them.
    pub author: Option<String>,
    /// Plain text.
    pub text: String,
}
