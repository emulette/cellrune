use super::*;

#[test]
fn conditional_aggregates_use_excel_range_rules_and_clamp_whole_columns() {
    let workbook = workbook_with_formulas(&[
        (2, 1, "1"),
        (3, 1, "2"),
        (4, 1, "3"),
        (2, 2, "10"),
        (3, 2, "20"),
        (4, 2, "30"),
        (1, 3, "SUMIF(A2:A4,\">1\",B2)"),
        (1, 4, "AVERAGEIF(A2:A4,\">1\",B2)"),
        (1, 5, "SUMIFS(B2:B4,A2:A4,\">1\")"),
        (1, 6, "SUMIFS(B2:B3,A2:A4,\">1\")"),
        (1, 7, "MODE.SNGL({1,1,2,2})"),
        (1, 8, "VLOOKUP(1,A:B,2,FALSE)"),
        (1, 9, "SUMIFS(B:B,A:A,1)"),
    ]);
    let limits = CalculationLimits::default()
        .with_max_function_iterations(10)
        .expect("nonzero function iteration limit");
    let calculation =
        calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));

    for (column, expected) in [
        (3, 50.0),
        (4, 25.0),
        (5, 50.0),
        (7, 1.0),
        (8, 10.0),
        (9, 10.0),
    ] {
        assert_number(&calculation, column, expected, 0.0);
    }
    assert_eq!(
        calculation.cell(cell_id(6)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::Value
        )))
    );
}

#[test]
fn wildcard_local_and_formula_cumulative_limits_are_both_observable() {
    let limits = CalculationLimits::default()
        .with_max_function_iterations(20)
        .expect("nonzero function iteration limit");
    let options = CalculationOptions::default().with_limits(limits);

    let single = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "COUNTIF(A2,\"a*\")"), (2, 1, "\"alpha\"")]),
        options,
    );
    assert_number(&single, 1, 1.0, 0.0);

    let cumulative = calculate_workbook(
        &workbook_with_formulas(&[
            (1, 1, "COUNTIF(A2,\"a*\")+COUNTIF(A2,\"a*\")"),
            (2, 1, "\"alpha\""),
        ]),
        options,
    );
    assert_issue(&cumulative, 1, CalculationIssueCode::ResourceLimitExceeded);

    let local_limits = CalculationLimits::default()
        .with_max_function_iterations(10)
        .expect("nonzero local wildcard limit");
    let local = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "COUNTIF(A2,\"a*\")"), (2, 1, "\"alpha\"")]),
        CalculationOptions::default().with_limits(local_limits),
    );
    assert_issue(&local, 1, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn all_conditional_families_keep_wildcard_semantics_after_compilation() {
    let workbook = workbook_with_formulas(&[
        (2, 1, "\"alpha\""),
        (3, 1, "\"beta\""),
        (4, 1, "\"*\""),
        (5, 1, "\"?\""),
        (6, 1, "\"~\""),
        (8, 1, "1/0"),
        (2, 2, "10"),
        (3, 2, "1/0"),
        (4, 2, "20"),
        (5, 2, "30"),
        (6, 2, "40"),
        (7, 2, "50"),
        (8, 2, "60"),
        (1, 3, "COUNTIF(A2:A8,\"A*\")"),
        (1, 4, "COUNTIF(A2:A8,\"~*\")"),
        (1, 5, "COUNTIF(A2:A8,\"~?\")"),
        (1, 6, "COUNTIF(A2:A8,\"~~\")"),
        (1, 7, "COUNTIF(A2:A8,\"\")"),
        (1, 8, "COUNTIF(A2:A8,1/0)"),
        (1, 9, "SUMIF(A2:A8,\"a*\",B2:B8)"),
        (1, 10, "AVERAGEIFS(B2:B8,A2:A8,\"a*\")"),
        (1, 11, "MAXIFS(B2:B8,A2:A8,\"a*\")"),
        (1, 12, "MINIFS(B2:B8,A2:A8,\"a*\")"),
        (1, 13, "SUMIFS(B2:B3,A2:A3,\"a*\")"),
        (1, 14, "SUMIFS(B2:B3,A2:A3,\"*\")"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (column, expected) in [
        (3, 1.0),
        (4, 1.0),
        (5, 1.0),
        (6, 1.0),
        (7, 1.0),
        (8, 1.0),
        (9, 10.0),
        (10, 10.0),
        (11, 10.0),
        (12, 10.0),
        (13, 10.0),
    ] {
        assert_number(&calculation, column, expected, 0.0);
    }
    assert_eq!(
        calculation.cell(cell_id(14)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::DivisionByZero
        )))
    );
}

#[test]
fn match_preserves_exact_approximate_error_and_resource_contracts() {
    let workbook = workbook_with_formulas(&[
        (2, 1, "1"),
        (3, 1, "2"),
        (4, 1, "3"),
        (2, 2, "\"Alpha\""),
        (3, 2, "\"beta\""),
        (4, 2, "\"a*\""),
        (2, 3, "3"),
        (3, 3, "2"),
        (4, 3, "1"),
        (1, 4, "MATCH(2,A2:A4,0)"),
        (1, 5, "MATCH(\"alpha\",B2:B4,0)"),
        (1, 6, "MATCH(\"a~*\",B2:B4,0)"),
        (1, 7, "MATCH(2.5,A2:A4,1)"),
        (1, 8, "MATCH(2.5,C2:C4,-1)"),
        (1, 9, "MATCH(\"\",B2:B4,0)"),
        (1, 10, "MATCH(1/0,A2:A4,0)"),
    ]);
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    for (column, expected) in [(4, 2.0), (5, 1.0), (6, 3.0), (7, 2.0), (8, 1.0)] {
        assert_number(&calculation, column, expected, 0.0);
    }
    for (column, error) in [
        (9, ExcelError::NotAvailable),
        (10, ExcelError::DivisionByZero),
    ] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(error)))
        );
    }

    let limits = CalculationLimits::default()
        .with_max_function_iterations(10)
        .expect("nonzero MATCH wildcard limit");
    let limited = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "MATCH(\"a*\",A2,0)"), (2, 1, "\"alpha\"")]),
        CalculationOptions::default().with_limits(limits),
    );
    assert_issue(&limited, 1, CalculationIssueCode::ResourceLimitExceeded);
}
