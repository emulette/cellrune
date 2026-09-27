from __future__ import annotations

import json
import pathlib

from cellrune import Workbook


def assert_compatibility() -> None:
    path = pathlib.Path(__file__).parents[3] / "binding-contract" / "compatibility-v021.json"
    corpus = json.loads(path.read_text(encoding="utf-8"))
    with Workbook.create() as workbook:
        workbook.set_number("Sheet1", "A1", corpus["initial_input"])
        for item in corpus["cases"]:
            workbook.set_formula("Sheet1", item["address"], item["formula"])
        targets = workbook.calculate_targets([{"sheet": "Sheet1", "start": "B1", "end": "B9"}])
        assert targets["evaluated_count"] == len(corpus["cases"])
        expected = [{"kind": "value", "value": item["initial"]} for item in corpus["cases"]]
        assert [cell["result"] for cell in targets["cells"]] == expected
        workbook.recalculate(mode="full")
        initial = workbook.read_range("Sheet1", "B1", "B9")
        assert [cell["calculated"] for cell in initial["cells"]] == expected
        summary = workbook.summary()
        history = workbook.changes_since()
        preview = workbook.preview_changes(summary["semantic_revision"], [{
            "kind": "set_value", "sheet": "Sheet1", "address": "A1",
            "value": {"kind": "number", "value": corpus["edited_input"]},
        }])
        details = workbook.preview_changes_page(preview["preview_id"], section="preview_results")
        preview_values = {
            entry["cell"]["address"]: entry["result"]
            for entry in details["items"] if entry["kind"] == "preview_result"
        }
        for item in corpus["cases"]:
            if item["initial"] != item["edited"]:
                assert preview_values[item["address"]] == {"kind": "value", "value": item["edited"]}
        assert workbook.summary() == summary
        assert workbook.changes_since() == history
        assert workbook.read_range("Sheet1", "B1", "B9") == initial
        workbook.discard_preview(preview["preview_id"])
        workbook.set_number("Sheet1", "A1", corpus["edited_input"])
        workbook.recalculate(mode="auto")
        expected = [{"kind": "value", "value": item["edited"]} for item in corpus["cases"]]
        assert [cell["calculated"] for cell in workbook.read_range("Sheet1", "B1", "B9")["cells"]] == expected
        with Workbook.from_bytes(workbook.to_bytes()) as reopened:
            assert [cell["source_value"] for cell in reopened.read_range("Sheet1", "B1", "B9")["cells"]] == [
                item["edited"] for item in corpus["cases"]
            ]
