use super::support::{assert_issue, assert_number, cell_id, workbook_with_formulas};
use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationLimits,
    CalculationOptions, CellAddress, CellValue, ExcelError, FormulaText, WorkbookDraft,
    calculate_workbook, scan_formula_capabilities,
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

#[test]
fn encodeurl_percent_encodes_the_documented_characters() {
    for (formula, expected) in [
        // Microsoft's example.
        (
            "ENCODEURL(\"http://contoso.sharepoint.com/Finance/Profit and Loss Statement.xlsx\")",
            "http%3A%2F%2Fcontoso.sharepoint.com%2FFinance%2FProfit%20and%20Loss%20Statement.xlsx",
        ),
        // Microsoft documents that alphanumeric characters stay.
        ("_xlfn.ENCODEURL(\"aZ09\")", "aZ09"),
        ("ENCODEURL(12.5)", "12.5"),
        ("ENCODEURL(TRUE)", "TRUE"),
        ("ENCODEURL(\"\")", ""),
    ] {
        let workbook = workbook_with_formulas(&[(1, 1, formula)]);
        assert!(scan_formula_capabilities(&workbook).is_supported());
        assert_eq!(
            calculate_workbook(&workbook, CalculationOptions::default()).cell(cell_id(1)),
            Some(&CalculationCellResult::Value(CellValue::Text(
                expected.to_owned()
            ))),
            "{formula}"
        );
    }
    assert_error("ENCODEURL(#N/A)", ExcelError::NotAvailable);
    // Percent-encoding conventions disagree on these, and Excel cannot be measured.
    for formula in [
        "ENCODEURL(\"a~b\")",
        "ENCODEURL(\"a-b_c\")",
        "ENCODEURL(\"a+b=c&d?e#f%\")",
        "ENCODEURL(\"é 한\")",
    ] {
        let result = calculate_workbook(
            &workbook_with_formulas(&[(1, 1, formula)]),
            CalculationOptions::default(),
        );
        assert_issue(&result, 1, CalculationIssueCode::UnsupportedExpression);
    }
}

#[test]
fn authored_encodeurl_is_stored_with_its_future_prefix() {
    let text = FormulaText::from_user_input("=ENCODEURL(\"a b\")").expect("user formula");
    assert_eq!(text.as_str(), "_xlfn.ENCODEURL(\"a b\")");
}

#[test]
fn filterxml_spills_the_string_values_of_the_selected_nodes_down_a_column() {
    let mut draft = WorkbookDraft::new();
    let sheet_id = draft.workbook().sheets()[0].id();
    let text = FormulaText::from_user_input(
        "=FILTERXML(\"<r><x k='1'>a</x><y><x>b &amp; c</x></y><x>d</x></r>\",\"//x[not(@k)]\")",
    )
    .expect("user formula");
    assert!(text.as_str().starts_with("_xlfn.FILTERXML("));
    draft
        .set_cell_dynamic_formula(
            sheet_id,
            CellAddress::from_a1("A1").expect("valid anchor"),
            text,
            None,
        )
        .expect("dynamic formula");
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    let cell = |address: &str| {
        calculation
            .materialized_cell(CalculationCellId::new(
                sheet_id,
                CellAddress::from_a1(address).expect("valid address"),
            ))
            .map(|cell| cell.result().clone())
    };
    assert_eq!(
        cell("A1"),
        Some(CalculationCellResult::Value(CellValue::Text(
            "b & c".to_owned()
        )))
    );
    assert_eq!(
        cell("A2"),
        Some(CalculationCellResult::Value(CellValue::Text(
            "d".to_owned()
        )))
    );
    assert_eq!(cell("A3"), None);
    assert_eq!(cell("B1"), None);
}

