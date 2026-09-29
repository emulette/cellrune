use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationLimits, CalculationOptions,
    CalculationSnapshot, CellValue, ExcelError, calculate_workbook, scan_formula_capabilities,
};

use super::support::{assert_issue, assert_number, cell_id, workbook_with_formulas};

#[test]
fn rank_counts_ties_in_both_directions_and_preserves_numeric_collection() {
    let formulas = [
        ("RANK(2,{3,2,2,1})", 2.0),
        ("RANK.EQ(2,{3,2,2,1},1)", 2.0),
        ("RANK.AVG(2,{3,2,2,1})", 2.5),
        ("_xlfn.RANK.AVG(2,{3,2,2,1},-1)", 2.5),
        ("RANK.AVG(0,{-1,0,-0,1})", 2.5),
        ("RANK.EQ(0,{-1,0,-0,1},1)", 2.0),
        ("RANK.AVG(2,{TRUE,\"2\",2,2,3})", 2.5),
        ("RANK.AVG(2,{2,2,2},)", 2.0),
    ];
    let workbook = workbook_with_formulas(
        &formulas
            .iter()
            .enumerate()
            .map(|(i, (formula, _))| (1, i as u32 + 1, *formula))
            .collect::<Vec<_>>(),
    );
    assert!(scan_formula_capabilities(&workbook).is_supported());
    let result = calculate_workbook(&workbook, CalculationOptions::default());
    for (i, (_, expected)) in formulas.iter().enumerate() {
        assert_number(&result, i as u32 + 1, *expected, 0.0);
    }
}

#[test]
fn rank_preserves_absence_and_error_order() {
    for (formula, expected) in [
        ("RANK.AVG(4,{1,2,3})", ExcelError::NotAvailable),
        ("RANK(4,{1,2,3})", ExcelError::NotAvailable),
        ("RANK.AVG(2,{1,#DIV/0!},#N/A)", ExcelError::DivisionByZero),
        ("RANK.AVG(#N/A,{#DIV/0!})", ExcelError::NotAvailable),
        ("RANK.AVG(2,{1,2},\"bad\")", ExcelError::Value),
    ] {
        let result = calculate_workbook(
            &workbook_with_formulas(&[(1, 1, formula)]),
            CalculationOptions::default(),
        );
        assert_eq!(
            result.cell(cell_id(1)),
            Some(&CalculationCellResult::Value(CellValue::Error(expected))),
            "{formula}"
        );
    }
}

#[test]
fn rank_counts_charge_linear_work() {
    let workbook = workbook_with_formulas(&[(1, 1, "RANK.AVG(2,{3,2,2,1})")]);
    let result = calculate_workbook(
        &workbook,
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(3)
                .unwrap(),
        ),
    );
    assert_issue(&result, 1, CalculationIssueCode::ResourceLimitExceeded);
    let result = calculate_workbook(
        &workbook,
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(4)
                .unwrap(),
        ),
    );
    assert_number(&result, 1, 2.5, 0.0);
}

#[test]
fn forecast_uses_paired_moments_and_ignores_incomplete_numeric_pairs() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "FORECAST.LINEAR(4,{3,5,7},{1,2,3})"),
        (1, 2, "FORECAST(4,{3,5,7},{1,2,3})"),
        (
            1,
            3,
            "FORECAST.LINEAR(1000000000004,{3,5,7},{1000000000001,1000000000002,1000000000003})",
        ),
        (1, 4, "FORECAST.LINEAR(4,{3,\"ignored\",7},{1,2,3})"),
        (1, 5, "FORECAST.LINEAR(9,{5,5,5},{1,2,3})"),
    ]);
    assert!(scan_formula_capabilities(&workbook).is_supported());
    let result = calculate_workbook(&workbook, CalculationOptions::default());
    for column in 1..=4 {
        assert_number(&result, column, 9.0, 1e-12);
    }
    assert_number(&result, 5, 5.0, 0.0);
}

#[test]
fn forecast_rejects_bad_shapes_degenerate_x_and_scalar_errors() {
    for (formula, expected) in [
        ("FORECAST.LINEAR(1,{1,2},{1})", ExcelError::NotAvailable),
        ("FORECAST.LINEAR(1,{1,2},{3,3})", ExcelError::DivisionByZero),
        (
            "FORECAST.LINEAR(1,{\"a\",TRUE},{1,2})",
            ExcelError::DivisionByZero,
        ),
        ("FORECAST.LINEAR(\"bad\",{1,2},{1,2})", ExcelError::Value),
        (
            "FORECAST.LINEAR(1,{1,#N/A},{1,2})",
            ExcelError::NotAvailable,
        ),
    ] {
        let result = calculate_workbook(
            &workbook_with_formulas(&[(1, 1, formula)]),
            CalculationOptions::default(),
        );
        assert_eq!(
            result.cell(cell_id(1)),
            Some(&CalculationCellResult::Value(CellValue::Error(expected))),
            "{formula}"
        );
    }
}

