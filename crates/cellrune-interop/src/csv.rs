//! CSV import and export DTOs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Options for opening CSV as a one-sheet workbook.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CsvReadOptionsDto {
    /// Name of the sheet the input is read into; omitted selects `Sheet1`.
    #[serde(default)]
    pub sheet_name: Option<String>,
    /// One ASCII field delimiter other than a quote or line break; omitted selects `,`.
    #[serde(default)]
    pub delimiter: Option<String>,
}

/// One sheet to export as CSV.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CsvWriteRequestDto {
    /// Case-insensitive sheet name.
    pub sheet: String,
    /// One ASCII field delimiter other than a quote or line break; omitted selects `,`.
    #[serde(default)]
    pub delimiter: Option<String>,
}