#[test]
fn filterxml_rejects_invalid_xml_and_leaves_undocumented_results_unsupported() {
    let workbook =
        workbook_with_formulas(&[(1, 1, "FILTERXML(\"<r><x>a</x><x>b</x></r>\",\"/r/x\")")]);
    assert!(scan_formula_capabilities(&workbook).is_supported());
    assert_eq!(
        calculate_workbook(&workbook, CalculationOptions::default()).cell(cell_id(1)),
        Some(&CalculationCellResult::Value(CellValue::Text(
            "a".to_owned()
        )))
    );
    // Microsoft documents #VALUE! for invalid XML and for an undeclared namespace prefix.
    assert_error("FILTERXML(\"<r><x>a</r>\",\"//x\")", ExcelError::Value);
    assert_error("FILTERXML(\"<p:r/>\",\"/*\")", ExcelError::Value);
    assert_error("FILTERXML(#N/A,\"//x\")", ExcelError::NotAvailable);
    for formula in [
        // Whether a value that reads as a number becomes one, and what an empty selection or a
        // non-node result returns, is not documented.
        "FILTERXML(\"<r><x>7</x></r>\",\"//x\")",
        "FILTERXML(\"<r><x/></r>\",\"//x\")",
        "FILTERXML(\"<r><x>a</x></r>\",\"//y\")",
        "FILTERXML(\"<r><x>a</x></r>\",\"count(//x)\")",
        "FILTERXML(\"<r><x>a</x></r>\",\"//x[\")",
    ] {
        let result = calculate_workbook(
            &workbook_with_formulas(&[(1, 1, formula)]),
            CalculationOptions::default(),
        );
        assert_issue(&result, 1, CalculationIssueCode::UnsupportedExpression);
    }
}

