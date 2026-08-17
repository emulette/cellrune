use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Transport-safe arithmetic compatibility policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub enum ArithmeticSemanticsDto {
    /// Apply Excel's narrow near-zero correction to a proven cancellation.
    #[default]
    #[serde(rename = "excel_near_zero")]
    ExcelNearZero,
    /// Preserve the raw IEEE-754 result used through CellRune 0.1.2.
    #[serde(rename = "ieee_754")]
    Ieee754,
}

/// Transport-safe iterative financial solver policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FinancialSolverSemanticsDto {
    /// Apply Microsoft's function-specific iteration budget and tolerance.
    #[default]
    ExcelIterationBudget,
    /// Use the longer, tighter search used through CellRune 0.1.2.
    ExtendedSearch,
}

/// Deterministic inputs supplied to one calculation.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalculationOptionsDto {
    /// Excel serial returned by `TODAY()`.
    pub today_serial: Option<f64>,
    /// Excel serial returned by `NOW()`.
    pub now_serial: Option<f64>,
    /// Policy for cancelling addition and subtraction.
    #[serde(default)]
    pub arithmetic_semantics: ArithmeticSemanticsDto,
    /// Policy for `IRR`, `XIRR`, and `RATE` convergence.
    #[serde(default)]
    pub financial_solver_semantics: FinancialSolverSemanticsDto,
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
        /// Optional declared iterative-calculation flag.
        iterative_calculation: Option<bool>,
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

/// Table-specific operations added by edit schema v2.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TableChangeV2Dto {
    /// Renames a table by stable ID.
    RenameTable {
        /// Stable workbook-local table ID.
        table_id: u32,
        /// New programmatic and display name.
        new_display_name: String,
    },
    /// Renames one table column by stable IDs.
    RenameTableColumn {
        /// Stable workbook-local table ID.
        table_id: u32,
        /// Stable table-local column ID.
        column_id: u32,
        /// New column name.
        new_name: String,
    },
    /// Changes a table's inclusive data-body row range.
    ResizeTableRows {
        /// Stable workbook-local table ID.
        table_id: u32,
        /// First one-based data-body row.
        first_data_row: u32,
        /// Last one-based data-body row.
        last_data_row: u32,
    },
}

/// One v2 workbook mutation.
///
/// Existing v1 operations retain their exact tagged JSON shape; the second arm adds only the
/// three table-authoring operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum WorkbookChangeV2Dto {
    /// An unchanged edit-schema-v1 operation.
    V1(WorkbookChangeDto),
    /// A table-authoring operation introduced by edit schema v2.
    Table(TableChangeV2Dto),
}

/// Ordered, atomic edit-schema-v2 change set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditBatchV2Dto {
    /// Operations in caller-declared order.
    #[schemars(length(min = 1))]
    pub changes: Vec<WorkbookChangeV2Dto>,
}

/// Result of one committed edit-schema-v2 batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditReceiptV2Dto {
    /// The unchanged v1 receipt fields, flattened into the v2 JSON object.
    #[serde(flatten)]
    pub receipt: EditReceiptDto,
    /// Stable IDs of tables changed by the batch.
    pub changed_table_ids: Vec<u32>,
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

/// Request for one workbook or sheet-local defined-name inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DefinedNameInspectionRequestDto {
    /// Case-insensitive defined-name spelling.
    pub name: String,
    /// Optional case-insensitive current sheet used for local-name lookup and unqualified geometry.
    pub current_sheet: Option<String>,
}

/// Stable workbook-order identity of a continuous 3-D sheet span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DefinedNameSheetSpanDto {
    /// Stable identifier of the first sheet in workbook order.
    pub start_sheet_id: u32,
    /// Resolved name of the first sheet.
    pub start_sheet_name: String,
    /// Stable identifier of the final sheet in workbook order.
    pub end_sheet_id: u32,
    /// Resolved name of the final sheet.
    pub end_sheet_name: String,
}

/// One ordered area in a non-rectangular defined-name result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DefinedNameReferenceAreaDto {
    /// One rectangle on one worksheet.
    Rectangular {
        /// Stable workbook-local sheet identifier.
        sheet_id: u32,
        /// Resolved sheet name.
        sheet_name: String,
        /// Resolved unqualified A1 rectangle.
        range: String,
    },
    /// One rectangle repeated across a continuous worksheet span.
    ThreeDimensional {
        /// Stable workbook-order sheet span.
        sheet_span: DefinedNameSheetSpanDto,
        /// Resolved unqualified A1 rectangle shared by each sheet.
        range: String,
    },
}

/// Dynamic reference construct represented by interop schema version 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DefinedNameDynamicKindDto {
    /// The terminal reference expression is `OFFSET`.
    Offset,
    /// The terminal reference expression is `INDIRECT`.
    Indirect,
    /// The terminal reference expression is a spill reference.
    Spill,
    /// Multiple dynamic reference constructs contribute to the result.
    Mixed,
}

