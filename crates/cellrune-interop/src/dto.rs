use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Deterministic inputs supplied to one calculation.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalculationOptionsDto {
    /// Excel serial returned by `TODAY()`.
    pub today_serial: Option<f64>,
    /// Excel serial returned by `NOW()`.
    pub now_serial: Option<f64>,
}

/// Requested stateful recalculation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecalculationModeDto {
    /// Select a safe incremental pass or fall back to full calculation.
    #[default]
    Auto,
    /// Require a safe incremental pass.
    Incremental,
    /// Evaluate every formula.
    Full,
}

/// One typed workbook mutation in an atomic interop batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkbookChangeDto {
    /// Sets a typed literal value.
    SetValue {
        /// Case-insensitive sheet name.
        sheet: String,
        /// Unqualified A1 address.
        address: String,
        /// New typed value.
        value: WritableCellValueDto,
    },
    /// Sets a normal or dynamic formula.
    SetFormula {
        /// Case-insensitive sheet name.
        sheet: String,
        /// Unqualified A1 address.
        address: String,
        /// User formula with a leading equals sign.
        formula: String,
        /// Optional dynamic spill range.
        dynamic_range: Option<String>,
    },
    /// Removes one sparse cell.
    ClearCell {
        /// Case-insensitive sheet name.
        sheet: String,
        /// Unqualified A1 address.
        address: String,
    },
    /// Replaces an existing cell number format.
    SetNumberFormat {
        /// Case-insensitive sheet name.
        sheet: String,
        /// Unqualified A1 address.
        address: String,
        /// Built-in or custom format identifier.
        id: u32,
        /// Custom format code; omitted for built-ins.
        code: Option<String>,
        /// Semantic format kind.
        format_kind: String,
    },
    /// Adds a visible empty sheet.
    AddSheet {
        /// New unique sheet name.
        name: String,
    },
    /// Renames one sheet and rewrites stored references.
    RenameSheet {
        /// Current case-insensitive sheet name.
        sheet: String,
        /// New unique sheet name.
        new_name: String,
    },
    /// Changes one sheet's visibility.
    SetSheetVisibility {
        /// Case-insensitive sheet name.
        sheet: String,
        /// `visible`, `hidden`, or `very_hidden`.
        visibility: String,
    },
    /// Adds or replaces a workbook or sheet-scoped defined name.
    SetDefinedName {
        /// Defined-name spelling.
        name: String,
        /// Optional case-insensitive sheet scope; omitted for workbook scope.
        scope_sheet: Option<String>,
        /// Formula with a leading equals sign.
        formula: String,
        /// Whether normal spreadsheet UI hides the name.
        hidden: bool,
    },
    /// Removes a workbook or sheet-scoped defined name.
    RemoveDefinedName {
        /// Defined-name spelling.
        name: String,
        /// Optional case-insensitive sheet scope; omitted for workbook scope.
        scope_sheet: Option<String>,
    },
    /// Changes the workbook date system.
    SetDateSystem {
        /// `excel_1900` or `excel_1904`.
        date_system: String,
    },
    /// Changes workbook calculation metadata.
    SetCalculationHints {
        /// Optional calculation mode.
        mode: Option<String>,
        /// Optional producer calculation identifier.
        calculation_id: Option<u32>,
        /// Optional full-calculation-on-load flag.
        full_calculation_on_load: Option<bool>,
        /// Optional force-full-calculation flag.
        force_full_calculation: Option<bool>,
    },
}

/// Ordered, atomic workbook change set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditBatchDto {
    /// Operations in caller-declared order.
    #[schemars(length(min = 1))]
    pub changes: Vec<WorkbookChangeDto>,
}

/// Result of one committed edit batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditReceiptDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Revision checked before commit.
    pub base_revision: u64,
    /// Revision installed after commit.
    pub result_revision: u64,
    /// Number of ordered operations applied.
    pub applied_change_count: u64,
    /// Cells whose source content or format changed.
    pub changed_cells: Vec<CellReferenceDto>,
    /// Cells whose source value or formula changed calculation semantics.
    pub calculation_changed_cells: Vec<CellReferenceDto>,
    /// Sheet IDs allocated by add-sheet operations.
    pub created_sheet_ids: Vec<u32>,
    /// Whether formula, name, or sheet topology changed.
    pub topology_changed: bool,
    /// Whether workbook-wide calculation interpretation changed.
    pub calculation_metadata_changed: bool,
}

/// Save behavior exposed across language boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteOptionsDto {
    /// Whether unavailable formulas have their stale caches invalidated instead of failing save.
    pub invalidate_unavailable: bool,
    /// Whether a path save may replace an existing destination.
    pub replace_existing: bool,
}

