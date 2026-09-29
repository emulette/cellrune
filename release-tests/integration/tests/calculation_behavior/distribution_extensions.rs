use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationLimits, CalculationOptions, CellValue,
    ExcelError, calculate_workbook, scan_formula_capabilities,
};

use super::support::{assert_issue, assert_number, cell_id, workbook_with_formulas};

#[test]
fn chi_square_names_share_the_gamma_cdf_density_and_direct_right_tail() {
    for (formula, expected) in [
        ("_xlfn.CHISQ.DIST(4,3,TRUE)", 0.738_535_870_050_889_5),
        ("CHISQ.DIST(\"2\",3,TRUE)", 0.427_593_295_529_120_2),
        ("CHISQ.DIST(0.5,1,TRUE)", 0.520_499_877_813_046_5),
        ("CHISQ.DIST(2,3,FALSE)", 0.207_553_748_710_297_36),
        // The degrees of freedom truncate: two degrees give 1 − e^(−x/2).
        ("CHISQ.DIST(2,2.9,TRUE)", 0.632_120_558_828_557_7),
        ("_xlfn.CHISQ.DIST.RT(4,3)", 0.261_464_129_949_110_56),
        ("CHIDIST(4,3)", 0.261_464_129_949_110_56),
        ("CHIDIST(\"2\",3)", 0.572_406_704_470_879_8),
        ("CHISQ.DIST.RT(18.307,10)", 0.050_000_589_091_398_12),
        ("CHISQ.DIST.RT(0,3)", 1.0),
        // Q(3/2, 500) = erfc(√500) + 2·√(500/π)·e^(−500), far below 1 − P's resolution.
        ("CHISQ.DIST.RT(1000,3)", 1.799_420_876_531_448e-216),
    ] {
        assert_formula_number(formula, expected, 1e-12);
    }
    assert_formula_number("CHISQ.DIST(0,3,TRUE)", 0.0, 0.0);
}

#[test]
fn weibull_names_share_the_closed_form_cdf_and_density() {
    for (formula, expected) in [
        ("_xlfn.WEIBULL.DIST(105,20,100,TRUE)", 0.929_581_390_069_277),
        ("WEIBULL(105,20,100,TRUE)", 0.929_581_390_069_277),
        ("WEIBULL.DIST(105,20,100,FALSE)", 0.035_588_864_024_504_34),
        // A CDF far below machine epsilon survives instead of rounding 1 − exp to zero.
        (
            "WEIBULL.DIST(\"2\",20,100,TRUE)",
            1.048_576_000_000_007_6e-34,
        ),
        ("WEIBULL(\"2\",20,100,TRUE)", 1.048_576_000_000_007_6e-34),
        // Shape one is the exponential distribution with mean beta.
        ("WEIBULL.DIST(1,1,2,TRUE)", 0.393_469_340_287_366_6),
        ("WEIBULL.DIST(0,1,4,FALSE)", 0.25),
    ] {
        assert_formula_number(formula, expected, 1e-13);
    }
    assert_formula_number("WEIBULL.DIST(0,0.5,1,TRUE)", 0.0, 0.0);
    assert_formula_number("WEIBULL.DIST(0,2,1,FALSE)", 0.0, 0.0);
}

#[test]
fn confidence_names_scale_normal_and_t_critical_values() {
    for (formula, expected) in [
        ("CONFIDENCE(0.05,2.5,100)", 0.489_990_996_135_013_4),
        (
            "_xlfn.CONFIDENCE.NORM(0.05,2.5,100)",
            0.489_990_996_135_013_4,
        ),
        ("CONFIDENCE.NORM(0.05,2.5,100.9)", 0.489_990_996_135_013_4),
        ("CONFIDENCE.NORM(0.05,2.5,50)", 0.692_951_912_174_839_1),
        ("_xlfn.CONFIDENCE.T(0.05,2.5,100)", 0.496_054_237_896_604_13),
        // t(0.975, 49)/√50 from a 50-digit bisection of the regularized incomplete beta.
        ("CONFIDENCE.T(0.05,1,50)", 0.284_196_855_495_729_76),
    ] {
        assert_formula_number(formula, expected, 1e-12);
    }
}

#[test]
fn new_distributions_reject_invalid_domains_with_excel_errors() {
    for (formula, expected) in [
        ("CHISQ.DIST(-1,3,TRUE)", ExcelError::Number),
        ("CHISQ.DIST.RT(-1,3)", ExcelError::Number),
        ("CHIDIST(-1,3)", ExcelError::Number),
        ("CHISQ.DIST(1,0.5,TRUE)", ExcelError::Number),
        ("CHISQ.DIST.RT(1,10000000001)", ExcelError::Number),
        ("CHISQ.DIST(\"x\",3,TRUE)", ExcelError::Value),
        ("CHISQ.DIST(1,3,\"x\")", ExcelError::Value),
        ("WEIBULL.DIST(-1,20,100,TRUE)", ExcelError::Number),
        ("WEIBULL(-1,20,100,TRUE)", ExcelError::Number),
        ("WEIBULL.DIST(1,0,100,TRUE)", ExcelError::Number),
        ("WEIBULL.DIST(1,20,-1,TRUE)", ExcelError::Number),
        ("WEIBULL.DIST(0,0.5,1,FALSE)", ExcelError::Number),
        ("WEIBULL.DIST(1,2,3,#N/A)", ExcelError::NotAvailable),
        ("CONFIDENCE(0,2.5,100)", ExcelError::Number),
        ("CONFIDENCE.NORM(0,2.5,100)", ExcelError::Number),
        ("CONFIDENCE.NORM(\"2\",2.5,100)", ExcelError::Number),
        ("CONFIDENCE.NORM(1,2.5,100)", ExcelError::Number),
        ("CONFIDENCE.NORM(0.05,0,100)", ExcelError::Number),
        ("CONFIDENCE.NORM(0.05,2.5,0.9)", ExcelError::Number),
        ("CONFIDENCE.NORM(\"x\",2.5,100)", ExcelError::Value),
        ("CONFIDENCE.T(0,2.5,100)", ExcelError::Number),
        ("CONFIDENCE.T(\"2\",2.5,100)", ExcelError::Number),
        ("CONFIDENCE.T(0.05,2.5,1)", ExcelError::DivisionByZero),
        ("CONFIDENCE.T(0.05,-1,1)", ExcelError::Number),
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
fn iterative_distribution_kernels_charge_refinement_work() {
    for formula in [
        "CHISQ.DIST(4,3,TRUE)",
        "CHISQ.DIST.RT(4,3)",
        "CONFIDENCE.NORM(0.05,2.5,100)",
        "CONFIDENCE.T(0.05,2.5,100)",
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
        let result = calculate_workbook(&workbook, CalculationOptions::default());
        assert!(
            matches!(
                result.cell(cell_id(1)),
                Some(CalculationCellResult::Value(CellValue::Number(_)))
            ),
            "{formula}"
        );
    }
}

fn assert_formula_number(formula: &str, expected: f64, relative_tolerance: f64) {
    let workbook = workbook_with_formulas(&[(1, 1, formula)]);
    assert!(
        scan_formula_capabilities(&workbook).is_supported(),
        "{formula}"
    );
    let result = calculate_workbook(&workbook, CalculationOptions::default());
    assert_number(&result, 1, expected, expected.abs() * relative_tolerance);
}
