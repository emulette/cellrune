from cellrune import (
    CalculationDelta,
    CellRuneError,
    CellValue,
    EditReceipt,
    RangePage,
    Workbook,
    WorkbookChange,
)


def check() -> None:
    with Workbook.create() as scoped:
        scoped.summary()
    assert scoped.closed

    workbook: Workbook = Workbook.create()
    workbook.set_number("Sheet1", "A1", 1.0)
    workbook.set_formula("Sheet1", "B1", "=A1+1")
    workbook.calculate()
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
    delta: CalculationDelta = workbook.recalculate(mode="incremental")
    assert delta["result_revision"] == receipt["result_revision"]
    workbook.changes_since(0)
    page: RangePage = workbook.read_range("Sheet1", "A1", "B1")
    value: CellValue = page["cells"][0]["source_value"]
    if value["kind"] == "number":
        value["value"].is_integer()
    output: bytes = workbook.to_bytes()
    reopened: Workbook = Workbook.from_bytes(output)
    reopened.close()


def inspect_error(error: CellRuneError) -> str:
    return f"{error.kind}:{error.code}"


if __name__ == "__main__":
    check()
