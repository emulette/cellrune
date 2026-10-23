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
fn chi_square_quantiles_invert_both_tails() {
    for (formula, expected, tolerance) in [
        // Two degrees of freedom have the closed forms −2·ln(1 − p) and −2·ln(p).
        ("_xlfn.CHISQ.INV(0.6,2)", 1.832_581_463_748_31, 1e-13),
        ("CHISQ.INV(1E-10,2.9)", 2.000_000_000_1e-10, 1e-9),
        ("_xlfn.CHISQ.INV.RT(0.05,2)", 5.991_464_547_107_982, 1e-13),
        ("CHIINV(0.05,2)", 5.991_464_547_107_982, 1e-13),
        ("CHIINV(1E-200,2)", 921.034_037_197_618_3, 1e-13),
        // Microsoft's examples publish 3.283020286, 18.306973, and the Oracle 7.81472790325118.
        ("CHISQ.INV(0.93,1)", 3.283_020_286, 1e-9),
        ("CHISQ.INV.RT(0.050001,10)", 18.306_973, 1e-7),
        ("CHISQ.INV(0.95,3)", 7.814_727_903_251_18, 1e-13),
        ("CHIINV(\"0.05\",3)", 7.814_727_903_251_18, 1e-13),
    ] {
        assert_formula_number(formula, expected, tolerance);
    }
    for (formula, expected) in [
        ("CHISQ.INV(0,3)", 0.0),
        ("CHISQ.INV.RT(1,3)", 0.0),
        ("CHIINV(1,3)", 0.0),
    ] {
        assert_formula_number(formula, expected, 0.0);
    }
}

#[test]
fn chi_square_quantiles_reach_excels_largest_degrees_of_freedom() {
    // Excel 16.114 for Mac (excel-measure H and H2), up to its 10^10 limit.
    for (formula, expected) in [
        ("CHISQ.INV(0.5,1E6)", 999_999.333_333_412_3),
        ("CHISQ.INV(0.5,1E8)", 99_999_999.333_333_33),
        ("CHISQ.INV(0.5,1E9)", 999_999_999.333_333_4),
        ("CHISQ.INV(0.05,1E10)", 9_999_767_383.706_299),
        ("CHISQ.INV(0.5,1E10)", 9_999_999_999.333_334),
        ("CHISQ.INV(0.999999,1E9)", 1_000_212_593.994_487_5),
        ("CHISQ.INV.RT(0.5,1E9)", 999_999_999.333_333_4),
        ("CHISQ.INV.RT(0.95,1E10)", 9_999_767_383.706_299),
        ("CHISQ.INV.RT(0.05,5E9)", 5_000_164_486.499_716),
        ("CHISQ.INV.RT(0.05,1E8)", 100_023_262.880_047_01),
        ("CHISQ.INV.RT(1E-10,1E9)", 1_000_284_514.125_350_8),
        ("CHISQ.INV.RT(0.01,1E9)", 1_000_104_040.380_967_1),
        ("CHIINV(0.5,1E9)", 999_999_999.333_333_4),
        ("CHISQ.INV(1E-300,1E9)", 998_344_117.685_295_8),
    ] {
        assert_formula_number(formula, expected, 1e-14);
    }
}

#[test]
fn chi_square_test_sums_the_statistic_over_the_contingency_table() {
    for (formula, expected) in [
        // Microsoft's example: χ² = 16.16957 with two degrees of freedom gives 0.0003082.
        (
            "_xlfn.CHISQ.TEST({58,35;11,25;10,23},{45.35,47.65;17.56,18.44;16.09,16.91})",
            0.000_308_192_017_008_309_36,
        ),
        (
            "CHITEST({58,35;11,25;10,23},{45.35,47.65;17.56,18.44;16.09,16.91})",
            0.000_308_192_017_008_309_36,
        ),
        // A single row has c − 1 degrees of freedom: χ² = 1 with two gives e^(−1/2).
        ("CHISQ.TEST({1,2,3},{2,2,2})", 0.606_530_659_712_633_4),
        ("CHISQ.TEST({1;2;3},{2;2;2})", 0.606_530_659_712_633_4),
    ] {
        assert_formula_number(formula, expected, 1e-12);
    }
}

