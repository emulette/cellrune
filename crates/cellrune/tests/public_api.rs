//! The public-surface semver invariant.
//!
//! Sixteen exported enums are not `#[non_exhaustive]`: adding a variant to any of them is a
//! breaking change, and under Cargo's 0.x rules a breaking change requires 0.2.0. This is one
//! compile-time invariant inside the normal test suite, not one test per enum. The matches below
//! deliberately have no wildcard arm, so the invariant grows as data while the test surface stays
//! fixed. Every variant payload is pinned at its exact type through [`payload`], whose
//! inference cannot deref-coerce, so changing a payload's type — including wrapping it in a
//! `Box` or another `Deref` wrapper — is a compile error too, not only adding a variant.
//!
//! The function-pointer constants freeze positional signatures for the same reason: an extra
//! positional argument or a changed parameter type is a breaking change. Extension happens
//! through builder methods and option structs, not through these signatures. Pinned are
//! `WorkbookSnapshot::new_with_metadata` and the monomorphic read → calculate → write entry
//! points; their generic companions (`read_xlsx_path`, `write_recalculated_xlsx`, …) share the
//! same shapes minus the generic parameter and are not separately pinnable as function pointers.
//! Trait implementations are not pinned here.
//!
//! When a change here is intentional, it must ship in 0.2.0 and be recorded in the changelog;
//! updating this file is part of that decision, never a side effect.

use std::marker::PhantomData;

use cellrune::{
    ApplyChangesError, CalculationCellResult, CalculationExecutionMode, CalculationHints,
    CalculationIssue, CalculationMode, CalculationOptions, CalculationSnapshot, CancellationToken,
    CellAddress, CellContent, CellRange, CellValue, CompletedWorkbookTransaction, DateSystem,
    DefinedName, DefinedNameAnalysis, DefinedNameAnalysisError, DefinedNameAnalysisOptions,
    DefinedNameExternalReference, DefinedNameExternalTargetKind, DefinedNameScope, Diagnostic,
    DiagnosticSeverity, EditBatch, FormulaCapability, FormulaCapabilityReport, FormulaCell,
    FunctionSupport, NumberFormatKind, PreparedWorkbookTransaction, Provenance, ReadOptions,
    RecalculatedWorkbook, RecalculationMode, RecalculationWriteOptions, SavedResult,
    SavedResultIssue, SessionError, SharedFormulaRole, Sheet, SheetId, SheetVisibility, Table,
    TableColumn, TableColumnId, TableId, TableName, TransactionDetailSection,
    TransactionImpactPage, TransactionPageCursor, ValidationError, WorkbookCalculationSession,
    WorkbookFingerprint, WorkbookSnapshot, WorkbookSource, WorkbookSourceKind,
    WorkbookTransactionReceipt, WorkbookTransactionReport, XlsxDocument, XlsxReadError,
    XlsxWriteError, analyze_defined_name, analyze_defined_name_cancellable,
    analyze_defined_name_with_options, calculate_workbook, read_xlsx_bytes,
    scan_formula_capabilities, write_recalculated_xlsx_bytes,
};

/// Captures a payload binding's exact type. The intermediate `let pinned = payload(…);` at every
/// call site is load-bearing: it has no expected type, so `T` unifies with the binding itself and
/// deref coercion cannot substitute a `Box`/`Arc` wrapper. Assigning the result to
/// `PhantomData<Expected>` afterwards is then an exact-type equation. A one-step
/// `let _: &Expected = binding;` would be a coercion site and would miss wrapper changes.
fn payload<T>(_: &T) -> PhantomData<T> {
    PhantomData
}

fn frozen_cell_content(content: &CellContent) {
    match content {
        CellContent::Literal(value) => {
            let pinned = payload(value);
            let _: PhantomData<CellValue> = pinned;
        }
        CellContent::Formula(formula) => {
            let pinned = payload(formula);
            let _: PhantomData<FormulaCell> = pinned;
        }
    }
}

fn frozen_sheet_visibility(visibility: SheetVisibility) {
    match visibility {
        SheetVisibility::Visible | SheetVisibility::Hidden | SheetVisibility::VeryHidden => {}
    }
}

