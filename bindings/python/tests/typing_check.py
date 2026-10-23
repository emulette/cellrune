from cellrune import (
    CapabilityPage,
    CellFormatPage,
    CommentPage,
    HyperlinkPage,
    SetCellFormatChange,
    DataValidationPage,
    CalculationLimitOptions,
    LocaleProfile,
    ReadLimitOptions,
    TargetCalculationResult,
    CalculationDelta,
    CellRuneError,
    CellValue,
    ConvertTableToRangeChange,
    CopySheetChange,
    CreateTableChange,
    DefinedNameInspection,
    EditReceipt,
    EditReceiptV2,
    EditWriteReport,
    FunctionCatalogReport,
    MoveSheetChange,
    PreviewChanges,
    PreviewCursor,
    RangePage,
    RenameDefinedNameChange,
    SetSheetAutoFilterChange,
    SetTableRangeChange,
    TableSummary,
    TransactionImpactPage,
    Workbook,
    WorkbookChange,
    WorkbookChangeV2,
    WorkbookTransactionReceipt,
    format_display_text,
    WriteReport,
    function_catalog,
)

table_summary: TableSummary = {
    "id": 1,
    "name": "SalesObject",
    "display_name": "Sales",
    "range": "A1:B3",
    "header_row_count": 1,
    "totals_row_count": 0,
    "columns": [
        {"id": 1, "name": "Region", "totals_row_function": None},
        {"id": 2, "name": "Amount", "totals_row_function": "sum"},
    ],
}


