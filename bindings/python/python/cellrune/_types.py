"""Runtime-visible result types for the public CellRune Python API."""

from typing import Literal, TypedDict


class ErrorDetails(TypedDict):
    source_code: str | None
    source_id: str | None
    detail: str | None


class BlankValue(TypedDict):
    kind: Literal["blank"]


class NumberValue(TypedDict):
    kind: Literal["number"]
    value: float


class TextValue(TypedDict):
    kind: Literal["text"]
    value: str


class LogicalValue(TypedDict):
    kind: Literal["logical"]
    value: bool


class ErrorValue(TypedDict):
    kind: Literal["error"]
    value: str


class UnsupportedValue(TypedDict):
    kind: Literal["unsupported"]


CellValue = (
    BlankValue
    | NumberValue
    | TextValue
    | LogicalValue
    | ErrorValue
    | UnsupportedValue
)


class CalculatedValue(TypedDict):
    kind: Literal["value"]
    value: CellValue


class UnavailableValue(TypedDict):
    kind: Literal["unavailable"]
    code: str
    message: str
    detail: str | None


CalculationResult = CalculatedValue | UnavailableValue


class Cell(TypedDict):
    address: str
    formula: str | None
    source_value: CellValue
    source_value_state: Literal["literal", "saved", "missing", "invalid"]
    calculated: CalculationResult | None


class RangePage(TypedDict):
    schema_version: int
    sheet: str
    start: str
    end: str
    total_cells: int
    offset: int
    next_offset: int | None
    cells: list[Cell]


class DefinedNameSheetSpan(TypedDict):
    start_sheet_id: int
    start_sheet_name: str
    end_sheet_id: int
    end_sheet_name: str


class DefinedNameRectangularArea(TypedDict):
    kind: Literal["rectangular"]
    sheet_id: int
    sheet_name: str
    range: str


class DefinedNameThreeDimensionalArea(TypedDict):
    kind: Literal["three_dimensional"]
    sheet_span: DefinedNameSheetSpan
    range: str


DefinedNameReferenceArea = (
    DefinedNameRectangularArea | DefinedNameThreeDimensionalArea
)


class DefinedNameRectangularResult(TypedDict):
    kind: Literal["rectangular"]
    sheet_id: int
    sheet_name: str
    range: str


class DefinedNameThreeDimensionalResult(TypedDict):
    kind: Literal["three_dimensional"]
    sheet_span: DefinedNameSheetSpan
    range: str


class DefinedNameNonRectangularResult(TypedDict):
    kind: Literal["non_rectangular"]
    areas: list[DefinedNameReferenceArea]


class DefinedNameEmptyReferenceResult(TypedDict):
    kind: Literal["empty_reference"]


class DefinedNameDynamicFormulaResult(TypedDict):
    kind: Literal["dynamic_formula"]
    dynamic_kind: Literal["offset", "indirect", "spill", "mixed"]
    formula: str


class DefinedNameConstantResult(TypedDict):
    kind: Literal["constant"]
    formula: str


class DefinedNameExternalReferenceResult(TypedDict):
    kind: Literal["external_reference"]
    locator: str | None
    workbook: str
    sheet: str | None
    sheet_end: str | None
    target_kind: Literal["reference", "defined_name", "structured_reference"]
    target_text: str


class DefinedNameInvalidResult(TypedDict):
    kind: Literal["invalid"]
    reason: Literal[
        "parse_error", "circular_reference", "unresolved_name", "invalid_reference"
    ]
    detail: str | None


class DefinedNameUnsupportedResult(TypedDict):
    kind: Literal["unsupported"]
    reason: Literal[
        "non_reference_expression", "context_dependent", "unsupported_expression"
    ]
    detail: str | None


class DefinedNameNotFoundResult(TypedDict):
    kind: Literal["not_found"]