fn frozen_date_system(date_system: DateSystem) {
    match date_system {
        DateSystem::Excel1900 | DateSystem::Excel1904 => {}
    }
}

fn frozen_calculation_mode(mode: CalculationMode) {
    match mode {
        CalculationMode::Automatic
        | CalculationMode::AutomaticExceptDataTables
        | CalculationMode::Manual => {}
    }
}

fn frozen_defined_name_scope(scope: &DefinedNameScope) {
    match scope {
        DefinedNameScope::Workbook => {}
        DefinedNameScope::Sheet(sheet) => {
            let pinned = payload(sheet);
            let _: PhantomData<SheetId> = pinned;
        }
    }
}

fn frozen_defined_name_external_reference_api(detail: &DefinedNameExternalReference) {
    let _: Option<&str> = detail.locator();
    let _: &str = detail.workbook();
    let _: Option<&str> = detail.sheet();
    let _: Option<&str> = detail.sheet_end();
    let _: DefinedNameExternalTargetKind = detail.target();
    let _: &str = detail.target_text();
}

fn frozen_saved_result(result: &SavedResult) {
    match result {
        SavedResult::Missing => {}
        SavedResult::Present(value) => {
            let pinned = payload(value);
            let _: PhantomData<CellValue> = pinned;
        }
        SavedResult::Invalid(issue) => {
            let pinned = payload(issue);
            let _: PhantomData<SavedResultIssue> = pinned;
        }
    }
}

fn frozen_recalculation_mode(mode: RecalculationMode) {
    match mode {
        RecalculationMode::Auto | RecalculationMode::Incremental | RecalculationMode::Full => {}
    }
}

fn frozen_calculation_execution_mode(mode: CalculationExecutionMode) {
    match mode {
        CalculationExecutionMode::Incremental | CalculationExecutionMode::Full => {}
    }
}

fn frozen_apply_changes_error(error: &ApplyChangesError) {
    match error {
        ApplyChangesError::Session(session) => {
            let pinned = payload(session);
            let _: PhantomData<SessionError> = pinned;
        }
        ApplyChangesError::Validation(validation) => {
            let pinned = payload(validation);
            let _: PhantomData<ValidationError> = pinned;
        }
    }
}

fn frozen_formula_capability(capability: &FormulaCapability) {
    match capability {
        FormulaCapability::Supported => {}
        FormulaCapability::Unsupported(issues) => {
            let pinned = payload(issues);
            let _: PhantomData<Vec<CalculationIssue>> = pinned;
        }
    }
}

fn frozen_function_support(support: FunctionSupport) {
    match support {
        FunctionSupport::Supported | FunctionSupport::Unsupported => {}
    }
}

fn frozen_calculation_cell_result(result: &CalculationCellResult) {
    match result {
        CalculationCellResult::Value(value) => {
            let pinned = payload(value);
            let _: PhantomData<CellValue> = pinned;
        }
        CalculationCellResult::Unavailable(issue) => {
            let pinned = payload(issue);
            let _: PhantomData<CalculationIssue> = pinned;
        }
    }
}

fn frozen_number_format_kind(kind: NumberFormatKind) {
    match kind {
        NumberFormatKind::General
        | NumberFormatKind::Number
        | NumberFormatKind::Date
        | NumberFormatKind::Time
        | NumberFormatKind::DateTime
        | NumberFormatKind::Duration => {}
    }
}

fn frozen_diagnostic_severity(severity: DiagnosticSeverity) {
    match severity {
        DiagnosticSeverity::Info | DiagnosticSeverity::Warning | DiagnosticSeverity::Error => {}
    }
}

fn frozen_shared_formula_role(role: SharedFormulaRole) {
    match role {
        SharedFormulaRole::Anchor => {}
        SharedFormulaRole::Follower { anchor } => {
            let pinned = payload(&anchor);
            let _: PhantomData<CellAddress> = pinned;
        }
    }
}

fn frozen_workbook_source_kind(kind: WorkbookSourceKind) {
    match kind {
        WorkbookSourceKind::Unknown
        | WorkbookSourceKind::Path
        | WorkbookSourceKind::Bytes
        | WorkbookSourceKind::Reader => {}
    }
}

