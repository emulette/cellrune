from __future__ import annotations

import ast
import pathlib

from cellrune import (
    SCHEMA_VERSION,
    CellRuneError,
    CellValue,
    RangePage,
    Workbook,
)

STUB_PATH = (
    pathlib.Path(__file__).parents[1] / "python" / "cellrune" / "_native.pyi"
)


def main() -> None:
    module = ast.parse(STUB_PATH.read_text(encoding="utf-8"))
    workbook = next(
        node
        for node in module.body
        if isinstance(node, ast.ClassDef) and node.name == "Workbook"
    )
    stub_methods = {
        node.name
        for node in workbook.body
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }
    runtime_methods = set(dir(Workbook))
    assert stub_methods <= runtime_methods
    assert SCHEMA_VERSION == 1
    assert CellValue is not None
    assert RangePage.__name__ == "RangePage"

    session = Workbook.create()
    try:
        session.set_number("Sheet1", "A0", 1.0)
    except CellRuneError as error:
        assert error.code == "validation.row_out_of_range"
        assert error.kind == "validation"
        assert set(error.details) == {"source_code", "source_id", "detail"}
    else:
        raise AssertionError("typed exception was not raised")


if __name__ == "__main__":
    main()
