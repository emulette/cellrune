use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationIssueCode, CalculationLimits,
    CalculationOptions, CellAddress, CellValue, DateSystem, ExcelError, FormulaText, WorkbookDraft,
    calculate_workbook,
};

fn calculate(
    date_system: DateSystem,
    formulas: &[&str],
    limits: Option<CalculationLimits>,
) -> Vec<CalculationCellResult> {
    let mut draft = WorkbookDraft::new();
    draft
        .set_date_system(date_system)
        .expect("test date system");
    let sheet = draft.workbook().sheets()[0].id();
    for (index, formula) in formulas.iter().enumerate() {
        draft
            .set_cell_formula(
                sheet,
                CellAddress::from_indices(index as u32 + 1, 1).expect("test address"),
                FormulaText::from_user_input(*formula).expect("test formula"),
            )
            .expect("test formula cell");
    }
    let options = limits.map_or_else(CalculationOptions::default, |limits| {
        CalculationOptions::default().with_limits(limits)
    });
    let calculation = calculate_workbook(draft.workbook(), options);
    (0..formulas.len())
        .map(|index| {
            calculation
                .cell(CalculationCellId::new(
                    sheet,
                    CellAddress::from_indices(index as u32 + 1, 1).expect("result address"),
                ))
                .expect("formula result")
                .clone()
        })
        .collect()
}

fn number(result: &CalculationCellResult) -> f64 {
    let CalculationCellResult::Value(CellValue::Number(number)) = result else {
        panic!("expected number, got {result:?}");
    };
    number.get()
}

fn assert_close(result: &CalculationCellResult, expected: f64) {
    let actual = number(result);
    assert!(
        (actual - expected).abs() <= 1e-15,
        "expected {expected}, got {actual}"
    );
}

fn assert_error(result: &CalculationCellResult, expected: ExcelError) {
    assert_eq!(
        result,
        &CalculationCellResult::Value(CellValue::Error(expected))
    );
}

#[test]
fn datevalue_and_timevalue_freeze_ascii_grammar_and_both_date_systems() {
    let formulas = [
        "=DATEVALUE(\"2024-02-29\")",
        "=DATE(2024,2,29)",
        "=DATEVALUE(\" \t2024-02-29\t \")",
        "=DATEVALUE(\"9999-12-31\")",
        "=DATE(9999,12,31)",
        "=DATEVALUE(\"1900-02-29\")",
        "=DATEVALUE(\"2023-02-29\")",
        "=DATEVALUE(\"2024/02/29\")",
        "=DATEVALUE(\"February 29, 2024\")",
        "=DATEVALUE(\"2024-02-29T00:00:00\")",
        "=DATEVALUE(\"２０２４-02-29\")",
        "=DATEVALUE(1)",
        "=DATEVALUE(TRUE)",
        "=DATEVALUE(\"0000-01-01\")",
        "=TIMEVALUE(\"00:00\")",
        "=TIMEVALUE(\"14:35:42\")",
        "=TIMEVALUE(\"23:59:59.1\")",
        "=TIMEVALUE(\"00:00:00.123456789\")",
        "=TIMEVALUE(\"12:34:56.1234567890\")",
        "=TIMEVALUE(\"24:00\")",
        "=TIMEVALUE(\"12:60\")",
        "=TIMEVALUE(\"12:00:60\")",
        "=TIMEVALUE(\"11:00 PM\")",
        "=TIMEVALUE(\"2024-02-29 12:00\")",
        "=TIMEVALUE(1)",
    ];
    let excel_1900 = calculate(DateSystem::Excel1900, &formulas, None);
    assert_eq!(excel_1900[0], excel_1900[1]);
    assert_eq!(excel_1900[0], excel_1900[2]);
    assert_eq!(excel_1900[3], excel_1900[4]);
    assert_close(&excel_1900[5], 60.0);
    for result in &excel_1900[6..=12] {
        assert_error(result, ExcelError::Value);
    }
    assert_error(&excel_1900[13], ExcelError::Number);
    assert_close(&excel_1900[14], 0.0);
    assert_close(
        &excel_1900[15],
        (14.0 * 3_600.0 + 35.0 * 60.0 + 42.0) / 86_400.0,
    );
    assert_close(&excel_1900[16], (86_399.0 + 0.1) / 86_400.0);
    assert_close(&excel_1900[17], 0.123_456_789 / 86_400.0);
    for result in &excel_1900[18..] {
        assert_error(result, ExcelError::Value);
    }

    let excel_1904 = calculate(
        DateSystem::Excel1904,
        &[
            "=DATEVALUE(\"2024-02-29\")",
            "=DATE(2024,2,29)",
            "=DATEVALUE(\"1904-01-01\")",
            "=DATEVALUE(\"9999-12-31\")",
            "=DATEVALUE(\"1900-02-29\")",
            "=DATEVALUE(\"1900-01-01\")",
        ],
        None,
    );
    assert_eq!(excel_1904[0], excel_1904[1]);
    assert_close(&excel_1904[2], 0.0);
    assert_eq!(number(&excel_1900[0]) - number(&excel_1904[0]), 1_462.0);
    assert_eq!(number(&excel_1900[3]) - number(&excel_1904[3]), 1_462.0);
    assert_error(&excel_1904[4], ExcelError::Value);
    assert_error(&excel_1904[5], ExcelError::Number);
}

