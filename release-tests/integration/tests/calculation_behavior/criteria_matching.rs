use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationLimitKind, CalculationLimits,
    CalculationOptions, CalculationSnapshot, CellValue, ExcelError, calculate_workbook,
};

use super::support::{assert_issue, assert_number, cell_id, workbook_with_formulas};

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
fn wildcard_work_uses_the_formula_cumulative_limit() {
    let limits = CalculationLimits::default()
        .with_max_function_iterations(20)
        .expect("nonzero function iteration limit");
    let options = CalculationOptions::default().with_limits(limits);

    let single = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "COUNTIF(A2,\"a*\")"), (2, 1, "\"alphabetical\"")]),
        options,
    );
    assert_number(&single, 1, 1.0, 0.0);

    let cumulative = calculate_workbook(
        &workbook_with_formulas(&[
            (1, 1, "COUNTIF(A2,\"a*\")+COUNTIF(A2,\"a*\")"),
            (2, 1, "\"alphabetical\""),
        ]),
        options,
    );
    assert_issue(&cumulative, 1, CalculationIssueCode::ResourceLimitExceeded);

    let tight_limits = CalculationLimits::default()
        .with_max_function_iterations(10)
        .expect("nonzero cumulative wildcard limit");
    let tight = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "COUNTIF(A2,\"a*\")"), (2, 1, "\"alphabetical\"")]),
        CalculationOptions::default().with_limits(tight_limits),
    );
    assert_issue(&tight, 1, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn text_comparisons_cost_one_unit_per_cell_regardless_of_text_length() {
    let candidate = format!("\"{}\"", "a".repeat(1_000));
    let lookup = "z".repeat(1_000);
    let ordered_criterion = format!("\">{lookup}\"");
    let countif_ordered = format!("COUNTIF(A2:A11,\">{lookup}\")");
    let countif_exact = format!("COUNTIF(A2:A11,\"{lookup}\")");
    let countifs = format!("COUNTIFS(A2:A11,\"{lookup}\")");
    let exact_match = format!("IFNA(MATCH(\"{lookup}\",A2:A11,0),-1)");
    let xlookup = format!("XLOOKUP(\"{lookup}\",A2:A11,A2:A11,-1)");
    let mut formulas = vec![
        (1, 1, "\"Name\""),
        (1, 3, "\"Name\""),
        (2, 3, ordered_criterion.as_str()),
        (1, 4, countif_ordered.as_str()),
        (1, 5, countif_exact.as_str()),
        (1, 6, countifs.as_str()),
        (1, 7, exact_match.as_str()),
        (1, 8, xlookup.as_str()),
        (1, 9, "COUNTA(UNIQUE(A2:A11))"),
        (1, 10, "DCOUNTA(A1:A11,1,C1:C2)"),
    ];
    formulas.extend((2..=11).map(|row| (row, 1, candidate.as_str())));
    let workbook = workbook_with_formulas(&formulas);
    let limits = CalculationLimits::default()
        .with_max_function_iterations(100)
        .expect("nonzero function work limit");
    let calculation =
        calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));

    for (column, expected) in [
        (4, 0.0),
        (5, 0.0),
        (6, 0.0),
        (7, -1.0),
        (8, -1.0),
        (9, 1.0),
        (10, 0.0),
    ] {
        assert_number(&calculation, column, expected, 0.0);
    }

    let cell_limits = CalculationLimits::default()
        .with_max_function_iterations(5)
        .expect("nonzero function work limit");
    let calculation = calculate_workbook(
        &workbook,
        CalculationOptions::default().with_limits(cell_limits),
    );
    assert_issue(&calculation, 5, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn conditional_scans_fit_the_cells_they_read_rather_than_every_cell_of_their_ranges() {
    // Sixty rows, four of them "a". Every cell of their ranges would exceed the 100-cell limit,
    // but a row stops at its first unmatched criterion: the first four scans read 64 to 68 cells.
    let mut formulas = vec![
        (1, 3, "SUMIFS(B1:B60,A1:A60,\"a\",B1:B60,\">1\")".to_owned()),
        (1, 4, "COUNTIFS(A1:A60,\"a\",B1:B60,\">1\")".to_owned()),
        (1, 5, "MAXIFS(B1:B60,A1:A60,\"a\",B1:B60,\">1\")".to_owned()),
        (1, 6, "SUMIF(A1:A60,\"a\",B1:B60)".to_owned()),
        // Every row matches the first criterion, so this scan reads 176 cells.
        (1, 7, "SUMIFS(B1:B60,B1:B60,\">0\",A1:A60,\"b\")".to_owned()),
        (1, 8, "COUNTIF(A1:A60,\"b\")".to_owned()),
    ];
    for row in 1..=60 {
        let text = if row <= 4 { "\"a\"" } else { "\"b\"" };
        formulas.push((row, 1, text.to_owned()));
        formulas.push((row, 2, row.to_string()));
    }
    let workbook = workbook_with_formulas(
        &formulas
            .iter()
            .map(|(row, column, text)| (*row, *column, text.as_str()))
            .collect::<Vec<_>>(),
    );
    let array_limits = CalculationLimits::default()
        .with_max_array_cells(100)
        .expect("nonzero array limit");
    let calculation = calculate_workbook(
        &workbook,
        CalculationOptions::default().with_limits(array_limits),
    );
    for (column, expected) in [(3, 9.0), (4, 3.0), (5, 4.0), (6, 10.0), (8, 56.0)] {
        assert_number(&calculation, column, expected, 0.0);
    }
    assert_limit(&calculation, 7, CalculationLimitKind::ArrayCells);

    // A scan whose ranges exceed the array limit reports that limit whatever else stops it; one
    // within it reports the limit it reaches.
    let work_limits = array_limits
        .with_max_function_iterations(10)
        .expect("nonzero function work limit");
    let calculation = calculate_workbook(
        &workbook,
        CalculationOptions::default().with_limits(work_limits),
    );
    assert_limit(&calculation, 3, CalculationLimitKind::ArrayCells);
    assert_limit(&calculation, 8, CalculationLimitKind::FunctionIterations);
}

#[test]
fn conditional_calls_sharing_criteria_ranges_match_calls_scanning_alone() {
    // Calls over the same criteria ranges share an index once two have scanned them. Each must
    // return exactly what it returns as the only call, which always scans: text in every case,
    // numeric text, logicals, blanks, and an error in a summed cell. Under tight budgets the calls
    // that add a growing range read show that an indexed call is charged what its scan would be.
    let mut data = Vec::new();
    for row in 1..=120_u32 {
        let category = ["\"Alpha\"", "\"alpha\"", "\"BETA\"", "\"beta\"", "\"\""][row as usize % 5];
        data.push((row, 1, category.to_owned()));
        let key = if row % 13 == 0 {
            "TRUE".to_owned()
        } else if row % 11 == 0 {
            "\" 3 \"".to_owned()
        } else if row % 19 == 0 {
            continue;
        } else {
            (row % 7).to_string()
        };
        data.push((row, 2, key));
        let value = if row == 117 {
            "NA()".to_owned()
        } else {
            format!("{row}/8")
        };
        data.push((row, 3, value));
    }
    let mut calls = [
        "SUMIFS(C1:C120,A1:A120,\"alpha\",B1:B120,3)",
        "SUMIFS(C1:C120,A1:A120,\"beta\",B1:B120,2)",
        "SUMIFS(C1:C120,A1:A120,\"beta\",B1:B120,TRUE)",
        "COUNTIFS(A1:A120,\"ALPHA\",B1:B120,3)",
        "COUNTIFS(A1:A120,\"beta\",B1:B120,TRUE)",
        "AVERAGEIFS(C1:C120,A1:A120,\"alpha\",B1:B120,1)",
        "MAXIFS(C1:C120,A1:A120,\"beta\",B1:B120,4)",
        "MINIFS(C1:C120,A1:A120,\"alpha\",B1:B120,0)",
        "SUMIF(A1:A120,\"alpha\",C1:C120)",
        "COUNTIF(B1:B120,3)",
        "SUMIFS(C1:C120,A1:A120,\"alpha\",B1:B120,\">2\")",
        "COUNTIFS(A1:A120,\"a*\",B1:B120,3)",
    ]
    .map(str::to_owned)
    .to_vec();
    for rows in 1..=120 {
        calls.push(format!(
            "SUMIFS(C1:C120,A1:A120,\"alpha\",B1:B120,3)+SUM(C1:C{rows})"
        ));
        calls.push(format!(
            "COUNTIFS(A1:A120,\"alpha\",B1:B120,3)+COUNTIF(A1:A{rows},\"zz\")"
        ));
    }
    let workbook = |calls: &[String]| {
        let mut formulas = data.clone();
        formulas.extend(
            calls
                .iter()
                .enumerate()
                .map(|(index, call)| (index as u32 + 1, 5, call.clone())),
        );
        workbook_with_formulas(
            &formulas
                .iter()
                .map(|(row, column, text)| (*row, *column, text.as_str()))
                .collect::<Vec<_>>(),
        )
    };
    let shared = workbook(&calls);
    for limits in [
        CalculationLimits::default(),
        CalculationLimits::default()
            .with_max_array_cells(260)
            .expect("nonzero array limit"),
        CalculationLimits::default()
            .with_max_function_iterations(140)
            .expect("nonzero function work limit"),
    ] {
        let options = CalculationOptions::default().with_limits(limits);
        let together = calculate_workbook(&shared, options);
        for (index, call) in calls.iter().enumerate() {
            let alone = calculate_workbook(&workbook(std::slice::from_ref(call)), options);
            assert_eq!(
                together.cell(super::calculation_cell_id(index as u32 + 1, 5)),
                alone.cell(super::calculation_cell_id(1, 5)),
                "{call} with {limits:?}",
            );
        }
    }
}

fn assert_limit(calculation: &CalculationSnapshot, column: u32, expected: CalculationLimitKind) {
    let Some(CalculationCellResult::Unavailable(issue)) = calculation.cell(cell_id(column)) else {
        panic!("column {column} must report a resource limit");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
    assert_eq!(issue.limit(), Some(expected), "column {column}");
}

#[test]
fn whole_column_operations_ignore_unrelated_column_extents() {
    let workbook = workbook_with_formulas(&[
        (2, 1, "1"),
        (2, 2, "10"),
        (200, 26, "1"),
        (1, 3, "COUNTIF(A:A,1)"),
        (1, 4, "COUNTIF(A:A,\"\")"),
        (1, 5, "SUMIFS(B:B,A:A,1)"),
        (1, 6, "MAXIFS(B:B,A:A,1)"),
        (1, 7, "MATCH(99,A:A,0)"),
        (1, 8, "XLOOKUP(99,A:A,B:B,,0,-1)"),
    ]);
    let limits = CalculationLimits::default()
        .with_max_array_cells(20)
        .expect("nonzero array cell limit");
    let calculation =
        calculate_workbook(&workbook, CalculationOptions::default().with_limits(limits));

    for (column, expected) in [(3, 1.0), (4, 1_048_575.0), (5, 10.0), (6, 10.0)] {
        assert_number(&calculation, column, expected, 0.0);
    }
    for column in [7, 8] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::NotAvailable,
            ))),
        );
    }
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
        &workbook_with_formulas(&[(1, 1, "MATCH(\"a*\",A2,0)"), (2, 1, "\"alphabetical\"")]),
        CalculationOptions::default().with_limits(limits),
    );
    assert_issue(&limited, 1, CalculationIssueCode::ResourceLimitExceeded);
}
