use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationLimits,
    CalculationOptions, CellAddress, CellValue, ExcelError, FormulaText, WorkbookDraft,
    calculate_workbook,
};

fn result(formula: &str, input: Option<CellValue>) -> CalculationCellResult {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    if let Some(value) = input {
        draft
            .set_cell_value(sheet, CellAddress::from_a1("A1").unwrap(), value)
            .unwrap();
    }
    let address = CellAddress::from_a1("B1").unwrap();
    draft
        .set_cell_formula(sheet, address, FormulaText::from_xlsx(formula).unwrap())
        .unwrap();
    calculate_workbook(draft.workbook(), CalculationOptions::default())
        .cell(CalculationCellId::new(sheet, address))
        .unwrap()
        .clone()
}

fn number(value: f64) -> CalculationCellResult {
    CalculationCellResult::Value(CellValue::number(value).unwrap())
}

fn error(value: ExcelError) -> CalculationCellResult {
    CalculationCellResult::Value(CellValue::Error(value))
}

#[test]
fn parity_coerces_numeric_text_without_changing_other_information_functions() {
    for (formula, expected) in [
        ("ISEVEN(\"2\")", true),
        ("ISODD(\"2\")", false),
        ("ISEVEN(\" -2.9 \" )", true),
        ("ISODD(-3.9)", true),
        ("ISEVEN(-0)", true),
        ("ISEVEN(1E20)", true),
        ("ISODD(-1E20)", false),
        ("ISNUMBER(\"2\")", false),
        ("ISTEXT(\"2\")", true),
        ("ISEVEN(A1)", true),
    ] {
        assert_eq!(
            result(formula, Some(CellValue::Text("2".into()))),
            CalculationCellResult::Value(CellValue::Logical(expected)),
            "{formula}",
        );
    }
    for formula in ["ISEVEN(\"bad\")", "ISODD(TRUE)", "ISEVEN(\"inf\")"] {
        assert_eq!(result(formula, None), error(ExcelError::Value), "{formula}");
    }
    assert_eq!(result("ISODD(#N/A)", None), error(ExcelError::NotAvailable));
}

#[test]
fn error_type_classifies_spill_without_hiding_engine_issues() {
    for (formula, expected) in [
        ("ERROR.TYPE(#SPILL!)", 9.0),
        ("ERROR.TYPE(A1)", 9.0),
        ("ERROR.TYPE(#DIV/0!)", 2.0),
        ("ERROR.TYPE(#N/A)", 7.0),
    ] {
        assert_eq!(
            result(formula, Some(CellValue::Error(ExcelError::Spill))),
            number(expected),
            "{formula}",
        );
    }
    assert_eq!(
        result("ERROR.TYPE(42)", None),
        error(ExcelError::NotAvailable)
    );
    let CalculationCellResult::Unavailable(issue) = result("ERROR.TYPE(NO_SUCH_FUNCTION())", None)
    else {
        panic!("engine issue must remain unavailable");
    };
    assert_eq!(issue.code(), CalculationIssueCode::UnsupportedFunction);
}

#[test]
fn straight_line_zero_life_keeps_its_own_error_contract() {
    for formula in ["SLN(10000,1000,0)", "SLN(10000,1000,\"0\")"] {
        assert_eq!(result(formula, None), error(ExcelError::DivisionByZero));
    }
    assert_eq!(result("SLN(100,10,2.5)", None), number(36.0));
    assert_eq!(result("SYD(100,10,0,1)", None), error(ExcelError::Number));
    assert_eq!(
        result("SLN(#N/A,10,0)", None),
        error(ExcelError::NotAvailable)
    );
    assert_eq!(
        result("SLN(100,10,\"bad\")", None),
        error(ExcelError::Value)
    );
}