#[test]
fn intl_workdays_freeze_weekend_holiday_range_and_budget_boundaries() {
    let results = calculate(
        DateSystem::Excel1900,
        &[
            "=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,11),1)",
            "=NETWORKDAYS.INTL(DATE(2026,1,11),DATE(2026,1,5),1)",
            "=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,5),2)",
            "=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,5),1)",
            "=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,5),12)",
            "=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,5),11)",
            "=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,5),\"1000000\")",
            "=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,5),\"0000001\")",
            "=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,11),\"1111111\")",
            "=WORKDAY.INTL(DATE(2026,1,5),1,\"1111111\")",
            "=WORKDAY.INTL(DATE(2026,1,10),0,1)",
            "=DATE(2026,1,10)",
            "=WORKDAY.INTL(DATE(2026,1,1),\"2\",1)",
            "=DATE(2026,1,5)",
            "=WORKDAY.INTL(DATE(2026,1,5),-1,1)",
            "=DATE(2026,1,2)",
            "=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,11),1,{46027,46027,46032})",
            "=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,11),1,{TRUE,\"x\",46032})",
            "=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,11),1,1/0)",
            "=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,11),,{46027})",
            "=NETWORKDAYS.INTL(1,2,\"000001\")",
            "=NETWORKDAYS.INTL(1,2,\"00000x1\")",
            "=NETWORKDAYS.INTL(1,2,\"2\")",
            "=NETWORKDAYS.INTL(1,2,8)",
            "=NETWORKDAYS.INTL(1,2,10.9)",
            "=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,5),11.9)",
            "=NETWORKDAYS.INTL(-1,1,1)",
            "=WORKDAY.INTL(DATE(9999,12,31),1,1)",
            "=WORKDAY.INTL(DATE(2026,1,1),\"x\",1)",
        ],
        None,
    );
    for (index, expected) in [
        (0, 5.0),
        (1, -5.0),
        (2, 0.0),
        (3, 1.0),
        (4, 0.0),
        (5, 1.0),
        (6, 0.0),
        (7, 1.0),
        (8, 0.0),
        (16, 4.0),
        (17, 5.0),
        (19, 4.0),
        (25, 1.0),
    ] {
        assert_close(&results[index], expected);
    }
    assert_error(&results[9], ExcelError::Value);
    assert_eq!(results[10], results[11]);
    assert_eq!(results[12], results[13]);
    assert_eq!(results[14], results[15]);
    assert_error(&results[18], ExcelError::DivisionByZero);
    for index in [20, 21, 22, 28] {
        assert_error(&results[index], ExcelError::Value);
    }
    for index in [23, 24, 26, 27] {
        assert_error(&results[index], ExcelError::Number);
    }

    let limited = calculate(
        DateSystem::Excel1900,
        &["=NETWORKDAYS.INTL(DATE(2026,1,5),DATE(2026,1,9),1,{46027,46028})"],
        Some(
            CalculationLimits::default()
                .with_max_function_iterations(6)
                .expect("positive cumulative calendar budget"),
        ),
    );
    let CalculationCellResult::Unavailable(issue) = &limited[0] else {
        panic!(
            "expected cumulative calendar resource failure, got {:?}",
            limited[0]
        );
    };
    assert_eq!(issue.code(), CalculationIssueCode::ResourceLimitExceeded);
}
