use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationLimits, CalculationOptions, CellValue,
    ExcelError, calculate_workbook,
};

use super::support::{assert_issue, assert_number, cell_id, workbook_with_formulas};

#[test]
fn database_criteria_keep_their_own_text_and_row_boolean_semantics() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "\"Name\""),
        (1, 2, "\"Amount\""),
        (2, 1, "\"Alice\""),
        (2, 2, "10"),
        (3, 1, "\"Bob\""),
        (3, 2, "20"),
        (4, 1, "\"Alfred\""),
        (4, 2, "30"),
        (1, 4, "\"Name\""),
        (2, 4, "\"Al\""),
        (1, 5, "\"Name\""),
        (2, 5, "\"=Al\""),
        (1, 6, "\"Rule\""),
        (2, 6, "B2>15"),
        (1, 8, "DSUM(A1:B4,\"Amount\",D1:D2)"),
        (1, 9, "DSUM(A1:B4,\"Amount\",E1:E2)"),
        (1, 10, "DSUM(A1:B4,\"Amount\",F1:F2)"),
        (1, 11, "DCOUNT(A1:B4,,D1:D2)"),
        (1, 12, "DCOUNTA(A1:B4,,D1:D2)"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    for (column, expected) in [(8, 40.0), (9, 0.0), (10, 50.0), (11, 2.0), (12, 2.0)] {
        assert_number(&calculation, column, expected, 0.0);
    }
}

#[test]
fn database_criteria_rows_are_or_groups_with_and_conditions() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "\"Name\""),
        (1, 2, "\"Amount\""),
        (2, 1, "\"Alice\""),
        (2, 2, "10"),
        (3, 1, "\"Bob\""),
        (3, 2, "20"),
        (4, 1, "\"Alfred\""),
        (4, 2, "30"),
        (1, 4, "\"Name\""),
        (1, 5, "\"Amount\""),
        (2, 4, "\"Al\""),
        (2, 5, "\">20\""),
        (3, 4, "\"Bob\""),
        (1, 7, "DSUM(A1:B4,\"Amount\",D1:E3)"),
        (4, 4, "\"\""),
        (4, 5, "\"\""),
        (1, 8, "DSUM(A1:B4,\"Amount\",D1:E4)"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    assert_number(&calculation, 7, 50.0, 0.0);
    assert_number(&calculation, 8, 60.0, 0.0);
}

#[test]
fn database_formula_criteria_reject_unsafe_or_non_boolean_formulas_before_scanning() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "\"Name\""),
        (1, 2, "\"Amount\""),
        (2, 1, "\"Alice\""),
        (2, 2, "10"),
        (3, 1, "\"Bob\""),
        (3, 2, "20"),
        (1, 4, "\"Rule\""),
        (2, 4, "B3>0"),
        (1, 5, "\"Rule\""),
        (2, 5, "OFFSET(B2,0,0)>0"),
        (1, 6, "\"Rule\""),
        (2, 6, "B2+1"),
        (1, 7, "\"Rule\""),
        (2, 7, "\"\""),
        (1, 13, "\"Rule\""),
        (2, 13, "B:B"),
        (1, 14, "\"Rule\""),
        (2, 14, "J$1>0"),
        (1, 17, "\"Rule\""),
        (2, 17, "INDEX(A1:B3,2,2)>0"),
        (1, 8, "DSUM(A1:B3,\"Amount\",D1:D2)"),
        (1, 9, "DSUM(A1:B3,\"Amount\",E1:E2)"),
        (1, 10, "DSUM(A1:B3,\"Amount\",F1:F2)"),
        (1, 11, "DSUM(A1:B3,\"Amount\",G1:G2)"),
        (1, 15, "DSUM(A1:B3,\"Amount\",M1:M2)"),
        (1, 16, "DSUM(A1:B3,\"Amount\",N1:N2)"),
        (1, 18, "DSUM(A1:B3,\"Amount\",Q1:Q2)"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    for column in [8, 9, 10, 11, 15, 16, 18] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            ))),
        );
    }
}

