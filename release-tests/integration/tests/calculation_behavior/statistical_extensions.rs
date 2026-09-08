use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationLimits, CalculationOptions, CellValue,
    ExcelError, calculate_workbook, scan_formula_capabilities,
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
