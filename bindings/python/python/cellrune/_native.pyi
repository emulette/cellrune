from collections.abc import Sequence
from os import PathLike
from types import TracebackType
from typing import Literal

from ._types import (
    CalculationLimitOptions,
    CalculationTarget,
    LocaleProfile,
    CellValue,
    TargetCalculationLimitOptions,
    TargetCalculationResult,
    CalculationDelta,
    CalculationDeltaPage,
    CalculationReport,
    CapabilityPage,
    DataValidationPage,
    DefinedNameInspection,
    EditReceipt,
    EditReceiptV2,
    EditWriteReport,
    ErrorDetails,
    FunctionCatalogReport,
    FunctionUsageReport,
    PreviewChanges,
    PreviewCursor,
    RangePage,
    ReadLimitOptions,
    TransactionImpactPage,
    WorkbookChange,
    WorkbookChangeV2,
    WorkbookSummary,
    WorkbookTransactionReceipt,
    WriteReport,
)

SCHEMA_VERSION: int

def function_catalog() -> FunctionCatalogReport: ...
def format_display_text(
    value: CellValue,
    format_code: str,
    *,
    date_system: Literal["excel_1900", "excel_1904"] = "excel_1900",
    locale_profile: LocaleProfile | None = None,
) -> str: ...

class CellRuneError(Exception):
    code: str
    kind: Literal["input", "validation", "read", "write", "state"]
    details: ErrorDetails

class Workbook:
    def calculate_targets(
        self,
        targets: Sequence[CalculationTarget],
        *,
        today_serial: float | None = None,
        now_serial: float | None = None,
        random_seed: int | None = None,
        locale_profile: LocaleProfile | None = None,
        arithmetic_semantics: Literal["excel_near_zero", "ieee_754"] = "excel_near_zero",
        financial_solver_semantics: Literal["excel_iteration_budget", "extended_search"] = "excel_iteration_budget",
        limits: TargetCalculationLimitOptions | None = None,
    ) -> TargetCalculationResult: ...
    @staticmethod
    def create() -> Workbook: ...
    @staticmethod
    def open_path(
        path: str | PathLike[str], *, read_limits: ReadLimitOptions | None = None
    ) -> Workbook: ...
    @staticmethod
    def from_bytes(
        bytes: bytes, *, read_limits: ReadLimitOptions | None = None
    ) -> Workbook: ...
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
    def inspect_defined_name(
        self,
        name: str,
        *,
        current_sheet: str | None = None,
    ) -> DefinedNameInspection: ...
    def function_usage(self) -> FunctionUsageReport: ...
    def data_validations(
        self,
        sheet: str,
        *,
        offset: int | None = None,
        limit: int | None = None,
    ) -> DataValidationPage: ...
    def scan_capabilities(
        self,
        *,
        offset: int | None = None,
        limit: int | None = None,
        today_serial: float | None = None,
        now_serial: float | None = None,
    ) -> CapabilityPage: ...
    def calculate(
        self,
        *,
        today_serial: float | None = None,
        now_serial: float | None = None,
        random_seed: int | None = None,
        locale_profile: LocaleProfile | None = None,
        arithmetic_semantics: Literal["excel_near_zero", "ieee_754"] = "excel_near_zero",
        financial_solver_semantics: Literal[
            "excel_iteration_budget", "extended_search"
        ] = "excel_iteration_budget",
        calculation_limits: CalculationLimitOptions | None = None,
    ) -> CalculationReport: ...
    def recalculate(
        self,
        *,
        mode: Literal["auto", "incremental", "full"] = "auto",
        today_serial: float | None = None,
        now_serial: float | None = None,
        random_seed: int | None = None,
        locale_profile: LocaleProfile | None = None,
        arithmetic_semantics: Literal["excel_near_zero", "ieee_754"] = "excel_near_zero",
        financial_solver_semantics: Literal[
            "excel_iteration_budget", "extended_search"
        ] = "excel_iteration_budget",
        calculation_limits: CalculationLimitOptions | None = None,
    ) -> CalculationDelta: ...
    def preview_changes(
        self,
        expected_revision: int,
        changes: Sequence[WorkbookChangeV2],
        *,
        mode: Literal["auto", "incremental", "full"] = "auto",
        today_serial: float | None = None,
        now_serial: float | None = None,
        random_seed: int | None = None,
        locale_profile: LocaleProfile | None = None,
        arithmetic_semantics: Literal["excel_near_zero", "ieee_754"] = "excel_near_zero",
        financial_solver_semantics: Literal[
            "excel_iteration_budget", "extended_search"
        ] = "excel_iteration_budget",
        calculation_limits: CalculationLimitOptions | None = None,
    ) -> PreviewChanges: ...
    def preview_changes_page(
        self,
        preview_id: int,
        *,
        section: Literal[
            "affected", "evaluated", "preview_results", "preview_issues", "install_results"
        ],
        cursor: PreviewCursor | None = None,
        limit: int | None = None,
    ) -> TransactionImpactPage: ...
    def commit_preview(self, preview_id: int) -> WorkbookTransactionReceipt: ...
    def discard_preview(self, preview_id: int) -> None: ...
    def apply_changes(
        self,
        expected_revision: int,
        changes: Sequence[WorkbookChange],
    ) -> EditReceipt: ...
    def apply_changes_v2(
        self,
        expected_revision: int,
        changes: Sequence[WorkbookChangeV2],
    ) -> EditReceiptV2: ...
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
    def delete_sheet(self, name: str) -> None: ...
    def to_bytes(self, *, invalidate_unavailable: bool = False) -> bytes: ...
    def save(
        self,
        path: str | PathLike[str],
        *,
        invalidate_unavailable: bool = False,
        replace_existing: bool = False,
    ) -> WriteReport: ...
    def edits_to_bytes(self) -> bytes: ...
    def save_edits(
        self,
        path: str | PathLike[str],
        *,
        replace_existing: bool = False,
    ) -> EditWriteReport: ...
