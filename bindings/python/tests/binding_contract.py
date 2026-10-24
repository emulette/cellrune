from __future__ import annotations

import hashlib
import json
import pathlib
import tempfile
from collections.abc import Callable
from typing import Literal, cast

from cellrune import (
    CalculationLimitOptions,
    CellRuneError,
    LocaleProfile,
    ReadLimitOptions,
    Workbook,
    format_display_text,
    function_catalog,
)
from function_usage import assert_function_usage
from compatibility import assert_compatibility

CORPUS_PATH = pathlib.Path(__file__).parents[3] / "binding-contract" / "v1.json"
DEFINED_NAME_CORPUS_PATH = (
    pathlib.Path(__file__).parents[3] / "binding-contract" / "defined-name-v1.json"
)
SHEET_METADATA_FIXTURE_PATH = (
    pathlib.Path(__file__).parents[3] / "binding-contract" / "sheet-metadata.xlsx"
)
SURFACE_AUTHORING_FIXTURE_PATH = (
    pathlib.Path(__file__).parents[3] / "binding-contract" / "surface-authoring.xlsx"
)
TABLE_AUTHORING_CONTRACT_PATH = (
    pathlib.Path(__file__).parents[3]
    / "binding-contract"
    / "table-authoring-v2.json"
)
ArithmeticSemantics = Literal["excel_near_zero", "ieee_754"]
FinancialSolverSemantics = Literal["excel_iteration_budget", "extended_search"]


def assert_catalog_contract() -> None:
    catalog = function_catalog()
    assert catalog["schema_version"] == 1
    assert len(catalog["entries"]) == 498
    entries = {entry["name"]: entry for entry in catalog["entries"]}
    assert all(
        name in entries
        for name in (
            "CODE", "NUMBERVALUE", "DDB", "XNPV",
            "MDETERM", "RANK.AVG", "FORECAST.LINEAR", "FORECAST",
            "NORM.INV", "NORM.S.INV", "NORMINV", "NORMSINV",
            "LOGNORM.DIST", "LOGNORM.INV", "LOGNORMDIST", "LOGINV",
            "PERCENTILE.EXC", "QUARTILE.EXC", "PERCENTRANK.EXC", "STDEVA", "STDEVPA",
            "VARA", "VARPA", "STEYX", "TRIMMEAN", "FISHER", "FISHERINV", "PROB",
            "CHISQ.DIST", "CHISQ.DIST.RT", "CHIDIST", "WEIBULL.DIST", "WEIBULL",
            "CONFIDENCE.NORM", "CONFIDENCE", "CONFIDENCE.T", "IMLOG10", "IMLOG2",
            "BETA.DIST", "BETA.INV", "BETADIST", "BETAINV", "BINOM.DIST",
            "BINOM.DIST.RANGE", "BINOM.INV", "BINOMDIST", "CRITBINOM", "GAMMA",
            "GAMMA.DIST", "GAMMA.INV", "GAMMADIST", "GAMMAINV", "GAMMALN",
            "GAMMALN.PRECISE", "HYPGEOM.DIST", "HYPGEOMDIST", "NEGBINOM.DIST",
            "NEGBINOMDIST",
            "F.DIST", "F.DIST.RT", "F.INV", "F.INV.RT", "F.TEST", "FDIST",
            "FINV", "FTEST",
            "T.DIST", "T.DIST.2T", "T.DIST.RT", "T.INV", "T.INV.2T", "T.TEST",
            "TDIST", "TINV", "TTEST",
            "Z.TEST", "ZTEST", "COVARIANCE.S",
            "CONVERT", "BESSELI", "BESSELJ", "BESSELK", "BESSELY",
            "COMPLEX", "IMABS", "IMAGINARY", "IMARGUMENT", "IMCONJUGATE",
            "IMREAL", "IMDIV", "IMPOWER", "IMPRODUCT", "IMSUB", "IMSUM",
            "IMEXP", "IMLN", "IMSQRT",
            "ACCRINT", "ACCRINTM", "COUPDAYBS", "COUPDAYS", "COUPDAYSNC",
            "COUPNCD", "COUPNUM", "COUPPCD", "DISC", "DURATION", "INTRATE",
            "MDURATION", "ODDFPRICE", "ODDFYIELD", "ODDLPRICE", "ODDLYIELD",
            "PRICE", "PRICEDISC", "PRICEMAT", "RECEIVED", "TBILLEQ",
            "TBILLPRICE", "TBILLYIELD", "YIELD", "YIELDDISC", "YIELDMAT",
            "DATEVALUE", "NETWORKDAYS.INTL", "TIMEVALUE", "WORKDAY.INTL",
            "RAND", "RANDBETWEEN", "RANDARRAY", "FIXED",
            "KURT", "SKEW", "SKEW.P",
            "FREQUENCY", "MODE.MULT",
            "CHISQ.TEST", "CHITEST", "CHISQ.INV", "CHISQ.INV.RT", "CHIINV",
            "CUMIPMT", "CUMPRINC", "VDB",
            "IMCOS", "IMCOSH", "IMCOT", "IMCSC", "IMCSCH", "IMSEC", "IMSECH", "IMSIN", "IMSINH",
            "IMTAN",
            "ENCODEURL", "FILTERXML",
            "CELL",
            "AMORDEGRC", "AMORLINC",
            "BAHTTEXT",
            "ASC", "DBCS", "FINDB", "LEFTB", "LENB", "MIDB", "REPLACEB", "RIGHTB",
            "SEARCHB", "AGGREGATE",
        )
    )
    assert all(
        entries[name]["returns_array"]
        for name in (
            "GROWTH", "LINEST", "LOGEST", "MINVERSE", "MUNIT", "RANDARRAY", "TREND",
            "XLOOKUP",
            "FREQUENCY", "MODE.MULT",
        )
    )
    assert all(entries[name]["official"] for name in entries if name != "__XLUDF.DUMMYFUNCTION")


def assert_error(code: str, operation: Callable[[], object]) -> None:
    try:
        operation()
    except CellRuneError as error:
        assert error.code == code
        assert error.kind == "input"
    else:
        raise AssertionError(f"{code} was not raised")


def read_with_invalid_offset(workbook: Workbook, value: object) -> object:
    return workbook.read_range(
        "Sheet1",
        "A1",
        "A1",
        offset=cast(int, value),
    )


def read_with_invalid_limit(workbook: Workbook, value: object) -> object:
    return workbook.read_range(
        "Sheet1",
        "A1",
        "A1",
        limit=cast(int, value),
    )


def set_invalid_number(workbook: Workbook, value: object) -> None:
    workbook.set_number("Sheet1", "A3", cast(float, value))


def calculate_with_invalid_today(workbook: Workbook, value: object) -> object:
    return workbook.calculate(today_serial=cast(float, value))


def recalculate_with_invalid_now(workbook: Workbook, value: object) -> object:
    return workbook.recalculate(now_serial=cast(float, value))

def calculate_with_invalid_seed(workbook: Workbook, value: object) -> object:
    return workbook.calculate(random_seed=cast(int, value))