#[test]
fn bahttext_reads_baht_and_satang_as_thai_numerals() {
    for (formula, expected) in [
        // Microsoft's example and the Oracle.
        ("BAHTTEXT(1234)", "หนึ่งพันสองร้อยสามสิบสี่บาทถ้วน"),
        ("BAHTTEXT(1234.5)", "หนึ่งพันสองร้อยสามสิบสี่บาทห้าสิบสตางค์"),
        ("BAHTTEXT(\"2\")", "สองบาทถ้วน"),
        // Ten, twenty, a one after a nonzero tens digit, and millions.
        ("BAHTTEXT(10.01)", "สิบบาทหนึ่งสตางค์"),
        ("BAHTTEXT(21)", "ยี่สิบเอ็ดบาทถ้วน"),
        ("BAHTTEXT(1000000)", "หนึ่งล้านบาทถ้วน"),
        ("BAHTTEXT(21000011.21)", "ยี่สิบเอ็ดล้านสิบเอ็ดบาทยี่สิบเอ็ดสตางค์"),
        ("BAHTTEXT(2500000)", "สองล้านห้าแสนบาทถ้วน"),
        // Excel 16.114 for Mac: a final one after any higher digit is เอ็ด, zero and amounts below
        // one baht, negative amounts, and rounding to the nearest satang.
        ("BAHTTEXT(101)", "หนึ่งร้อยเอ็ดบาทถ้วน"),
        ("BAHTTEXT(1001)", "หนึ่งพันเอ็ดบาทถ้วน"),
        ("BAHTTEXT(1000001)", "หนึ่งล้านเอ็ดบาทถ้วน"),
        ("BAHTTEXT(21.25)", "ยี่สิบเอ็ดบาทยี่สิบห้าสตางค์"),
        ("BAHTTEXT(0)", "ศูนย์บาทถ้วน"),
        ("BAHTTEXT(0.5)", "ห้าสิบสตางค์"),
        ("BAHTTEXT(-5)", "ลบห้าบาทถ้วน"),
        ("BAHTTEXT(1.235)", "หนึ่งบาทยี่สิบสี่สตางค์"),
        // The same rules elsewhere.
        ("BAHTTEXT(0.01)", "หนึ่งสตางค์"),
        ("BAHTTEXT(-0.5)", "ลบห้าสิบสตางค์"),
        ("BAHTTEXT(1.234)", "หนึ่งบาทยี่สิบสามสตางค์"),
        ("BAHTTEXT(2000001.016)", "สองล้านเอ็ดบาทสองสตางค์"),
    ] {
        let workbook = workbook_with_formulas(&[(1, 1, formula)]);
        assert!(scan_formula_capabilities(&workbook).is_supported());
        assert_eq!(
            calculate_workbook(&workbook, CalculationOptions::default()).cell(cell_id(1)),
            Some(&CalculationCellResult::Value(CellValue::Text(
                expected.to_owned()
            ))),
            "{formula}"
        );
    }
    assert_error("BAHTTEXT(\"K001\")", ExcelError::Value);
    // A half satang where rounding up and to even disagree (1.225, 1.125), where the binary value
    // lies below the half (2.675), and a negative amount that rounds to zero.
    for formula in [
        "BAHTTEXT(1.225)",
        "BAHTTEXT(1.125)",
        "BAHTTEXT(2.675)",
        "BAHTTEXT(-0.001)",
    ] {
        let result = calculate_workbook(
            &workbook_with_formulas(&[(1, 1, formula)]),
            CalculationOptions::default(),
        );
        assert_issue(&result, 1, CalculationIssueCode::UnsupportedExpression);
    }
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

#[test]
fn text_lengths_and_positions_count_utf16_code_units() {
    let number =
        |value: f64| CalculationCellResult::Value(CellValue::number(value).expect("finite"));
    let text = |value: &str| CalculationCellResult::Value(CellValue::Text(value.to_owned()));
    let error = |value: ExcelError| CalculationCellResult::Value(CellValue::Error(value));
    for (formula, expected) in [
        ("LEN(\"😀\")", number(2.0)),
        ("MID(\"한😀글\",2,2)", text("😀")),
        ("FIND(\"글\",\"한😀글\")", number(4.0)),
        ("SEARCH(\"글\",\"한😀글\")", number(4.0)),
        ("FIND(\"a\",\"😀a\",2)", number(3.0)),
        ("FIND(\"a\",\"😀a\",4)", error(ExcelError::Value)),
        // A position inside a surrogate pair splits it, and U+FFFD, one code unit, takes the
        // place of the half that is left.
        ("LEN(REPLACE(\"😀abc\",1,1,\"x\"))", number(5.0)),
        ("REPLACE(\"😀abc\",1,1,\"x\")", text("x\u{FFFD}abc")),
        ("UNICODE(MID(\"😀\",1,1))", number(65533.0)),
        // LEFT and RIGHT keep a surrogate pair whole.
        ("LEFT(\"😀abc\",2)", text("😀a")),
        ("RIGHT(\"ab😀\",2)", text("b😀")),
        // The text limit is 32,767 code units.
        ("LEN(REPT(\"가\",32767))", number(32767.0)),
        ("REPT(\"가\",32768)", error(ExcelError::Value)),
        ("REPT(\"😀\",16384)", error(ExcelError::Value)),
        (
            "LEN(REPT(\"가\",11000)&REPT(\"가\",11000))",
            number(22000.0),
        ),
        (
            "LEN(CONCAT(REPT(\"가\",11000),REPT(\"가\",11000)))",
            number(22000.0),
        ),
        (
            "LEN(TEXTJOIN(\"\",TRUE,REPT(\"가\",11000),REPT(\"가\",11000)))",
            number(22000.0),
        ),
        (
            "LEN(CONCATENATE(REPT(\"가\",11000),REPT(\"가\",11000)))",
            number(22000.0),
        ),
        (
            "LEN(SUBSTITUTE(REPT(\"가\",20000),\"가\",\"나\"))",
            number(20000.0),
        ),
        (
            "LEN(REPLACE(REPT(\"가\",20000),1,1,\"나\"))",
            number(20000.0),
        ),
        (
            "LEN(_xlfn.REGEXREPLACE(REPT(\"가\",20000),\"가\",\"나\"))",
            number(20000.0),
        ),
    ] {
        let calculation = calculate_workbook(
            &workbook_with_formulas(&[(1, 1, formula)]),
            CalculationOptions::default(),
        );
        assert_eq!(calculation.cell(cell_id(1)), Some(&expected), "{formula}");
    }
    let calculation = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "REPT(\"가\",20000)&REPT(\"가\",20000)")]),
        CalculationOptions::default(),
    );
    assert_issue(&calculation, 1, CalculationIssueCode::ResourceLimitExceeded);
}