DefinedNameInspectionResult = (
    DefinedNameRectangularResult
    | DefinedNameThreeDimensionalResult
    | DefinedNameNonRectangularResult
    | DefinedNameEmptyReferenceResult
    | DefinedNameDynamicFormulaResult
    | DefinedNameConstantResult
    | DefinedNameExternalReferenceResult
    | DefinedNameInvalidResult
    | DefinedNameUnsupportedResult
    | DefinedNameNotFoundResult
)


class DefinedNameInspection(TypedDict):
    schema_version: int
    result: DefinedNameInspectionResult


class TableColumn(TypedDict):
    id: int
    name: str
    totals_row_function: str | None


class TableSummary(TypedDict):
    id: int
    name: str
    display_name: str
    range: str
    header_row_count: int
    totals_row_count: int
    columns: list[TableColumn]


class SheetSummary(TypedDict):
    id: int
    name: str
    visibility: Literal["visible", "hidden", "very_hidden"]
    cell_count: int
    used_range: str | None
    merged_ranges: list[str]
    tables: list[TableSummary]


class WorkbookFingerprint(TypedDict):
    schema_version: int
    digest_hex: str


class WorkbookSummary(TypedDict):
    schema_version: int
    semantic_revision: int
    fingerprint: WorkbookFingerprint
    document_backed: bool
    document_kind: Literal["xlsx", "xlsm", "new_xlsx", "open_xml"]
    date_system: Literal["excel_1900", "excel_1904"]
    diagnostic_count: int
    sheets: list[SheetSummary]


class CalculationReport(TypedDict):
    schema_version: int
    semantic_revision: int
    formula_count: int
    value_count: int
    unavailable_count: int
    materialized_cell_count: int


class CellReference(TypedDict):
    sheet_id: int
    sheet_name: str
    address: str


WritableCellValue = BlankValue | NumberValue | TextValue | LogicalValue | ErrorValue


class SetValueChange(TypedDict):
    kind: Literal["set_value"]
    sheet: str
    address: str
    value: WritableCellValue


class SetFormulaChangeOptions(TypedDict, total=False):
    dynamic_range: str | None


class SetFormulaChange(SetFormulaChangeOptions):
    kind: Literal["set_formula"]
    sheet: str
    address: str
    formula: str


class ClearCellChange(TypedDict):
    kind: Literal["clear_cell"]
    sheet: str
    address: str


class SetNumberFormatChangeOptions(TypedDict, total=False):
    code: str | None


class SetNumberFormatChange(SetNumberFormatChangeOptions):
    kind: Literal["set_number_format"]
    sheet: str
    address: str
    id: int
    format_kind: Literal[
        "general", "number", "date", "time", "date_time", "duration"
    ]


class AddSheetChange(TypedDict):
    kind: Literal["add_sheet"]
    name: str


class RenameSheetChange(TypedDict):
    kind: Literal["rename_sheet"]
    sheet: str
    new_name: str


class SetSheetVisibilityChange(TypedDict):
    kind: Literal["set_sheet_visibility"]
    sheet: str
    visibility: Literal["visible", "hidden", "very_hidden"]


class SetDefinedNameChangeOptions(TypedDict, total=False):
    scope_sheet: str | None


class SetDefinedNameChange(SetDefinedNameChangeOptions):
    kind: Literal["set_defined_name"]
    name: str
    formula: str
    hidden: bool


class RemoveDefinedNameChangeOptions(TypedDict, total=False):
    scope_sheet: str | None


class RemoveDefinedNameChange(RemoveDefinedNameChangeOptions):
    kind: Literal["remove_defined_name"]
    name: str


class SetDateSystemChange(TypedDict):
    kind: Literal["set_date_system"]
    date_system: Literal["excel_1900", "excel_1904"]


class SetCalculationHintsChangeOptions(TypedDict, total=False):
    mode: Literal["automatic", "automatic_except_data_tables", "manual"] | None
    calculation_id: int | None
    full_calculation_on_load: bool | None
    force_full_calculation: bool | None
    iterative_calculation: bool | None


class SetCalculationHintsChange(SetCalculationHintsChangeOptions):
    kind: Literal["set_calculation_hints"]