def assert_random_seed() -> None:
    workbook = Workbook.create()
    workbook.set_formula("Sheet1", "A1", "=RAND()")
    workbook.set_formula("Sheet1", "A2", "=RANDBETWEEN(1,1000000)")

    def numbers() -> list[float]:
        return [
            cell["calculated"]["value"]["value"]
            for cell in workbook.read_range("Sheet1", "A1", "A2")["cells"]
        ]

    assert workbook.calculate()["unavailable_count"] == 2
    assert [
        cell["calculated"]["code"]
        for cell in workbook.read_range("Sheet1", "A1", "A2")["cells"]
    ] == ["calculation.volatile_input_missing"] * 2

    assert workbook.calculate(random_seed=7)["unavailable_count"] == 0
    first = numbers()
    assert 0 <= first[0] < 1 and 1 <= first[1] <= 1_000_000
    workbook.recalculate(mode="full", random_seed=7)
    assert numbers() == first
    targeted = workbook.calculate_targets(
        [{"sheet": "Sheet1", "start": "A1", "end": "A2"}], random_seed=7
    )
    assert [cell["result"]["value"]["value"] for cell in targeted["cells"]] == first
    workbook.calculate(random_seed=8)
    assert numbers() != first

    preview = workbook.preview_changes(
        workbook.summary()["semantic_revision"],
        [
            {
                "kind": "set_value",
                "sheet": "Sheet1",
                "address": "B1",
                "value": {"kind": "number", "value": 1.0},
            }
        ],
        random_seed=7,
    )
    assert preview["report"]["calculation_options"]["random_seed"] == 7
    workbook.discard_preview(preview["preview_id"])
    default_preview = workbook.preview_changes(
        workbook.summary()["semantic_revision"],
        [
            {
                "kind": "set_value",
                "sheet": "Sheet1",
                "address": "B1",
                "value": {"kind": "number", "value": 1.0},
            }
        ],
    )
    assert "locale_profile" not in default_preview["report"]["calculation_options"]
    workbook.discard_preview(default_preview["preview_id"])
    for invalid in (-1, 1.5, 2**32, True, "7"):
        assert_error(
            "interop.value.number_invalid",
            lambda invalid=invalid: calculate_with_invalid_seed(workbook, invalid),
        )


def calculate_with_invalid_locale(workbook: Workbook, value: object) -> object:
    return workbook.calculate(locale_profile=cast(LocaleProfile, value))


def target_with_invalid_locale(workbook: Workbook, value: object) -> object:
    return workbook.calculate_targets(
        [{"sheet": "Sheet1", "start": "A1"}], locale_profile=cast(LocaleProfile, value)
    )


def assert_locale_profile() -> None:
    workbook = Workbook.create()
    workbook.set_formula("Sheet1", "A1", "=DOLLAR(1234.567,2)")
    workbook.set_formula("Sheet1", "A2", '=LENB("가나")')
    korean: LocaleProfile = {"locale": "ko-KR"}

    def values() -> list[object]:
        return [
            cell["calculated"]["value"]["value"]
            for cell in workbook.read_range("Sheet1", "A1", "A2")["cells"]
        ]

    workbook.calculate()
    assert values() == ["$1,234.57", 2]
    workbook.calculate(locale_profile=korean)
    assert values() == ["₩1,234.57", 4]
    workbook.recalculate(mode="full")
    assert values() == ["$1,234.57", 2]
    workbook.recalculate(mode="full", locale_profile=korean)
    assert values() == ["₩1,234.57", 4]
    targeted = workbook.calculate_targets(
        [{"sheet": "Sheet1", "start": "A1"}], locale_profile=korean
    )
    assert targeted["cells"][0]["result"]["value"]["value"] == "₩1,234.57"
    # An explicit display_language of None is absent and follows the locale.
    workbook.calculate(locale_profile={"locale": "ko-KR", "display_language": None})
    assert values() == ["₩1,234.57", 4]

    preview = workbook.preview_changes(
        workbook.summary()["semantic_revision"],
        [
            {
                "kind": "set_value",
                "sheet": "Sheet1",
                "address": "B1",
                "value": {"kind": "number", "value": 1.0},
            }
        ],
        locale_profile={"locale": "ko-KR", "display_language": "en-US"},
    )
    assert preview["report"]["calculation_options"]["locale_profile"] == {
        "locale": "ko-KR",
        "display_language": "en-US",
    }
    workbook.discard_preview(preview["preview_id"])
    for invalid in (
        "ko-KR",
        {"locale": "ja-JP"},
        {"display_language": "ko-KR"},
        {"locale": "ko-KR", "region": "KR"},
        {"locale": 1},
    ):
        for operation in (calculate_with_invalid_locale, target_with_invalid_locale):
            assert_error(
                "interop.calculation.locale_profile_invalid",
                lambda operation=operation, invalid=invalid: operation(workbook, invalid),
            )


def assert_display_format() -> None:
    assert format_display_text({"kind": "number", "value": 1234.567}, "#,##0.00") == "1,234.57"
    assert (
        format_display_text({"kind": "number", "value": 0.0}, "yyyy-mm-dd", date_system="excel_1904")
        == "1904-01-01"
    )
    assert format_display_text({"kind": "text", "value": "abc"}, '"name: "@') == "name: abc"
    assert format_display_text({"kind": "logical", "value": True}, "0") == "TRUE"
    assert_error(
        "display_format.unsupported",
        lambda: format_display_text(
            {"kind": "number", "value": 46230.0}, "mmmm", locale_profile={"locale": "ko-KR"}
        ),
    )
    assert_error(
        "interop.display_format.request_invalid",
        lambda: format_display_text(
            {"kind": "number", "value": 1.0},
            "0",
            date_system=cast(Literal["excel_1900"], "excel_1901"),
        ),
    )
    assert_error(
        "interop.calculation.locale_profile_invalid",
        lambda: format_display_text(
            {"kind": "number", "value": 1.0},
            "0",
            locale_profile=cast(LocaleProfile, {"locale": "ja-JP"}),
        ),
    )


def assert_exact_numbers() -> None:
    # Both have 17 significant digits; a JSON parser without correct rounding reads them one
    # ULP away.
    serial = 45158.311281307004
    number = 1234.5678901234567
    workbook = Workbook.create()
    workbook.set_formula("Sheet1", "A1", "=NOW()")
    workbook.set_formula("Sheet1", "A2", "=TODAY()")
    targeted = workbook.calculate_targets(
        [{"sheet": "Sheet1", "start": "A1", "end": "A2"}],
        now_serial=serial,
        today_serial=serial,
    )
    assert [cell["result"]["value"]["value"] for cell in targeted["cells"]] == [
        serial,
        serial,
    ]

    def set_number(address: str) -> list[dict[str, object]]:
        return [
            {
                "kind": "set_value",
                "sheet": "Sheet1",
                "address": address,
                "value": {"kind": "number", "value": number},
            }
        ]

    preview = workbook.preview_changes(
        workbook.summary()["semantic_revision"], set_number("B1")
    )
    workbook.commit_preview(preview["preview_id"])
    workbook.apply_changes(workbook.summary()["semantic_revision"], set_number("B2"))
    workbook.apply_changes_v2(workbook.summary()["semantic_revision"], set_number("B3"))
    assert [
        cell["source_value"]["value"]
        for cell in workbook.read_range("Sheet1", "B1", "B3")["cells"]
    ] == [number, number, number]


def assert_volatile_input_recalculation() -> None:
    workbook = Workbook.create()
    workbook.set_number("Sheet1", "A1", 1.0)
    workbook.set_formula("Sheet1", "B1", "=TODAY()")
    workbook.set_formula("Sheet1", "C1", "=RAND()+B1")
    workbook.set_formula("Sheet1", "D1", "=A1+1")
    initial = workbook.recalculate(today_serial=45_000.0, random_seed=1)
    assert initial["mode"] == "full"

    for seed, changed in ((1, ["B1", "C1"]), (2, ["C1"])):
        delta = workbook.recalculate(today_serial=45_001.0, random_seed=seed)
        assert delta["mode"] == "incremental"
        assert delta["reason"] == "dirty_subset"
        assert delta["evaluated_count"] == len(changed)
        assert [cell["cell"]["address"] for cell in delta["changed_cells"]] == changed

    forced = workbook.recalculate(mode="incremental", today_serial=45_001.0, random_seed=3)
    assert forced["reason"] == "incremental_requested"
    assert forced["evaluated_count"] == 1
    full = workbook.recalculate(
        today_serial=45_001.0, random_seed=3, arithmetic_semantics="ieee_754"
    )
    assert full["mode"] == "full"
    assert full["reason"] == "options_changed"


def calculate_with_invalid_arithmetic_semantics(
    workbook: Workbook, value: str
) -> object:
    return workbook.calculate(
        arithmetic_semantics=cast(ArithmeticSemantics, value)
    )


def recalculate_with_invalid_solver_semantics(
    workbook: Workbook, value: str
) -> object:
    return workbook.recalculate(
        financial_solver_semantics=cast(FinancialSolverSemantics, value)
    )


