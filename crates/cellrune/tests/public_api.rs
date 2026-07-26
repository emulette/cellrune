//! The public-surface semver invariant.
//!
//! Sixteen exported enums are not `#[non_exhaustive]`: adding a variant to any of them is a
//! breaking change, and under Cargo's 0.x rules a breaking change requires 0.2.0. This is one
//! compile-time invariant inside the normal test suite, not one test per enum. The matches below
//! deliberately have no wildcard arm, so the invariant grows as data while the test surface stays
//! fixed.
//!
//! The function-pointer constant freezes the positional signature of
//! `WorkbookSnapshot::new_with_metadata` for the same reason: an eighth positional argument is
//! a breaking change. Extension happens through builder methods, not through this signature.
//!
//! When a change here is intentional, it must ship in 0.2.0 and be recorded in the changelog;
//! updating this file is part of that decision, never a side effect.

use cellrune::{
    ApplyChangesError, CalculationCellResult, CalculationExecutionMode, CalculationHints,
    CalculationMode, CellContent, CellValue, DateSystem, DefinedName, DefinedNameScope, Diagnostic,
    DiagnosticSeverity, FormulaCapability, FunctionSupport, NumberFormatKind, Provenance,
    RecalculationMode, SavedResult, SharedFormulaRole, Sheet, SheetVisibility, ValidationError,
    WorkbookSnapshot, WorkbookSource, WorkbookSourceKind,
};

fn frozen_cell_content(content: &CellContent) {
    match content {
        CellContent::Literal(_) | CellContent::Formula(_) => {}
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
        DefinedNameScope::Workbook | DefinedNameScope::Sheet(_) => {}
    }
}

fn frozen_saved_result(result: &SavedResult) {
    match result {
        SavedResult::Missing | SavedResult::Present(_) | SavedResult::Invalid(_) => {}
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
        ApplyChangesError::Session(_) | ApplyChangesError::Validation(_) => {}
    }
}

fn frozen_formula_capability(capability: &FormulaCapability) {
    match capability {
        FormulaCapability::Supported | FormulaCapability::Unsupported(_) => {}
    }
}

fn frozen_function_support(support: FunctionSupport) {
    match support {
        FunctionSupport::Supported | FunctionSupport::Unsupported => {}
    }
}

fn frozen_calculation_cell_result(result: &CalculationCellResult) {
    match result {
        CalculationCellResult::Value(_) | CalculationCellResult::Unavailable(_) => {}
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
        SharedFormulaRole::Anchor | SharedFormulaRole::Follower { .. } => {}
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
}
