//! Transport contract for immutable, bounded partial calculation.

use crate::{
    CalculationOptionsDto, CalculationOptionsReportDto, CalculationResultDto, CellReferenceDto,
    ProviderIdentityDto, WorkbookFingerprintDto,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One cell or inclusive rectangle to calculate on a named sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalculationTargetDto {
    /// Case-insensitive sheet name.
    pub sheet: String,
    /// Unqualified A1 address of the first cell.
    pub start: String,
    /// Last cell; omission requests only `start`.
    #[serde(default)]
    pub end: Option<String>,
}

/// Request-wide bounds, separate from the formula kernel limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TargetCalculationLimitsDto {
    /// Maximum number of input rectangles. Default: 1024.
    pub max_targets: u32,
    /// Maximum distinct returned cells. Default: 10000.
    pub max_result_cells: u32,
    /// Maximum evaluator invocations, including dynamic retries. Default: 100000.
    pub max_evaluated_cells: u32,
}

impl Default for TargetCalculationLimitsDto {
    fn default() -> Self {
        Self {
            max_targets: 1024,
            max_result_cells: 10000,
            max_evaluated_cells: 100000,
        }
    }
}

/// Explicit targets and deterministic inputs for one partial calculation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetCalculationRequestDto {
    /// Requested cells and rectangles; overlap is deduplicated.
    pub targets: Vec<CalculationTargetDto>,
    /// Deterministic numeric and volatile-function inputs.
    #[serde(default)]
    pub options: CalculationOptionsDto,
    /// Request-wide resource limits.
    #[serde(default)]
    pub limits: TargetCalculationLimitsDto,
}

/// Explicit marker that the result covers requested cells only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TargetCalculationScopeDto {
    /// Requested cells, with required precedents evaluated internally.
    Targets,
}

/// One requested cell value or issue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetCalculationCellDto {
    /// Stable identity and current address of the requested cell.
    pub cell: CellReferenceDto,
    /// Calculated value or stable unavailable issue.
    pub result: CalculationResultDto,
}

/// Complete bounded response to a partial calculation, never an installed workbook cache.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetCalculationResultDto {
    /// Version of the serialized interop contract.
    pub schema_version: u32,
    /// Explicit partial-result scope.
    pub scope: TargetCalculationScopeDto,
    /// Revision of the immutable source workbook.
    pub semantic_revision: u64,
    /// Semantic identity of the source workbook.
    pub source_fingerprint: WorkbookFingerprintDto,
    /// Original package hash, when available.
    pub input_sha256: Option<String>,
    /// Calculator identity and version.
    pub calculator_provider: ProviderIdentityDto,
    /// Complete deterministic options and formula limits.
    pub calculation_options: CalculationOptionsReportDto,
    /// Request-wide resource limits.
    pub limits: TargetCalculationLimitsDto,
    /// Requested cells, deduplicated and sorted by sheet ID, row, then column.
    pub cells: Vec<TargetCalculationCellDto>,
    /// Evaluator invocations, including precedents and dynamic retries.
    pub evaluated_count: u64,
    /// Formula cells parsed for this request.
    pub parsed_formula_count: u64,
    /// Requested materialized results reused from a current complete calculation.
    pub reused_count: u64,
}
