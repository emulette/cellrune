use cellrune::{
    CalculationCellId, CalculationCellResult, CalculationLimits, CalculationOptions, CellAddress,
    CellValue, DateSystem, ExcelError, FormulaText, WorkbookDraft, calculate_workbook,
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
    let mut options = CalculationOptions::default();
    if let Some(limits) = limits {
        options = options.with_limits(limits);
    }
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

fn numbers(results: &[CalculationCellResult]) -> Vec<f64> {
    results
        .iter()
        .map(|result| match result {
            CalculationCellResult::Value(CellValue::Number(number)) => number.get(),
            other => panic!("expected number, got {other:?}"),
        })
        .collect()
}

fn assert_close(actual: f64, expected: f64) {
    let tolerance = 5e-12_f64.max(expected.abs() * 5e-12);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, got {actual}"
    );
}

fn assert_num(result: &CalculationCellResult) {
    assert_eq!(
        result,
        &CalculationCellResult::Value(CellValue::Error(ExcelError::Number))
    );
}

#[test]
fn excel_host_defaults_truncation_and_tbill_boundaries_are_frozen() {
    let results = calculate(
        DateSystem::Excel1900,
        &[
            "=ACCRINTM(DATE(2024,1,1),DATE(2025,1,1),0.05)",
            "=ACCRINTM(DATE(2024,1,1),DATE(2025,1,1),0.05,)",
            "=ACCRINTM(DATE(2024,1,1),DATE(2025,1,1),0.05,,0)",
            "=ACCRINT(DATE(2024,1,1),DATE(2024,7,1),DATE(2025,1,1),0.05,,2,0)",
            "=ACCRINT(DATE(2007,3,1),DATE(2008,8,31),DATE(2008,5,1),0.1,1000,2,0,)",
            "=PRICE(DATE(2025,1,1),DATE(2030,1,1),0.05,0.04,100,2,0)",
            "=PRICE(DATE(2025,1,1),DATE(2030,1,1),0.05,0.04,100,2.9,0)",
            "=ODDFPRICE(DATE(2025,2,1),DATE(2030,3,1),DATE(2024,11,15),DATE(2025,3,1),0.05,0.06,100,2,0)",
            "=ODDFPRICE(DATE(2025,2,1),DATE(2030,3,1),DATE(2024,11,15),DATE(2025,3,1),0.05,0.06,100,2.9,0)",
            "=ODDFYIELD(DATE(2025,2,1),DATE(2030,3,1),DATE(2024,11,15),DATE(2025,3,1),0.05,95,100,2,0)",
            "=ODDFYIELD(DATE(2025,2,1),DATE(2030,3,1),DATE(2024,11,15),DATE(2025,3,1),0.05,95,100,2.9,0)",
            "=ODDLPRICE(DATE(2025,2,1),DATE(2025,6,15),DATE(2024,10,15),0.05,0.06,100,2,0)",
            "=ODDLPRICE(DATE(2025,2,1),DATE(2025,6,15),DATE(2024,10,15),0.05,0.06,100,2.9,0)",
            "=ODDLYIELD(DATE(2025,2,1),DATE(2025,6,15),DATE(2024,10,15),0.05,99,100,2,0)",
            "=ODDLYIELD(DATE(2025,2,1),DATE(2025,6,15),DATE(2024,10,15),0.05,99,100,2.9,0)",
            "=TBILLEQ(DATE(2025,1,1),DATE(2025,1,1),0.04)",
            "=TBILLPRICE(DATE(2025,1,1),DATE(2025,1,1),0.04)",
            "=TBILLYIELD(DATE(2025,1,1),DATE(2025,1,1),98)",
            "=TBILLEQ(DATE(2025,1,1),DATE(2025,7,2),0.04)",
            "=TBILLEQ(DATE(2025,1,1),DATE(2025,7,3),0.04)",
            "=TBILLEQ(DATE(2025,1,1),DATE(2026,1,1),0.04)",
            "=TBILLEQ(DATE(2023,7,1),DATE(2024,7,1),0.04)",
            "=TBILLEQ(DATE(2023,7,1),DATE(2024,7,2),0.04)",
        ],
        None,
    );

    for (index, expected) in [0.0, 50.0, 50.0, 50.0, 116.944_444_444_444]
        .into_iter()
        .enumerate()
    {
        assert_close(numbers(&results[index..=index])[0], expected);
    }
    for (left, right) in [(5, 6), (7, 8), (9, 10), (11, 12), (13, 14)] {
        assert_eq!(results[left], results[right]);
    }
    for result in &results[15..=17] {
        assert_num(result);
    }
    for (index, expected) in [
        (18, 0.041_392_606_033_114),
        (19, 0.041_394_959_763_901),
        (20, 0.041_832_345_790_172),
        (21, 0.041_950_586_074_360),
    ] {
        assert_close(numbers(&results[index..=index])[0], expected);
    }
    assert_num(&results[22]);
}

