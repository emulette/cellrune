//! Bounded XLSX/XLSM reading, deterministic calculation, editing, and writing.
//!
//! `CellRune` separates source data from recalculated results:
//!
//! 1. [`read_xlsx_path`], [`read_xlsx_bytes`], or [`read_xlsx`] creates an immutable
//!    [`WorkbookSnapshot`].
//! 2. [`calculate_workbook`] creates a separate owned [`CalculationSnapshot`] without changing
//!    the source workbook or its saved XLSX results.
//! 3. Each formula result is either a typed [`CalculationCellResult::Value`] or a structured
//!    [`CalculationCellResult::Unavailable`] issue.
//!
//! # Quick start
//!
//! ```no_run
//! use cellrune::{
//!     CalculationCellResult, CalculationOptions, FiniteNumber, ReadOptions, calculate_workbook,
//!     read_xlsx_path,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let workbook = read_xlsx_path("input.xlsx", ReadOptions::default())?;
//!
//! let options = CalculationOptions::default()
//!     .with_today_serial(FiniteNumber::new(46_225.0)?);
//! let calculation = calculate_workbook(&workbook, options);
//!
//! if let Some(sheet) = workbook.sheet_by_name("Sheet1") {
//!     let source_cell = sheet.cell_by_a1("A1")?;
//!     let _ = source_cell;
//! }
//!
//! for (cell, result) in calculation.cells() {
//!     match result {
//!         CalculationCellResult::Value(value) => println!("{cell:?}: {value:?}"),
//!         CalculationCellResult::Unavailable(issue) => {
//!             eprintln!("{cell:?}: {}", issue.code().as_str());
//!         }
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Failure model
//!
//! `CellRune` keeps failures at their owning boundary:
//!
//! - [`XlsxReadError`] means no trustworthy workbook snapshot could be produced. Its
//!   [`XlsxReadError::code`] is stable and machine-readable.
//! - [`Diagnostic`] records a compatibility caveat on a successfully read workbook.
//! - [`CalculationIssue`] explains why one formula has no recalculated value. Unsupported engine
//!   capabilities are not converted into Excel errors and cannot be hidden by `IFERROR`.
//! - [`CellValue::Error`] represents an actual spreadsheet error value.
//! - [`ValidationError`] rejects invalid caller-provided model values and addresses.
//!
//! Reading never executes macros or follows external links. Calculation is explicit and does not
//! mutate or implicitly write a workbook. Recalculated result materialization is a separate
//! explicit operation on package-backed documents.
//!
//! # Numeric contract
//!
//! Calculated numbers are not guaranteed to be bit-identical to Excel's.
//! [`docs/NUMERICS.md`](https://github.com/emulette/cellrune/blob/main/docs/NUMERICS.md)
//! records every known deliberate difference, the Excel build each statement was
//! measured against, and which function families remain unmeasured. Compare results with a
//! tolerance rather than for equality.
//!
//! Two behaviors are selectable through [`CalculationOptions`], and both default to matching
//! Excel rather than to what releases up to 0.1.2 did:
//!
//! - [`ArithmeticSemantics`] decides whether Excel's narrow near-zero correction is applied to a
//!   decimal/rational cancellation, or every IEEE-754 residue is preserved.
//! - [`FinancialSolverSemantics`] decides whether `IRR`, `XIRR`, and `RATE` stop at the iteration
//!   budget Microsoft documents, or search longer and return values where Excel reports `#NUM!`.
//!
//! Select [`ArithmeticSemantics::Ieee754`] and [`FinancialSolverSemantics::ExtendedSearch`] to
//! restore the 0.1.2 behavior.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod address;
mod calculation;
mod cell;
mod defined_name;
mod diagnostic;
mod draft;
mod error;
mod formula;
mod presentation;
mod table;
mod workbook;
mod xlsx;

pub(crate) fn case_insensitive_eq(left: &str, right: &str) -> bool {
    left.chars()
        .flat_map(char::to_lowercase)
        .eq(right.chars().flat_map(char::to_lowercase))
}

