//! Precedent and dependent query DTOs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::CellReferenceDto;

/// Bounded request for one page of a cell's precedents or dependents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellDependencyRequestDto {
    /// Case-insensitive sheet name.
    pub sheet: String,
    /// Unqualified A1 address of the cell.
    pub address: String,
    /// Zero-based entry offset.
    #[serde(default)]
    pub offset: u64,
    /// Page size; zero selects the default, with a hard maximum of 10,000.
    #[serde(default)]
    pub limit: u32,
}

/// One rectangle a formula reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrecedentDto {
    /// Stable workbook-local sheet identifier.
    pub sheet_id: u32,
    /// Resolved sheet name.
    pub sheet_name: String,
    /// The rectangle in A1 notation; one cell is a single address.
    pub range: String,
    /// `value` when the formula reads the cells' values, `spill` when it reads the spill range of
    /// the dynamic-array formula anchored at the cell (`A1#`), `formula_text` when it reads the
    /// cell's formula (`FORMULATEXT`, `ISFORMULA`), or `value_dependent` for the cells a
    /// reference end that moves with values, such as an `INDEX` range end, can reach.
    pub kind: String,
}

/// One page of the rectangles a cell's formula reads, from the installed calculation's graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrecedentPageDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// The queried cell.
    pub cell: CellReferenceDto,
    /// Whether the formula's references depend on values, as `INDIRECT` and `OFFSET` do, so the
    /// precedents are those the installed calculation's values resolved.
    pub dynamic: bool,
    /// Number of precedents; a cell without a formula has none.
    pub total: u64,
    /// Offset represented by the first returned precedent.
    pub offset: u64,
    /// Offset for the next page, or `None` at the end.
    pub next_offset: Option<u64>,
    /// Returned precedents in sheet, row, column, and kind order.
    pub entries: Vec<PrecedentDto>,
}

/// One formula that reads the queried cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DependentDto {
    /// The formula cell.
    pub cell: CellReferenceDto,
    /// Whether the formula's references depend on values, so it reads the cell under the
    /// installed calculation's values and may not after they change.
    pub dynamic: bool,
}

/// One page of the formulas that read a cell directly, from the installed calculation's graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DependentPageDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// The queried cell.
    pub cell: CellReferenceDto,
    /// Formulas with references that depend on values that are not listed: they do not read the
    /// cell under the installed calculation's values but can after a value changes, which the
    /// graph cannot resolve without calculating.
    pub unlisted_dynamic_formulas: u64,
    /// Number of dependents.
    pub total: u64,
    /// Offset represented by the first returned dependent.
    pub offset: u64,
    /// Offset for the next page, or `None` at the end.
    pub next_offset: Option<u64>,
    /// Returned dependents in sheet and address order.
    pub entries: Vec<DependentDto>,
}