impl DefinedNameDynamicKindDto {
    /// Returns the stable transport spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Offset => "offset",
            Self::Indirect => "indirect",
            Self::Spill => "spill",
            Self::Mixed => "mixed",
        }
    }
}

/// External target category represented by interop schema version 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DefinedNameExternalTargetKindDto {
    /// A cell, area, whole-row, or whole-column reference.
    Reference,
    /// An external defined name.
    DefinedName,
    /// An external structured table reference.
    StructuredReference,
}

impl DefinedNameExternalTargetKindDto {
    /// Returns the stable transport spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::DefinedName => "defined_name",
            Self::StructuredReference => "structured_reference",
        }
    }
}

/// Defined-name invalidity represented by interop schema version 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DefinedNameInvalidReasonDto {
    /// The selected or reachable formula does not parse.
    ParseError,
    /// A non-callable value-name chain contains a cycle.
    CircularReference,
    /// A reachable name is absent from its applicable scope chain.
    UnresolvedName,
    /// A static reference names an absent sheet, table, column, or invalid range.
    InvalidReference,
}

impl DefinedNameInvalidReasonDto {
    /// Returns the stable transport spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ParseError => "parse_error",
            Self::CircularReference => "circular_reference",
            Self::UnresolvedName => "unresolved_name",
            Self::InvalidReference => "invalid_reference",
        }
    }
}

/// Unsupported defined-name category represented by interop schema version 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DefinedNameUnsupportedReasonDto {
    /// The formula is a callable or general non-reference expression.
    NonReferenceExpression,
    /// The result needs a current cell, calculated value, or other runtime state.
    ContextDependent,
    /// The typed AST is valid but outside the current inspection resolver.
    UnsupportedExpression,
}

impl DefinedNameUnsupportedReasonDto {
    /// Returns the stable transport spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NonReferenceExpression => "non_reference_expression",
            Self::ContextDependent => "context_dependent",
            Self::UnsupportedExpression => "unsupported_expression",
        }
    }
}

/// Typed result of inspecting one defined name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DefinedNameInspectionResultDto {
    /// The name resolves to one rectangle on one worksheet.
    Rectangular {
        /// Stable workbook-local sheet identifier.
        sheet_id: u32,
        /// Resolved sheet name.
        sheet_name: String,
        /// Resolved unqualified A1 rectangle.
        range: String,
    },
    /// The name resolves to one rectangle across a continuous worksheet span.
    ThreeDimensional {
        /// Stable workbook-order sheet span.
        sheet_span: DefinedNameSheetSpanDto,
        /// Resolved unqualified A1 rectangle shared by each sheet.
        range: String,
    },
    /// The name resolves to multiple ordered areas.
    NonRectangular {
        /// Areas in source order with duplicates and 3-D identity preserved.
        areas: Vec<DefinedNameReferenceAreaDto>,
    },
    /// The name resolves to a valid reference containing no cells.
    EmptyReference,
    /// The terminal reference shape depends on calculation state.
    DynamicFormula {
        /// `offset`, `indirect`, `spill`, or `mixed`.
        dynamic_kind: DefinedNameDynamicKindDto,
        /// Terminal definition formula with a leading equals sign.
        formula: String,
    },
    /// The terminal definition is dependency-free constant syntax.
    Constant {
        /// Terminal definition formula with a leading equals sign.
        formula: String,
    },
    /// The typed syntax addresses another workbook.
    ExternalReference {
        /// Optional path or URI prefix before the bracketed workbook token.
        locator: Option<String>,
        /// External workbook token without surrounding brackets or locator.
        workbook: String,
        /// Optional first external sheet token.
        sheet: Option<String>,
        /// Optional final external sheet token of a 3-D prefix.
        sheet_end: Option<String>,
        /// `reference`, `defined_name`, or `structured_reference`.
        target_kind: DefinedNameExternalTargetKindDto,
        /// Canonical external target without its workbook or sheet prefix.
        target_text: String,
    },
    /// The root or one reachable value-name definition is invalid.
    Invalid {
        /// Stable semantic invalidity code.
        reason: DefinedNameInvalidReasonDto,
        /// Optional source-specific detail.
        detail: Option<String>,
    },
    /// The valid formula cannot be represented as static reference geometry.
    Unsupported {
        /// Stable unsupported reason.
        reason: DefinedNameUnsupportedReasonDto,
        /// Optional source-specific detail.
        detail: Option<String>,
    },
    /// No root name exists in the selected lookup chain.
    NotFound,
}

/// Versioned response for one defined-name inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DefinedNameInspectionDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Typed semantic inspection result.
    pub result: DefinedNameInspectionResultDto,
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

/// One column of an Excel table, keyed by the stable XLSX column identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TableColumnDto {
    /// Stable XLSX column identifier; survives column renames.
    pub id: u32,
    /// Column name.
    pub name: String,
    /// OOXML totals-row token (`sum`, `average`, ...), absent when the column declares none.
    pub totals_row_function: Option<String>,
}