def assert_incomplete_save_names_cells() -> None:
    workbook = Workbook.create()
    workbook.set_formula("Sheet1", "B2", "=MYSTERY(1)")
    workbook.calculate()
    try:
        workbook.to_bytes()
    except CellRuneError as error:
        assert error.code == "xlsx.write.incomplete_calculation"
        assert error.details["cells"] == [
            {"sheet_id": 1, "sheet_name": "Sheet1", "address": "B2"}
        ]
    else:
        raise AssertionError("an incomplete save did not raise CellRuneError")


def assert_edits_save() -> None:
    source = Workbook.create()
    source.set_number("Sheet1", "A1", 2.0)
    source.set_formula("Sheet1", "B1", "=A1*2")
    source.set_formula("Sheet1", "C1", "=MYSTERY(D1)")
    source.calculate()
    workbook = Workbook.from_bytes(source.to_bytes(invalidate_unavailable=True))
    summary = workbook.summary()
    assert summary["full_calculation_on_load"] is True
    assert summary["sheets"][0]["auto_filter"] is None
    assert "auto_filter_filtered_columns" not in summary["sheets"][0]
    assert "hidden_rows" not in summary["sheets"][0]
    workbook.set_number("Sheet1", "A1", 3.0)

    edited = workbook.edits_to_bytes()
    with tempfile.TemporaryDirectory() as temporary_directory:
        saved_path = pathlib.Path(temporary_directory) / "edited.xlsx"
        report = workbook.save_edits(saved_path)
        assert saved_path.read_bytes() == edited
        try:
            workbook.save_edits(saved_path)
        except CellRuneError as error:
            assert error.code == "xlsx.write.destination_exists"
        else:
            raise AssertionError("an existing destination was replaced without permission")
        workbook.save_edits(saved_path, replace_existing=True)
    assert report["affected_formulas"] == [
        {"sheet_id": 1, "sheet_name": "Sheet1", "address": "B1"}
    ]
    assert report["requests_host_recalculation"] is True
    assert report["output_sha256"] == hashlib.sha256(edited).hexdigest()
    reopened = Workbook.from_bytes(edited)
    cell = reopened.read_range("Sheet1", "A1", "A1")["cells"][0]
    assert cell["source_value"] == {"kind": "number", "value": 3.0}
    assert cell["number_format"] == {"id": 0, "code": "General", "kind": "general"}

    try:
        Workbook.create().edits_to_bytes()
    except CellRuneError as error:
        assert error.code == "xlsx.write.invalid_package_plan"
    else:
        raise AssertionError("a created workbook saved edits without a source package")


def assert_limits() -> None:
    workbook = Workbook.create()
    workbook.set_formula("Sheet1", "A1", "=SUM(B1:B200)")
    assert workbook.calculate(calculation_limits={"max_array_cells": 100})["unavailable_count"] == 1
    assert workbook.calculate()["unavailable_count"] == 0
    for limits, code in [
        ({"max_aray_cells": 100}, "interop.limit.payload_invalid"),
        ({"max_array_cells": -1}, "interop.limit.payload_invalid"),
        ({"max_array_cells": None}, "interop.limit.payload_invalid"),
        ([100], "interop.limit.payload_invalid"),
        ((100,), "interop.limit.payload_invalid"),
        ([], "interop.limit.payload_invalid"),
        ({"max_array_cells": 0}, "interop.limit.invalid"),
    ]:
        try:
            workbook.calculate(calculation_limits=cast(CalculationLimitOptions, limits))
        except CellRuneError as error:
            assert error.code == code, (limits, error.code)
            assert error.kind == "input"
        else:
            raise AssertionError(f"{limits} was accepted")

    source = Workbook.create()
    source.set_number("Sheet1", "A1", 1.0)
    source.calculate()
    package = source.to_bytes()
    try:
        Workbook.from_bytes(package, read_limits={"max_entry_uncompressed_bytes": 64})
    except CellRuneError as error:
        assert error.code == "xlsx.entry_too_large"
    else:
        raise AssertionError("a lowered read limit did not reject the package")
    Workbook.from_bytes(package, read_limits={"max_entry_uncompressed_bytes": 1 << 20})
    for read_limits in ([1], [], {"max_entry_uncompressed_bytes": None}):
        try:
            Workbook.from_bytes(package, read_limits=cast(ReadLimitOptions, read_limits))
        except CellRuneError as error:
            assert error.code == "interop.limit.payload_invalid", (read_limits, error.code)
        else:
            raise AssertionError(f"{read_limits} was accepted")


def assert_capability_scan() -> None:
    workbook = Workbook.create()
    workbook.set_formula("Sheet1", "A1", "=SUM(1,2)")
    workbook.set_formula("Sheet1", "A2", "=MYSTERY(1)")
    workbook.set_formula("Sheet1", "A3", "=TODAY()")
    page = workbook.scan_capabilities(limit=2)
    assert (page["formula_count"], page["supported_count"]) == (3, 2)
    assert [entry["cell"]["address"] for entry in page["entries"]] == ["A1", "A2"]
    assert page["entries"][1]["issue_codes"] == ["calculation.unsupported_function"]
    assert page["next_offset"] == 2
    rest = workbook.scan_capabilities(offset=2, today_serial=45_000)
    assert rest["supported_count"] == 2
    assert rest["entries"][0]["supported"] is True
    assert rest["next_offset"] is None


def assert_delete_sheet() -> None:
    with Workbook.create() as workbook:
        workbook.add_sheet("Data")
        workbook.set_number("Data", "A1", 2.0)
        workbook.set_formula("Data", "B1", "=A1*10")
        workbook.set_formula("Sheet1", "A1", "=Data!A1+1")
        workbook.calculate()

        workbook.delete_sheet("data")
        assert [sheet["name"] for sheet in workbook.summary()["sheets"]] == ["Sheet1"]
        cell = workbook.read_range("Sheet1", "A1", "A1", limit=1)["cells"][0]
        assert cell["formula"] == "=#REF!+1"
        delta = workbook.recalculate()
        # The deletion leaves every formula of the workbook to calculate again.
        assert (delta["mode"], delta["reason"]) == ("full", "dirty_set_covers_workbook")
        assert delta["removed_materialized_cells"] == [
            {"sheet_id": 2, "sheet_name": "Data", "address": "B1"}
        ]
        assert [change["result"] for change in delta["changed_cells"]] == [
            {"kind": "value", "value": {"kind": "error", "value": "#REF!"}}
        ]

        receipt = workbook.apply_changes(
            workbook.summary()["semantic_revision"],
            [
                {"kind": "add_sheet", "name": "Data"},
                {"kind": "delete_sheet", "sheet": "Data"},
            ],
        )
        assert receipt["created_sheet_ids"] == [3]
        assert receipt["topology_changed"]
        try:
            workbook.delete_sheet("Sheet1")
        except CellRuneError as error:
            assert error.code == "validation.last_visible_sheet"
            assert error.kind == "validation"
        else:
            raise AssertionError("the last visible sheet must stay")
        assert_error("interop.sheet.not_found", lambda: workbook.delete_sheet("Data"))


def assert_import_sheet() -> None:
    with Workbook.create() as source, Workbook.create() as target:
        source.set_number("Sheet1", "A1", 3.0)
        source.set_formula("Sheet1", "B1", "=Sheet1!A1*2")
        receipt = target.import_sheet(
            target.summary()["semantic_revision"], source, "sheet1", 1
        )
        assert receipt["created_sheet_ids"] == [2]
        assert receipt["inverse"] is not None
        assert [sheet["name"] for sheet in target.summary()["sheets"]] == [
            "Sheet1",
            "Sheet1 (2)",
        ]
        cell = target.read_range("Sheet1 (2)", "B1", "B1")["cells"][0]
        assert cell["formula"] == "='Sheet1 (2)'!A1*2"
        # A workbook can import from itself.
        own = target.import_sheet(target.summary()["semantic_revision"], target, "Sheet1", 0)
        assert own["created_sheet_ids"] == [3]
        source.set_formula("Sheet1", "C1", "=Sheet9!A1")
        assert_validation_error(
            "validation.sheet_import_unsupported",
            lambda: target.import_sheet(
                target.summary()["semantic_revision"], source, "Sheet1", 0
            ),
        )


