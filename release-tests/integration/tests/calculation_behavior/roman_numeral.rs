use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationLimits, CalculationOptions, CellValue,
    ExcelError, calculate_workbook, scan_formula_capabilities,
};

use super::support::{assert_issue, assert_number, cell_id, workbook_with_formulas};

#[test]
fn roman_numeral_conversion_matches_the_frozen_excel_contract() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "ARABIC(\"MMXXVI\")"),
        (1, 2, "ARABIC(\"  mxmvii  \")"),
        (1, 3, "ARABIC(\"-MMXI\")"),
        (1, 4, "ARABIC(\" \")"),
        (1, 5, "ARABIC(\"not-roman\")"),
        (1, 6, "ARABIC(12)"),
        (1, 7, "ARABIC(TRUE)"),
        (1, 8, "ROMAN(2026)"),
        (1, 9, "ROMAN(\"2\",0)"),
        (1, 10, "ROMAN(0,0)"),
        (1, 11, "ROMAN(499,0)"),
        (1, 12, "ROMAN(499,1)"),
        (1, 13, "ROMAN(499,2)"),
        (1, 14, "ROMAN(499,3)"),
        (1, 15, "ROMAN(499,4)"),
        (1, 16, "ROMAN(499,TRUE)"),
        (1, 17, "ROMAN(499,FALSE)"),
        (1, 18, "ROMAN(-1)"),
        (1, 19, "ROMAN(4000)"),
        (1, 20, "ROMAN(1,5)"),
        (1, 21, "ROMAN(3.9,0)"),
        (1, 22, "ROMAN(499,1.9)"),
        (1, 23, "ARABIC(\"\")"),
        (1, 24, "ROMAN(TRUE)"),
    ]);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    for (column, expected) in [(1, 2026.0), (2, 1997.0), (3, -2011.0), (4, 0.0), (23, 0.0)] {
        assert_number(&calculation, column, expected, 0.0);
    }
    for (column, expected) in [
        (8, "MMXXVI"),
        (9, "II"),
        (10, ""),
        (11, "CDXCIX"),
        (12, "LDVLIV"),
        (13, "XDIX"),
        (14, "VDIV"),
        (15, "ID"),
        (16, "CDXCIX"),
        (17, "ID"),
        (21, "III"),
        (22, "LDVLIV"),
        (24, "I"),
    ] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Text(
                expected.to_owned()
            ))),
            "unexpected Roman text in column {column}",
        );
    }
    for column in [5, 6, 7, 18, 19, 20] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            ))),
            "unexpected boundary result in column {column}",
        );
    }
}

#[test]
fn roman_numeral_conversion_respects_work_and_text_limits() {
    let arabic_formula = format!("ARABIC(\"{}\")", "M".repeat(20));
    let arabic = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, arabic_formula.as_str())]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(10)
                .expect("positive Roman parsing work limit"),
        ),
    );
    assert_issue(&arabic, 1, CalculationIssueCode::ResourceLimitExceeded);

    let roman = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "ROMAN(3999,0)")]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_text_bytes(8)
                .expect("positive Roman output limit"),
        ),
    );
    assert_issue(&roman, 1, CalculationIssueCode::ResourceLimitExceeded);
}
