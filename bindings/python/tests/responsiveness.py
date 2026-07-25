from __future__ import annotations

import threading
import time

from cellrune import Workbook


def main() -> None:
    workbook = Workbook.create()
    workbook.set_number("Sheet1", "A1", 1.0)
    for row in range(2, 25_001):
        workbook.set_formula("Sheet1", f"A{row}", f"=A{row - 1}+1")

    stop = threading.Event()
    progress = [0]

    def worker() -> None:
        while not stop.is_set():
            progress[0] += 1

    thread = threading.Thread(target=worker)
    thread.start()
    try:
        deadline = time.monotonic() + 2.0
        while progress[0] == 0 and time.monotonic() < deadline:
            time.sleep(0.001)
        before = progress[0]
        report = workbook.calculate()
        after = progress[0]
        assert report["unavailable_count"] == 0
        assert after > before, "native calculation retained the Python interpreter lock"

        before = progress[0]
        usage = workbook.function_usage()
        after = progress[0]
        assert usage["formula_count"] == 24_999
        assert after > before, "function-usage scan retained the Python interpreter lock"

        before = progress[0]
        page = workbook.read_range("Sheet1", "A1", "A10000", limit=10_000)
        after = progress[0]
        assert len(page["cells"]) == 10_000
        assert after > before, "range read retained the Python interpreter lock"
    finally:
        stop.set()
        thread.join()


if __name__ == "__main__":
    main()