def assert_validation_error(code: str, operation: Callable[[], object]) -> None:
    try:
        operation()
    except CellRuneError as error:
        assert error.code == code
        assert error.kind == "validation"
    else:
        raise AssertionError(f"{code} was not raised")


def assert_sheet_structure_changes() -> None:
    with Workbook.create() as workbook:
        workbook.add_sheet("Data")
        workbook.set_number("Data", "A1", 2.0)
        workbook.set_formula("Data", "B1", "=Data!A1*10")
        workbook.apply_changes(
            workbook.summary()["semantic_revision"],
            [
                {
                    "kind": "set_defined_name",
                    "name": "Rate",
                    "formula": "=Data!$A$1",
                    "hidden": False,
                },
                {"kind": "set_formula", "sheet": "Sheet1", "address": "A1", "formula": "=Rate*2"},
            ],
        )

        receipt = workbook.apply_changes(
            workbook.summary()["semantic_revision"],
            [
                {"kind": "copy_sheet", "sheet": "data", "position": 1},
                {
                    "kind": "set_value",
                    "sheet": "Data (2)",
                    "address": "A1",
                    "value": {"kind": "number", "value": 7.0},
                },
                {"kind": "move_sheet", "sheet": "Data", "position": 0},
                {"kind": "rename_defined_name", "name": "rate", "new_name": "Price"},
                {"kind": "set_sheet_auto_filter", "sheet": "Sheet1", "range": "A1:C9"},
            ],
        )
        assert receipt["created_sheet_ids"] == [3]
        assert receipt["topology_changed"]
        summary = workbook.summary()
        assert [(sheet["id"], sheet["name"]) for sheet in summary["sheets"]] == [
            (2, "Data"),
            (1, "Sheet1"),
            (3, "Data (2)"),
        ]
        assert summary["sheets"][1]["auto_filter"] == "A1:C9"
        copied = workbook.read_range("Data (2)", "A1", "B1")["cells"]
        assert copied[0]["source_value"] == {"kind": "number", "value": 7.0}
        assert copied[1]["formula"] == "='Data (2)'!A1*10"
        sheet1 = workbook.read_range("Sheet1", "A1", "A1")["cells"][0]
        assert sheet1["formula"] == "=Price*2"

        workbook.apply_changes(
            workbook.summary()["semantic_revision"],
            [{"kind": "set_sheet_auto_filter", "sheet": "Sheet1"}],
        )
        assert workbook.summary()["sheets"][1]["auto_filter"] is None
        revision = workbook.summary()["semantic_revision"]
        assert_validation_error(
            "validation.sheet_position_out_of_range",
            lambda: workbook.apply_changes(
                revision, [{"kind": "move_sheet", "sheet": "Data", "position": 3}]
            ),
        )
        assert_validation_error(
            "validation.unknown_defined_name",
            lambda: workbook.apply_changes(
                revision,
                [{"kind": "rename_defined_name", "name": "Rate", "new_name": "Other"}],
            ),
        )

        for address, text in [("F1", "Item"), ("G1", "Amount"), ("F2", "Pen"), ("F3", "Ink")]:
            workbook.set_text("Sheet1", address, text)
        workbook.set_number("Sheet1", "G2", 2.0)
        workbook.set_number("Sheet1", "G3", 3.0)
        assert_validation_error(
            "validation.table_range_invalid",
            lambda: workbook.apply_changes_v2(
                workbook.summary()["semantic_revision"],
                [{"kind": "create_table", "sheet": "Sheet1", "range": "F1:G1"}],
            ),
        )
        created = workbook.apply_changes_v2(
            workbook.summary()["semantic_revision"],
            [
                {"kind": "create_table", "sheet": "sheet1", "range": "F1:G2"},
                {"kind": "set_table_range", "table_id": 1, "range": "F1:G3"},
            ],
        )
        assert created["changed_table_ids"] == [1]
        table = workbook.summary()["sheets"][1]["tables"][0]
        assert (table["id"], table["display_name"], table["range"]) == (1, "Table1", "F1:G3")
        workbook.apply_changes_v2(
            workbook.summary()["semantic_revision"],
            [{"kind": "convert_table_to_range", "table_id": 1}],
        )
        assert workbook.summary()["sheets"][1]["tables"] == []


def assert_grid_edits() -> None:
    with Workbook.create() as workbook:
        for row in range(1, 6):
            workbook.set_number("Sheet1", f"A{row}", float(row))
        workbook.set_formula("Sheet1", "B1", "=SUM(A1:A5)")
        workbook.set_formula("Sheet1", "B2", "=A3*10")
        workbook.calculate()

        def formula(address: str) -> object:
            return workbook.read_range("Sheet1", address, address, limit=1)["cells"][0][
                "formula"
            ]

        receipt = workbook.apply_changes(
            workbook.summary()["semantic_revision"],
            [{"kind": "insert_rows", "sheet": "sheet1", "row": 2, "count": 2}],
        )
        assert receipt["topology_changed"]
        assert (formula("B1"), formula("B4")) == ("=SUM(A1:A7)", "=A5*10")
        assert receipt["inverse"] == {
            "changes": [
                {"kind": "delete_rows", "sheet": "Sheet1", "first": 2, "last": 3}
            ],
            "unrestored": [],
        }
        workbook.apply_changes_v2(
            workbook.summary()["semantic_revision"], receipt["inverse"]["changes"]
        )
        assert (formula("B1"), formula("B2")) == ("=SUM(A1:A5)", "=A3*10")

        receipt = workbook.apply_changes(
            workbook.summary()["semantic_revision"],
            [
                {
                    "kind": "copy_range",
                    "sheet": "Sheet1",
                    "source": "B2",
                    "destination": "C2:C3",
                },
                {
                    "kind": "move_columns",
                    "sheet": "Sheet1",
                    "first": 3,
                    "last": 3,
                    "destination": 5,
                },
            ],
        )
        assert formula("E3") == "=B4*10"
        assert "inverse" in receipt
        delta = workbook.recalculate()
        assert delta["reason"] == "topology_changed"
        # Pasting over A3 leaves the formula that read it a #REF!, as Excel does.
        workbook.apply_changes(
            workbook.summary()["semantic_revision"],
            [{"kind": "move_range", "sheet": "Sheet1", "source": "D1", "destination": "A3"}],
        )
        assert (formula("B1"), formula("B2")) == ("=SUM(A1:A5)", "=#REF!*10")
        receipt = workbook.apply_changes(
            workbook.summary()["semantic_revision"],
            [{"kind": "set_value", "sheet": "Sheet1", "address": "D1", "value": {"kind": "blank"}}],
        )
        assert "inverse" not in receipt


def assert_data_validations() -> None:
    workbook = Workbook.from_bytes(SHEET_METADATA_FIXTURE_PATH.read_bytes())
    page = workbook.data_validations("data", limit=2)
    assert (page["sheet"], page["total"], page["next_offset"]) == ("Data", 3, 2)
    assert page["entries"][0] == {
        "ranges": ["A1:A5", "C1"],
        "validation_type": "list",
        "operator": "between",
        "formula1": '"Yes,No"',
        "formula2": None,
        "allow_blank": True,
        "in_cell_dropdown": True,
        "show_input_message": False,
        "show_error_message": True,
        "error_style": "warning",
        "error_title": None,
        "error_message": "Pick one",
        "prompt_title": None,
        "prompt_message": None,
    }
    whole = page["entries"][1]
    assert (whole["validation_type"], whole["operator"]) == ("whole", "notBetween")
    assert (whole["formula1"], whole["formula2"]) == ("1", "9")
    assert whole["in_cell_dropdown"] is False
    assert whole["prompt_message"] == "Outside 1-9"
    rest = workbook.data_validations("Data", offset=2)
    assert rest["entries"][0]["formula1"] == "Lists!$A$1:$A$2"
    assert rest["next_offset"] is None
    assert workbook.data_validations("Lists")["entries"] == []
    assert_error("interop.sheet.not_found", lambda: workbook.data_validations("Missing"))
    assert_error(
        "interop.page.offset_out_of_range",
        lambda: workbook.data_validations("Data", offset=4),
    )
    assert_error(
        "interop.page.limit_invalid",
        lambda: workbook.data_validations("Data", limit=cast(int, True)),
    )


