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