#[test]
fn both_date_systems_and_excel_1900_serial_60_are_explicit() {
    let formulas = [
        "=DISC(0,1,95,100,2)",
        "=DISC(59,60,95,100,2)",
        "=DISC(60,61,95,100,2)",
        "=DISC(59,61,95,100,2)",
        "=ACCRINTM(59,61,0.05,1000,2)",
        "=COUPDAYS(59,61,2,1)",
        "=COUPDAYBS(59,61,2,1)",
        "=COUPDAYSNC(59,61,2,1)",
        "=COUPPCD(59,61,2,1)",
        "=PRICE(59,61,0.05,0.04,100,2,2)",
        "=COUPNCD(DATE(2025,3,15),DATE(2027,1,1),2,0)",
    ];
    let excel_1900 = calculate(DateSystem::Excel1900, &formulas, None);
    let excel_1904 = calculate(DateSystem::Excel1904, &formulas, None);

    for (index, expected) in [18.0, 18.0, 18.0, 9.0, 0.277_777_777_777_778]
        .into_iter()
        .enumerate()
    {
        assert_close(numbers(&excel_1900[index..=index])[0], expected);
        assert_close(numbers(&excel_1904[index..=index])[0], expected);
    }
    assert_close(numbers(&excel_1900[5..=5])[0], 181.0);
    assert_close(numbers(&excel_1904[5..=5])[0], 182.0);
    for (index, expected_1900, expected_1904) in [(6, 59.0, 180.0), (7, 2.0, 2.0), (8, 0.0, -121.0)]
    {
        assert_close(numbers(&excel_1900[index..=index])[0], expected_1900);
        assert_close(numbers(&excel_1904[index..=index])[0], expected_1904);
    }
    assert_num(&excel_1900[9]);
    assert_close(numbers(&excel_1904[9..=9])[0], 100.0);
    assert_close(numbers(&excel_1900[10..=10])[0], 45_839.0);
    assert_close(numbers(&excel_1904[10..=10])[0], 44_377.0);
}

#[test]
fn basis_frequency_and_coupon_day_grid_matches_excel() {
    let results = calculate(
        DateSystem::Excel1900,
        &[
            "=DISC(DATE(2024,1,1),DATE(2025,1,1),95,100,0)",
            "=DISC(DATE(2024,1,1),DATE(2025,1,1),95,100,1)",
            "=DISC(DATE(2024,1,1),DATE(2025,1,1),95,100,2)",
            "=DISC(DATE(2024,1,1),DATE(2025,1,1),95,100,3)",
            "=DISC(DATE(2024,1,1),DATE(2025,1,1),95,100,4)",
            "=COUPNUM(DATE(2025,3,15),DATE(2027,1,1),1,0)",
            "=COUPNUM(DATE(2025,3,15),DATE(2027,1,1),2,0)",
            "=COUPNUM(DATE(2025,3,15),DATE(2027,1,1),4,0)",
            "=COUPNUM(DATE(2025,3,15),DATE(2027,1,1),3,0)",
            "=COUPNUM(DATE(2025,3,15),DATE(2027,1,1),1.9,0)",
            "=COUPNUM(DATE(2025,3,15),DATE(2027,1,1),4.9,0)",
            "=COUPDAYBS(DATE(2025,6,30),DATE(2027,1,1),2,0)",
            "=COUPDAYBS(DATE(2025,7,1),DATE(2027,1,1),2,0)",
            "=COUPDAYBS(DATE(2025,7,2),DATE(2027,1,1),2,0)",
            "=COUPDAYSNC(DATE(2025,6,30),DATE(2027,1,1),2,0)",
            "=COUPDAYSNC(DATE(2025,7,1),DATE(2027,1,1),2,0)",
            "=COUPDAYSNC(DATE(2025,7,2),DATE(2027,1,1),2,0)",
        ],
        None,
    );
    for (index, expected) in [
        0.05,
        0.05,
        0.049_180_327_868_853,
        0.049_863_387_978_142,
        0.05,
        2.0,
        4.0,
        8.0,
    ]
    .into_iter()
    .enumerate()
    {
        assert_close(numbers(&results[index..=index])[0], expected);
    }
    assert_num(&results[8]);
    assert_eq!(results[5], results[9]);
    assert_eq!(results[7], results[10]);
    for (index, expected) in [179.0, 0.0, 1.0, 1.0, 180.0, 179.0].into_iter().enumerate() {
        assert_close(numbers(&results[index + 11..=index + 11])[0], expected);
    }
}