def check() -> None:
    partial: TargetCalculationResult = Workbook.create().calculate_targets([{"sheet": "Sheet1", "start": "A1"}], limits={"max_result_cells": 10})
    partial["cells"][0]["cell"]["address"].upper()
    catalog: FunctionCatalogReport = function_catalog()
    catalog["entries"][0]["canonical_name"].upper()
    with Workbook.create() as scoped:
        scoped.summary()
    assert scoped.closed

    workbook: Workbook = Workbook.create()
    workbook.set_number("Sheet1", "A1", 1.0)
    workbook.set_formula("Sheet1", "B1", "=A1+1")
    workbook.calculate()
    workbook.calculate(
        arithmetic_semantics="ieee_754",
        financial_solver_semantics="extended_search",
    )
    changes: list[WorkbookChange] = [
        {
            "kind": "set_value",
            "sheet": "Sheet1",
            "address": "A1",
            "value": {"kind": "number", "value": 2.0},
        }
    ]
    receipt: EditReceipt = workbook.apply_changes(
        workbook.summary()["semantic_revision"], changes
    )
    assert isinstance(receipt["calculation_changed_cells"], list)
    assert isinstance(receipt["calculation_metadata_changed"], bool)
    delta: CalculationDelta = workbook.recalculate(
        mode="incremental",
        arithmetic_semantics="ieee_754",
        financial_solver_semantics="extended_search",
    )
    preview: PreviewChanges = workbook.preview_changes(delta["result_revision"], changes)
    preview_page: TransactionImpactPage = workbook.preview_changes_page(
        preview["preview_id"], section="preview_results", limit=1
    )
    cursor: PreviewCursor | None = preview_page["next_cursor"]
    if cursor is not None:
        cursor["token"].upper()
    transaction_receipt: WorkbookTransactionReceipt = workbook.commit_preview(
        preview["preview_id"]
    )
    assert transaction_receipt["calculation_delta"]["result_revision"] == preview["report"]["result_revision"]
    assert delta["result_revision"] == receipt["result_revision"]
    workbook.changes_since(0)
    page: RangePage = workbook.read_range("Sheet1", "A1", "B1")
    inspection: DefinedNameInspection = workbook.inspect_defined_name(
        "InputArea", current_sheet="Sheet1"
    )
    inspected = inspection["result"]
    if inspected["kind"] == "rectangular":
        inspected["sheet_name"].upper()
        inspected["range"].upper()
    if inspected["kind"] == "external_reference":
        if inspected["locator"] is not None:
            inspected["locator"].upper()
        inspected["workbook"].upper()
        inspected["target_text"].upper()
    value: CellValue = page["cells"][0]["source_value"]
    if value["kind"] == "number":
        value["value"].is_integer()
    output: bytes = workbook.to_bytes()
    limits: CalculationLimitOptions = {"max_array_cells": 2_000_000}
    workbook.calculate(calculation_limits=limits)
    korean: LocaleProfile = {"locale": "ko-KR", "display_language": "en-US"}
    workbook.calculate(locale_profile=korean)
    shown: str = format_display_text(
        {"kind": "number", "value": 1.5}, "0.00", date_system="excel_1904", locale_profile=korean
    )
    shown.upper()
    read_limits: ReadLimitOptions = {"max_entry_uncompressed_bytes": 1 << 27}
    reopened: Workbook = Workbook.from_bytes(output, read_limits=read_limits)
    edited: bytes = reopened.edits_to_bytes()
    capabilities: CapabilityPage = reopened.scan_capabilities(limit=10)
    for entry in capabilities["entries"]:
        entry["cell"]["address"].upper()
    formats: CellFormatPage = reopened.read_formats("Sheet1", "A1", "B2", limit=4)
    for formatted in formats["cells"]:
        fill = formatted["format"]["fill"]
        if fill["kind"] == "pattern" and fill["foreground"] is not None:
            fill["foreground"]["kind"].upper()
    bold: SetCellFormatChange = {
        "kind": "set_cell_format",
        "sheet": "Sheet1",
        "address": "A1",
        "font": {"bold": True},
    }
    reopened.apply_changes_v2(reopened.summary()["semantic_revision"], [bold])
    csv_workbook = Workbook.from_csv(b"1,2", delimiter=",")
    csv_workbook.calculate()
    exported: bytes = csv_workbook.to_csv("Sheet1")
    exported.hex()
    links: HyperlinkPage = reopened.hyperlinks("Sheet1")
    for link in links["entries"]:
        link["range"].upper()
    notes: CommentPage = reopened.comments("Sheet1", offset=0)
    for note in notes["entries"]:
        note["text"].upper()
    validations: DataValidationPage = reopened.data_validations("Sheet1", limit=10)
    for rule in validations["entries"]:
        rule["ranges"][0].upper()
        if rule["validation_type"] == "list" and rule["formula1"] is not None:
            rule["formula1"].upper()
    edited.hex()
    reopened.add_sheet("Scratch")
    removal: list[WorkbookChange] = [{"kind": "delete_sheet", "sheet": "Scratch"}]
    reopened.apply_changes(reopened.summary()["semantic_revision"], removal)
    reopened.add_sheet("Other")
    reopened.delete_sheet("Other")
    copy: CopySheetChange = {"kind": "copy_sheet", "sheet": "Sheet1", "position": 1}
    move: MoveSheetChange = {"kind": "move_sheet", "sheet": "Sheet1 (2)", "position": 0}
    sheet_filter: SetSheetAutoFilterChange = {
        "kind": "set_sheet_auto_filter",
        "sheet": "Sheet1",
        "range": "A1:B2",
    }
    rename: RenameDefinedNameChange = {
        "kind": "rename_defined_name",
        "name": "Typed",
        "scope_sheet": None,
        "new_name": "Renamed",
    }
    structure: list[WorkbookChange] = [
        copy,
        move,
        sheet_filter,
        {"kind": "set_defined_name", "name": "Typed", "formula": "=Sheet1!$A$1", "hidden": False},
        rename,
        {"kind": "set_sheet_auto_filter", "sheet": "Sheet1"},
    ]
    copied: EditReceipt = reopened.apply_changes(
        reopened.summary()["semantic_revision"], structure
    )
    copied["created_sheet_ids"][0].bit_length()
    create: CreateTableChange = {
        "kind": "create_table",
        "sheet": "Sheet1",
        "range": "D1:E3",
        "name": "Scores",
    }
    created: EditReceiptV2 = reopened.apply_changes_v2(
        reopened.summary()["semantic_revision"], [create]
    )
    table_id = created["changed_table_ids"][0]
    resize: SetTableRangeChange = {"kind": "set_table_range", "table_id": table_id, "range": "D1:E4"}
    convert: ConvertTableToRangeChange = {"kind": "convert_table_to_range", "table_id": table_id}
    reopened.apply_changes_v2(reopened.summary()["semantic_revision"], [resize, convert])
    reopened.close()
    write_report: WriteReport = workbook.save("typing-check.xlsx")
    write_report["output_sha256"].upper()


def check_edit_report(workbook: Workbook) -> None:
    report: EditWriteReport = workbook.save_edits("edited.xlsx", replace_existing=True)
    report["input_sha256"].upper()
    report["affected_formulas"][0]["address"].upper()
    if report["requests_host_recalculation"]:
        report["changed_parts"].sort()


def check_v2_types(workbook: Workbook, revision: int) -> None:
    changes_v2: list[WorkbookChangeV2] = [
        {
            "kind": "rename_table_column",
            "table_id": 1,
            "column_id": 2,
            "new_name": "Gross Amount",
        }
    ]
    receipt_v2: EditReceiptV2 = workbook.apply_changes_v2(revision, changes_v2)
    assert isinstance(receipt_v2["changed_table_ids"], list)


def inspect_error(error: CellRuneError) -> str:
    return f"{error.kind}:{error.code}"


if __name__ == "__main__":
    check()
