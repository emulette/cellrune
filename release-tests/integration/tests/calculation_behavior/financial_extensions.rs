use super::support::{assert_issue, assert_number, cell_id, workbook_with_formulas};
use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationLimits, CalculationOptions, CellValue,
    ExcelError, calculate_workbook, scan_formula_capabilities,
};

#[test]
fn ddb_caps_depreciation_at_salvage_and_preserves_fractional_life() {
    for (formula, expected) in [
        ("DDB(2400,300,10,1)", 480.0),
        ("DDB(2400,300,10,2,1.5)", 306.0),
        ("DDB(10000,1000,5,2,2)", 2400.0),
        ("DDB(100,10,12.7,0.3,1)", 7.874015748032),
        ("DDB(100,10,13,0.3,50.3)", 90.0),
        ("DDB(100,10,13,2,50.3)", 0.0),
        ("DDB(100,90,5,1)", 10.0),
        ("DDB(100,90,5,2)", 0.0),
        ("DDB(2,1000,5,2,2)", 0.0),
        ("DDB(0,0,1,1)", 0.0),
        ("DDB(100,10,0.5,0.25)", 90.0),
        // Continuous declining balance for periods beyond the first.
        ("DDB(1000,100,5,2.5)", 185.903200617956),
    ] {
        let workbook = workbook_with_formulas(&[(1, 1, formula)]);
        assert!(scan_formula_capabilities(&workbook).is_supported());
        assert_number(
            &calculate_workbook(&workbook, CalculationOptions::default()),
            1,
            expected,
            1e-9,
        );
    }
    for formula in [
        "DDB(-1,0,1,1)",
        "DDB(1,-1,1,1)",
        "DDB(1,0,0,1)",
        "DDB(1,0,1,0)",
        "DDB(1,0,1,2)",
        "DDB(1,0,1,1,0)",
    ] {
        assert_error(formula, ExcelError::Number);
    }
    assert_error("DDB(\"bad\",0,1,1)", ExcelError::Value);
}

#[test]
fn xnpv_aligns_cashflows_with_truncated_dates_without_a_sign_constraint() {
    for (formula, expected) in [
        ("XNPV(0.1,{-100,110},{1,366})", 0.0),
        ("XNPV(0,{10,20,-5},{1,366,100})", 25.0),
        ("XNPV(0.1,{100,110},{1,366})", 200.0),
        ("XNPV(0.1,{-100,-110},{1,366})", -200.0),
        ("XNPV(0.1,{-100,110},{1.9,366.1})", 0.0),
        ("XNPV(0.1,{-100,50,50},{1,1,1})", 0.0),
        ("XNPV(0.1,{100},{1})", 100.0),
        ("XNPV(-0.5,{-100,100},{1,366})", 100.0),
        ("XNPV(-2,{-100,110},{1,366})", -210.0),
        // Independent maintained reference: ExcelFinancialFunctions spotXnpv.
        ("XNPV(0.14,{1,3,4},{25629,32176,36224})", 1.375214),
    ] {
        let workbook = workbook_with_formulas(&[(1, 1, formula)]);
        assert!(scan_formula_capabilities(&workbook).is_supported());
        assert_number(
            &calculate_workbook(&workbook, CalculationOptions::default()),
            1,
            expected,
            1e-6,
        );
    }
}

#[test]
fn xnpv_rejects_misalignment_invalid_dates_and_nonfinite_discounting() {
    for (formula, expected) in [
        ("XNPV(0.1,{1,2},{1})", ExcelError::Number),
        ("XNPV(0.1,{1,2},{2,1})", ExcelError::Number),
        ("XNPV(0.1,{1,2},{1,-1})", ExcelError::Value),
        ("XNPV(0.1,{1,2},{1,2958466})", ExcelError::Value),
        ("XNPV(0.1,{1,\"2\"},{1,366})", ExcelError::Value),
        ("XNPV(0.1,{1,2},{1,\"366\"})", ExcelError::Value),
        ("XNPV(0.1,{1,#N/A},{1,366})", ExcelError::NotAvailable),
        ("XNPV(-1,{1,2},{1,366})", ExcelError::Number),
        ("XNPV(-2,{1,2},{1,2})", ExcelError::Number),
        ("XNPV(\"bad\",{1,2},{1,2})", ExcelError::Value),
    ] {
        assert_error(formula, expected);
    }
}