#[test]
fn quartile_truncates_before_indexing_without_changing_percentile() {
    for function in ["QUARTILE.INC", "QUARTILE"] {
        for (quart, expected) in [
            ("0", 0.0),
            ("1.9", 10.0),
            ("2", 20.0),
            ("3", 30.0),
            ("4", 40.0),
            ("4.9", 40.0),
            ("-0.9", 0.0),
            ("\"1.9\"", 10.0),
        ] {
            let formula = format!("{function}({{0,10,20,30,40}},{quart})");
            assert_eq!(result(&formula, None), number(expected), "{formula}");
        }
        for quart in ["-1", "5", "1E100"] {
            assert_eq!(
                result(&format!("{function}({{1,2}},{quart})"), None),
                error(ExcelError::Number)
            );
        }
        assert_eq!(result(&format!("{function}({{7}},1.9)"), None), number(7.0));
        assert_eq!(
            result(&format!("{function}({{#N/A}},1.9)"), None),
            error(ExcelError::NotAvailable)
        );
    }
    assert_eq!(
        result("PERCENTILE.INC({0,10,20,30,40},0.475)", None),
        number(19.0)
    );
}

#[test]
fn lookup_does_not_use_numeric_candidates_for_text_searches() {
    for formula in [
        "LOOKUP(\"2\",{1,2,3},{10,20,30})",
        "LOOKUP(\"x\",{1,2,3},{10,20,30})",
        "LOOKUP(\"x\",{1,10;2,20;3,30})",
        "LOOKUP(TRUE,{1,2,3})",
    ] {
        assert_eq!(
            result(formula, None),
            error(ExcelError::NotAvailable),
            "{formula}"
        );
    }
    for (formula, expected) in [
        ("LOOKUP(2,{1,2,2,3},{10,20,21,30})", 21.0),
        ("LOOKUP(2.5,{1,2,3},{10,20,30})", 20.0),
        ("LOOKUP(\"B\",{1,\"a\",\"b\"},{10,20,30})", 30.0),
        ("LOOKUP(\"b\",{1,\"a\",\"c\"},{10,20,30})", 20.0),
        ("LOOKUP(2,{1;2;3},{10;20;30})", 20.0),
        ("LOOKUP(2,{1,2,3;10,20,30})", 20.0),
    ] {
        assert_eq!(result(formula, None), number(expected), "{formula}");
    }
}

#[test]
fn calendar_functions_reject_invalid_holidays_instead_of_ignoring_them() {
    for (function, arguments) in [
        ("NETWORKDAYS", "DATE(2026,1,1),DATE(2026,1,2)"),
        ("WORKDAY", "DATE(2026,1,1),1"),
        ("NETWORKDAYS.INTL", "DATE(2026,1,1),DATE(2026,1,2),1"),
        ("WORKDAY.INTL", "DATE(2026,1,1),1,1"),
    ] {
        for holiday in ["\"bad\"", "{\"bad\"}", "A1", "A1:A2"] {
            let formula = format!("{function}({arguments},{holiday})");
            assert_eq!(
                result(&formula, Some(CellValue::Text("bad".into()))),
                error(ExcelError::Value),
                "{formula}"
            );
        }
        assert_eq!(
            result(&format!("{function}({arguments},#N/A)"), None),
            error(ExcelError::NotAvailable)
        );
    }
}

#[test]
fn information_arrays_apply_the_same_coercion_and_error_mapping() {
    assert_eq!(
        result("INDEX(ISEVEN({\"2\",3,1E20}),1,1)", None),
        CalculationCellResult::Value(CellValue::Logical(true))
    );
    assert_eq!(
        result("INDEX(ERROR.TYPE({#SPILL!,#N/A}),1,1)", None),
        number(9.0)
    );
}

#[test]
fn compatibility_functions_keep_resource_failures_distinct() {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    let address = CellAddress::from_a1("A1").unwrap();
    draft
        .set_cell_formula(
            sheet,
            address,
            FormulaText::from_xlsx("ERROR.TYPE(QUARTILE.INC({1,2,3},1.9))").unwrap(),
        )
        .unwrap();
    let limits = CalculationLimits::default()
        .with_max_array_cells(1)
        .unwrap();
    let calculation = calculate_workbook(
        draft.workbook(),
        CalculationOptions::default().with_limits(limits),
    );
    let Some(CalculationCellResult::Unavailable(issue)) =
        calculation.cell(CalculationCellId::new(sheet, address))
    else {
        panic!("resource failure must remain unavailable");
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
}