#[test]
fn chi_square_test_follows_excel_for_shapes_text_and_expected_values() {
    // Excel 16.114 for Mac: the counts must match but not the shapes, the degrees of freedom come
    // from the first argument's shape, and a pair with text drops out of the sum.
    for (formula, expected) in [
        ("CHISQ.TEST({1,2;3,4},{1,2,3,4})", 1.0),
        ("CHISQ.TEST({1,\"a\"},{1,2})", 1.0),
        ("CHISQ.TEST({1,2;3,4},{2,1;4,3})", 0.148_914_673_178_765_7),
        ("CHISQ.TEST({1,2,3,4},{2,1,4,3})", 0.555_291_895_481_792_4),
        ("CHISQ.TEST({1;2;3;4},{2;1;4;3})", 0.555_291_895_481_792_4),
        ("CHISQ.TEST({1,2;3,4},{2,1,4,3})", 0.148_914_673_178_765_7),
        ("CHISQ.TEST({1,2,3,4},{2,1;4,3})", 0.555_291_895_481_792_4),
        // The text cell still counts toward the 3 − 1 degrees of freedom.
        ("CHISQ.TEST({1,\"a\",3},{2,2,2})", 0.606_530_659_712_633_4),
    ] {
        assert_formula_number(formula, expected, 1e-14);
    }
    // The same rules for ranges, with text in either argument.
    for (formula, expected) in [
        ("CHISQ.TEST(A10:C10,A11:C11)", 0.606_530_659_712_633_4),
        ("CHISQ.TEST(A12:B13,A14:B15)", 0.083_264_516_663_550_45),
        ("CHISQ.TEST(A11:C11,A16:C16)", 1.0),
        ("CHISQ.TEST(A10:B10,A11:B11)", 0.479_500_122_186_953_5),
    ] {
        let workbook = workbook_with_formulas(&[
            (1, 1, formula),
            (10, 1, "1"),
            (10, 2, "\"a\""),
            (10, 3, "3"),
            (11, 1, "2"),
            (11, 2, "2"),
            (11, 3, "2"),
            (12, 1, "1"),
            (12, 2, "\"a\""),
            (13, 1, "3"),
            (13, 2, "4"),
            (14, 1, "2"),
            (14, 2, "2"),
            (15, 1, "2"),
            (15, 2, "2"),
            (16, 1, "2"),
            (16, 2, "\"b\""),
            (16, 3, "2"),
        ]);
        let result = calculate_workbook(&workbook, CalculationOptions::default());
        assert_number(&result, 1, expected, expected * 1e-14);
    }
}

#[test]
fn chi_square_test_rejects_mismatched_tables_and_non_positive_expectations() {
    for (formula, expected) in [
        ("CHISQ.TEST({1,2},{1,2,3})", ExcelError::NotAvailable),
        ("CHISQ.TEST({1,2,3},{2,1})", ExcelError::NotAvailable),
        ("CHISQ.TEST({1},{2})", ExcelError::NotAvailable),
        ("CHITEST(4,5)", ExcelError::NotAvailable),
        ("CHISQ.TEST({1,2},{#DIV/0!,2,3})", ExcelError::NotAvailable),
        // Excel 16.114 for Mac for an expected 0 and a negative expected value.
        ("CHISQ.TEST({1,2},{0,2})", ExcelError::DivisionByZero),
        ("CHISQ.TEST({1,2},{-1,2})", ExcelError::Number),
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
    // Excel was not measured on these.
    for formula in [
        "CHISQ.TEST({1,2},{1,TRUE})",
        "CHISQ.TEST({1,#N/A},{1,2})",
        "CHISQ.TEST({\"a\",2},{0,2})",
        "CHISQ.TEST({1,2},{0,-1})",
        "CHISQ.TEST(A10:B10,A11:B11)",
    ] {
        let result = calculate_workbook(
            &workbook_with_formulas(&[(1, 1, formula), (10, 1, "1"), (11, 1, "2"), (11, 2, "2")]),
            CalculationOptions::default(),
        );
        assert_issue(&result, 1, CalculationIssueCode::UnsupportedExpression);
    }
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
    ] {
        assert_formula_number(formula, expected, 1e-13);
    }
    assert_formula_number("WEIBULL.DIST(0,0.5,1,TRUE)", 0.0, 0.0);
    // Excel 16.0 returns a zero density at the origin for every shape.
    for formula in [
        "WEIBULL.DIST(0,2,1,FALSE)",
        "WEIBULL.DIST(0,1,4,FALSE)",
        "WEIBULL.DIST(0,0.5,1,FALSE)",
    ] {
        assert_formula_number(formula, 0.0, 0.0);
    }
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
        ("CHISQ.INV(-0.1,3)", ExcelError::Number),
        ("CHISQ.INV(1.1,3)", ExcelError::Number),
        ("CHISQ.INV(1,3)", ExcelError::Number),
        ("CHISQ.INV(0.5,0.9)", ExcelError::Number),
        ("CHISQ.INV(0.5,10000000001)", ExcelError::Number),
        ("CHISQ.INV.RT(0,3)", ExcelError::Number),
        ("CHISQ.INV.RT(2,3)", ExcelError::Number),
        ("CHIINV(\"2\",3)", ExcelError::Number),
        ("CHIINV(-1,3)", ExcelError::Number),
        ("CHIINV(\"x\",3)", ExcelError::Value),
        ("CHISQ.INV.RT(0.5,#N/A)", ExcelError::NotAvailable),
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
        "CHISQ.INV(0.95,3)",
        "CHISQ.INV.RT(0.05,3)",
        "CHISQ.TEST({1,2,3},{2,2,2})",
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
