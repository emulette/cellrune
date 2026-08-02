use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationSnapshot, CellValue,
    WorkbookSnapshot,
};

pub(super) fn assert_issue(
    calculation: &CalculationSnapshot,
    column: u32,
    expected: CalculationIssueCode,
) {
    let Some(CalculationCellResult::Unavailable(issue)) = calculation.cell(cell_id(column)) else {
        panic!("expected unavailable calculation result in column {column}");
    };
    assert_eq!(issue.code(), expected);
}

pub(super) fn assert_number(
    calculation: &CalculationSnapshot,
    column: u32,
    expected: f64,
    tolerance: f64,
) {
    let Some(CalculationCellResult::Value(CellValue::Number(actual))) =
        calculation.cell(cell_id(column))
    else {
        panic!(
            "expected numeric calculation result in column {column}, got {:?}",
            calculation.cell(cell_id(column))
        );
    };
    assert!(
        (actual.get() - expected).abs() <= tolerance,
        "unexpected result in column {column}: expected {expected}, got {}",
        actual.get(),
    );
}

pub(super) fn cell_id(column: u32) -> CalculationCellId {
    super::calculation_cell_id(1, column)
}

pub(super) fn workbook_with_formulas(formulas: &[(u32, u32, &str)]) -> WorkbookSnapshot {
    super::workbook_with_formulas_and_names(formulas, &[])
}
