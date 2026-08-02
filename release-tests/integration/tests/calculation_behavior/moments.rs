use cellrune::{CalculationOptions, calculate_workbook};

use super::support::{assert_number, workbook_with_formulas};

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
