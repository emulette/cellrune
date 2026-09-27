from __future__ import annotations

import json
import pathlib

from cellrune import Workbook


def assert_function_usage() -> None:
    path = pathlib.Path(__file__).parents[3] / "binding-contract" / "function-usage-v2.json"
    corpus = json.loads(path.read_text(encoding="utf-8"))
    with Workbook.create() as workbook:
        workbook.apply_changes(0, [
            {
                "kind": "set_defined_name",
                "name": f"Usage_{depth}",
                "formula": "=SUM(1)" if depth == 0 else f"=Usage_{depth - 1}+Usage_{depth - 1}",
                "scope_sheet": None,
                "hidden": False,
            }
            for depth in range(corpus["max_depth"] + 1)
        ])
        for item in corpus["cases"]:
            workbook.set_formula("Sheet1", "A1", item["formula"])
            summary = workbook.summary()
            history = workbook.changes_since(0)
            usage = workbook.function_usage()
            assert usage["schema_version"] == corpus["schema_version"]
            assert usage["formula_count"] == 1
            assert usage["parsed_formula_count"] == 1
            assert usage["unparsed_formula_count"] == 0
            assert usage["entries"] == [{
                "name": "SUM", "supported": True, "call_count": int(item["call_count"]),
                "formula_count": 1,
                "sample_cells": [{"sheet_id": 1, "sheet_name": "Sheet1", "address": "A1"}],
            }]
            assert type(usage["entries"][0]["call_count"]) is int
            assert workbook.function_usage() == usage
            assert workbook.summary() == summary
            assert workbook.changes_since(0) == history
