use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationLimits,
    CalculationOptions, CellAddress, CellValue, DateSystem, ExcelError, FormulaText, WorkbookDraft,
    calculate_workbook,
};

fn result(formula: &str, input: Option<CellValue>) -> CalculationCellResult {
    result_in_system(formula, input, DateSystem::Excel1900)
}

fn result_in_system(
    formula: &str,
    input: Option<CellValue>,
    system: DateSystem,
) -> CalculationCellResult {
    let mut draft = WorkbookDraft::new();
    draft.set_date_system(system).unwrap();
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
        ("ISEVEN(2)", true),
        ("ISODD(3.9)", true),
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
    // Preserve the existing blank-reference contract while changing numeric text coercion.
    assert_eq!(result("ISEVEN(A1)", None), error(ExcelError::Value));
}

#[test]
fn error_type_classifies_spill_without_hiding_engine_issues() {
    for (formula, expected) in [
        ("ERROR.TYPE(#SPILL!)", 9.0),
        ("ERROR.TYPE(#NULL!)", 1.0),
        ("ERROR.TYPE(#VALUE!)", 3.0),
        ("ERROR.TYPE(#REF!)", 4.0),
        ("ERROR.TYPE(#NAME?)", 5.0),
        ("ERROR.TYPE(#NUM!)", 6.0),
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
    assert_eq!(result("SLN(100,10,-1)", None), error(ExcelError::Number));
    assert_eq!(
        result("SLN(100,#REF!,0)", None),
        error(ExcelError::Reference)
    );
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
            ("1", 10.0),
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
            result(&format!("{function}(A1:A2,1)"), None),
            error(ExcelError::Number)
        );
        assert_eq!(
            result(&format!("{function}({{0,\"ignored\",TRUE,10}},2)"), None),
            number(5.0)
        );
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
        "LOOKUP(0,{1,2,3})",
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
        ("LOOKUP(10,{1,2,3},{10,20,30})", 30.0),
        ("LOOKUP(TRUE,{FALSE,TRUE},{10,20})", 20.0),
        ("LOOKUP(2,{#N/A,1,2},{5,10,20})", 20.0),
        ("LOOKUP(A1,{0,1},{10,20})", 10.0),
    ] {
        assert_eq!(result(formula, None), number(expected), "{formula}");
    }
    assert_eq!(
        result("LOOKUP(0,A1:A2)", None),
        error(ExcelError::NotAvailable)
    );
}

