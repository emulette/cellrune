import io
import zipfile

from cellrune import CellRuneError, TargetCalculationResult, Workbook


def saved_precedent_fixture() -> bytes:
    """One sheet whose B1 calls WEBSERVICE, which CellRune does not calculate, with the saved
    result 42, and whose C1 reads it."""
    parts = {
        "[Content_Types].xml": '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>',
        "_rels/.rels": '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>',
        "xl/workbook.xml": '<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>',
        "xl/_rels/workbook.xml.rels": '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>',
        "xl/worksheets/sheet1.xml": '<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>5</v></c><c r="B1"><f>WEBSERVICE("https://example.com/"&amp;A1)</f><v>42</v></c><c r="C1"><f>B1*2</f><v>84</v></c></row></sheetData></worksheet>',
    }
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_STORED) as archive:
        for name, contents in parts.items():
            archive.writestr(name, contents)
    return buffer.getvalue()


def assert_saved_precedents() -> None:
    with Workbook.from_bytes(saved_precedent_fixture()) as workbook:
        target = [{"sheet": "Sheet1", "start": "C1"}]
        default = workbook.calculate_targets(target)["cells"][0]
        assert default["result"]["kind"] == "unavailable"
        assert "saved_precedents" not in default
        saved = workbook.calculate_targets(target, uncalculable_precedents="saved_result")["cells"][0]
        assert saved["result"] == {"kind": "value", "value": {"kind": "number", "value": 84.0}}
        assert [precedent["address"] for precedent in saved.get("saved_precedents", [])] == ["B1"]


def assert_marked_minimal_save() -> None:
    target = [{"sheet": "Sheet1", "start": "C1"}]
    for mark, expected in ((True, "value"), (False, "unavailable")):
        with Workbook.from_bytes(saved_precedent_fixture()) as workbook:
            revision = workbook.summary()["semantic_revision"]
            workbook.apply_changes_v2(
                revision,
                [{"kind": "set_formula", "sheet": "Sheet1", "address": "Z1", "formula": "=1+1", "dynamic_range": None}],
            )
            edited = workbook.edits_to_bytes(saved_result_mark=mark)
        with Workbook.from_bytes(edited) as reopened:
            cell = reopened.calculate_targets(target, uncalculable_precedents="saved_result")["cells"][0]
            assert cell["result"]["kind"] == expected, (mark, cell)


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
        for target in (["Sheet1", "B1"], ("Sheet1", "B1"), "Sheet1"):
            try:
                workbook.calculate_targets([target])
            except CellRuneError as error:
                assert error.code == "interop.calculation.target.invalid_payload", (target, error.code)
            else:
                raise AssertionError(f"{target} accepted as a target")
        for limits in ([1], [], "1", 1):
            try:
                workbook.calculate_targets(targets, limits=limits)
            except CellRuneError as error:
                assert error.code == "interop.calculation.target.invalid_payload", (limits, error.code)
            else:
                raise AssertionError(f"{limits} accepted")
        # Targeted calculation keeps the default calculation limits.
        for options in ({"calculation_limits": {"max_array_cells": 1}}, {"unknown_option": 1}):
            try:
                workbook.calculate_targets(targets, **options)
            except CellRuneError as error:
                assert error.code == "interop.calculation.target.invalid_payload", error.code
            else:
                raise AssertionError(f"{options} accepted")


if __name__ == "__main__":
    main()
    assert_saved_precedents()
    assert_marked_minimal_save()