WorkbookChange = (
    SetValueChange
    | SetFormulaChange
    | ClearCellChange
    | SetNumberFormatChange
    | AddSheetChange
    | RenameSheetChange
    | SetSheetVisibilityChange
    | SetDefinedNameChange
    | RemoveDefinedNameChange
    | SetDateSystemChange
    | SetCalculationHintsChange
)

class RenameTableChange(TypedDict):
    kind: Literal["rename_table"]
    table_id: int
    new_display_name: str


class RenameTableColumnChange(TypedDict):
    kind: Literal["rename_table_column"]
    table_id: int
    column_id: int
    new_name: str


class ResizeTableRowsChange(TypedDict):
    kind: Literal["resize_table_rows"]
    table_id: int
    first_data_row: int
    last_data_row: int


WorkbookChangeV2 = (
    WorkbookChange
    | RenameTableChange
    | RenameTableColumnChange
    | ResizeTableRowsChange
)


class EditReceipt(TypedDict):
    schema_version: int
    base_revision: int
    result_revision: int
    applied_change_count: int
    changed_cells: list[CellReference]
    calculation_changed_cells: list[CellReference]
    created_sheet_ids: list[int]
    topology_changed: bool
    calculation_metadata_changed: bool


class EditReceiptV2(EditReceipt):
    changed_table_ids: list[int]


class CalculationDeltaCell(TypedDict):
    cell: CellReference
    origin: Literal[
        "direct_formula", "legacy_array", "dynamic_spill", "unknown"
    ]
    anchor: CellReference | None
    range: str | None
    result: CalculationResult


class CalculationDelta(TypedDict):
    schema_version: int
    cursor: int
    base_revision: int
    result_revision: int
    mode: Literal["incremental", "full"]
    reason: Literal[
        "initial_calculation",
        "full_requested",
        "incremental_requested",
        "dirty_subset",
        "no_dirty_formulas",
        "topology_changed",
        "options_changed",
        "dynamic_topology",
        "dirty_set_covers_workbook",
        "unknown",
    ]
    dirty_count: int
    evaluated_count: int
    parsed_formula_count: int
    changed_cells: list[CalculationDeltaCell]
    removed_materialized_cells: list[CellReference]


class CalculationDeltaPage(TypedDict):
    schema_version: int
    requested_cursor: int
    next_cursor: int | None
    deltas: list[CalculationDelta]


class FunctionUsageEntry(TypedDict):
    name: str
    supported: bool
    call_count: int
    formula_count: int
    sample_cells: list[CellReference]


class FunctionUsageReport(TypedDict):
    schema_version: int
    formula_count: int
    parsed_formula_count: int
    unparsed_formula_count: int
    entries: list[FunctionUsageEntry]


class FunctionCatalogEntry(TypedDict):
    name: str
    canonical_name: str
    alias: bool
    returns_array: bool
    official: bool


class FunctionCatalogReport(TypedDict):
    schema_version: int
    entries: list[FunctionCatalogEntry]


class WriteReport(TypedDict):
    schema_version: int
    complete: bool
    policy: Literal["require_complete", "invalidate_unavailable", "unknown"]
    materialized_count: int
    invalidated_cells: list[CellReference]
    changed_parts: list[str]
    removed_parts: list[str]
    diagnostic_count: int


class CalculationLimits(TypedDict):
    max_formula_tokens: int
    max_formula_source_bytes: int
    max_formula_ast_nodes: int
    max_formula_nesting_depth: int
    max_dependency_edges: int
    max_reference_areas: int
    max_array_cells: int
    max_text_bytes: int
    max_function_iterations: int
    max_let_bindings: int
    max_lambda_depth: int
    max_lambda_invocations: int


class TransactionCalculationOptions(TypedDict):
    today_serial: float | None
    now_serial: float | None
    arithmetic_semantics: Literal["excel_near_zero", "ieee_754"]
    financial_solver_semantics: Literal[
        "excel_iteration_budget", "extended_search"
    ]
    limits: CalculationLimits