#[test]
fn calendar_functions_reject_invalid_holidays_instead_of_ignoring_them() {
    for (function, arguments) in [
        ("NETWORKDAYS", "DATE(2026,1,1),DATE(2026,1,2)"),
        ("WORKDAY", "DATE(2026,1,1),1"),
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
fn holiday_serials_preserve_date_systems_deduplication_and_intl_contracts() {
    for (system, holiday) in [
        (DateSystem::Excel1900, 46024),
        (DateSystem::Excel1904, 44562),
    ] {
        for expression in [
            format!("{holiday}"),
            format!("{holiday}.9"),
            format!("\"{holiday}\""),
            format!("{{{holiday},{holiday}}}"),
            "A1".into(),
            "A1:A2".into(),
        ] {
            let formula = format!("NETWORKDAYS(DATE(2026,1,1),DATE(2026,1,2),{expression})");
            assert_eq!(
                result_in_system(&formula, Some(CellValue::Text(holiday.to_string())), system),
                number(1.0),
                "{formula}"
            );
        }
        for (formula, expected) in [
            (
                "NETWORKDAYS(DATE(2026,1,2),DATE(2026,1,1),DATE(2026,1,2))",
                -1.0,
            ),
            (
                "NETWORKDAYS(DATE(2026,1,1),DATE(2026,1,2),DATE(2026,1,3))",
                2.0,
            ),
            ("NETWORKDAYS(DATE(2026,1,1),DATE(2026,1,2),A1:A2)", 2.0),
            (
                "NETWORKDAYS.INTL(DATE(2026,1,1),DATE(2026,1,2),1,{TRUE,\"bad\"})",
                2.0,
            ),
        ] {
            assert_eq!(
                result_in_system(formula, None, system),
                number(expected),
                "{formula}"
            );
        }
        for text in ["bad", "2026-01-02", "", "inf"] {
            assert_eq!(
                result_in_system(
                    "NETWORKDAYS(DATE(2026,1,1),DATE(2026,1,2),A1)",
                    Some(CellValue::Text(text.into())),
                    system
                ),
                error(ExcelError::Value)
            );
        }
        assert_eq!(
            result_in_system(
                "NETWORKDAYS(DATE(2026,1,1),DATE(2026,1,2),TRUE)",
                None,
                system
            ),
            number(2.0)
        );
    }
}

#[test]
fn information_arrays_apply_the_same_coercion_and_error_mapping() {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    for (address, formula) in [
        ("A1", "ISEVEN({\"2\",3,1E20})"),
        ("A3", "ERROR.TYPE({#SPILL!,#N/A})"),
        ("A5", "ISEVEN({\"bad\",TRUE,#N/A,-2.9})"),
    ] {
        draft
            .set_cell_dynamic_formula(
                sheet,
                CellAddress::from_a1(address).unwrap(),
                FormulaText::from_xlsx(formula).unwrap(),
                None,
            )
            .unwrap();
    }
    let calculated = calculate_workbook(draft.workbook(), CalculationOptions::default());
    for (address, expected) in [
        ("A1", CalculationCellResult::Value(CellValue::Logical(true))),
        (
            "B1",
            CalculationCellResult::Value(CellValue::Logical(false)),
        ),
        ("C1", CalculationCellResult::Value(CellValue::Logical(true))),
        ("A3", number(9.0)),
        ("B3", number(7.0)),
        ("A5", error(ExcelError::Value)),
        ("B5", error(ExcelError::Value)),
        ("C5", error(ExcelError::NotAvailable)),
        ("D5", CalculationCellResult::Value(CellValue::Logical(true))),
    ] {
        assert_eq!(
            calculated
                .materialized_cell(CalculationCellId::new(
                    sheet,
                    CellAddress::from_a1(address).unwrap()
                ))
                .map(|cell| cell.result()),
            Some(&expected),
            "{address}"
        );
    }
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

fn sheet_results(
    values: &[(&str, CellValue)],
    formulas: &[(&str, &str)],
    dynamic: &[(&str, &str)],
) -> Vec<CalculationCellResult> {
    let mut draft = WorkbookDraft::new();
    let sheet = draft.workbook().sheets()[0].id();
    for (address, value) in values {
        draft
            .set_cell_value(sheet, CellAddress::from_a1(address).unwrap(), value.clone())
            .unwrap();
    }
    for (address, formula) in formulas {
        draft
            .set_cell_formula(
                sheet,
                CellAddress::from_a1(address).unwrap(),
                FormulaText::from_xlsx(*formula).unwrap(),
            )
            .unwrap();
    }
    for (address, formula) in dynamic {
        draft
            .set_cell_dynamic_formula(
                sheet,
                CellAddress::from_a1(address).unwrap(),
                FormulaText::from_xlsx(*formula).unwrap(),
                None,
            )
            .unwrap();
    }
    let calculation = calculate_workbook(draft.workbook(), CalculationOptions::default());
    formulas
        .iter()
        .chain(dynamic)
        .map(|(address, _)| {
            let id = CalculationCellId::new(sheet, CellAddress::from_a1(address).unwrap());
            calculation.cell(id).unwrap().clone()
        })
        .collect()
}

fn assert_close(actual: &CalculationCellResult, expected: f64, formula: &str) {
    let CalculationCellResult::Value(CellValue::Number(number)) = actual else {
        panic!("{formula}: expected a number, got {actual:?}");
    };
    let actual = number.get();
    assert!(
        (actual - expected).abs() <= 1e-12 * expected.abs().max(1.0),
        "{formula}: {actual} != {expected}",
    );
}

#[test]
fn rate_solvers_recover_after_stepping_past_the_minus_one_pole() {
    let cashflows = [
        -1000.0, 180.0, 220.0, 260.0, 300.0, 340.0, 380.0, -50.0, 420.0, 500.0,
    ];
    let values = cashflows
        .iter()
        .enumerate()
        .map(|(row, value)| (format!("A{}", row + 1), CellValue::number(*value).unwrap()))
        .collect::<Vec<_>>();
    let values = values
        .iter()
        .map(|(address, value)| (address.as_str(), value.clone()))
        .collect::<Vec<_>>();
    let formulas = [
        ("C1", "IRR({-100,1,1,1,1,1},10)"),
        ("C2", "IRR(A1:A10,\"2\")"),
        ("C3", "IRR(A1:A10)"),
        ("C4", "IRR({-100,60,60})"),
    ];
    let results = sheet_results(&values, &formulas, &[]);
    for ((_, formula), (result, expected)) in formulas.iter().zip(results.iter().zip([
        -0.553_500_302_130_925_5,
        0.211_724_438_710_438_9,
        0.211_724_438_710_438_9,
        0.130_662_386_291_807_6,
    ])) {
        assert_close(result, expected, formula);
    }
}

#[test]
fn lambda_and_let_accept_bare_row_and_column_axis_names() {
    for (formula, expected) in [
        ("LET(a,2,b,5,c,a*b,c+a)", 12.0),
        ("LET(r,3,R*2)", 6.0),
        ("LAMBDA(c,c+1)(2)", 3.0),
        ("LET(_xlpm.c,4,c)", 4.0),
    ] {
        assert_eq!(result(formula, None), number(expected), "{formula}");
    }
    for formula in ["LET(RC,1,RC)", "LET(R1C1,1,R1C1)", "LET(A1,1,A1)"] {
        assert_eq!(result(formula, None), error(ExcelError::Value), "{formula}");
    }
}

#[test]
fn xlookup_intersects_multi_column_results_in_legacy_formulas() {
    let values = [
        ("A1", CellValue::Text("K1".into())),
        ("A2", CellValue::Text("K2".into())),
        ("E1", CellValue::number(100.0).unwrap()),
        ("F1", CellValue::number(1.0).unwrap()),
        ("E2", CellValue::number(200.0).unwrap()),
        ("F2", CellValue::number(2.0).unwrap()),
    ];
    let results = sheet_results(
        &values,
        &[
            ("F5", "XLOOKUP(\"K2\",A1:A2,E1:F2)"),
            ("E6", "XLOOKUP(\"K2\",A1:A2,E1:F2)"),
            ("H5", "XLOOKUP(\"K2\",A1:A2,E1:F2)"),
            ("H6", "XLOOKUP(\"K2\",A1:A2,E1:E2)"),
            ("H7", "XLOOKUP(\"K9\",A1:A2,E1:F2,\"missing\")"),
        ],
        &[("J1", "XLOOKUP(\"K2\",A1:A2,E1:F2)")],
    );
    assert_eq!(results[0], number(2.0));
    assert_eq!(results[1], number(200.0));
    assert_eq!(results[2], error(ExcelError::Value));
    assert_eq!(results[3], number(200.0));
    assert_eq!(
        results[4],
        CalculationCellResult::Value(CellValue::Text("missing".into()))
    );
    assert_eq!(results[5], number(200.0));
}

#[test]
fn match_selects_its_mode_by_the_sign_of_the_match_type() {
    let values = [
        ("A1", CellValue::number(10.0).unwrap()),
        ("A2", CellValue::number(20.0).unwrap()),
        ("A3", CellValue::number(30.0).unwrap()),
        ("B1", CellValue::number(30.0).unwrap()),
        ("B2", CellValue::number(20.0).unwrap()),
        ("B3", CellValue::number(10.0).unwrap()),
    ];
    let formulas = [
        ("D1", "MATCH(25,A1:A3,\"2\")"),
        ("D2", "MATCH(25,A1:A3,0.5)"),
        ("D3", "MATCH(25,B1:B3,-2)"),
        ("D4", "MATCH(20,A1:A3,-0)"),
        ("D5", "MATCH(25,A1:A3,0)"),
    ];
    let results = sheet_results(&values, &formulas, &[]);
    assert_eq!(results[0], number(2.0));
    assert_eq!(results[1], number(2.0));
    assert_eq!(results[2], number(1.0));
    assert_eq!(results[3], number(2.0));
    assert_eq!(results[4], error(ExcelError::NotAvailable));
}

#[test]
fn datedif_day_difference_keeps_negative_month_end_counts_without_overflow() {
    for (formula, expected) in [
        ("DATEDIF(DATE(2024,1,31),DATE(2024,3,1),\"MD\")", -1.0),
        ("DATEDIF(DATE(2023,1,31),DATE(2023,3,1),\"MD\")", -2.0),
        ("DATEDIF(DATE(2024,1,15),DATE(2024,3,10),\"MD\")", 24.0),
        ("DATEDIF(DATE(2024,1,15),DATE(2024,3,20),\"MD\")", 5.0),
    ] {
        assert_eq!(result(formula, None), number(expected), "{formula}");
    }
}