fn frozen_table_column_scalar_api() {
    let id: u32 = 1;
    let column = TableColumn::new(id, "Borrowed", None).expect("valid borrowed name");
    let _owned = TableColumn::new(id, String::from("Owned"), None).expect("valid owned name");
    let _: u32 = column.id();
    let _: TableColumnId = column.column_id();
}

#[allow(clippy::type_complexity)]
const _FROZEN_TABLE_NEW: fn(
    TableId,
    TableName,
    TableName,
    CellRange,
    u32,
    u32,
    Vec<TableColumn>,
) -> Result<Table, ValidationError> = Table::new;

#[allow(clippy::type_complexity)]
const _FROZEN_NEW_WITH_METADATA: fn(
    Vec<Sheet>,
    Vec<DefinedName>,
    Vec<Diagnostic>,
    DateSystem,
    CalculationHints,
    WorkbookSource,
    Provenance,
) -> Result<WorkbookSnapshot, ValidationError> = WorkbookSnapshot::new_with_metadata;

const _FROZEN_READ_XLSX_BYTES: fn(&[u8], ReadOptions) -> Result<WorkbookSnapshot, XlsxReadError> =
    read_xlsx_bytes;

const _FROZEN_CALCULATE_WORKBOOK: fn(&WorkbookSnapshot, CalculationOptions) -> CalculationSnapshot =
    calculate_workbook;

const _FROZEN_WORKBOOK_FINGERPRINT: fn(&WorkbookSnapshot) -> WorkbookFingerprint =
    WorkbookSnapshot::fingerprint;

const _FROZEN_CALCULATION_SOURCE_FINGERPRINT: fn(&CalculationSnapshot) -> WorkbookFingerprint =
    CalculationSnapshot::source_fingerprint;

#[allow(clippy::type_complexity)]
const _FROZEN_PREPARE_TRANSACTION: fn(
    &WorkbookCalculationSession,
    u64,
    EditBatch,
    RecalculationMode,
    CalculationOptions,
    CancellationToken,
) -> Result<PreparedWorkbookTransaction, ApplyChangesError> =
    WorkbookCalculationSession::prepare_transaction;

const _FROZEN_RUN_TRANSACTION: fn(
    PreparedWorkbookTransaction,
) -> Result<CompletedWorkbookTransaction, SessionError> = PreparedWorkbookTransaction::run;

const _FROZEN_TRANSACTION_REPORT: fn(&CompletedWorkbookTransaction) -> &WorkbookTransactionReport =
    CompletedWorkbookTransaction::report;

#[allow(clippy::type_complexity)]
const _FROZEN_TRANSACTION_PAGE: fn(
    &CompletedWorkbookTransaction,
    TransactionDetailSection,
    Option<&TransactionPageCursor>,
    usize,
) -> Result<TransactionImpactPage, SessionError> = CompletedWorkbookTransaction::page;

#[allow(clippy::type_complexity)]
const _FROZEN_TRANSACTION_PAGE_CANCELLABLE: fn(
    &CompletedWorkbookTransaction,
    TransactionDetailSection,
    Option<&TransactionPageCursor>,
    usize,
    &CancellationToken,
) -> Result<TransactionImpactPage, SessionError> = CompletedWorkbookTransaction::page_cancellable;

#[allow(clippy::type_complexity)]
const _FROZEN_TRANSACTION_PAGE_FROM_TOKEN: fn(
    &CompletedWorkbookTransaction,
    TransactionDetailSection,
    Option<&str>,
    usize,
) -> Result<TransactionImpactPage, SessionError> = CompletedWorkbookTransaction::page_from_token;

#[allow(clippy::type_complexity)]
const _FROZEN_TRANSACTION_PAGE_FROM_TOKEN_CANCELLABLE: fn(
    &CompletedWorkbookTransaction,
    TransactionDetailSection,
    Option<&str>,
    usize,
    &CancellationToken,
) -> Result<
    TransactionImpactPage,
    SessionError,
> = CompletedWorkbookTransaction::page_from_token_cancellable;

const _FROZEN_TRANSACTION_CURSOR_TOKEN: fn(&TransactionPageCursor) -> String =
    TransactionPageCursor::to_token;