/// A transport-safe spreadsheet value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CellValueDto {
    /// Empty cell value.
    Blank,
    /// Finite numeric value.
    Number {
        /// Numeric payload.
        value: f64,
    },
    /// Unicode text value.
    Text {
        /// Text payload.
        value: String,
    },
    /// Boolean value.
    Logical {
        /// Boolean payload.
        value: bool,
    },
    /// Canonical Excel error value.
    Error {
        /// Error display such as `#DIV/0!`.
        value: String,
    },
    /// A value introduced by a newer core cannot be represented by this schema version.
    Unsupported,
}

/// A transport-safe spreadsheet value accepted by edit operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WritableCellValueDto {
    /// Empty cell value.
    Blank,
    /// Finite numeric value.
    Number {
        /// Numeric payload.
        value: f64,
    },
    /// Unicode text value.
    Text {
        /// Text payload.
        value: String,
    },
    /// Boolean value.
    Logical {
        /// Boolean payload.
        value: bool,
    },
    /// Canonical Excel error value.
    Error {
        /// Error display such as `#DIV/0!`.
        value: String,
    },
}

/// How the source value on a returned cell was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SavedValueStateDto {
    /// The cell stores a non-formula literal.
    Literal,
    /// A formula stores a valid saved result.
    Saved,
    /// A formula has no usable saved result.
    Missing,
    /// A formula has a saved result that could not be interpreted.
    Invalid,
}

/// One deterministic formula result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CalculationResultDto {
    /// A typed spreadsheet result was produced.
    Value {
        /// Calculated value.
        value: CellValueDto,
    },
    /// Calculation was deliberately withheld.
    Unavailable {
        /// Stable CellRune calculation issue code.
        code: String,
        /// Shared human-readable issue message.
        message: String,
        /// Optional source-specific context.
        detail: Option<String>,
    },
}

/// One changed direct or materialized result in a stateful calculation delta.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalculationDeltaCellDto {
    /// Changed cell.
    pub cell: CellReferenceDto,
    /// `direct_formula`, `legacy_array`, or `dynamic_spill`.
    pub origin: String,
    /// Array anchor for materialized array results.
    pub anchor: Option<CellReferenceDto>,
    /// Complete materialized range for array results.
    pub range: Option<String>,
    /// New typed result or stable issue.
    pub result: CalculationResultDto,
}

/// Bounded result changes from one installed stateful calculation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalculationDeltaDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Monotonic installed-delta cursor.
    pub cursor: u64,
    /// Prior installed result revision.
    pub base_revision: u64,
    /// Workbook revision calculated by this delta.
    pub result_revision: u64,
    /// `incremental` or `full`.
    pub mode: String,
    /// Stable mode-selection reason.
    pub reason: String,
    /// Formula count invalidated before calculation.
    pub dirty_count: u64,
    /// Formula count whose evaluator ran.
    pub evaluated_count: u64,
    /// Formula count parsed while preparing this pass.
    pub parsed_formula_count: u64,
    /// Cells whose result or materialization origin changed.
    pub changed_cells: Vec<CalculationDeltaCellDto>,
    /// Cells removed from the materialization view.
    pub removed_materialized_cells: Vec<CellReferenceDto>,
}

/// Cursor page of complete calculation deltas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalculationDeltaPageDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Exclusive cursor supplied by the caller.
    pub requested_cursor: u64,
    /// Cursor for the next page, or `None` when caught up.
    pub next_cursor: Option<u64>,
    /// Complete deltas in ascending cursor order.
    pub deltas: Vec<CalculationDeltaDto>,
}

/// One cell in a paged range response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellDto {
    /// Unqualified A1 address.
    pub address: String,
    /// Formula with a leading `=`, when this is a formula cell.
    pub formula: Option<String>,
    /// Literal or saved source value.
    pub source_value: CellValueDto,
    /// Source-value interpretation state.
    pub source_value_state: SavedValueStateDto,
    /// Current calculation result, including dynamic spill followers, when calculated.
    pub calculated: Option<CalculationResultDto>,
}

/// Bounded range-read request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangeRequestDto {
    /// Case-insensitive sheet name.
    pub sheet: String,
    /// Inclusive range start in unqualified A1 notation.
    pub start: String,
    /// Inclusive range end in unqualified A1 notation.
    pub end: String,
    /// Zero-based row-major cell offset.
    #[serde(default)]
    pub offset: u64,
    /// Requested page size. Zero selects the documented default.
    #[serde(default)]
    pub limit: u32,
}

