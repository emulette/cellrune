from cellrune import CellRuneError, TargetCalculationResult, Workbook


def main() -> None:
    with Workbook.create() as workbook:
        workbook.set_number("Sheet1", "A1", 2)
        workbook.set_formula("Sheet1", "B1", "=A1+1")
        workbook.set_formula("Sheet1", "C1", "=B1*2")
        workbook.set_formula("Sheet1", "Z1", "=UNKNOWN_FUNCTION(1)")
        targets = [{"sheet": "Sheet1", "start": "B1", "end": "D1"}]
        result: TargetCalculationResult = workbook.calculate_targets(targets)
        assert result["scope"] == "targets"
        assert result["semantic_revision"] == workbook.summary()["semantic_revision"]
        assert result["evaluated_count"] == 2
        assert result["parsed_formula_count"] == 2
        assert [item["result"] for item in result["cells"]] == [
            {"kind": "value", "value": {"kind": "number", "value": 3}},
            {"kind": "value", "value": {"kind": "number", "value": 6}},
            {"kind": "value", "value": {"kind": "blank"}},
        ]
        assert workbook.read_range("Sheet1", "C1", "C1")["cells"][0]["calculated"] is None
        assert not workbook.changes_since()["deltas"]
        try:
            workbook.to_bytes()
        except CellRuneError as error:
            assert error.code == "interop.calculation.required"
        else:
            raise AssertionError("partial result enabled saving")
        workbook.recalculate()
        cached = workbook.calculate_targets(targets)
        assert cached["reused_count"] == 2
        assert cached["evaluated_count"] == 0
        workbook.set_number("Sheet1", "A1", 5)
        edited = workbook.calculate_targets([{"sheet": "Sheet1", "start": "C1"}])
        assert edited["cells"][0]["result"] == {"kind": "value", "value": {"kind": "number", "value": 12}}
        assert edited["reused_count"] == 0
        for invalid in ([], [{"sheet": "Sheet1", "start": "A1", "end": "XFD1048576"}]):
            try:
                workbook.calculate_targets(invalid)
            except CellRuneError:
                pass
            else:
                raise AssertionError("invalid targets accepted")
        try:
            workbook.calculate_targets(targets, limits={"max_result_cells": True})
        except CellRuneError:
            pass
        else:
            raise AssertionError("boolean limit accepted")


if __name__ == "__main__":
    main()