/// The Excel oracle's shared data: `A2:A21` holds its column E (10 through 200 in steps of 10)
/// and `B2:B21` its column F (1 through 7, repeating). `Z2:Z4` holds three text keys, `X2:X4`
/// holds TRUE, 3 and a text value, and `Y2:Y4` is empty.
fn calculate_with_oracle_data(formulas: &[&str]) -> CalculationSnapshot {
    const Y: [&str; 20] = [
        "10", "20", "30", "40", "50", "60", "70", "80", "90", "100", "110", "120", "130", "140",
        "150", "160", "170", "180", "190", "200",
    ];
    const X: [&str; 20] = [
        "1", "2", "3", "4", "5", "6", "7", "1", "2", "3", "4", "5", "6", "7", "1", "2", "3", "4",
        "5", "6",
    ];
    let mut cells = Vec::new();
    for (row, (y, x)) in (2..).zip(Y.iter().zip(X)) {
        cells.push((row, 1, *y));
        cells.push((row, 2, x));
    }
    cells.extend([
        (2, 26, "\"K001\""),
        (3, 26, "\"K002\""),
        (4, 26, "\"K003\""),
        (2, 24, "TRUE"),
        (3, 24, "3"),
        (4, 24, "\"x\""),
    ]);
    cells.extend(
        (3..)
            .zip(formulas)
            .map(|(column, formula)| (1, column, *formula)),
    );
    let workbook = workbook_with_formulas(&cells);
    assert!(scan_formula_capabilities(&workbook).is_supported());
    calculate_workbook(&workbook, CalculationOptions::default())
}

/// Asserts each formula's value, the formulas occupying consecutive columns from `C1`.
fn assert_numbers(formulas: &[(&str, f64)], relative_tolerance: f64) {
    let calculation = calculate_with_oracle_data(
        &formulas
            .iter()
            .map(|(formula, _)| *formula)
            .collect::<Vec<_>>(),
    );
    for (column, (_, expected)) in (3..).zip(formulas) {
        assert_number(
            &calculation,
            column,
            *expected,
            expected.abs() * relative_tolerance,
        );
    }
}

fn assert_errors(formulas: &[(&str, ExcelError)]) {
    let calculation = calculate_with_oracle_data(
        &formulas
            .iter()
            .map(|(formula, _)| *formula)
            .collect::<Vec<_>>(),
    );
    for (column, (formula, expected)) in (3..).zip(formulas) {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(*expected))),
            "{formula}"
        );
    }
}

#[test]
fn exclusive_percentiles_interpolate_between_the_n_plus_one_ranks() {
    assert_numbers(
        &[
            ("_xlfn.PERCENTILE.EXC(A2:A21,0.75)", 157.5),
            ("PERCENTILE.EXC({1,2,3,6,6,6,7,8,9},0.25)", 2.5),
            ("PERCENTILE.EXC({1,2,3},0.25)", 1.0),
            ("PERCENTILE.EXC({1,2,3},0.75)", 3.0),
            ("_xlfn.QUARTILE.EXC(A2:A21,3)", 157.5),
            ("QUARTILE.EXC(A2:A21,\"2\")", 105.0),
            ("QUARTILE.EXC(A2:A21,1.9)", 52.5),
            ("QUARTILE.EXC({6,7,15,36,39,40,41,42,43,47,49},1)", 15.0),
            ("QUARTILE.EXC({6,7,15,36,39,40,41,42,43,47,49},3)", 43.0),
            ("_xlfn.PERCENTRANK.EXC(A2:A21,100)", 0.476),
            ("PERCENTRANK.EXC({1,2,3,6,6,6,7,8,9},7)", 0.7),
            ("PERCENTRANK.EXC({1,2,3,6,6,6,7,8,9},5.43)", 0.381),
            ("PERCENTRANK.EXC({1,2,3,6,6,6,7,8,9},5.43,1)", 0.3),
            ("PERCENTRANK.EXC({1,2,3,6,6,6,7,8,9},6)", 0.4),
        ],
        1e-15,
    );
}