class ProviderIdentity(TypedDict):
    name: str
    version: str


class PreviewCursor(TypedDict):
    preview_id: int
    token: str


class TransactionDetailCounts(TypedDict):
    affected: int
    evaluated: int
    preview_results: int
    preview_issues: int
    install_results: int


class MaterializedResultOrigin(TypedDict):
    kind: Literal["direct_formula", "legacy_array", "dynamic_spill", "unknown"]
    anchor: CellReference | None
    range: str | None


class CalculationIssue(TypedDict):
    code: str
    message: str
    detail: str | None


class AffectedTransactionDetail(TypedDict):
    kind: Literal["affected"]
    cell: CellReference
    cause: str


class EvaluatedTransactionDetail(TypedDict):
    kind: Literal["evaluated"]
    cell: CellReference


class PreviewResultTransactionDetail(TypedDict):
    kind: Literal["preview_result"]
    cell: CellReference
    previous_origin: MaterializedResultOrigin | None
    previous_result: CalculationResult | None
    result_origin: MaterializedResultOrigin | None
    result: CalculationResult | None


class PreviewIssueTransactionDetail(TypedDict):
    kind: Literal["preview_issue"]
    cell: CellReference
    change_kind: Literal["introduced", "resolved", "changed"]
    previous: CalculationIssue | None
    current: CalculationIssue | None


class InstallResultTransactionDetail(TypedDict):
    kind: Literal["install_result"]
    cell: CellReference
    origin: MaterializedResultOrigin | None
    result: CalculationResult | None


class UnknownTransactionDetail(TypedDict):
    kind: Literal["unknown"]


TransactionDetailItem = (
    AffectedTransactionDetail
    | EvaluatedTransactionDetail
    | PreviewResultTransactionDetail
    | PreviewIssueTransactionDetail
    | InstallResultTransactionDetail
    | UnknownTransactionDetail
)


class TransactionReport(TypedDict):
    contract_version: int
    base_revision: int
    result_revision: int
    base_fingerprint: WorkbookFingerprint
    result_fingerprint: WorkbookFingerprint
    input_sha256: str | None
    calculator_provider: ProviderIdentity
    calculation_options: TransactionCalculationOptions
    base_calculation_reused: bool
    base_execution_mode: str
    base_decision_reason: str
    candidate_requested_mode: Literal["auto", "incremental", "full"]
    candidate_execution_mode: str
    candidate_decision_reason: str
    edit_receipt: EditReceipt
    impact_coverage: Literal["exact", "conservative_full"]
    direct_affected_count: int
    transitive_affected_count: int
    conservative_affected_count: int
    base_evaluated_count: int
    candidate_evaluated_count: int
    parsed_formula_count: int
    function_iteration_count: int
    reference_cell_count: int
    preview_changed_count: int
    preview_removed_count: int
    introduced_issue_count: int
    resolved_issue_count: int
    changed_issue_count: int
    install_delta_count: int
    installed_calculation_revision: int | None
    installed_calculation_fingerprint: WorkbookFingerprint | None
    installed_calculation_options: TransactionCalculationOptions | None
    install_delta_basis_differs_from_preview_base: bool
    install_delta_basis_reasons: list[str]
    detail_counts: TransactionDetailCounts


class PreviewChanges(TypedDict):
    schema_version: int
    preview_id: int
    report: TransactionReport


class TransactionImpactPage(TypedDict):
    schema_version: int
    preview_id: int
    section: Literal[
        "affected", "evaluated", "preview_results", "preview_issues", "install_results"
    ]
    items: list[TransactionDetailItem]
    next_cursor: PreviewCursor | None
    total_count: int


class WorkbookTransactionReceipt(TypedDict):
    schema_version: int
    edit: EditReceipt
    calculation_delta: CalculationDelta
    base_fingerprint: WorkbookFingerprint
    result_fingerprint: WorkbookFingerprint