/// One bounded row-major page of cells.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RangePageDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Resolved sheet name.
    pub sheet: String,
    /// Inclusive range start.
    pub start: String,
    /// Inclusive range end.
    pub end: String,
    /// Total cell count in the requested rectangle.
    pub total_cells: u64,
    /// Offset represented by the first returned cell.
    pub offset: u64,
    /// Offset for the next page, or `None` at the end.
    pub next_offset: Option<u64>,
    /// Returned cells in row-major order, including blanks.
    pub cells: Vec<CellDto>,
}

/// Stable workbook-local cell reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CellReferenceDto {
    /// Stable workbook-local sheet identifier.
    pub sheet_id: u32,
    /// Resolved sheet name.
    pub sheet_name: String,
    /// Unqualified A1 address.
    pub address: String,
}

/// Summary of one worksheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetSummaryDto {
    /// Stable workbook-local sheet identifier.
    pub id: u32,
    /// Sheet name.
    pub name: String,
    /// `visible`, `hidden`, or `very_hidden`.
    pub visibility: String,
    /// Number of stored sparse cells.
    pub cell_count: u64,
    /// Smallest used rectangle, when the sheet is non-empty.
    pub used_range: Option<String>,
}

/// Bounded workbook metadata returned without cell contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkbookSummaryDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Current monotonic semantic revision.
    pub semantic_revision: u64,
    /// Whether the session preserves an opened XLSX or XLSM package.
    pub document_backed: bool,
    /// `xlsx`, `xlsm`, or `new_xlsx`.
    pub document_kind: String,
    /// Workbook date system.
    pub date_system: String,
    /// Read-time compatibility diagnostic count.
    pub diagnostic_count: u64,
    /// Sheets in workbook order.
    pub sheets: Vec<SheetSummaryDto>,
}

/// Result counts for one completed calculation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalculationReportDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Workbook revision used for the result.
    pub semantic_revision: u64,
    /// Direct formula result count.
    pub formula_count: u64,
    /// Direct formula values produced.
    pub value_count: u64,
    /// Direct formulas whose result was withheld.
    pub unavailable_count: u64,
    /// Formula and array materialization cell count.
    pub materialized_cell_count: u64,
}

/// One statically scanned formula capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityEntryDto {
    /// Formula cell.
    pub cell: CellReferenceDto,
    /// Whether the formula is supported under current deterministic inputs.
    pub supported: bool,
    /// Stable issue codes when unsupported.
    pub issue_codes: Vec<String>,
}

/// Paged static capability scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityPageDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Total formula count.
    pub formula_count: u64,
    /// Supported formula count.
    pub supported_count: u64,
    /// Zero-based entry offset.
    pub offset: u64,
    /// Offset for the next page, or `None`.
    pub next_offset: Option<u64>,
    /// Returned entries.
    pub entries: Vec<CapabilityEntryDto>,
}

/// Aggregated use of one normalized function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FunctionUsageEntryDto {
    /// Uppercase normalized function name.
    pub name: String,
    /// Whether the function has an implemented kernel.
    pub supported: bool,
    /// Total call count.
    pub call_count: u64,
    /// Distinct formula-cell count.
    pub formula_count: u64,
    /// Deterministic bounded cell samples.
    pub sample_cells: Vec<CellReferenceDto>,
}

/// Workbook-level function demand report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FunctionUsageReportDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Formula cells inspected.
    pub formula_count: u64,
    /// Formula cells whose syntax could be analyzed.
    pub parsed_formula_count: u64,
    /// Formula cells excluded due to parse failures.
    pub unparsed_formula_count: u64,
    /// Entries ordered by normalized name.
    pub entries: Vec<FunctionUsageEntryDto>,
}

/// One accepted name in the calculation function catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FunctionCatalogEntryDto {
    /// Accepted Excel-facing name.
    pub name: String,
    /// Canonical calculation-kernel name.
    pub canonical_name: String,
    /// Whether the name is a compatibility alias.
    pub alias: bool,
    /// Whether the function may return a multi-cell array.
    pub returns_array: bool,
    /// Whether the name is in the tracked official function list.
    pub official: bool,
}

/// Versioned function catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FunctionCatalogReportDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Deterministically ordered accepted names.
    pub entries: Vec<FunctionCatalogEntryDto>,
}

/// Result of saving a verified workbook package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteReportDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Whether every required result was materialized.
    pub complete: bool,
    /// Unavailable-result policy used.
    pub policy: String,
    /// Typed direct-formula, legacy-array-region, and dynamic-spill-region cells written.
    pub materialized_count: u64,
    /// Cells whose stale saved result was invalidated.
    pub invalidated_cells: Vec<CellReferenceDto>,
    /// Package parts rewritten.
    pub changed_parts: Vec<String>,
    /// Package parts intentionally removed.
    pub removed_parts: Vec<String>,
    /// Write diagnostic count.
    pub diagnostic_count: u64,
}