const _FROZEN_DISCARD_TRANSACTION: fn(
    &mut CompletedWorkbookTransaction,
) -> Result<(), SessionError> = CompletedWorkbookTransaction::discard;

const _FROZEN_INSTALL_TRANSACTION: fn(
    &mut WorkbookCalculationSession,
    &mut CompletedWorkbookTransaction,
) -> Result<WorkbookTransactionReceipt, SessionError> =
    WorkbookCalculationSession::install_transaction;

const _FROZEN_INSTALL_TRANSACTION_CANCELLABLE: fn(
    &mut WorkbookCalculationSession,
    &mut CompletedWorkbookTransaction,
    &CancellationToken,
) -> Result<
    WorkbookTransactionReceipt,
    SessionError,
> = WorkbookCalculationSession::install_transaction_cancellable;

const _FROZEN_SCAN_FORMULA_CAPABILITIES: fn(&WorkbookSnapshot) -> FormulaCapabilityReport =
    scan_formula_capabilities;

#[allow(clippy::type_complexity)]
const _FROZEN_ANALYZE_DEFINED_NAME: fn(
    &WorkbookSnapshot,
    &str,
    Option<SheetId>,
) -> Result<DefinedNameAnalysis, DefinedNameAnalysisError> = analyze_defined_name;

#[allow(clippy::type_complexity)]
const _FROZEN_ANALYZE_DEFINED_NAME_WITH_OPTIONS: fn(
    &WorkbookSnapshot,
    &str,
    Option<SheetId>,
    DefinedNameAnalysisOptions,
) -> Result<
    DefinedNameAnalysis,
    DefinedNameAnalysisError,
> = analyze_defined_name_with_options;

#[allow(clippy::type_complexity)]
const _FROZEN_ANALYZE_DEFINED_NAME_CANCELLABLE: fn(
    &WorkbookSnapshot,
    &str,
    Option<SheetId>,
    DefinedNameAnalysisOptions,
    &CancellationToken,
) -> Result<
    DefinedNameAnalysis,
    DefinedNameAnalysisError,
> = analyze_defined_name_cancellable;

#[allow(clippy::type_complexity)]
const _FROZEN_WRITE_RECALCULATED_XLSX_BYTES: fn(
    &XlsxDocument,
    &CalculationSnapshot,
    RecalculationWriteOptions,
) -> Result<RecalculatedWorkbook, XlsxWriteError> = write_recalculated_xlsx_bytes;

#[test]
fn the_frozen_enums_are_exhaustively_matched() {
    frozen_cell_content(&CellContent::Literal(CellValue::Blank));
    frozen_sheet_visibility(SheetVisibility::Visible);
    frozen_sheet_visibility(SheetVisibility::Hidden);
    frozen_sheet_visibility(SheetVisibility::VeryHidden);
    frozen_date_system(DateSystem::Excel1900);
    frozen_date_system(DateSystem::Excel1904);
    frozen_calculation_mode(CalculationMode::Automatic);
    frozen_calculation_mode(CalculationMode::AutomaticExceptDataTables);
    frozen_calculation_mode(CalculationMode::Manual);
    frozen_defined_name_scope(&DefinedNameScope::Workbook);
    let _: fn(&DefinedNameExternalReference) = frozen_defined_name_external_reference_api;
    frozen_saved_result(&SavedResult::Missing);
    frozen_recalculation_mode(RecalculationMode::Auto);
    frozen_calculation_execution_mode(CalculationExecutionMode::Full);
    frozen_apply_changes_error(&ApplyChangesError::Validation(
        ValidationError::CellAddressInvalid,
    ));
    frozen_formula_capability(&FormulaCapability::Supported);
    frozen_function_support(FunctionSupport::Supported);
    frozen_calculation_cell_result(&CalculationCellResult::Value(CellValue::Blank));
    frozen_number_format_kind(NumberFormatKind::General);
    frozen_diagnostic_severity(DiagnosticSeverity::Info);
    frozen_shared_formula_role(SharedFormulaRole::Anchor);
    frozen_workbook_source_kind(WorkbookSourceKind::Unknown);
    frozen_table_column_scalar_api();
}