def assert_formats() -> None:
    workbook = Workbook.from_bytes(SHEET_METADATA_FIXTURE_PATH.read_bytes())
    page = workbook.read_formats("Data", "A1", "B1")
    styled = page["cells"][0]["format"]
    assert styled["font"]["name"] == "Arial"
    assert styled["font"]["color"] == {"kind": "argb", "argb": "FFFF0000", "tint": 0.0}
    assert styled["fill"] == {
        "kind": "pattern",
        "pattern": "solid",
        "foreground": {"kind": "argb", "argb": "FFFFFF00", "tint": 0.0},
        "background": None,
    }
    assert styled["alignment"]["horizontal"] == "center"
    assert page["cells"][1]["format"]["font"]["name"] == "Calibri"
    assert page["column_widths"] == [{"first": 1, "last": 2, "width": 18.5}]
    assert page["row_heights"] == [{"row": 1, "height": 24.0}]

    created = Workbook.create()
    revision = created.summary()["semantic_revision"]
    receipt = created.apply_changes_v2(
        revision,
        [
            {
                "kind": "set_cell_format",
                "sheet": "Sheet1",
                "address": "B2",
                "font": {"bold": True, "size": 14.0},
                "border": {"bottom": {"style": "double", "color": {"kind": "indexed", "index": 8}}},
            },
            {"kind": "set_column_width", "sheet": "Sheet1", "first": 2, "last": 2, "width": 25.0},
            {"kind": "set_row_height", "sheet": "Sheet1", "row": 2, "height": 30.0},
        ],
    )
    assert receipt["result_revision"] == revision
    created.calculate()
    reopened = Workbook.from_bytes(created.to_bytes())
    edited = reopened.read_formats("Sheet1", "B2", "B2")
    assert edited["cells"][0]["format"]["font"]["bold"] is True
    assert edited["cells"][0]["format"]["font"]["size"] == 14.0
    assert edited["cells"][0]["format"]["border"]["bottom"] == {
        "style": "double",
        "color": {"kind": "indexed", "index": 8, "tint": 0.0},
    }
    assert edited["column_widths"] == [{"first": 2, "last": 2, "width": 25.0}]
    assert edited["row_heights"] == [{"row": 2, "height": 30.0}]
    revision = reopened.summary()["semantic_revision"]
    try:
        reopened.apply_changes_v2(
            revision,
            [{"kind": "set_cell_format", "sheet": "Sheet1", "address": "A1", "fill": {"pattern": "plaid"}}],  # type: ignore[list-item]
        )
    except CellRuneError as error:
        assert error.code == "interop.change.invalid", error.code
    else:
        raise AssertionError("an unknown fill pattern was accepted")


def assert_annotations() -> None:
    workbook = Workbook.from_bytes(SHEET_METADATA_FIXTURE_PATH.read_bytes())
    hyperlinks = workbook.hyperlinks("Data")
    assert hyperlinks["entries"] == [
        {
            "range": "B4",
            "target": "https://example.com/docs?a=1&b=2",
            "location": None,
            "display": "Docs",
            "tooltip": "Open the docs",
        },
        {
            "range": "B5:C5",
            "target": None,
            "location": "Lists!A1",
            "display": "Lists",
            "tooltip": None,
        },
    ]
    comments = workbook.comments("Data", limit=1)
    assert comments["entries"] == [
        {"address": "A1", "author": "Ana", "text": "Ana: check & confirm"}
    ]
    assert comments["next_offset"] == 1
    assert workbook.comments("Data", offset=1)["entries"][0]["author"] == "Bo"
    assert workbook.hyperlinks("Lists")["total"] == 0
    assert_error("interop.sheet.not_found", lambda: workbook.comments("Missing"))


def assert_surface_authoring() -> None:
    workbook = Workbook.from_bytes(SURFACE_AUTHORING_FIXTURE_PATH.read_bytes())
    data, filtered = workbook.summary()["sheets"]
    assert data["frozen_pane"] == {"rows": 1, "columns": 0}
    assert "frozen_pane" not in filtered
    phonetics = workbook.phonetics("data")
    assert (phonetics["sheet"], phonetics["total"], phonetics["next_offset"]) == (
        "Data",
        1,
        None,
    )
    assert phonetics["entries"][0] == {
        "address": "D2",
        "runs": [
            {"start_utf16": 0, "end_utf16": 2, "text": "トウキョウ"},
            {"start_utf16": 2, "end_utf16": 3, "text": "ト"},
        ],
        "properties": {"font_id": 0, "phonetic_type": "Hiragana", "alignment": None},
        "visible": True,
    }
    assert_error("interop.sheet.not_found", lambda: workbook.phonetics("Missing"))

    revision = workbook.summary()["semantic_revision"]
    receipt = workbook.apply_changes_v2(
        revision,
        [
            {"kind": "widen_table_columns", "table_id": 1, "range": "A1:C3"},
            {"kind": "widen_sheet_auto_filter", "sheet": "Filter", "range": "A1:C3"},
        ],
    )
    assert receipt["changed_table_ids"] == [1]
    presentation = workbook.apply_changes_v2(
        receipt["result_revision"],
        [
            {"kind": "set_frozen_pane", "sheet": "Filter", "rows": 2, "columns": 1},
            {
                "kind": "set_phonetics",
                "sheet": "Data",
                "address": "A2",
                "runs": [{"start_utf16": 0, "end_utf16": 5, "text": "ノース"}],
                "visible": False,
                "properties": {"alignment": "center"},
            },
            {"kind": "clear_phonetics", "sheet": "Data", "address": "D2"},
        ],
    )
    assert presentation["result_revision"] == receipt["result_revision"]
    reopened = Workbook.from_bytes(workbook.edits_to_bytes())
    data, filtered = reopened.summary()["sheets"]
    assert data["tables"][0]["columns"][2]["name"] == "Region"
    assert filtered["auto_filter"] == "A1:C3"
    assert filtered["frozen_pane"] == {"rows": 2, "columns": 1}
    assert [entry["address"] for entry in reopened.phonetics("Data")["entries"]] == ["A2"]

    try:
        Workbook.from_bytes(
            SURFACE_AUTHORING_FIXTURE_PATH.read_bytes(),
            read_limits={"max_phonetic_runs_per_item": 1},
        )
    except CellRuneError as error:
        assert error.code == "xlsx.too_many_phonetic_runs"
        assert error.details["limit"] == {
            "name": "max_phonetic_runs_per_item",
            "observed": 2,
        }
    else:
        raise AssertionError("two phonetic runs passed a per-item limit of one")
    with Workbook.create() as limited:
        limited.set_formula("Sheet1", "A1", "=1+2+3+4+5")
        limited.calculate(calculation_limits={"max_formula_tokens": 3})
        result = limited.read_range("Sheet1", "A1", "A1")["cells"][0]["calculated"]
        assert result is not None and result["kind"] == "unavailable"
        assert result["code"] == "calculation.resource_limit_exceeded"
        assert result.get("limit") == "max_formula_tokens"
        try:
            limited.calculate_targets(
                [{"sheet": "Sheet1", "start": "A1", "end": "A3"}],
                limits={"max_result_cells": 2},
            )
        except CellRuneError as error:
            assert error.code == "calculation.target.target_limit_exceeded"
            assert error.details["limit"] == {"requested_cell_count": 3}
        else:
            raise AssertionError("three cells passed a result limit of two")


