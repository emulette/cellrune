use cellrune::{
    CalculationCellResult, CalculationIssueCode, CalculationLimits, CalculationOptions, CellValue,
    ExcelError, calculate_workbook, scan_formula_capabilities,
};

use super::support::{assert_issue, assert_number, cell_id, workbook_with_formulas};

#[test]
fn gamma_family_matches_the_excel_oracle() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "GAMMA(2.5)"),
        (1, 2, "GAMMA(\"2\")"),
        (1, 3, "GAMMA(-3.75)"),
        (1, 4, "GAMMA.DIST(2,3,1.5,TRUE)"),
        (1, 5, "GAMMA.DIST(\"2\",3,1.5,TRUE)"),
        (1, 6, "GAMMA.DIST(2,3,1.5,FALSE)"),
        (1, 7, "GAMMA.DIST(0,1,2,FALSE)"),
        (1, 8, "GAMMA.DIST(0,2,2,FALSE)"),
        (1, 9, "GAMMA.DIST(0,3,1.5,TRUE)"),
        (1, 10, "GAMMA.INV(0.7,3,1.5)"),
        (1, 11, "GAMMA.INV(0,3,1.5)"),
        (1, 12, "GAMMALN(4.5)"),
        (1, 13, "GAMMALN(\"2\")"),
        (1, 14, "GAMMALN.PRECISE(4.5)"),
        (1, 15, "GAMMADIST(2,3,1.5,TRUE)"),
        (1, 16, "GAMMAINV(0.7,3,1.5)"),
        // Aliases share the kernel, so their results are bit-identical.
        (1, 17, "GAMMADIST(2,3,1.5,TRUE)=GAMMA.DIST(2,3,1.5,TRUE)"),
        (1, 18, "GAMMAINV(0.7,3,1.5)=GAMMA.INV(0.7,3,1.5)"),
        (1, 19, "GAMMALN(4.5)=GAMMALN.PRECISE(4.5)"),
        // Extreme quantile tails; reference: mpmath 1.4.1, dps=30.
        (1, 20, "GAMMA.INV(0.000000000001,3,1.5)"),
        (1, 21, "GAMMA.INV(0.999999999999,3,1.5)"),
    ]);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    // Oracle-pinned values (both Excel profiles agree) within ~1e-9 relative.
    for (column, expected, tolerance) in [
        (1, 1.329340388179137, 1.4e-9),
        (2, 1.0, 1e-9),
        (3, 0.2678661288614166, 1e-9),
        (4, 0.15063144384932486, 1.6e-10),
        (5, 0.15063144384932486, 1.6e-10),
        (6, 0.15620571147598625, 1.6e-10),
        (7, 0.5, 1e-12),
        (8, 0.0, 0.0),
        (9, 0.0, 0.0),
        // Oracle prints 5.4233514987989846; same f64 as this shortest form.
        (10, 5.423_351_498_798_985, 5.5e-9),
        (11, 0.0, 0.0),
        (12, 2.4537365708424423, 2.5e-9),
        (13, 0.0, 1e-12),
        (14, 2.4537365708424423, 2.5e-9),
        (15, 0.15063144384932486, 1.6e-10),
        (16, 5.423_351_498_798_985, 5.5e-9),
        (20, 0.00027258047193956167, 2.8e-13),
        // Upper-tail quantiles are ULP-limited when inverting through the
        // lower CDF: half an ULP of 1 over pdf(x*) allows ~6e-5 of x-noise.
        (21, 51.07859647422942, 1.8e-4),
    ] {
        assert_number(&calculation, column, expected, tolerance);
    }
    for column in [17, 18, 19] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Logical(true))),
            "alias must be bit-identical to its canonical name in column {column}",
        );
    }
}

#[test]
fn gamma_family_rejects_domain_violations_with_excel_errors() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "GAMMA(0)"),
        (1, 2, "GAMMA(-2)"),
        (1, 3, "GAMMA(180)"),
        (1, 4, "GAMMA(\"abc\")"),
        (1, 5, "GAMMA.DIST(-1,3,1.5,TRUE)"),
        (1, 6, "GAMMA.DIST(2,0,1.5,TRUE)"),
        (1, 7, "GAMMA.DIST(2,3,0,TRUE)"),
        (1, 8, "GAMMA.DIST(0,0.5,1,FALSE)"),
        (1, 9, "GAMMA.DIST(2,3,1.5,\"abc\")"),
        (1, 10, "GAMMA.INV(\"2\",3,1.5)"),
        (1, 11, "GAMMA.INV(2,3,1.5)"),
        (1, 12, "GAMMA.INV(-0.1,3,1.5)"),
        (1, 13, "GAMMA.INV(1,3,1.5)"),
        (1, 14, "GAMMA.INV(0.7,0,1.5)"),
        (1, 15, "GAMMA.INV(0.7,3,0)"),
        (1, 16, "GAMMALN(0)"),
        (1, 17, "GAMMALN(-1.5)"),
        (1, 18, "GAMMALN.PRECISE(0)"),
        (1, 19, "GAMMADIST(-1,3,1.5,TRUE)"),
        (1, 20, "GAMMAINV(2,3,1.5)"),
    ]);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    for column in [
        1, 2, 3, 5, 6, 7, 8, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
    ] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Number
            ))),
            "unexpected domain result in column {column}",
        );
    }
    for column in [4, 9] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Value
            ))),
            "unexpected coercion result in column {column}",
        );
    }
}

#[test]
fn gamma_inverse_reports_na_when_the_quantile_search_cannot_converge() {
    // The Gamma(1e-8, 1) median underflows below the smallest f64, so the
    // solver cannot converge; Microsoft documents #N/A for this outcome.
    let workbook = workbook_with_formulas(&[(1, 1, "GAMMA.INV(0.5,0.00000001,1)")]);
    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());
    assert_eq!(
        calculation.cell(cell_id(1)),
        Some(&CalculationCellResult::Value(CellValue::Error(
            ExcelError::NotAvailable
        ))),
    );
}

#[test]
fn gamma_inverse_respects_the_function_iteration_budget() {
    let calculation = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "GAMMA.INV(0.7,3,1.5)")]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(10)
                .expect("positive inverse-solver work limit"),
        ),
    );
    // The budgeted solver must fail closed: no partial quantile is installed.
    assert_issue(&calculation, 1, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn gamma_cumulative_distribution_respects_the_function_iteration_budget() {
    let calculation = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "GAMMA.DIST(2,3,1.5,TRUE)")]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(2)
                .expect("positive series work limit"),
        ),
    );
    assert_issue(&calculation, 1, CalculationIssueCode::ResourceLimitExceeded);
}