#[test]
fn exclusive_percentiles_reject_ranks_outside_the_sample() {
    assert_errors(&[
        ("PERCENTILE.EXC(A2:A21,2)", ExcelError::Number),
        ("PERCENTILE.EXC(A2:A21,\"2\")", ExcelError::Number),
        (
            "PERCENTILE.EXC({1,2,3,6,6,6,7,8,9},0.01)",
            ExcelError::Number,
        ),
        ("PERCENTILE.EXC({1,2,3},0.2)", ExcelError::Number),
        ("PERCENTILE.EXC({1,2,3},0.8)", ExcelError::Number),
        ("PERCENTILE.EXC(Y2:Y4,0.5)", ExcelError::Number),
        ("QUARTILE.EXC(A2:A21,5)", ExcelError::Number),
        ("QUARTILE.EXC(A2:A21,0)", ExcelError::Number),
        ("QUARTILE.EXC(A2:A21,4)", ExcelError::Number),
        ("QUARTILE.EXC({1,2},1)", ExcelError::Number),
        ("PERCENTRANK.EXC(A2:A21,\"2\")", ExcelError::NotAvailable),
        ("PERCENTRANK.EXC(A2:A21,201)", ExcelError::NotAvailable),
        ("PERCENTRANK.EXC(A2:A21,Z2)", ExcelError::Value),
        ("PERCENTRANK.EXC(A2:A21,100,0)", ExcelError::Number),
        ("PERCENTRANK.EXC(Y2:Y4,1)", ExcelError::Number),
        (
            "PERCENTILE.EXC({1,#DIV/0!},0.5)",
            ExcelError::DivisionByZero,
        ),
    ]);
}

#[test]
fn trimmed_mean_excludes_an_equal_count_from_each_end() {
    assert_numbers(
        &[
            ("TRIMMEAN(A2:A21,0.2)", 105.0),
            ("TRIMMEAN({4,5,6,7,2,3,4,5,1,2,3},0.2)", 34.0 / 9.0),
            ("TRIMMEAN({100,1,4,2,3},0.4)", 3.0),
            ("TRIMMEAN({100,1,4,2,3},0.39)", 22.0),
            ("TRIMMEAN({100,1,4,2,3},0)", 22.0),
            ("TRIMMEAN({5},0.99)", 5.0),
            ("TRIMMEAN(A2:A21,\"0.2\")", 105.0),
        ],
        1e-15,
    );
    assert_errors(&[
        ("TRIMMEAN(A2:A21,\"2\")", ExcelError::Number),
        ("TRIMMEAN(A2:A21,1.5)", ExcelError::Number),
        ("TRIMMEAN(A2:A21,1)", ExcelError::Number),
        ("TRIMMEAN(A2:A21,-0.1)", ExcelError::Number),
        ("TRIMMEAN(Y2:Y4,0.1)", ExcelError::Number),
        ("TRIMMEAN(A2:A21,Z2)", ExcelError::Value),
        ("TRIMMEAN({1,#N/A},0.1)", ExcelError::NotAvailable),
    ]);
}

#[test]
fn a_variance_family_counts_logical_values_and_referenced_text() {
    assert_numbers(
        &[
            ("STDEVA(A2:A21)", 59.160_797_830_996_16),
            ("STDEVPA(A2:A21)", 57.662_812_973_353_98),
            ("VARA(A2:A21)", 3500.0),
            ("VARPA(A2:A21)", 3325.0),
            (
                "STDEVA(1345,1301,1368,1322,1310,1370,1318,1350,1303,1299)",
                27.463_915_719_843_495,
            ),
            (
                "STDEVPA(1345,1301,1368,1322,1310,1370,1318,1350,1303,1299)",
                26.054_558_142_482_477,
            ),
            // Referenced text counts as zero and TRUE as one: {1, 3, 0}.
            ("VARA(X2:X4)", 7.0 / 3.0),
            ("VARPA(X2:X4)", 14.0 / 9.0),
            ("VARA(TRUE,3,5)", 4.0),
            ("STDEVA(TRUE,3,5)", 2.0),
            ("VARA(\"2\",4)", 2.0),
            ("VARA({\"x\",2})", 2.0),
            ("VARPA(FALSE,2)", 1.0),
            ("STDEVPA(7)", 0.0),
            ("STDEVA(Z2:Z4)", 0.0),
            ("STDEVPA(Z2:Z4)", 0.0),
            ("VARA(Z2:Z4)", 0.0),
            ("VARPA(Z2:Z4)", 0.0),
        ],
        1e-15,
    );
    assert_errors(&[
        ("STDEVA(Y2:Y4)", ExcelError::DivisionByZero),
        ("STDEVPA(Y2:Y4)", ExcelError::DivisionByZero),
        ("VARA(Y2:Y4)", ExcelError::DivisionByZero),
        ("VARPA(Y2:Y4)", ExcelError::DivisionByZero),
        ("STDEVA(7)", ExcelError::DivisionByZero),
        ("STDEV.S(X2:X4)", ExcelError::DivisionByZero),
        ("VARA(\"x\",4)", ExcelError::Value),
        ("VARPA(1,#N/A)", ExcelError::NotAvailable),
    ]);
}

