from cellrune import (
    CalculationDelta,
    CellRuneError,
    CellValue,
    DefinedNameInspection,
    EditReceipt,
    EditReceiptV2,
    RangePage,
    TableSummary,
    Workbook,
    WorkbookChange,
    WorkbookChangeV2,
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
    reopened: Workbook = Workbook.from_bytes(output)
    reopened.close()


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
