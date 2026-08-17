from __future__ import annotations

import hashlib
import json
import pathlib
import tempfile
from collections.abc import Callable
from typing import Literal, cast

from cellrune import CellRuneError, Workbook, function_catalog

CORPUS_PATH = pathlib.Path(__file__).parents[3] / "binding-contract" / "v1.json"
DEFINED_NAME_CORPUS_PATH = (
    pathlib.Path(__file__).parents[3] / "binding-contract" / "defined-name-v1.json"
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
    assert len(catalog["entries"]) == 417
    entries = {entry["name"]: entry for entry in catalog["entries"]}
    assert all(
        name in entries
        for name in (
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
        )
    )
    assert all(
        entries[name]["returns_array"]
        for name in (
            "GROWTH", "LINEST", "LOGEST", "MINVERSE", "MUNIT", "TREND", "XLOOKUP"
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


def main() -> None:
    assert_catalog_contract()
    assert_preview_contract()
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
        page = workbook.read_range("Sheet1", "A1", "F2", limit=100)
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
