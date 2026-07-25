from os import PathLike
from types import TracebackType
from typing import Literal

from ._types import (
    CalculationDelta,
    CalculationDeltaPage,
    CalculationReport,
    EditReceipt,
    ErrorDetails,
    FunctionUsageReport,
    RangePage,
    WorkbookChange,
    WorkbookSummary,
    WriteReport,
)

SCHEMA_VERSION: int

class CellRuneError(Exception):
    code: str
    kind: Literal["input", "validation", "read", "write", "state"]
    details: ErrorDetails

class Workbook:
    @staticmethod
    def create() -> Workbook: ...
    @staticmethod
    def open_path(path: str | PathLike[str]) -> Workbook: ...
    @staticmethod
    def from_bytes(bytes: bytes) -> Workbook: ...
    @property
    def closed(self) -> bool: ...
    def close(self) -> None: ...
    def __enter__(self) -> Workbook: ...
    def __exit__(
        self,
        exception_type: type[BaseException] | None,
        exception: BaseException | None,
        traceback: TracebackType | None,
    ) -> None: ...
    def summary(self) -> WorkbookSummary: ...
    def read_range(
        self,
        sheet: str,
        start: str,
        end: str,
        *,
        offset: int | None = None,
        limit: int | None = None,
    ) -> RangePage: ...
    def function_usage(self) -> FunctionUsageReport: ...
    def calculate(
        self,
        *,
        today_serial: float | None = None,
        now_serial: float | None = None,
    ) -> CalculationReport: ...
    def recalculate(
        self,
        *,
        mode: Literal["auto", "incremental", "full"] = "auto",
        today_serial: float | None = None,
        now_serial: float | None = None,
    ) -> CalculationDelta: ...
    def apply_changes(
        self,
        expected_revision: int,
        changes: list[WorkbookChange],
    ) -> EditReceipt: ...
    def changes_since(
        self,
        cursor: int = 0,
        *,
        limit: int | None = None,
    ) -> CalculationDeltaPage: ...
    def cancel_calculation(self) -> bool: ...
    def calculation_active(self) -> bool: ...
    def set_blank(self, sheet: str, address: str) -> None: ...
    def set_number(self, sheet: str, address: str, value: float) -> None: ...
    def set_text(self, sheet: str, address: str, value: str) -> None: ...
    def set_logical(self, sheet: str, address: str, value: bool) -> None: ...
    def set_error(self, sheet: str, address: str, value: str) -> None: ...
    def set_formula(
        self,
        sheet: str,
        address: str,
        formula: str,
        *,
        dynamic_range: str | None = None,
    ) -> None: ...
    def clear_cell(self, sheet: str, address: str) -> bool: ...
    def add_sheet(self, name: str) -> int: ...
    def rename_sheet(self, current_name: str, new_name: str) -> None: ...
    def to_bytes(self, *, invalidate_unavailable: bool = False) -> bytes: ...
    def save(
        self,
        path: str | PathLike[str],
        *,
        invalidate_unavailable: bool = False,
        replace_existing: bool = False,
    ) -> WriteReport: ...