#[test]
fn xnpv_discounting_respects_work_limits() {
    let result = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "XNPV(0,{1,2,3},{1,2,3})")]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(2)
                .unwrap(),
        ),
    );
    assert_issue(&result, 1, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn cumulative_payments_split_level_payments_into_interest_and_principal() {
    for (formula, expected, tolerance) in [
        // Microsoft's examples publish -11135.23213, -937.5, -934.1071234, and -68.27827118.
        ("CUMIPMT(0.09/12,360,125000,13,24,0)", -11_135.232_13, 1e-5),
        ("CUMIPMT(0.09/12,360,125000,1,1,0)", -937.5, 1e-9),
        ("CUMPRINC(0.09/12,360,125000,13,24,0)", -934.107_123_4, 1e-6),
        ("CUMPRINC(0.09/12,360,125000,1,1,0)", -68.278_271_18, 1e-7),
        // The Oracle records -917.99101493065655 and -3611.1050596319687.
        (
            "CUMIPMT(0.05/12,60,20000,1,12,0)",
            -917.991_014_930_656_6,
            1e-9,
        ),
        (
            "CUMPRINC(0.05/12,60,20000,1,12,0)",
            -3_611.105_059_631_968_7,
            1e-9,
        ),
        // The whole loan repays the principal.
        ("CUMPRINC(0.1,3,1000,1,3,0)", -1_000.0, 1e-9),
        (
            "CUMIPMT(\"0.1\",3,1000,1,3,0)",
            -206.344_410_876_131_86,
            1e-9,
        ),
    ] {
        let workbook = workbook_with_formulas(&[(1, 1, formula)]);
        assert!(scan_formula_capabilities(&workbook).is_supported());
        assert_number(
            &calculate_workbook(&workbook, CalculationOptions::default()),
            1,
            expected,
            tolerance,
        );
    }
    for formula in [
        "CUMIPMT(0,12,1000,1,2,0)",
        "CUMIPMT(0.1,0,1000,1,1,0)",
        "CUMIPMT(0.1,12,-1000,1,2,0)",
        "CUMIPMT(0.1,12,1000,0,2,0)",
        "CUMIPMT(0.1,12,1000,3,2,0)",
        "CUMIPMT(0.1,2,1000,1,12,0)",
        "CUMPRINC(0.1,12,1000,1,2,2)",
        "CUMPRINC(0.1,12,1000,1,2,0.5)",
    ] {
        assert_error(formula, ExcelError::Number);
    }
    assert_error("CUMPRINC(0.1,\"x\",1000,1,2,0)", ExcelError::Value);
    for formula in [
        "CUMIPMT(0.1,12,1000,1,2,1)",
        "CUMPRINC(0.1,12,1000,1.5,2,0)",
        "CUMPRINC(0.1,12,1000,1,2.5,0)",
    ] {
        let result = calculate_workbook(
            &workbook_with_formulas(&[(1, 1, formula)]),
            CalculationOptions::default(),
        );
        assert_issue(&result, 1, CalculationIssueCode::UnsupportedExpression);
    }
}

#[test]
fn vdb_switches_to_straight_line_when_it_exceeds_the_declining_balance() {
    for (formula, expected) in [
        // Microsoft's examples publish $1.32, $40.00, $480.00, $396.31, and $311.81.
        ("VDB(2400,300,3650,0,1)", 1.315_068_493_150_684_9),
        ("VDB(2400,300,120,0,1)", 40.0),
        ("VDB(2400,300,10,0,1)", 480.0),
        ("VDB(2400,300,120,6,18)", 396.306_053_264_751_9),
        ("VDB(2400,300,120,6,18,1.5)", 311.808_936_658_233_1),
        // The Oracle records 2400 and, for salvage above cost, 0.
        ("VDB(10000,1000,5,1,2,2,FALSE)", 2_400.0),
        ("VDB(\"2\",1000,5,1,2,2,FALSE)", 0.0),
        // Double declining balance leaves 2160 after three years; spreading it over the last two
        // years (1080 each) beats the 864 the declining balance would take in year four.
        ("VDB(10000,0,5,3,4)", 1_080.0),
        ("VDB(10000,0,5,3,5)", 2_160.0),
        ("VDB(10000,0,5,3,4,2,TRUE)", 864.0),
        ("VDB(10000,0,5,3,5,2,TRUE)", 1_382.4),
        // With a salvage of 1000 the straight line never overtakes the declining balance.
        ("VDB(10000,1000,5,3,5)", 1_160.0),
        ("VDB(2400,300,10,0,10)", 2_100.0),
        ("VDB(2400,300,10,4,4)", 0.0),
    ] {
        let workbook = workbook_with_formulas(&[(1, 1, formula)]);
        assert!(scan_formula_capabilities(&workbook).is_supported());
        assert_number(
            &calculate_workbook(&workbook, CalculationOptions::default()),
            1,
            expected,
            1e-9,
        );
    }
    for formula in [
        "VDB(-1,0,5,0,1)",
        "VDB(100,-1,5,0,1)",
        "VDB(100,0,0,0,1)",
        "VDB(100,0,5,-1,1)",
        "VDB(100,0,5,2,1)",
        "VDB(100,0,5,0,1,0)",
    ] {
        assert_error(formula, ExcelError::Number);
    }
    assert_error("VDB(100,0,5,0,1,2,\"maybe\")", ExcelError::Value);
    for formula in ["VDB(2400,300,10,0,0.875,1.5)", "VDB(100,0,5,0,6)"] {
        let result = calculate_workbook(
            &workbook_with_formulas(&[(1, 1, formula)]),
            CalculationOptions::default(),
        );
        assert_issue(&result, 1, CalculationIssueCode::UnsupportedExpression);
    }
    let result = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "VDB(100,0,1000,0,1000)")]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(999)
                .unwrap(),
        ),
    );
    assert_issue(&result, 1, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn french_depreciation_prorates_the_first_period_and_depreciates_whole_periods() {
    for (formula, expected) in [
        // Microsoft's examples publish 776 and 360; the Oracle records 2344 and 1500.
        ("AMORDEGRC(2400,39679,39813,300,1,0.15,1)", 776.0),
        ("AMORLINC(2400,39679,39813,300,1,0.15,1)", 360.0),
        (
            "AMORDEGRC(10000,DATE(2023,1,1),DATE(2023,12,31),1000,1,0.15,0)",
            2_344.0,
        ),
        (
            "AMORLINC(10000,DATE(2023,1,1),DATE(2023,12,31),1000,1,0.15)",
            1_500.0,
        ),
        // Period 0 is the first period prorated over 134 of 366 days, rounded for AMORDEGRC.
        ("AMORDEGRC(2400,39679,39813,300,0,0.15,1)", 330.0),
        (
            "AMORLINC(2400,39679,39813,300,0,0.15,1)",
            2_400.0 * 0.15 * 134.0 / 366.0,
        ),
        // AMORLINC takes the remainder after five whole periods, then nothing.
        (
            "AMORLINC(2400,39679,39813,300,6,0.15,1)",
            2_100.0 - 5.0 * 360.0 - 2_400.0 * 0.15 * 134.0 / 366.0,
        ),
        ("AMORLINC(2400,39679,39813,300,7,0.15,1)", 0.0),
        // A four-year life uses the 1.5 coefficient: 1000 × 0.375 = 375, then 625 × 0.375.
        (
            "AMORDEGRC(1000,DATE(2023,1,1),DATE(2023,12,31),0,1,0.25,0)",
            234.0,
        ),
    ] {
        let workbook = workbook_with_formulas(&[(1, 1, formula)]);
        assert!(scan_formula_capabilities(&workbook).is_supported());
        assert_number(
            &calculate_workbook(&workbook, CalculationOptions::default()),
            1,
            expected,
            1e-9,
        );
    }
    for formula in [
        "AMORDEGRC(\"2\",DATE(2023,1,1),DATE(2023,12,31),1000,1,0.15,0)",
        "AMORLINC(\"2\",DATE(2023,1,1),DATE(2023,12,31),1000,1,0.15,0)",
        "AMORDEGRC(2400,39679,39813,300,1,0.22,1)",
        "AMORDEGRC(2400,39679,39813,300,1,0.4,1)",
    ] {
        assert_error(formula, ExcelError::Number);
    }
    assert_error(
        "AMORLINC(\"x\",DATE(2023,1,1),DATE(2023,12,31),1000,1,0.15,0)",
        ExcelError::Value,
    );
    for formula in [
        "AMORLINC(2400,39679,39813,300,1,0.15,2)",
        "AMORLINC(2400,39679,39813,300,1.5,0.15,1)",
        "AMORLINC(2400,39813,39679,300,1,0.15,1)",
        "AMORLINC(2400,39679,39813,300,1,0,1)",
        "AMORDEGRC(2400,39679,39813,300,1,1,1)",
        // The last two periods of the life and a stop at salvage are not documented exactly.
        "AMORDEGRC(2400,39679,39813,300,5,0.15,1)",
        "AMORDEGRC(2400,39679,39813,2000,1,0.15,1)",
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