#[test]
fn database_formula_criteria_accept_only_absolute_defined_name_targets() {
    let workbook = super::workbook_with_formulas_and_names(
        &[
            (1, 1, "\"Name\""),
            (1, 2, "\"Amount\""),
            (2, 1, "\"Alice\""),
            (2, 2, "10"),
            (3, 1, "\"Bob\""),
            (3, 2, "20"),
            (1, 4, "\"Rule\""),
            (2, 4, "AbsoluteTarget>0"),
            (1, 5, "\"Rule\""),
            (2, 5, "RelativeTarget>0"),
            (1, 7, "DSUM(A1:B3,\"Amount\",D1:D2)"),
            (1, 8, "DSUM(A1:B3,\"Amount\",E1:E2)"),
            (1, 10, "1"),
        ],
        &[("AbsoluteTarget", "Sheet1!$J$1"), ("RelativeTarget", "J1")],
    );
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    assert_number(&calculation, 7, 30.0, 0.0);
    assert_eq!(
        calculation.cell(cell_id(8)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Value
        ))),
    );
}

#[test]
fn database_field_and_dget_contracts_are_strict() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "\"Name\""),
        (1, 2, "\"Amount\""),
        (2, 1, "\"Alice\""),
        (2, 2, "10"),
        (3, 1, "\"Bob\""),
        (3, 2, "20"),
        (1, 4, "\"Name\""),
        (2, 4, "\"Bob\""),
        (1, 5, "\"Name\""),
        (2, 5, "\"Nobody\""),
        (1, 6, "\"Name\""),
        (2, 6, "\"\""),
        (1, 8, "DGET(A1:B3,\"Amount\",D1:D2)"),
        (1, 9, "DGET(A1:B3,\"Amount\",E1:E2)"),
        (1, 10, "DGET(A1:B3,\"Amount\",F1:F2)"),
        (1, 11, "DSUM(A1:B3,3,D1:D2)"),
        (1, 12, "DSUM(A1:B3,,D1:D2)"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    assert_number(&calculation, 8, 20.0, 0.0);
    for (column, error) in [
        (9, ExcelError::Value),
        (10, ExcelError::Number),
        (11, ExcelError::Value),
        (12, ExcelError::Value),
    ] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(error))),
        );
    }
}

#[test]
fn duplicate_database_headers_require_an_unambiguous_numeric_field() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "\"Name\""),
        (1, 2, "\"Amount\""),
        (1, 3, "\"Amount\""),
        (2, 1, "\"Alice\""),
        (2, 2, "10"),
        (2, 3, "100"),
        (3, 1, "\"Bob\""),
        (3, 2, "20"),
        (3, 3, "200"),
        (1, 5, "\"Name\""),
        (2, 5, "\"Alice\""),
        (1, 7, "DSUM(A1:C3,\"Amount\",E1:E2)"),
        (1, 8, "DSUM(A1:C3,2,E1:E2)"),
        (1, 9, "DSUM(A1:C3,3,E1:E2)"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    assert_eq!(
        calculation.cell(cell_id(7)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Value
        ))),
    );
    assert_number(&calculation, 8, 10.0, 0.0);
    assert_number(&calculation, 9, 100.0, 0.0);
}

#[test]
fn database_scans_use_the_formula_cumulative_work_budget() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "\"Name\""),
        (1, 2, "\"Amount\""),
        (2, 1, "\"Alice\""),
        (2, 2, "10"),
        (3, 1, "\"Bob\""),
        (3, 2, "20"),
        (4, 1, "\"Alfred\""),
        (4, 2, "30"),
        (1, 4, "\"Name\""),
        (2, 4, "\"\""),
        (1, 6, "IFERROR(DSUM(A1:B4,\"Amount\",D1:D2),999)"),
    ]);
    let limits = CalculationLimits::default()
        .with_max_function_iterations(10)
        .expect("nonzero database scan limit");
    let calculation =
        calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));

    assert_issue(&calculation, 6, CalculationIssueCode::ResourceLimitExceeded);
}
