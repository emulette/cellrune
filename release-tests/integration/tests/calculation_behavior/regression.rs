use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationLimits,
    CalculationOptions, CellAddress, CellValue, ExcelError, FormulaText, SheetId, WorkbookDraft,
    calculate_workbook, scan_formula_capabilities,
};

use super::materialized_result;

#[test]
fn matrix_and_regression_arrays_follow_the_frozen_shapes_and_statistics_contract() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, formula) in [
        ("A1", "=MINVERSE({4,7;2,6})"),
        ("D1", "=MUNIT(\"2\")"),
        ("G1", "=MUNIT(TRUE)"),
        ("I1", "=LINEST({5;7;9;11},{1;2;3;4},TRUE,TRUE)"),
        ("L1", "=LOGEST({6;12;24;48},{1;2;3;4},TRUE,TRUE)"),
        ("O1", "=TREND({5;7;9;11},{1;2;3;4},{21,22,23})"),
        ("O3", "=GROWTH({6;12;24;48},{1;2;3;4},{5,6})"),
        ("A5", "=MINVERSE({1,2;2,4})"),
        ("D5", "=MUNIT(FALSE)"),
        ("A7", "=LINEST({3;5;7},{1,2;2,4;3,6},TRUE,TRUE)"),
        ("A13", "=LINEST({2;4;6},{1;2;3},FALSE,TRUE)"),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid dynamic anchor"),
                FormulaText::from_user_input(formula).expect("valid formula"),
                None,
            )
            .expect("dynamic formula mutation");
    }

    assert!(scan_formula_capabilities(draft.workbook()).is_supported());
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());

    for (address, expected, tolerance) in [
        ("A1", 0.6, 1.0e-15),
        ("B1", -0.7, 1.0e-15),
        ("A2", -0.2, 1.0e-15),
        ("B2", 0.4, 1.0e-15),
        ("D1", 1.0, 0.0),
        ("E1", 0.0, 0.0),
        ("D2", 0.0, 0.0),
        ("E2", 1.0, 0.0),
        ("G1", 1.0, 0.0),
        ("I1", 2.0, 1.0e-12),
        ("J1", 3.0, 1.0e-12),
        ("I2", 0.0, 0.0),
        ("J2", 0.0, 0.0),
        ("I3", 1.0, 1.0e-12),
        ("J3", 0.0, 0.0),
        ("J4", 2.0, 0.0),
        ("I5", 20.0, 1.0e-10),
        ("J5", 0.0, 0.0),
        ("L1", 2.0, 1.0e-12),
        ("M1", 3.0, 1.0e-12),
        ("O1", 45.0, 1.0e-10),
        ("P1", 47.0, 1.0e-10),
        ("Q1", 49.0, 1.0e-10),
        ("O3", 96.0, 1.0e-10),
        ("P3", 192.0, 1.0e-10),
        ("A7", 0.0, 0.0),
        ("B7", 2.0, 1.0e-12),
        ("C7", 1.0, 1.0e-12),
        ("A8", 0.0, 0.0),
        ("A13", 2.0, 1.0e-12),
        ("B13", 0.0, 0.0),
    ] {
        assert_materialized_approx(&calculation, sheet_id, address, expected, tolerance);
    }
    for (address, error) in [
        ("I4", ExcelError::Number),
        ("A5", ExcelError::Number),
        ("D5", ExcelError::Value),
        ("B14", ExcelError::NotAvailable),
    ] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(CellValue::Error(error))),
            "unexpected error at {address}",
        );
    }
}

