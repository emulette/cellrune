use super::support::{assert_issue, assert_number, cell_id, workbook_with_formulas};
use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationLimits, CalculationOptions, CellValue,
    ExcelError, calculate_workbook, scan_formula_capabilities,
};

#[test]
fn code_uses_the_existing_windows_1252_character_mapping() {
    let cases = [
        ("CODE(\"ABC\")", 65.0),
        ("CODE(\"€\")", 128.0),
        ("CODE(\"Œ\")", 140.0),
        ("CODE(\"ÿ\")", 255.0),
        ("CODE(42)", 52.0),
        ("CODE(TRUE)", 84.0),
        ("CODE(CHAR(2))", 2.0),
    ];
    for (formula, expected) in cases {
        let workbook = workbook_with_formulas(&[(1, 1, formula)]);
        assert!(scan_formula_capabilities(&workbook).is_supported());
        assert_number(
            &calculate_workbook(&workbook, CalculationOptions::default()),
            1,
            expected,
            0.0,
        );
    }
    for formula in [
        "CODE(\"\")",
        "CODE(\"한\")",
        "CODE(\"😀\")",
        "CODE(UNICHAR(129))",
    ] {
        assert_error(formula, ExcelError::Value);
    }
    assert_error("CODE(#N/A)", ExcelError::NotAvailable);
    // Every defined CHAR entry round-trips through the same encoding table.
    let formulas = (1..=255)
        .filter(|code| ![129, 141, 143, 144, 157].contains(code))
        .map(|code| (code, format!("CODE(CHAR({code}))")))
        .collect::<Vec<_>>();
    let workbook = workbook_with_formulas(
        &formulas
            .iter()
            .map(|(code, formula)| (1, *code, formula.as_str()))
            .collect::<Vec<_>>(),
    );
    let result = calculate_workbook(&workbook, CalculationOptions::default());
    for (code, _) in formulas {
        assert_number(&result, code, f64::from(code), 0.0);
    }
}

#[test]
fn numbervalue_handles_explicit_separators_whitespace_and_percent() {
    for (formula, expected) in [
        ("NUMBERVALUE(\"2.500,27\",\",\",\".\")", 2500.27),
        ("NUMBERVALUE(\"1,234.56\")", 1234.56),
        ("NUMBERVALUE(\"3.5%\",,)", 0.035),
        ("NUMBERVALUE(\"9%%\")", 0.0009),
        ("NUMBERVALUE(\" 3 000 \" )", 3000.0),
        ("NUMBERVALUE(\"\")", 0.0),
        ("NUMBERVALUE(\" \" )", 0.0),
        ("NUMBERVALUE(\"1,2,3.4\")", 123.4),
        ("NUMBERVALUE(\"1:5\",\":rest\",\";ignored\")", 1.5),
        ("NUMBERVALUE(\"1٫5\",\"٫\",\"٬\")", 1.5),
        ("NUMBERVALUE(\"-1.25e+2\")", -125.0),
        ("NUMBERVALUE(12.5)", 12.5),
    ] {
        let workbook = workbook_with_formulas(&[(1, 1, formula)]);
        assert!(scan_formula_capabilities(&workbook).is_supported());
        assert_number(
            &calculate_workbook(&workbook, CalculationOptions::default()),
            1,
            expected,
            1e-12,
        );
    }
}

#[test]
fn numbervalue_rejects_malformed_numbers_and_propagates_errors() {
    for formula in [
        "NUMBERVALUE(\"1.2.3\")",
        "NUMBERVALUE(\"1.2,3\")",
        "NUMBERVALUE(\"1%2\")",
        "NUMBERVALUE(\"$12\")",
        "NUMBERVALUE(TRUE)",
        "NUMBERVALUE(\"NaN\")",
        "NUMBERVALUE(\"inf\")",
        "NUMBERVALUE(\"1e309\")",
        "NUMBERVALUE(\"1\",\".\",\".\")",
        "NUMBERVALUE(\"1\",\"\",\",\")",
        "NUMBERVALUE(\"1e2.3\")",
        "NUMBERVALUE(\"1e2,3\")",
    ] {
        assert_error(formula, ExcelError::Value);
    }
    assert_error("NUMBERVALUE(#N/A)", ExcelError::NotAvailable);
    assert_error("NUMBERVALUE(\"1\",#DIV/0!)", ExcelError::DivisionByZero);
}

#[test]
fn numbervalue_recognizes_parentheses_controls_and_separator_precedence() {
    for (formula, expected) in [
        ("NUMBERVALUE(\"(12.5)\")", -12.5),
        ("NUMBERVALUE(\"(128)%\")", -1.28),
        ("NUMBERVALUE(\" - 0 \t1\t2\r .\n3 4 \" )", -12.34),
        ("NUMBERVALUE(\"12%3\",\".\",\"%\")", 123.0),
        ("NUMBERVALUE(\"12%3\",\"%\",\",\")", 12.3),
        ("NUMBERVALUE(\"12 3\",\" \",\",\")", 12.3),
    ] {
        let result = calculate_workbook(
            &workbook_with_formulas(&[(1, 1, formula)]),
            CalculationOptions::default(),
        );
        assert_number(&result, 1, expected, 1e-12);
    }
    assert_error("NUMBERVALUE(\"1\",\".\",\"\")", ExcelError::Value);
    assert_error("NUMBERVALUE(\",\")", ExcelError::Value);
    assert_error("NUMBERVALUE(\"(-12)\")", ExcelError::Value);
}

#[test]
fn numbervalue_text_scans_observe_the_function_work_limit() {
    let result = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "NUMBERVALUE(\"12345\")")]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(4)
                .unwrap(),
        ),
    );
    assert_issue(&result, 1, CalculationIssueCode::ResourceLimitExceeded);
}

fn assert_error(formula: &str, expected: ExcelError) {
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
