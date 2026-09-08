use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationLimits, CalculationOptions, CellValue,
    ExcelError, calculate_workbook, scan_formula_capabilities,
};

use super::support::{assert_issue, assert_number, cell_id, workbook_with_formulas};

#[test]
fn determinant_handles_pivot_parity_singularity_and_scaled_products() {
    for (formula, expected) in [
        ("MDETERM({4,7;2,6})", 10.0),
        ("MDETERM({0,1;2,3})", -2.0),
        ("MDETERM({0,1,0;0,0,1;1,0,0})", 1.0),
        ("MDETERM({1,2;2,4})", 0.0),
        ("MDETERM({0,0;0,0})", 0.0),
        ("_xlfn.MDETERM({-3})", -3.0),
        ("MDETERM({1,0;0,1e-20})", 1e-20),
        (
            "MDETERM({1e200,0,0,0;0,1e200,0,0;0,0,1e-200,0;0,0,0,1e-200})",
            1.0,
        ),
        ("MDETERM({1e-200,0;0,1e-200})", 0.0),
    ] {
        let workbook = workbook_with_formulas(&[(1, 1, formula)]);
        assert!(
            scan_formula_capabilities(&workbook).is_supported(),
            "{formula}"
        );
        let result = calculate_workbook(&workbook, CalculationOptions::default());
        assert_number(&result, 1, expected, expected.abs() * 1e-14);
    }
}

#[test]
fn determinant_rejects_non_numeric_non_square_and_non_finite_results() {
    for (formula, expected) in [
        ("MDETERM({1,2})", ExcelError::Value),
        ("MDETERM({1,\"2\";3,4})", ExcelError::Value),
        ("MDETERM({1,TRUE;3,4})", ExcelError::Value),
        ("MDETERM(B1:C2)", ExcelError::Value),
        ("MDETERM({1,#N/A;3,4})", ExcelError::NotAvailable),
        ("MDETERM({1e200,0;0,1e200})", ExcelError::Number),
        ("MDETERM()", ExcelError::Value),
        ("MDETERM({1},{2})", ExcelError::Value),
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
fn determinant_charges_elimination_work_and_limits_its_workspace() {
    let workbook = workbook_with_formulas(&[(1, 1, "MDETERM({4,7;2,6})")]);
    for limits in [
        CalculationLimits::default()
            .with_max_function_iterations(3)
            .unwrap(),
        CalculationLimits::default()
            .with_max_array_cells(7)
            .unwrap(),
    ] {
        let result =
            calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));
        assert_issue(&result, 1, CalculationIssueCode::ResourceLimitExceeded);
    }
    let result = calculate_workbook(
        &workbook,
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(4)
                .unwrap()
                .with_max_array_cells(8)
                .unwrap(),
        ),
    );
    assert_number(&result, 1, 10.0, 1e-14);
}
