use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationLimits, CalculationOptions, CellValue,
    ExcelError, calculate_workbook,
};

use super::support::{assert_issue, assert_number, cell_id, workbook_with_formulas};

#[test]
fn variance_and_paired_statistics_share_stable_moments_across_input_shapes() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "1000000000001"),
        (2, 1, "1000000000002"),
        (3, 1, "1000000000003"),
        (4, 1, "1000000000004"),
        (1, 2, "3000000000003"),
        (2, 2, "3000000000006"),
        (3, 2, "3000000000009"),
        (4, 2, "3000000000012"),
        (1, 4, "VAR.S(A1:A4)"),
        (
            1,
            5,
            "VAR.S({1000000000001,1000000000002,1000000000003,1000000000004})",
        ),
        (
            1,
            6,
            "VAR.S(1000000000001,1000000000002,1000000000003,1000000000004)",
        ),
        (1, 7, "STDEV.S(A1:A4)"),
        (1, 8, "VAR.P(A1:A4)"),
        (1, 9, "STDEV.P(A1:A4)"),
        (1, 10, "COVARIANCE.P(B1:B4,A1:A4)"),
        (1, 11, "SLOPE(B1:B4,A1:A4)"),
        (1, 12, "INTERCEPT(B1:B4,A1:A4)"),
        (1, 13, "CORREL(B1:B4,A1:A4)"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    for column in [4, 5, 6] {
        assert_number(&calculation, column, 5.0 / 3.0, 1.0e-15);
    }
    for (column, expected, tolerance) in [
        (7, (5.0_f64 / 3.0).sqrt(), 1.0e-15),
        (8, 1.25, 0.0),
        (9, 1.25_f64.sqrt(), 1.0e-15),
        (10, 3.75, 0.0),
        (11, 3.0, 0.0),
        (12, 0.0, 0.0),
        (13, 1.0, 2.0e-15),
    ] {
        assert_number(&calculation, column, expected, tolerance);
    }
}

#[test]
fn skewness_and_kurtosis_standardize_the_documented_sample() {
    // Microsoft's SKEW, SKEW.P, and KURT examples share this data set and publish 0.359543,
    // 0.303193, and -0.151799637.
    let data = ["3", "4", "5", "2", "3", "4", "5", "6", "4", "7"];
    let mut formulas = data
        .iter()
        .enumerate()
        .map(|(row, value)| (row as u32 + 1, 1, *value))
        .collect::<Vec<_>>();
    formulas.extend([
        (1, 2, "SKEW(A1:A10)"),
        (1, 3, "_xlfn.SKEW.P(A1:A10)"),
        (1, 4, "KURT(A1:A10)"),
        // Typed text numbers and logical values count; referenced text and logical values do not.
        (1, 5, "SKEW(1,\"2\",TRUE,3,10)"),
        (1, 6, "SKEW.P(1,2,3,10,A11:A12)"),
        (1, 7, "KURT({1,2,3,10},A11:A12)"),
        (11, 1, "\"text\""),
        (12, 1, "TRUE"),
    ]);
    let calculation = calculate_workbook(
        &workbook_with_formulas(&formulas),
        CalculationOptions::default(),
    );
    for (column, expected) in [
        (2, 0.359_543_071_406_797_8),
        (3, 0.303_193_339_354_144_3),
        (4, -0.151_799_637_208_412_7),
        (5, 1.980_548_193_033_738_5),
        (6, 1.018_233_764_908_628_4),
        (7, 3.228),
    ] {
        assert_number(&calculation, column, expected, 1.0e-12);
    }
}

#[test]
fn skewness_and_kurtosis_need_enough_varying_values_and_propagate_errors() {
    let formulas = [
        ("SKEW(1,2)", ExcelError::DivisionByZero),
        ("SKEW.P(1,2)", ExcelError::DivisionByZero),
        ("KURT(1,2,3)", ExcelError::DivisionByZero),
        ("SKEW(4,4,4)", ExcelError::DivisionByZero),
        ("SKEW.P(4,4,4)", ExcelError::DivisionByZero),
        ("KURT(4,4,4,4)", ExcelError::DivisionByZero),
        ("SKEW({1,\"a\",2,TRUE})", ExcelError::DivisionByZero),
        ("SKEW(1,2,3,#N/A)", ExcelError::NotAvailable),
        ("KURT(1,2,3,\"four\")", ExcelError::Value),
    ];
    let calculation = calculate_workbook(
        &workbook_with_formulas(
            &formulas
                .iter()
                .enumerate()
                .map(|(index, (formula, _))| (1, index as u32 + 1, *formula))
                .collect::<Vec<_>>(),
        ),
        CalculationOptions::default(),
    );
    for (index, (formula, expected)) in formulas.iter().enumerate() {
        assert_eq!(
            calculation.cell(cell_id(index as u32 + 1)),
            Some(&CalculationCellResult::Value(CellValue::Error(*expected))),
            "{formula}",
        );
    }
}

#[test]
fn skewness_and_kurtosis_charge_both_passes_over_the_numbers() {
    for formula in ["SKEW({1,2,4,8})", "SKEW.P({1,2,4,8})", "KURT({1,2,4,8})"] {
        let calculate = |iterations: u64| {
            calculate_workbook(
                &workbook_with_formulas(&[(1, 1, formula)]),
                CalculationOptions::default().with_limits(
                    CalculationLimits::default()
                        .with_max_function_iterations(iterations)
                        .expect("valid limit"),
                ),
            )
        };
        // The mean pass and the standardizing pass each take one iteration per number.
        assert_issue(&calculate(7), 1, CalculationIssueCode::ResourceLimitExceeded);
        assert!(
            matches!(
                calculate(8).cell(cell_id(1)),
                Some(CalculationCellResult::Value(CellValue::Number(_)))
            ),
            "{formula}"
        );
    }
}
