from __future__ import annotations

import json
import pathlib
from collections.abc import Callable
from typing import Literal, cast

from cellrune import CellRuneError, Workbook

CORPUS_PATH = pathlib.Path(__file__).parents[3] / "binding-contract" / "v1.json"
ArithmeticSemantics = Literal["excel_near_zero", "ieee_754"]
FinancialSolverSemantics = Literal["excel_iteration_budget", "extended_search"]


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
    corpus = json.loads(CORPUS_PATH.read_text(encoding="utf-8"))
    with Workbook.create() as workbook:
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

        workbook.set_formula("Sheet1", "A3", "=100.1-100-0.1")
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

        output = workbook.to_bytes()

    reopened = Workbook.from_bytes(output)
    assert reopened.summary()["document_kind"] == "xlsx"

    try:
        reopened.set_number("Sheet1", corpus["invalid_address"], 1.0)
    except CellRuneError as error:
        assert error.code == corpus["invalid_address_code"]
        assert error.kind == "validation"
    else:
        raise AssertionError("invalid address did not raise CellRuneError")


if __name__ == "__main__":
    main()
