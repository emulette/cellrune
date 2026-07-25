from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path
from threading import Thread
from time import monotonic, sleep

from cellrune import CellRuneError, Workbook, WorkbookChange


def main() -> None:
    corpus_path = (
        Path(__file__).resolve().parents[3] / "conformance" / "interactive-v1.json"
    )
    corpus = json.loads(corpus_path.read_text(encoding="utf-8"))
    workbook = Workbook.create()
    receipt = workbook.apply_changes(0, corpus["initial_changes"])
    assert receipt["result_revision"] == corpus["expected"]["initial_revision"]
    assert receipt["applied_change_count"] == len(corpus["initial_changes"])
    assert len(receipt["calculation_changed_cells"]) == len(
        corpus["initial_changes"]
    )
    assert not receipt["calculation_metadata_changed"]

    first = workbook.recalculate()
    assert first["mode"] == "full"
    assert first["result_revision"] == receipt["result_revision"]

    receipt = workbook.apply_changes(
        receipt["result_revision"], corpus["incremental_changes"]
    )
    assert len(receipt["calculation_changed_cells"]) == 1
    delta = workbook.recalculate()
    assert delta["mode"] == corpus["expected"]["incremental_mode"]
    assert (
        delta["evaluated_count"]
        == corpus["expected"]["incremental_evaluated_count"]
    )
    assert delta["parsed_formula_count"] == 0
    assert delta["result_revision"] == receipt["result_revision"]

    page = workbook.read_range("Sheet1", "B1", "C1")
    b1 = page["cells"][0]["calculated"]
    c1 = page["cells"][1]["calculated"]
    assert b1 is not None and b1["kind"] == "value"
    assert c1 is not None and c1["kind"] == "value"
    assert b1["value"]["kind"] == "number"
    assert c1["value"]["kind"] == "number"
    assert b1["value"]["value"] == corpus["expected"]["b1"]
    assert c1["value"]["value"] == corpus["expected"]["c1"]

    history = workbook.changes_since(0, limit=1)
    assert len(history["deltas"]) == 1
    assert history["next_cursor"] is not None
    next_page = workbook.changes_since(history["next_cursor"], limit=1)
    assert len(next_page["deltas"]) == 1
    assert next_page["next_cursor"] is None

    try:
        workbook.apply_changes(
            0,
            [
                {
                    "kind": "set_value",
                    "sheet": "Sheet1",
                    "address": "A1",
                    "value": {"kind": "number", "value": 99.0},
                }
            ],
        )
    except CellRuneError as error:
        assert error.code == corpus["expected"]["revision_error"]
    else:
        raise AssertionError("stale revision must fail")

    assert_context_manager_closes()
    assert_concurrent_edit_and_cancellation()


def assert_context_manager_closes() -> None:
    workbook = Workbook.create()
    with workbook as entered:
        assert entered is workbook
        assert not entered.closed
        entered.summary()
    assert workbook.closed
    workbook.close()
    assert_closed(workbook.summary)
    assert_closed(workbook.__enter__)


def assert_concurrent_edit_and_cancellation() -> None:
    workbook = Workbook.create()
    changes: list[WorkbookChange] = [
        {
            "kind": "set_value",
            "sheet": "Sheet1",
            "address": "A1",
            "value": {"kind": "number", "value": 1.0},
        }
    ]
    for row in range(1, 30_001):
        changes.append(
            {
                "kind": "set_formula",
                "sheet": "Sheet1",
                "address": f"B{row}",
                "formula": f"=A1+{row}",
                "dynamic_range": None,
            }
        )
    receipt = workbook.apply_changes(0, changes)

    stale_errors: list[BaseException] = []
    stale_thread = Thread(
        target=run_recalculation,
        args=(workbook, stale_errors),
        daemon=True,
    )
    stale_thread.start()
    wait_until_active(workbook, stale_thread)
    workbook.apply_changes(
        receipt["result_revision"],
        [
            {
                "kind": "set_value",
                "sheet": "Sheet1",
                "address": "A1",
                "value": {"kind": "number", "value": 2.0},
            }
        ],
    )
    stale_thread.join(timeout=30)
    assert not stale_thread.is_alive()
    assert len(stale_errors) == 1
    assert isinstance(stale_errors[0], CellRuneError)
    assert stale_errors[0].code == "session.stale_result"

    cancelled_errors: list[BaseException] = []
    cancelled_thread = Thread(
        target=run_recalculation,
        args=(workbook, cancelled_errors),
        daemon=True,
    )
    cancelled_thread.start()
    wait_until_active(workbook, cancelled_thread)
    assert workbook.cancel_calculation()
    cancelled_thread.join(timeout=30)
    assert not cancelled_thread.is_alive()
    assert len(cancelled_errors) == 1
    assert isinstance(cancelled_errors[0], CellRuneError)
    assert cancelled_errors[0].code == "session.cancelled"

    closed_errors: list[BaseException] = []
    closed_thread = Thread(
        target=run_recalculation,
        args=(workbook, closed_errors),
        daemon=True,
    )
    closed_thread.start()
    wait_until_active(workbook, closed_thread)
    workbook.close()
    workbook.close()
    assert workbook.closed
    closed_thread.join(timeout=30)
    assert not closed_thread.is_alive()
    assert len(closed_errors) == 1
    assert isinstance(closed_errors[0], CellRuneError)
    assert closed_errors[0].code == "interop.session.closed"
    assert_closed(workbook.summary)
    assert_closed(workbook.recalculate)


def run_recalculation(
    workbook: Workbook,
    errors: list[BaseException],
) -> None:
    try:
        workbook.recalculate(mode="full")
    except BaseException as error:
        errors.append(error)


def wait_until_active(workbook: Workbook, thread: Thread) -> None:
    deadline = monotonic() + 10
    while True:
        try:
            if workbook.calculation_active():
                return
        except CellRuneError as error:
            if error.code != "interop.session.unavailable":
                raise
        if not thread.is_alive():
            raise AssertionError(
                "calculation completed before its active state was observable"
            )
        if monotonic() >= deadline:
            raise AssertionError("calculation did not become active")
        sleep(0.001)


def assert_closed(operation: Callable[[], object]) -> None:
    try:
        operation()
    except CellRuneError as error:
        assert error.code == "interop.session.closed"
        assert error.kind == "state"
    else:
        raise AssertionError("closed workbook operation must fail")


if __name__ == "__main__":
    main()