def assert_range_values() -> None:
    with Workbook.create() as workbook:
        receipt = workbook.write_values(
            "Sheet1",
            "B2",
            [
                [{"kind": "number", "value": 1.5}, {"kind": "text", "value": "a"}],
                [{"kind": "logical", "value": True}, {"kind": "error", "value": "#N/A"}],
            ],
        )
        assert (receipt["base_revision"], receipt["result_revision"]) == (0, 1)
        assert receipt["applied_change_count"] == 4
        assert workbook.read_values("sheet1", "A2", "C3") == [
            [{"kind": "blank"}, {"kind": "number", "value": 1.5}, {"kind": "text", "value": "a"}],
            [
                {"kind": "blank"},
                {"kind": "logical", "value": True},
                {"kind": "error", "value": "#N/A"},
            ],
        ]
        workbook.set_formula("Sheet1", "D2", "=B2*2")
        workbook.set_formula("Sheet1", "D3", "=NOT_A_FUNCTION(1)")
        assert workbook.read_values("Sheet1", "D2", "D2") == [[{"kind": "blank"}]]
        workbook.calculate()
        calculated = workbook.read_values("Sheet1", "D2", "D3")
        assert calculated[0] == [{"kind": "number", "value": 3.0}]
        unavailable = calculated[1][0]
        assert unavailable["kind"] == "unavailable"
        assert unavailable["code"] == "calculation.unsupported_function"
        for rows in ([], [[]], [[{"kind": "blank"}], []]):
            try:
                workbook.write_values("Sheet1", "A1", rows)
            except CellRuneError as error:
                assert error.code == "interop.change.invalid"
            else:
                raise AssertionError("a non-rectangular write was accepted")
        assert_error(
            "interop.change.payload_invalid",
            lambda: workbook.write_values(
                "Sheet1", "A1", [[{"kind": "number", "value": True}]]
            ),
        )
        assert_error(
            "interop.page.limit_exceeded",
            lambda: workbook.read_values("Sheet1", "A1", "B1048576"),
        )
        assert workbook.summary()["semantic_revision"] == 3


def assert_range_read_forms() -> None:
    with Workbook.create() as workbook:
        workbook.set_number("Sheet1", "A1", 1)
        workbook.set_formula("Sheet1", "C2", "=A1*3")
        workbook.calculate()
        plain = workbook.read_range("Sheet1", "A1", "C2")
        assert len(plain["cells"]) == 6
        assert "values" not in plain and "rows" not in plain
        sparse = workbook.read_range(
            "Sheet1", "A1", "C2", omit_empty=True, values_only=True
        )
        assert sparse["cells"] == []
        assert sparse.get("values") == [
            {"address": "A1", "value": {"kind": "number", "value": 1.0}},
            {"address": "C2", "value": {"kind": "number", "value": 3.0}},
        ]
        compact = workbook.read_range(
            "Sheet1", "A1", "C2", offset=1, compact=True, omit_empty=True
        )
        assert compact.get("rows") == [
            [],
            [{"kind": "blank"}, {"kind": "blank"}, {"kind": "number", "value": 3.0}],
        ]
        prefix = workbook.read_range("Sheet1", "A1", "C2", max_bytes=700)
        assert 0 < len(prefix["cells"]) < 6
        assert prefix["next_offset"] == len(prefix["cells"])
        try:
            workbook.read_range("Sheet1", "A1", "C2", max_bytes=50)
        except CellRuneError as error:
            assert error.code == "interop.page.byte_limit_exceeded"
        else:
            raise AssertionError("a page with no room for a cell was returned")
        for invalid in (True, -1, 1.5, "700"):
            assert_error(
                "interop.page.byte_limit_invalid",
                lambda invalid=invalid: workbook.read_range(
                    "Sheet1", "A1", "C2", max_bytes=invalid
                ),
            )


def assert_dependencies() -> None:
    with Workbook.create() as workbook:
        workbook.set_formula("Sheet1", "B1", "=SUM(A1:A3)")
        workbook.set_formula("Sheet1", "C1", "=B1+INDIRECT(\"A2\")")
        try:
            workbook.precedents("Sheet1", "C1")
        except CellRuneError as error:
            assert error.code == "interop.calculation.required"
        else:
            raise AssertionError("precedents were read without a calculation")
        workbook.calculate()
        precedents = workbook.precedents("sheet1", "C1")
        assert precedents["dynamic"] is True
        assert {"sheet_id": 1, "sheet_name": "Sheet1", "range": "B1", "kind": "value"} in precedents[
            "entries"
        ]
        assert workbook.precedents("Sheet1", "B1")["entries"] == [
            {"sheet_id": 1, "sheet_name": "Sheet1", "range": "A1:A3", "kind": "value"}
        ]
        dependents = workbook.dependents("Sheet1", "A1", limit=1)
        assert dependents["total"] == 1
        assert dependents["entries"] == [
            {
                "cell": {"sheet_id": 1, "sheet_name": "Sheet1", "address": "B1"},
                "dynamic": False,
            }
        ]
        assert dependents["unlisted_dynamic_formulas"] == 1
        assert_error(
            "interop.page.offset_out_of_range",
            lambda: workbook.dependents("Sheet1", "A1", offset=2),
        )


def assert_csv() -> None:
    workbook = Workbook.from_csv(
        b'item;amount\r\n"Widget; large";2.5\r\n', sheet_name="Import", delimiter=";"
    )
    page = workbook.read_range("Import", "A2", "B2")
    assert page["cells"][0]["source_value"] == {"kind": "text", "value": "Widget; large"}
    assert page["cells"][1]["source_value"] == {"kind": "number", "value": 2.5}
    assert workbook.to_csv("Import") == b"item,amount\r\nWidget; large,2.5\r\n"
    workbook.set_formula("Import", "C2", "=B2*2")
    try:
        workbook.to_csv("Import")
    except CellRuneError as error:
        assert (error.code, error.kind) == ("interop.calculation.required", "state")
    else:
        raise AssertionError("an uncalculated formula was exported")
    workbook.calculate()
    assert workbook.to_csv("Import") == b"item,amount,\r\nWidget; large,2.5,5\r\n"
    try:
        Workbook.from_csv(b'"open')
    except CellRuneError as error:
        assert (error.code, error.kind) == ("csv.read.unterminated_quote", "read")
    else:
        raise AssertionError("an unterminated quote was accepted")