#[test]
fn regression_normalization_preserves_single_and_multi_variable_orientations() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    for (address, formula) in [
        ("A1", "=TREND({3,5,7},{1,2,3})"),
        ("A3", "=TREND({3;5;7})"),
        ("D1", "=TREND({8;7;12;7},{1,2;2,1;3,3;4,0},{5,4;6,2})"),
        ("G1", "=TREND({8,7,12,7},{1,2,3,4;2,1,3,0},{5,6;4,2})"),
        ("A7", "=GROWTH({6;12;24})"),
        ("D7", "=TREND({\"a\";\"b\"},{1;2})"),
        ("F7", "=LOGEST({1;0},{1;2})"),
        ("H7", "=TREND({8;7;12;7},{1,2;2,1;3,3;4,0},{5,6,7})"),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet_id,
                CellAddress::from_a1(address).expect("valid dynamic anchor"),
                FormulaText::from_user_input(formula).expect("valid formula"),
                None,
            )
            .expect("dynamic formula mutation");
    }
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for (address, expected) in [
        ("A1", 3.0),
        ("B1", 5.0),
        ("C1", 7.0),
        ("A3", 3.0),
        ("A4", 5.0),
        ("A5", 7.0),
        ("D1", 16.0),
        ("D2", 13.0),
        ("G1", 16.0),
        ("H1", 13.0),
        ("A7", 6.0),
        ("A8", 12.0),
        ("A9", 24.0),
    ] {
        assert_materialized_approx(&calculation, sheet_id, address, expected, 1.0e-10);
    }
    for (address, error) in [
        ("D7", ExcelError::Value),
        ("F7", ExcelError::Number),
        ("H7", ExcelError::Reference),
    ] {
        assert_eq!(
            materialized_result(&calculation, sheet_id, address),
            Some(&CalculationCellResult::Value(CellValue::Error(error))),
            "unexpected normalization error at {address}",
        );
    }
}

#[test]
fn matrix_and_regression_workspace_limits_surface_as_engine_issues() {
    let mut matrix_draft = WorkbookDraft::new();
    let sheet_id = matrix_draft.workbook().sheets()[0].id();
    matrix_draft
        .set_cell_dynamic_formula(
            sheet_id,
            CellAddress::from_a1("A1").unwrap(),
            FormulaText::from_user_input("=MUNIT(5)").unwrap(),
            None,
        )
        .unwrap();
    let array_limits = CalculationLimits::default()
        .with_max_array_cells(20)
        .expect("positive array limit");
    assert_resource_issue(
        &calculate_workbook(
            matrix_draft.workbook(),
            CalculationOptions::default().with_limits(array_limits),
        ),
        sheet_id,
        "max_array_cells",
    );
    let iteration_limits = CalculationLimits::default()
        .with_max_function_iterations(20)
        .expect("positive iteration limit");
    assert_resource_issue(
        &calculate_workbook(
            matrix_draft.workbook(),
            CalculationOptions::default().with_limits(iteration_limits),
        ),
        sheet_id,
        "max_function_iterations",
    );

    let mut regression_draft = WorkbookDraft::new();
    let regression_sheet = regression_draft.workbook().sheets()[0].id();
    regression_draft
        .set_cell_dynamic_formula(
            regression_sheet,
            CellAddress::from_a1("A1").unwrap(),
            FormulaText::from_user_input("=LINEST({5;7;9;11},{1;2;3;4},TRUE,TRUE)").unwrap(),
            None,
        )
        .unwrap();
    let regression_limits = CalculationLimits::default()
        .with_max_array_cells(69)
        .expect("positive regression limit");
    assert_resource_issue(
        &calculate_workbook(
            regression_draft.workbook(),
            CalculationOptions::default().with_limits(regression_limits),
        ),
        regression_sheet,
        "max_array_cells",
    );
}

fn assert_resource_issue(
    calculation: &cellrune::CalculationSnapshot,
    sheet_id: SheetId,
    detail: &str,
) {
    let id = CalculationCellId::new(sheet_id, CellAddress::from_a1("A1").unwrap());
    let Some(CalculationCellResult::Unavailable(issue)) = calculation.cell(id) else {
        panic!("expected a resource issue, got {:?}", calculation.cell(id));
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.detail(), Some(detail));
}

fn assert_materialized_approx(
    calculation: &cellrune::CalculationSnapshot,
    sheet_id: SheetId,
    address: &str,
    expected: f64,
    tolerance: f64,
) {
    let Some(CalculationCellResult::Value(CellValue::Number(actual))) =
        materialized_result(calculation, sheet_id, address)
    else {
        panic!(
            "expected numeric materialized value at {address}, got {:?}",
            materialized_result(calculation, sheet_id, address)
        );
    };
    assert!(
        (actual.get() - expected).abs() <= tolerance,
        "unexpected value at {address}: expected {expected}, got {}",
        actual.get(),
    );
}
