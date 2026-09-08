use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationLimits, CalculationOptions, CellValue,
    ExcelError, calculate_workbook, scan_formula_capabilities,
};

use super::support::{assert_issue, assert_number, cell_id, workbook_with_formulas};

#[test]
fn normal_inverse_names_share_center_tail_and_scale_semantics() {
    for (formula, expected) in [
        ("NORM.S.INV(0.5)", 0.0),
        ("NORMSINV(0.975)", 1.959_963_984_540_053_8),
        ("_xlfn.NORM.S.INV(1e-300)", -37.047_096_299_361_2),
        ("NORM.INV(0.5,12,3)", 12.0),
        ("NORMINV(0.975,12,3)", 17.879_891_953_620_16),
        ("NORM.INV(\"0.5\",\"12\",TRUE)", 12.0),
        ("NORM.INV(0.9772498680518208,-1e308,1e308)", 1e308),
    ] {
        assert_formula_number(formula, expected);
    }
}

#[test]
fn lognormal_names_share_density_cdf_and_inverse_semantics() {
    for (formula, expected) in [
        ("LOGNORM.DIST(1,0,2,TRUE)", 0.5),
        ("LOGNORMDIST(1,0,2)", 0.5),
        ("LOGNORM.DIST(1,0,2,FALSE)", 0.199_471_140_200_716_35),
        ("LOGNORM.INV(0.5,2,3)", 7.389_056_098_930_65),
        ("LOGINV(0.5,2,3)", 7.389_056_098_930_65),
        ("LOGNORM.INV(0.5,-1000,1)", 0.0),
        (
            "LOGNORM.DIST(1e-300,LN(1e-300),1,FALSE)",
            3.989_422_804_014_327e299,
        ),
        ("LOGNORM.DIST(1e300,0,1,TRUE)", 1.0),
        ("LOGNORM.DIST(1e-300,0,1,TRUE)", 0.0),
    ] {
        assert_formula_number(formula, expected);
    }
}

#[test]
fn normal_extensions_reject_invalid_domains_arity_and_preserve_error_order() {
    for (formula, expected) in [
        ("NORM.S.INV(0)", ExcelError::Number),
        ("NORM.S.INV(1)", ExcelError::Number),
        ("NORM.INV(0.5,0,0)", ExcelError::Number),
        ("NORM.INV(0.5,0,-1)", ExcelError::Number),
        ("NORM.INV(0.975,0,1e308)", ExcelError::Number),
        ("LOGNORM.DIST(0,0,1,TRUE)", ExcelError::Number),
        ("LOGNORM.DIST(1,0,0,TRUE)", ExcelError::Number),
        ("LOGNORM.INV(1,0,1)", ExcelError::Number),
        ("LOGNORM.INV(0.5,1000,1)", ExcelError::Number),
        ("NORM.S.INV(\"bad\")", ExcelError::Value),
        ("NORM.INV(#N/A,#DIV/0!,0)", ExcelError::NotAvailable),
        ("NORM.INV(0,#DIV/0!,0)", ExcelError::Number),
        ("LOGNORM.DIST(1,0,1,#N/A)", ExcelError::NotAvailable),
        ("LOGNORMDIST(1,0,1,TRUE)", ExcelError::Value),
        ("LOGNORM.DIST(1,0,1)", ExcelError::Value),
        ("NORM.S.INV(0.5,0)", ExcelError::Value),
    ] {
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
}

#[test]
fn normal_quantile_charges_refinement_work() {
    for formula in [
        "NORM.S.INV(1e-300)",
        "NORMINV(0.975,0,1)",
        "LOGINV(0.975,0,1)",
    ] {
        let workbook = workbook_with_formulas(&[(1, 1, formula)]);
        let result = calculate_workbook(
            &workbook,
            CalculationOptions::default().with_limits(
                CalculationLimits::default()
                    .with_max_function_iterations(1)
                    .unwrap(),
            ),
        );
        assert_issue(&result, 1, CalculationIssueCode::ResourceLimitExceeded);
        let result = calculate_workbook(
            &workbook,
            CalculationOptions::default().with_limits(
                CalculationLimits::default()
                    .with_max_function_iterations(2640)
                    .unwrap(),
            ),
        );
        assert!(matches!(
            result.cell(cell_id(1)),
            Some(CalculationCellResult::Value(CellValue::Number(_)))
        ));
    }
}

fn assert_formula_number(formula: &str, expected: f64) {
    let workbook = workbook_with_formulas(&[(1, 1, formula)]);
    assert!(
        scan_formula_capabilities(&workbook).is_supported(),
        "{formula}"
    );
    let result = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&result, 1, expected, expected.abs() * 1e-13);
}