def main() -> None:
    assert_sheet_structure_changes()
    assert_delete_sheet()
    assert_import_sheet()
    assert_grid_edits()
    assert_surface_authoring()
    assert_range_values()
    assert_range_read_forms()
    assert_dependencies()
    assert_csv()
    assert_annotations()
    assert_formats()
    assert_data_validations()
    assert_incomplete_save_names_cells()
    assert_capability_scan()
    assert_limits()
    assert_edits_save()
    assert_function_usage()
    assert_compatibility()
    assert_catalog_contract()
    assert_preview_contract()
    assert_object_payloads()
    assert_random_seed()
    assert_locale_profile()
    assert_display_format()
    assert_exact_numbers()
    assert_volatile_input_recalculation()
    corpus = json.loads(CORPUS_PATH.read_text(encoding="utf-8"))
    defined_name_corpus = json.loads(
        DEFINED_NAME_CORPUS_PATH.read_text(encoding="utf-8")
    )
    table_contract = json.loads(
        TABLE_AUTHORING_CONTRACT_PATH.read_text(encoding="utf-8")
    )
    assert defined_name_corpus["schema_version"] == 1
    with Workbook.create() as workbook:
        fingerprint = workbook.summary()["fingerprint"]
        assert fingerprint["schema_version"] == 7
        assert len(fingerprint["digest_hex"]) == 64
        assert all(
            character in "0123456789abcdef"
            for character in fingerprint["digest_hex"]
        )
        for operation in corpus["operations"]:
            kind = operation["kind"]
            if kind == "set_number":
                workbook.set_number(
                    operation["sheet"], operation["address"], operation["value"]
                )
            elif kind == "set_formula":
                workbook.set_formula(
                    operation["sheet"], operation["address"], operation["formula"]
                )
            elif kind == "set_dynamic_formula":
                workbook.set_formula(
                    operation["sheet"],
                    operation["address"],
                    operation["formula"],
                    dynamic_range=operation["dynamic_range"],
                )
            else:
                raise AssertionError(f"unknown corpus operation: {kind}")

        report = workbook.calculate()
        assert report["unavailable_count"] == 0
        page = workbook.read_range("Sheet1", "A1", "AU2", limit=100)
        values: dict[str, float] = {}
        for cell in page["cells"]:
            result = cell["calculated"]
            value = result["value"] if result and result["kind"] == "value" else cell["source_value"]
            if value["kind"] == "number":
                values[cell["address"]] = value["value"]
        for expected in corpus["expected_numbers"]:
            assert values[expected["address"]] == expected["value"]

        workbook.set_number("Sheet1", "H1", 0.05)
        workbook.set_formula(
            "Sheet1",
            "I1",
            "=ACCRINT(DATE(2024,1,1),DATE(2024,7,1),DATE(2025,1,1),H1,1000,2)",
        )
        fixed_income_usage = next(
            entry
            for entry in workbook.function_usage()["entries"]
            if entry["name"] == "ACCRINT"
        )
        assert fixed_income_usage["supported"]
        assert fixed_income_usage["call_count"] == 1
        assert fixed_income_usage["sample_cells"] == [
            {"sheet_id": 1, "sheet_name": "Sheet1", "address": "I1"}
        ]
        workbook.recalculate(mode="full")
        accrued = workbook.read_range("Sheet1", "I1", "I1", limit=1)["cells"][0]
        assert accrued["calculated"] == {
            "kind": "value",
            "value": {"kind": "number", "value": 50.0},
        }
        workbook.set_number("Sheet1", "H1", 0.06)
        fixed_income_delta = workbook.recalculate(mode="incremental")
        assert fixed_income_delta["mode"] == "incremental"
        assert fixed_income_delta["dirty_count"] == 1
        assert fixed_income_delta["evaluated_count"] == 1
        assert [
            cell["cell"]["address"] for cell in fixed_income_delta["changed_cells"]
        ] == ["I1"]
        accrued = workbook.read_range("Sheet1", "I1", "I1", limit=1)["cells"][0]
        assert accrued["calculated"] == {
            "kind": "value",
            "value": {"kind": "number", "value": 60.0},
        }

        for offset in (-1, 2**64, 1.5, True):
            assert_error(
                "interop.page.offset_invalid",
                lambda: read_with_invalid_offset(workbook, offset),
            )
        for limit in (-1, 2**32, 1.5, True):
            assert_error(
                "interop.page.limit_invalid",
                lambda: read_with_invalid_limit(workbook, limit),
            )
        assert_error(
            "interop.page.offset_out_of_range",
            lambda: workbook.read_range("Sheet1", "A1", "A1", offset=2**64 - 1),
        )
        for limit in (10_001, 2**32 - 1):
            assert_error(
                "interop.page.limit_exceeded",
                lambda: workbook.read_range("Sheet1", "A1", "A1", limit=limit),
            )

        for invalid_number in (True, False, "1"):
            assert_error(
                "interop.value.number_invalid",
                lambda: set_invalid_number(workbook, invalid_number),
            )
            assert_error(
                "interop.value.number_invalid",
                lambda: calculate_with_invalid_today(workbook, invalid_number),
            )
            assert_error(
                "interop.value.number_invalid",
                lambda: recalculate_with_invalid_now(workbook, invalid_number),
            )

        assert_error(
            "interop.calculation.arithmetic_semantics_invalid",
            lambda: calculate_with_invalid_arithmetic_semantics(workbook, "binary"),
        )
        assert_error(
            "interop.calculation.financial_solver_semantics_invalid",
            lambda: recalculate_with_invalid_solver_semantics(workbook, "unbounded"),
        )

        workbook.set_formula("Sheet1", "A3", "=0.1+0.2-0.3")
        workbook.set_formula("Sheet1", "A4", "=IRR({-1,100000})")
        workbook.recalculate(mode="full")
        defaults = workbook.read_range("Sheet1", "A3", "A4", limit=2)["cells"]
        assert defaults[0]["calculated"] == {
            "kind": "value",
            "value": {"kind": "number", "value": 0.0},
        }
        assert defaults[1]["calculated"] == {
            "kind": "value",
            "value": {"kind": "error", "value": "#NUM!"},
        }
        workbook.recalculate(
            mode="full",
            arithmetic_semantics="ieee_754",
            financial_solver_semantics="extended_search",
        )
        legacy = workbook.read_range("Sheet1", "A3", "A4", limit=2)["cells"]
        legacy_arithmetic = legacy[0]["calculated"]
        legacy_solver = legacy[1]["calculated"]
        assert legacy_arithmetic is not None and legacy_arithmetic["kind"] == "value"
        assert legacy_arithmetic["value"]["kind"] == "number"
        assert legacy_arithmetic["value"]["value"] != 0.0
        assert legacy_solver is not None and legacy_solver["kind"] == "value"
        assert legacy_solver["value"]["kind"] == "number"
        assert abs(legacy_solver["value"]["value"] - 99_999.0) < 1e-5

        for sheet_name in defined_name_corpus["sheets"]:
            workbook.add_sheet(sheet_name)
        workbook.apply_changes(
            workbook.summary()["semantic_revision"],
            [
                {
                    "kind": "set_defined_name",
                    "name": item["name"],
                    "scope_sheet": item["scope_sheet"],
                    "formula": item["formula"],
                    "hidden": item["hidden"],
                }
                for item in defined_name_corpus["defined_names"]
            ],
        )
        assert workbook.inspect_defined_name("WorkbookAlias", current_sheet="Sheet1")[
            "result"
        ] == {
            "kind": "rectangular",
            "sheet_id": 1,
            "sheet_name": "Sheet1",
            "range": "A1:A1",
        }
        assert workbook.inspect_defined_name("LocalAlias", current_sheet="Sheet1")[
            "result"
        ] == {
            "kind": "rectangular",
            "sheet_id": 1,
            "sheet_name": "Sheet1",
            "range": "B2:B2",
        }
        assert workbook.inspect_defined_name("QualifiedLocal")["result"] == {
            "kind": "rectangular",
            "sheet_id": 1,
            "sheet_name": "Sheet1",
            "range": "B2:B2",
        }
        assert workbook.inspect_defined_name("ExplicitSingleSpan")["result"] == {
            "kind": "three_dimensional",
            "sheet_span": {
                "start_sheet_id": 2,
                "start_sheet_name": "Middle",
                "end_sheet_id": 2,
                "end_sheet_name": "Middle",
            },
            "range": "D4:D4",
        }
        assert workbook.inspect_defined_name("Dynamic")["result"] == {
            "kind": "dynamic_formula",
            "dynamic_kind": "offset",
            "formula": "=OFFSET(Sheet1!A1,1,0)",
        }
        assert (
            workbook.inspect_defined_name("IndirectDynamic")["result"]["dynamic_kind"]
            == "indirect"
        )
        assert (
            workbook.inspect_defined_name("SpillDynamic")["result"]["dynamic_kind"]
            == "spill"
        )
        assert (
            workbook.inspect_defined_name("MixedDynamic")["result"]["dynamic_kind"]
            == "mixed"
        )
        areas = workbook.inspect_defined_name("Areas")["result"]
        assert areas["kind"] == "non_rectangular"
        assert [area["kind"] for area in areas["areas"]] == [
            "rectangular",
            "rectangular",
            "three_dimensional",
            "rectangular",
        ]
        assert areas["areas"][2]["sheet_span"] == {
            "start_sheet_id": 1,
            "start_sheet_name": "Sheet1",
            "end_sheet_id": 3,
            "end_sheet_name": "Sheet3",
        }
        assert workbook.inspect_defined_name("ConstantValue")["result"]["kind"] == "constant"
        assert workbook.inspect_defined_name("ExternalValue")["result"] == {
            "kind": "external_reference",
            "locator": None,
            "workbook": "Book.xlsx",
            "sheet": "Data",
            "sheet_end": None,
            "target_kind": "reference",
            "target_text": "A1",
        }
        assert workbook.inspect_defined_name("InvalidValue")["result"]["reason"] == "parse_error"
        assert (
            workbook.inspect_defined_name("CallableValue")["result"]["reason"]
            == "non_reference_expression"
        )
        assert workbook.inspect_defined_name("Missing")["result"] == {"kind": "not_found"}
        assert_error(
            "interop.sheet.not_found",
            lambda: workbook.inspect_defined_name(
                "Areas", current_sheet="missing"
            ),
        )

        workbook.recalculate(mode="full")
        output = workbook.to_bytes()
        with tempfile.TemporaryDirectory() as temporary_directory:
            saved_path = pathlib.Path(temporary_directory) / "saved.xlsx"
            write_report = workbook.save(saved_path)
            assert write_report["output_sha256"] == hashlib.sha256(
                saved_path.read_bytes()
            ).hexdigest()

    reopened = Workbook.from_bytes(output)
    assert reopened.summary()["document_kind"] == "xlsx"
    assert reopened.inspect_defined_name("Dynamic")["result"]["kind"] == "dynamic_formula"
    assert reopened.inspect_defined_name("ExplicitSingleSpan")["result"] == {
        "kind": "three_dimensional",
        "sheet_span": {
            "start_sheet_id": 2,
            "start_sheet_name": "Middle",
            "end_sheet_id": 2,
            "end_sheet_name": "Middle",
        },
        "range": "D4:D4",
    }
    assert reopened.inspect_defined_name("ExternalValue")["result"]["target_text"] == "A1"

    try:
        reopened.set_number("Sheet1", corpus["invalid_address"], 1.0)
    except CellRuneError as error:
        assert error.code == corpus["invalid_address_code"]
        assert error.kind == "validation"
    else:
        raise AssertionError("invalid address did not raise CellRuneError")

    table_workbook = Workbook.open_path(
        TABLE_AUTHORING_CONTRACT_PATH.parent / table_contract["fixture"]
    )
    assert_error(
        "interop.change.payload_invalid",
        lambda: table_workbook.apply_changes_v2(
            table_workbook.summary()["semantic_revision"],
            [
                {
                    "kind": "rename_table",
                    "table_id": table_contract["table_id"],
                    "new_display_name": table_contract["new_display_name"],
                    "unexpected": True,
                }
            ],
        ),
    )
    table_receipt = table_workbook.apply_changes_v2(
        table_workbook.summary()["semantic_revision"],
        [
            {
                "kind": "rename_table",
                "table_id": table_contract["table_id"],
                "new_display_name": table_contract["new_display_name"],
            },
            {
                "kind": "rename_table_column",
                "table_id": table_contract["table_id"],
                "column_id": table_contract["column_id"],
                "new_name": table_contract["new_column_name"],
            },
            {
                "kind": "resize_table_rows",
                "table_id": table_contract["table_id"],
                "first_data_row": table_contract["first_data_row"],
                "last_data_row": table_contract["last_data_row"],
            },
        ],
    )
    assert table_receipt["schema_version"] == table_contract["schema_version"]
    assert table_receipt["changed_table_ids"] == [table_contract["table_id"]]
    assert_table_authoring_result(table_workbook, table_contract)
    table_workbook.recalculate(mode="full")
    reopened_table_workbook = Workbook.from_bytes(
        table_workbook.to_bytes(invalidate_unavailable=True)
    )
    assert_table_authoring_result(reopened_table_workbook, table_contract)