/// Summary of one Excel table owned by its worksheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TableSummaryDto {
    /// Stable non-zero OOXML table identifier, unique within the workbook.
    pub id: u32,
    /// Worksheet-local programmatic object-model name.
    pub name: String,
    /// Workbook-global formula and UI name; it may differ from `name`.
    pub display_name: String,
    /// Full table range in A1 notation, including header and totals rows.
    pub range: String,
    /// Declared header row count.
    pub header_row_count: u32,
    /// Declared totals row count.
    pub totals_row_count: u32,
    /// Columns in XLSX declaration order.
    pub columns: Vec<TableColumnDto>,
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
    /// Merged ranges in A1 notation, sorted by top-left address.
    ///
    /// `#[serde(default)]` is deliberate: an absent list means "no merges", which is
    /// semantically honest, so payloads from older producers keep deserializing.
    #[serde(default)]
    pub merged_ranges: Vec<String>,
    /// Tables owned by this sheet in XLSX declaration order.
    ///
    /// `#[serde(default)]` is deliberate for the same reason as `merged_ranges`.
    #[serde(default)]
    pub tables: Vec<TableSummaryDto>,
}

/// Transport-safe versioned workbook semantic fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkbookFingerprintDto {
    /// Semantic fingerprint schema version.
    pub schema_version: u16,
    /// Lower-case 64-character digest hexadecimal.
    pub digest_hex: String,
}

/// Bounded workbook metadata returned without cell contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkbookSummaryDto {
    /// Interop schema version.
    pub schema_version: u32,
    /// Current monotonic semantic revision.
    pub semantic_revision: u64,
    /// Versioned history-independent semantic fingerprint.
    pub fingerprint: WorkbookFingerprintDto,
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

#[cfg(test)]
mod tests {
    use super::{
        DefinedNameInspectionResultDto, TableChangeV2Dto, WorkbookChangeDto, WorkbookChangeV2Dto,
    };

    /// The DTOs carry `deny_unknown_fields`, so adding a field to a variant would be a breaking
    /// change to the JSON boundary if a missing one were also rejected. It is not: serde reads an
    /// absent `Option` as `None`. Interop is `publish = false` and reaches Python, Node.js, and
    /// MCP as JSON, so this is what makes a field addition non-breaking for those consumers.
    #[test]
    fn an_absent_optional_field_is_read_as_none() {
        let without_flag = r#"{
            "kind": "set_calculation_hints",
            "mode": "manual",
            "calculation_id": null,
            "full_calculation_on_load": null,
            "force_full_calculation": null
        }"#;
        let parsed: WorkbookChangeDto = serde_json::from_str(without_flag)
            .expect("a client that predates the field still parses");
        assert_eq!(
            parsed,
            WorkbookChangeDto::SetCalculationHints {
                mode: Some("manual".to_owned()),
                calculation_id: None,
                full_calculation_on_load: None,
                force_full_calculation: None,
                iterative_calculation: None,
            }
        );
    }

    #[test]
    fn an_unknown_field_is_still_rejected() {
        let misspelled = r#"{"kind": "set_calculation_hints", "iterativeCalculation": true}"#;
        serde_json::from_str::<WorkbookChangeDto>(misspelled)
            .expect_err("deny_unknown_fields must still catch a misspelled field");
    }

    #[test]
    fn empty_defined_name_reference_has_a_stable_tagged_shape() {
        assert_eq!(
            serde_json::to_value(DefinedNameInspectionResultDto::EmptyReference)
                .expect("empty reference serializes"),
            serde_json::json!({"kind": "empty_reference"})
        );
    }

    #[test]
    fn edit_v2_adds_table_operations_without_changing_v1_json_shapes() {
        let v1: WorkbookChangeV2Dto = serde_json::from_value(serde_json::json!({
            "kind": "rename_sheet",
            "sheet": "Old",
            "new_name": "New"
        }))
        .expect("v1 operation in v2");
        assert!(matches!(
            v1,
            WorkbookChangeV2Dto::V1(WorkbookChangeDto::RenameSheet { .. })
        ));
        let table: WorkbookChangeV2Dto = serde_json::from_value(serde_json::json!({
            "kind": "rename_table_column",
            "table_id": 7,
            "column_id": 3,
            "new_name": "Gross Amount"
        }))
        .expect("table operation");
        assert_eq!(
            table,
            WorkbookChangeV2Dto::Table(TableChangeV2Dto::RenameTableColumn {
                table_id: 7,
                column_id: 3,
                new_name: "Gross Amount".to_owned(),
            })
        );
        serde_json::from_value::<WorkbookChangeV2Dto>(serde_json::json!({
            "kind": "rename_table",
            "table_id": 7,
            "new_display_name": "Orders",
            "unexpected": true
        }))
        .expect_err("v2 table variants remain closed");
    }
}