#[test]
fn steyx_measures_the_residuals_of_the_paired_linear_fit() {
    assert_numbers(
        &[
            ("STEYX(A2:A21,B2:B21)", 59.068_141_865_235_45),
            (
                "STEYX({2,3,9,1,8,7,5},{6,5,11,7,5,4,4})",
                3.305_718_950_210_041,
            ),
            ("STEYX({0,2,2,0},{1,2,3,4})", std::f64::consts::SQRT_2),
            (
                "STEYX({0,2,\"x\",2,0},{1,2,9,3,4})",
                std::f64::consts::SQRT_2,
            ),
            ("STEYX({3,5,7},{1,2,3})", 0.0),
        ],
        1e-14,
    );
    // An exact fit whose residual sum of squares rounds below zero still reports no error.
    let calculation =
        calculate_with_oracle_data(&["STEYX({0.3,0.6,0.9,1.2,1.5},{0.1,0.2,0.3,0.4,0.5})"]);
    assert_number(&calculation, 3, 0.0, 1e-7);
    assert_errors(&[
        ("STEYX(Z2:Z4,B2:B21)", ExcelError::NotAvailable),
        ("STEYX(Y2:Y4,B2:B21)", ExcelError::NotAvailable),
        ("STEYX({1,2,3},{1,2})", ExcelError::NotAvailable),
        ("STEYX({1,2},{1,2})", ExcelError::DivisionByZero),
        ("STEYX(Z2:Z4,A2:A4)", ExcelError::DivisionByZero),
        ("STEYX({1,2,3},{4,4,4})", ExcelError::DivisionByZero),
        ("STEYX({1,2,#N/A},{1,2,3})", ExcelError::NotAvailable),
    ]);
}

#[test]
fn prob_sums_the_probabilities_within_inclusive_limits() {
    assert_numbers(
        &[
            ("PROB({1,2,3,4},{0.1,0.2,0.3,0.4},2,4)", 0.9),
            ("PROB({1,2,3,4},{0.1,0.2,0.3,0.4},\"2\",4)", 0.9),
            ("PROB({0,1,2,3},{0.2,0.3,0.1,0.4},2)", 0.1),
            ("PROB({0,1,2,3},{0.2,0.3,0.1,0.4},1,3)", 0.8),
            ("PROB({0,1,2,3},{0.2,0.3,0.1,0.4},1.5)", 0.0),
            ("PROB({0,1,2,3},{0.2,0.3,0.1,0.4},-5,5)", 1.0),
        ],
        1e-15,
    );
    assert_errors(&[
        ("PROB({1,2,3,4},{0.1,0.2,0.3,0.4},Z2,4)", ExcelError::Value),
        ("PROB({1,2,3},{0.1,0.2,0.3},1,3)", ExcelError::Number),
        ("PROB({1,2},{1.5,-0.5},1,2)", ExcelError::Number),
        ("PROB({1,2,3},{0.5,0.5},1,3)", ExcelError::NotAvailable),
        ("PROB({1,2},{0.5,#DIV/0!},1,2)", ExcelError::DivisionByZero),
    ]);
}

#[test]
fn fisher_transformation_and_its_inverse_stay_inside_the_open_interval() {
    assert_numbers(
        &[
            ("FISHER(0.75)", 0.972_955_074_527_656_6),
            ("FISHERINV(0.9)", 0.716_297_870_199_024_5),
            ("FISHERINV(\"2\")", 0.964_027_580_075_816_9),
            ("FISHERINV(FISHER(0.75))", 0.75),
            ("FISHER(-0.5)", -0.549_306_144_334_054_9),
        ],
        1e-15,
    );
    assert_errors(&[
        ("FISHER(1)", ExcelError::Number),
        ("FISHER(-1)", ExcelError::Number),
        ("FISHER(\"2\")", ExcelError::Number),
        ("FISHERINV(Z2)", ExcelError::Value),
        ("FISHER(#N/A)", ExcelError::NotAvailable),
    ]);
}