def assert_preview_contract() -> None:
    workbook = Workbook.create()
    initial = workbook.apply_changes(
        0,
        [
            {
                "kind": "set_value",
                "sheet": "Sheet1",
                "address": "A1",
                "value": {"kind": "number", "value": 1.0},
            },
            {
                "kind": "set_formula",
                "sheet": "Sheet1",
                "address": "A2",
                "formula": "=A1+1",
            },
        ],
    )
    preview = workbook.preview_changes(
        initial["result_revision"],
        [
            {
                "kind": "set_value",
                "sheet": "Sheet1",
                "address": "A1",
                "value": {"kind": "number", "value": 4.0},
            }
        ],
        today_serial=None,
        now_serial=None,
    )
    assert preview["report"]["base_revision"] == initial["result_revision"]
    assert preview["report"]["result_revision"] == initial["result_revision"] + 1
    assert preview["report"]["calculation_options"]["limits"]["max_array_cells"] == 1_000_000
    page = workbook.preview_changes_page(
        preview["preview_id"], section="preview_results", limit=1
    )
    assert page["preview_id"] == preview["preview_id"]
    assert page["items"]
    receipt = workbook.commit_preview(preview["preview_id"])
    assert receipt["edit"]["result_revision"] == preview["report"]["result_revision"]
    try:
        workbook.preview_changes_page(preview["preview_id"], section="affected")
    except CellRuneError as error:
        assert error.code == "interop.preview.not_found"
    else:
        raise AssertionError("committed preview remained pageable")


def assert_object_payloads() -> None:
    """Rejects a sequence where a payload expects an object; JSON would fill it by position."""
    workbook = Workbook.create()
    value = {"kind": "number", "value": 1.0}
    revision = workbook.apply_changes(
        0,
        [
            {"kind": "set_value", "sheet": "Sheet1", "address": "A1", "value": value},
            {"kind": "set_formula", "sheet": "Sheet1", "address": "B1", "formula": "=A1+1"},
            {"kind": "set_formula", "sheet": "Sheet1", "address": "B2", "formula": "=A1+2"},
        ],
    )["result_revision"]
    for changes in (
        [["set_value", "Sheet1", "C1", value]],
        [("set_value", "Sheet1", "C1", value)],
        [{"kind": "set_value", "sheet": "Sheet1", "address": "C1", "value": ["number", 1.0]}],
    ):
        assert_error(
            "interop.change.payload_invalid",
            lambda: workbook.apply_changes(revision, changes),
        )
        assert_error(
            "interop.change.payload_invalid",
            lambda: workbook.apply_changes_v2(revision, changes),
        )
        assert_error(
            "interop.change.payload_invalid",
            lambda: workbook.preview_changes(revision, changes),
        )
    assert_error(
        "interop.change.payload_invalid",
        lambda: workbook.apply_changes_v2(revision, [["rename_table", 1, "Renamed"]]),
    )
    assert workbook.summary()["semantic_revision"] == revision
    assert workbook.read_range("Sheet1", "C1", "C1")["cells"][0]["source_value"] == {
        "kind": "blank"
    }
    preview = workbook.preview_changes(
        revision,
        [
            {
                "kind": "set_value",
                "sheet": "Sheet1",
                "address": "A1",
                "value": {"kind": "number", "value": 2.0},
            }
        ],
    )
    page = workbook.preview_changes_page(
        preview["preview_id"], section="affected", limit=1
    )
    cursor = page["next_cursor"]
    assert cursor is not None
    for sequence in (
        [cursor["preview_id"], cursor["token"]],
        (cursor["preview_id"], cursor["token"]),
    ):
        assert_error(
            "interop.preview.cursor_invalid",
            lambda: workbook.preview_changes_page(
                preview["preview_id"], section="affected", cursor=sequence, limit=1
            ),
        )
    assert workbook.preview_changes_page(
        preview["preview_id"], section="affected", cursor=cursor, limit=1
    )["items"]


def assert_table_authoring_result(
    workbook: Workbook, contract: dict[str, object]
) -> None:
    tables = workbook.summary()["sheets"][0]["tables"]
    table = next(candidate for candidate in tables if candidate["id"] == contract["table_id"])
    assert table["id"] == contract["table_id"]
    assert table["name"] == contract["new_display_name"]
    assert table["display_name"] == contract["new_display_name"]
    assert table["range"] == contract["expected_range"]
    assert table["columns"][1]["id"] == contract["column_id"]
    assert table["columns"][1]["name"] == contract["new_column_name"]
    address = str(contract["expected_header_address"])
    header = workbook.read_range("Data", address, address)["cells"][0]
    assert header["source_value"] == {
        "kind": "text",
        "value": contract["new_column_name"],
    }
    formula = workbook.read_range("Data", "E1", "E1")["cells"][0]["formula"]
    assert formula == "=SUM(Orders[Gross Amount])"
    empty_table = next(
        candidate
        for candidate in tables
        if candidate["id"] == contract["empty_table_id"]
    )
    assert empty_table["name"] == contract["empty_table_name"]
    assert empty_table["range"] == contract["empty_table_range"]
    assert (
        workbook.inspect_defined_name(str(contract["empty_defined_name"]))["result"][
            "kind"
        ]
        == contract["empty_defined_name_result"]
    )


if __name__ == "__main__":
    main()
