use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationLimits, CalculationOptions,
    CancellationToken, CellAddress, CellContent, CellValue, EditBatch, FiniteNumber, FormulaText,
    InstallDeltaBasisReason, RecalculationMode, SessionErrorCode, SessionLimits, SheetId,
    TransactionDetailItem, TransactionDetailSection, TransactionImpactCause,
    TransactionImpactCoverage, TransactionIssueChangeKind, WorkbookCalculationSession,
    WorkbookChange,
};

fn sheet() -> SheetId {
    SheetId::new(1).expect("constant sheet ID")
}

fn address(value: &str) -> CellAddress {
    CellAddress::from_a1(value).expect("valid test address")
}

fn number(value: f64) -> CellValue {
    CellValue::Number(FiniteNumber::new(value).expect("finite test number"))
}

fn formula(value: &str) -> FormulaText {
    FormulaText::from_xlsx(value).expect("valid test formula")
}

fn set_value(cell: &str, value: f64) -> WorkbookChange {
    WorkbookChange::set_cell_value(sheet(), address(cell), number(value))
}

fn calculated_session() -> WorkbookCalculationSession {
    let mut session = WorkbookCalculationSession::create();
    session
        .apply_changes(
            0,
            EditBatch::new([
                set_value("A1", 1.0),
                WorkbookChange::set_cell_formula(sheet(), address("B1"), formula("A1+1")),
                WorkbookChange::set_cell_formula(sheet(), address("C1"), formula("B1+1")),
            ]),
        )
        .expect("initial edit installs");
    session
        .recalculate(
            RecalculationMode::Auto,
            CalculationOptions::default(),
            CancellationToken::new(),
        )
        .expect("initial calculation installs");
    session
}

fn result_number(result: &CalculationCellResult) -> f64 {
    match result {
        CalculationCellResult::Value(CellValue::Number(value)) => value.get(),
        other => panic!("expected numeric result, got {other:?}"),
    }
}

fn detail_cell(item: &TransactionDetailItem) -> Option<CalculationCellId> {
    match item {
        TransactionDetailItem::Affected(detail) => Some(detail.cell()),
        TransactionDetailItem::Evaluated(cell) => Some(*cell),
        TransactionDetailItem::PreviewResult(change) => Some(change.cell()),
        TransactionDetailItem::PreviewIssue(change) => Some(change.cell()),
        TransactionDetailItem::InstallResult(change) => Some(change.cell()),
        _ => None,
    }
}

#[path = "workbook_transaction/lifecycle.rs"]
mod lifecycle;
#[path = "workbook_transaction/paging.rs"]
mod paging;
#[path = "workbook_transaction/semantics.rs"]
mod semantics;
