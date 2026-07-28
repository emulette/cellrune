from __future__ import annotations

import sys
import threading
from collections.abc import Callable
from typing import TypeVar

from cellrune import Workbook


T = TypeVar("T")

INTERPRETER_SWITCH_INTERVAL_SECONDS = 1.0
WORKER_TIMEOUT_SECONDS = 2.0


def main() -> None:
    workbook = Workbook.create()
    workbook.set_number("Sheet1", "A1", 1.0)
    for row in range(2, 25_001):
        workbook.set_formula("Sheet1", f"A{row}", f"=A{row - 1}+1")

    report = assert_allows_interpreter_progress(
        workbook.calculate,
        "native calculation retained the Python interpreter lock",
    )
    assert report["unavailable_count"] == 0

    usage = assert_allows_interpreter_progress(
        workbook.function_usage,
        "function-usage scan retained the Python interpreter lock",
    )
    assert usage["formula_count"] == 24_999

    page = assert_allows_interpreter_progress(
        lambda: workbook.read_range("Sheet1", "A1", "A10000", limit=10_000),
        "range read retained the Python interpreter lock",
        attempts=20,
    )
    assert len(page["cells"]) == 10_000


def assert_allows_interpreter_progress(
    operation: Callable[[], T],
    message: str,
    *,
    attempts: int = 1,
) -> T:
    ready = threading.Event()
    requested = threading.Event()
    progressed = threading.Event()

    def worker() -> None:
        ready.set()
        requested.wait()
        progressed.set()

    previous_switch_interval = sys.getswitchinterval()
    sys.setswitchinterval(INTERPRETER_SWITCH_INTERVAL_SECONDS)
    thread = threading.Thread(target=worker)
    thread.start()
    try:
        assert ready.wait(WORKER_TIMEOUT_SECONDS), "worker thread did not become ready"
        requested.set()
        result = operation()
        for _ in range(1, attempts):
            if progressed.is_set():
                break
            result = operation()
        assert progressed.is_set(), message
        return result
    finally:
        requested.set()
        sys.setswitchinterval(previous_switch_interval)
        thread.join(WORKER_TIMEOUT_SECONDS)
        assert not thread.is_alive(), "worker thread did not stop"


if __name__ == "__main__":
    main()