pub use address::{CellAddress, CellRange, Column, EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS, Row};
pub use calculation::{
    ApplyChangesError, ArithmeticSemantics, CalculationCellId, CalculationCellResult,
    CalculationDecisionReason, CalculationDelta, CalculationDeltaCell, CalculationDeltaPage,
    CalculationExecutionMode, CalculationIssue, CalculationIssueCode, CalculationLimits,
    CalculationOptions, CalculationOptionsError, CalculationSnapshot, CancellationToken,
    CompletedCalculation, DefinedNameAnalysis, DefinedNameAnalysisError,
    DefinedNameAnalysisErrorKind, DefinedNameAnalysisLimitKind, DefinedNameAnalysisOptions,
    DefinedNameAnalysisOptionsError, DefinedNameDynamicKind, DefinedNameExternalReference,
    DefinedNameExternalTargetKind, DefinedNameInvalidReason, DefinedNameReferenceArea,
    DefinedNameSheetSpan, DefinedNameUnsupportedReason, FinancialSolverSemantics,
    FormulaCapability, FormulaCapabilityEntry, FormulaCapabilityReport, FunctionCatalogEntry,
    FunctionSupport, FunctionUsageEntry, FunctionUsageReport, MaterializedCalculationCell,
    MaterializedResultOrigin, PreparedCalculation, PreparedEditBatch, RecalculationMode,
    SessionError, SessionErrorCode, SessionLimits, WorkbookCalculationSession,
    analyze_defined_name, analyze_defined_name_cancellable, analyze_defined_name_with_options,
    calculate_workbook, scan_formula_capabilities, scan_formula_capabilities_with_options,
    scan_function_usage, scan_function_usage_with_options, supported_function_catalog,
};
pub use cell::{
    Cell, CellContent, CellValue, ExcelError, FiniteNumber, NumberFormat, NumberFormatKind,
};
pub use defined_name::{DefinedName, DefinedNameScope};
pub use diagnostic::{
    Diagnostic, DiagnosticCode, DiagnosticSeverity, InputHash, Provenance, ProviderIdentity,
    SourceId, SourceLocation,
};
pub use draft::{EditBatch, EditReceipt, WorkbookChange, WorkbookDraft};
pub use error::{ValidationError, ValidationErrorCode};
pub use formula::{
    FormulaCell, FormulaDialect, FormulaMetadata, FormulaText, SavedResult, SavedResultIssue,
    SharedFormulaRole,
};
pub use presentation::{
    CellPhonetics, ColumnPhoneticVisibility, DocumentPresentation, FrozenPane, PhoneticAlignment,
    PhoneticProperties, PhoneticRun, PhoneticTextRange, PhoneticType, PhoneticWriteOptions,
    ResolvedPhoneticRun,
};
pub(crate) use presentation::{CellPresentation, PhoneticAnnotation};
pub use table::{
    Table, TableAutoFilter, TableCalendarType, TableColorFilter, TableColumn, TableColumnId,
    TableColumnName, TableCustomFilter, TableCustomFilterOperator, TableCustomFilters,
    TableDateGroupItem, TableDateTimeGrouping, TableDateTimeValue, TableDynamicFilter,
    TableDynamicFilterType, TableFilterColumn, TableFilterCriteria, TableFilterItem, TableFormula,
    TableIconFilter, TableIconSet, TableId, TableName, TableNumericValue, TableSortBy,
    TableSortCondition, TableSortMethod, TableSortState, TableStyleInfo, TableTopFilter, TableType,
    TableValueFilters, TotalsRowFunction,
};
pub use workbook::{
    CalculationHints, CalculationMode, DateSystem, Sheet, SheetId, SheetName, SheetVisibility,
    WorkbookSnapshot, WorkbookSource, WorkbookSourceKind,
};
pub use xlsx::{
    OpenOptions, PackageSummary, ReadLimits, ReadOptions, ReadOptionsError, RecalculatedWorkbook,
    RecalculationWriteOptions, RecalculationWritePolicy, WriteLimits, WriteOptions,
    WriteOptionsError, WriteProvenance, WriteReport, XlsxDocument, XlsxDocumentKind, XlsxErrorCode,
    XlsxReadError, XlsxWriteError, XlsxWriteErrorCode, inspect_package, open_xlsx_document,
    open_xlsx_document_bytes, open_xlsx_document_path, read_xlsx, read_xlsx_bytes, read_xlsx_path,
    write_preserved_xlsx_bytes, write_recalculated_xlsx, write_recalculated_xlsx_bytes,
    write_recalculated_xlsx_path, write_xlsx_draft, write_xlsx_draft_bytes, write_xlsx_draft_path,
};

/// Test/benchmark-only access to the internal work counters.
///
/// These symbols are hidden from the public documentation and exist only so
/// integration tests and benches can reset and read the deterministic
/// performance-axis counters without exposing them as a supported API.
#[doc(hidden)]
pub mod testing {
    pub use crate::calculation::performance_counters::{
        WorkCounter, WorkCounterSnapshot, lock_work_counters, reset_work_counters,
        snapshot_work_counters,
    };
}