#[test]
fn regular_and_odd_price_yield_reference_grid_matches_excel() {
    let results = calculate(
        DateSystem::Excel1900,
        &[
            "=ODDFPRICE(DATE(2025,2,1),DATE(2030,3,1),DATE(2024,11,15),DATE(2025,3,1),0.05,0.06,100,2,0)",
            "=ODDFPRICE(DATE(2025,2,1),DATE(2030,3,1),DATE(2023,9,1),DATE(2025,3,1),0.05,0.06,100,2,0)",
            "=ODDFYIELD(DATE(2025,2,1),DATE(2030,3,1),DATE(2023,9,1),DATE(2025,3,1),0.05,95.6442326278118,100,2,0)",
            "=ODDLPRICE(DATE(2025,2,1),DATE(2025,6,15),DATE(2025,1,15),0.05,0.06,100,2,0)",
            "=ODDLYIELD(DATE(2025,2,1),DATE(2025,6,15),DATE(2025,1,15),0.05,99,100,2,0)",
            "=ODDLPRICE(DATE(2025,2,1),DATE(2025,6,15),DATE(2024,10,15),0.05,0.06,100,2,0)",
            "=ODDLYIELD(DATE(2025,2,1),DATE(2025,6,15),DATE(2024,10,15),0.05,99,100,2,0)",
            "=YIELD(DATE(2025,1,1),DATE(2030,1,1),0.05,125,100,2,0)",
            "=YIELD(DATE(2025,1,1),DATE(2030,1,1),0.05,124.999999,100,2,0)",
            "=YIELD(DATE(2025,1,1),DATE(2030,1,1),0.05,150,100,2,0)",
            "=PRICE(DATE(2025,1,1),DATE(2030,1,1),0.05,-0.01,100,2,0)",
            "=DURATION(DATE(2025,1,1),DATE(2030,1,1),0.05,-0.01,2,0)",
        ],
        None,
    );
    for (index, expected) in [
        95.673_855_249_014_6,
        95.644_232_627_811_8,
        0.060_000_000_003_764,
        99.631_054_595_515,
        0.077_468_202_102_589,
        99.603_747_781_038_3,
        0.076_504_400_859_952,
        0.0,
        0.000_000_001_758_242,
        -0.039_470_046_880_992,
    ]
    .into_iter()
    .enumerate()
    {
        assert_close(numbers(&results[index..=index])[0], expected);
    }
    assert_num(&results[10]);
    assert_num(&results[11]);
}

#[test]
fn required_values_date_order_and_negative_yield_domains_are_explicit() {
    let results = calculate(
        DateSystem::Excel1900,
        &[
            "=PRICE(DATE(2025,1,1),DATE(2030,1,1),0.05,0.04,,2,0)",
            "=PRICEDISC(DATE(2025,1,1),DATE(2025,7,1),0.04,,0)",
            "=ODDFPRICE(DATE(2025,2,1),DATE(2030,3,1),DATE(2024,11,15),DATE(2025,3,1),0.05,0.06,,2,0)",
            "=ODDLPRICE(DATE(2025,2,1),DATE(2025,6,15),DATE(2024,10,15),0.05,0.06,,2,0)",
            "=PRICE(DATE(2025,1,1),DATE(2030,1,1),0.05,-0.01,100,2,0)",
            "=DURATION(DATE(2025,1,1),DATE(2030,1,1),0.05,-0.01,2,0)",
            "=MDURATION(DATE(2025,1,1),DATE(2030,1,1),0.05,-0.01,2,0)",
            "=PRICEMAT(DATE(2025,1,1),DATE(2025,7,1),DATE(2025,1,1),0.05,0.04,0)",
            "=YIELDMAT(DATE(2025,1,1),DATE(2025,7,1),DATE(2025,2,1),0.05,98,0)",
            "=ACCRINT(DATE(2025,1,1),DATE(2025,7,1),DATE(2025,1,1),0.05,1000,2,0)",
            "=YIELD(DATE(2025,1,1),DATE(2030,1,1),0.05,0,100,2,0)",
            "=YIELD(DATE(2025,1,1),DATE(2030,1,1),0.05,95,0,2,0)",
        ],
        None,
    );
    assert_eq!(results.len(), 12);
    for result in &results {
        assert_num(result);
    }
}

#[test]
fn long_coupon_work_is_rejected_before_partial_numeric_materialization() {
    let limits = CalculationLimits::default()
        .with_max_function_iterations(11)
        .expect("iteration limit");
    let results = calculate(
        DateSystem::Excel1900,
        &["=PRICE(DATE(2025,1,1),DATE(2030,1,1),0.05,0.04,100,2,0)"],
        Some(limits),
    );
    let CalculationCellResult::Unavailable(issue) = &results[0] else {
        panic!("expected resource issue, got {:?}", results[0]);
    };
    assert_eq!(issue.detail(), Some("max_function_iterations"));
}
