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
fn beta_family_matches_the_excel_oracle() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "BETA.DIST(0.6,8,10,TRUE,0,1)"),
        (1, 2, "BETA.INV(0.6,8,10,0,1)"),
        (1, 3, "BETADIST(0.5,2,3,0,1)"),
        (1, 4, "BETAINV(0.5,2,3,0,1)"),
        // Omitted bounds must be bit-identical to the explicit [0, 1] pair.
        (
            1,
            5,
            "BETA.DIST(0.6,8,10,TRUE)=BETA.DIST(0.6,8,10,TRUE,0,1)",
        ),
        (1, 6, "BETA.INV(0.6,8,10)=BETA.INV(0.6,8,10,0,1)"),
        (1, 7, "BETADIST(0.5,2,3)=BETADIST(0.5,2,3,0,1)"),
        (1, 8, "BETAINV(0.5,2,3)=BETAINV(0.5,2,3,0,1)"),
        // Legacy names share the canonical kernels, so results are
        // bit-identical.
        (1, 9, "BETADIST(0.5,2,3,0,1)=BETA.DIST(0.5,2,3,TRUE,0,1)"),
        (1, 10, "BETAINV(0.5,2,3,0,1)=BETA.INV(0.5,2,3,0,1)"),
        // Custom interval [1, 5]: u = (3 − 1)/4 = 0.5; reference: mpmath
        // 1.4.1, dps = 30 — I_0.5(2,3) = 0.6875 and pdf(0.5;2,3)/4 = 0.375
        // (the density carries the documented 1/(B − A) Jacobian).
        (1, 11, "BETA.DIST(3,2,3,TRUE,1,5)"),
        (1, 12, "BETA.DIST(3,2,3,FALSE,1,5)"),
        (1, 13, "BETA.INV(0.6875,2,3,1,5)"),
        // p = 1 is inside BETA.INV's documented domain; the quantile is B.
        (1, 14, "BETA.INV(1,2,3,1,5)"),
    ]);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    // Oracle-pinned values (both Excel profiles agree) within ~1e-9 relative.
    for (column, expected, tolerance) in [
        // Oracle prints 0.90810074582876155 and 0.38572756813238951; each is
        // the same f64 as its shortest form below.
        (1, 0.908_100_745_828_761_5, 1e-12),
        (2, 0.4725265938468981, 4.8e-10),
        (3, 0.6875, 1e-14),
        (4, 0.385_727_568_132_389_5, 3.9e-10),
        (11, 0.6875, 1e-14),
        (12, 0.375, 1e-14),
        (13, 3.0, 3.0e-9),
        (14, 5.0, 0.0),
    ] {
        assert_number(&calculation, column, expected, tolerance);
    }
    for column in [5, 6, 7, 8, 9, 10] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Logical(true))),
            "spelling must be bit-identical to its canonical form in column {column}",
        );
    }
}

#[test]
fn beta_family_rejects_domain_violations_with_excel_errors() {
    let workbook = workbook_with_formulas(&[
        // "2" coerces to 2, which then falls outside [0, 1]: the oracle pins
        // these coercion probes to #NUM!, not #VALUE!.
        (1, 1, "BETA.DIST(\"2\",8,10,TRUE,0,1)"),
        (1, 2, "BETA.DIST(-1,8,10,TRUE,0,1)"),
        (1, 3, "BETA.DIST(0.5,0,10,TRUE,0,1)"),
        (1, 4, "BETA.DIST(0.5,8,-1,TRUE,0,1)"),
        (1, 5, "BETA.DIST(1,8,10,TRUE,1,1)"),
        (1, 6, "BETA.DIST(0.5,8,10,TRUE,2,4)"),
        (1, 7, "BETA.INV(\"2\",8,10,0,1)"),
        (1, 8, "BETA.INV(2,8,10,0,1)"),
        // p = 0 is documented #NUM! for BETA.INV, unlike GAMMA.INV.
        (1, 9, "BETA.INV(0,8,10,0,1)"),
        (1, 10, "BETA.INV(-0.5,8,10,0,1)"),
        (1, 11, "BETA.INV(0.5,0,10,0,1)"),
        (1, 12, "BETA.INV(0.5,8,0,0,1)"),
        (1, 13, "BETA.INV(0.5,8,10,1,1)"),
        (1, 14, "BETADIST(\"2\",2,3,0,1)"),
        (1, 15, "BETADIST(-1,2,3,0,1)"),
        (1, 16, "BETAINV(\"2\",2,3,0,1)"),
        (1, 17, "BETAINV(2,2,3,0,1)"),
        (1, 18, "BETA.DIST(0.5,8,10,\"abc\",0,1)"),
        (1, 19, "BETA.DIST(\"abc\",8,10,TRUE,0,1)"),
        (1, 20, "BETA.INV(\"abc\",8,10,0,1)"),
        // A reversed interval A > B is invalid for every beta-family name —
        // including BETA.INV, whose quantile would otherwise be a point its
        // own distribution rejects, and even for x inside [B, A] or p = 1.
        (1, 21, "BETA.INV(0.5,2,3,5,1)"),
        (1, 22, "BETA.DIST(4,2,3,TRUE,5,1)"),
        (1, 23, "BETADIST(4,2,3,5,1)"),
        (1, 24, "BETAINV(0.5,2,3,5,1)"),
        (1, 25, "BETA.INV(1,2,3,5,1)"),
    ]);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    for column in [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 21, 22, 23, 24, 25,
    ] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Number
            ))),
            "unexpected domain result in column {column}",
        );
    }
    for column in [18, 19, 20] {
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
fn beta_density_endpoints_mirror_the_gamma_origin_contract() {
    let workbook = workbook_with_formulas(&[
        // Lower endpoint u = 0: pole below alpha = 1, exact limit at 1,
        // zero above; the upper endpoint mirrors it in beta.
        (1, 1, "BETA.DIST(0,0.5,3,FALSE,0,1)"),
        (1, 2, "BETA.DIST(0,1,3,FALSE,0,1)"),
        (1, 3, "BETA.DIST(0,2,3,FALSE,0,1)"),
        (1, 4, "BETA.DIST(1,3,0.5,FALSE,0,1)"),
        (1, 5, "BETA.DIST(1,3,1,FALSE,0,1)"),
        (1, 6, "BETA.DIST(1,3,2,FALSE,0,1)"),
        // The endpoint limits scale by the same 1/(B − A) Jacobian.
        (1, 7, "BETA.DIST(1,1,3,FALSE,1,3)"),
        // The cumulative form stays exact at both endpoints.
        (1, 8, "BETA.DIST(0,2,3,TRUE,0,1)"),
        (1, 9, "BETA.DIST(1,2,3,TRUE,0,1)"),
    ]);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    for column in [1, 4] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Number
            ))),
            "unexpected endpoint pole result in column {column}",
        );
    }
    for (column, expected) in [
        (2, 3.0),
        (3, 0.0),
        (5, 3.0),
        (6, 0.0),
        (7, 1.5),
        (8, 0.0),
        (9, 1.0),
    ] {
        assert_number(&calculation, column, expected, 0.0);
    }
}

#[test]
fn beta_inverse_reports_na_when_the_quantile_search_cannot_converge() {
    // Beta(1e-8, 1e-8) concentrates half its mass at each endpoint: the
    // p = 0.25 quantile satisfies ln u* ≈ −5e7, far below the smallest
    // positive double (the CDF is already ≈ 0.4999963 at 5e-324), so no
    // representable bracket can meet the residual tolerance and Microsoft
    // documents #N/A for the failed search.
    let workbook = workbook_with_formulas(&[(1, 1, "BETA.INV(0.25,0.00000001,0.00000001)")]);
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
fn beta_inverse_respects_the_function_iteration_budget() {
    let calculation = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "BETA.INV(0.6,8,10,0,1)")]),
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

#[test]
fn binomial_family_matches_the_excel_oracle() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "BINOM.DIST(3,10,0.4,FALSE)"),
        (1, 2, "BINOM.DIST(\"2\",10,0.4,FALSE)"),
        (1, 3, "BINOMDIST(3,10,0.4,FALSE)"),
        (1, 4, "BINOMDIST(\"2\",10,0.4,FALSE)"),
        (1, 5, "BINOM.DIST.RANGE(10,0.4,3,6)"),
        (1, 6, "BINOM.INV(10,0.4,0.6)"),
        (1, 7, "BINOM.INV(\"2\",0.4,0.6)"),
        (1, 8, "CRITBINOM(10,0.4,0.6)"),
        (1, 9, "CRITBINOM(\"2\",0.4,0.6)"),
        (1, 10, "NEGBINOM.DIST(6,4,0.4,TRUE)"),
        (1, 11, "NEGBINOM.DIST(\"2\",4,0.4,TRUE)"),
        (1, 12, "NEGBINOMDIST(6,4,0.4)"),
        (1, 13, "NEGBINOMDIST(\"2\",4,0.4)"),
        (1, 14, "BINOM.DIST(3,10,0.4,TRUE)"),
        (1, 15, "BINOM.DIST.RANGE(10,0.4,4)"),
        // Aliases and the legacy adapter share kernels bit for bit.
        (
            1,
            16,
            "BINOMDIST(3,10,0.4,FALSE)=BINOM.DIST(3,10,0.4,FALSE)",
        ),
        (
            1,
            17,
            "BINOMDIST(\"2\",10,0.4,FALSE)=BINOM.DIST(\"2\",10,0.4,FALSE)",
        ),
        (1, 18, "CRITBINOM(10,0.4,0.6)=BINOM.INV(10,0.4,0.6)"),
        (1, 19, "NEGBINOMDIST(6,4,0.4)=NEGBINOM.DIST(6,4,0.4,FALSE)"),
        // Excel truncates the counts before the domain rules apply.
        (
            1,
            20,
            "BINOM.DIST(3.9,10.9,0.4,FALSE)=BINOM.DIST(3,10,0.4,FALSE)",
        ),
        (
            1,
            21,
            "BINOM.DIST.RANGE(10.9,0.4,3.9,6.9)=BINOM.DIST.RANGE(10,0.4,3,6)",
        ),
        (
            1,
            22,
            "NEGBINOM.DIST(6.9,4.9,0.4,TRUE)=NEGBINOM.DIST(6,4,0.4,TRUE)",
        ),
        (1, 23, "BINOM.INV(10.9,0.4,0.6)=BINOM.INV(10,0.4,0.6)"),
    ]);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    // Oracle-pinned values (both Excel profiles agree) within ~1e-9 relative.
    for (column, expected, tolerance) in [
        (1, 0.21499084800000007, 2.2e-10),
        (2, 0.12093235200000005, 1.3e-10),
        (3, 0.21499084800000007, 2.2e-10),
        (4, 0.12093235200000005, 1.3e-10),
        // Oracle prints 0.77794836480000018, 0.61771939840000012 and
        // 0.092159999999999992; the same f64s as these shortest forms.
        (5, 0.777_948_364_800_000_2, 7.8e-10),
        (6, 4.0, 0.0),
        (7, 1.0, 0.0),
        (8, 4.0, 0.0),
        (9, 1.0, 0.0),
        (10, 0.617_719_398_400_000_1, 6.2e-10),
        (11, 0.17920000000000005, 1.8e-10),
        (12, 0.10032906240000003, 1.1e-10),
        (13, 0.092_159_999_999_999_99, 1e-10),
        // reference: mpmath 1.4.1, mp.dps = 30.
        (14, 0.38228060159999994, 3.9e-10),
        (15, 0.250822656, 2.6e-10),
    ] {
        assert_number(&calculation, column, expected, tolerance);
    }
    for column in 16..=23 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Logical(true))),
            "expected a bit-identical pair in column {column}",
        );
    }
}

#[test]
fn binomial_family_rejects_domain_violations_with_excel_errors() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "BINOM.DIST(-1,10,0.4,FALSE)"),
        (1, 2, "BINOMDIST(-1,10,0.4,FALSE)"),
        (1, 3, "BINOM.DIST(11,10,0.4,FALSE)"),
        (1, 4, "BINOM.DIST(3,10,-0.1,FALSE)"),
        (1, 5, "BINOM.DIST(3,10,1.1,FALSE)"),
        // Coerced trials 2 sits below number_s 3 — the oracle's #NUM! pin.
        (1, 6, "BINOM.DIST.RANGE(\"2\",0.4,3,6)"),
        (1, 7, "BINOM.DIST.RANGE(-1,0.4,3,6)"),
        (1, 8, "BINOM.DIST.RANGE(10,0.4,-1,6)"),
        (1, 9, "BINOM.DIST.RANGE(10,0.4,3,2)"),
        (1, 10, "BINOM.DIST.RANGE(10,0.4,3,11)"),
        (1, 11, "BINOM.DIST.RANGE(10,1.5,3,6)"),
        (1, 12, "BINOM.INV(10,0.4,2)"),
        (1, 13, "CRITBINOM(10,0.4,2)"),
        (1, 14, "BINOM.INV(10,0.4,-0.1)"),
        (1, 15, "BINOM.INV(-1,0.4,0.6)"),
        (1, 16, "BINOM.INV(10,-0.1,0.6)"),
        (1, 17, "BINOM.INV(10,1.1,0.6)"),
        (1, 18, "NEGBINOM.DIST(-1,4,0.4,TRUE)"),
        (1, 19, "NEGBINOM.DIST(6,0,0.4,TRUE)"),
        (1, 20, "NEGBINOM.DIST(6,4,-0.1,TRUE)"),
        (1, 21, "NEGBINOM.DIST(6,4,1.1,TRUE)"),
        (1, 22, "NEGBINOMDIST(-1,4,0.4)"),
        (1, 23, "NEGBINOMDIST(6,0.9,0.4)"),
        (1, 24, "BINOM.DIST(\"abc\",10,0.4,FALSE)"),
        (1, 25, "BINOM.DIST(3,10,0.4,\"abc\")"),
        (1, 26, "NEGBINOMDIST(\"abc\",4,0.4)"),
    ]);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    for column in 1..=23 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Number
            ))),
            "unexpected domain result in column {column}",
        );
    }
    for column in [24, 25, 26] {
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
fn binomial_inverse_pins_the_interoperable_unit_interval_boundaries() {
    // Microsoft's worksheet page claims #NUM! at the alpha and probability_s
    // boundaries; its own VBA documentation, ODF OpenFormula 1.3 §6.18.19 and
    // interoperating engines accept them, and CellRune pins that policy.
    let workbook = workbook_with_formulas(&[
        (1, 1, "BINOM.INV(10,0.4,0)"),
        (1, 2, "BINOM.INV(10,0.4,1)"),
        (1, 3, "BINOM.INV(10,0,0.6)"),
        (1, 4, "BINOM.INV(10,1,0.6)"),
        (1, 5, "BINOM.INV(10,1,0)"),
        (1, 6, "BINOM.INV(0,0.4,0.6)"),
        (1, 7, "BINOM.DIST(0,10,0,FALSE)"),
        (1, 8, "BINOM.DIST(10,10,1,FALSE)"),
        (1, 9, "BINOM.DIST(3,10,0,FALSE)"),
        (1, 10, "BINOM.DIST(3,10,0,TRUE)"),
        (1, 11, "BINOM.DIST(3,10,1,TRUE)"),
        (1, 12, "NEGBINOM.DIST(0,4,1,FALSE)"),
        (1, 13, "NEGBINOM.DIST(3,4,0,TRUE)"),
    ]);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    for (column, expected) in [
        (1, 0.0),
        (2, 10.0),
        (3, 0.0),
        (4, 10.0),
        (5, 0.0),
        (6, 0.0),
        (7, 1.0),
        (8, 1.0),
        (9, 0.0),
        (10, 1.0),
        (11, 0.0),
        (12, 1.0),
        (13, 0.0),
    ] {
        assert_number(&calculation, column, expected, 0.0);
    }
}

#[test]
fn binomial_cumulative_distribution_respects_the_function_iteration_budget() {
    let calculation = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "BINOM.DIST(500000,1000000,0.5,TRUE)")]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(2)
                .expect("positive incomplete-beta work limit"),
        ),
    );
    // The budgeted continued fraction must fail closed: no partial CDF is installed.
    assert_issue(&calculation, 1, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn binomial_inverse_large_support_completes_with_the_default_budget() {
    let calculation = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "BINOM.INV(200000,0.5,0.6)")]),
        CalculationOptions::default(),
    );
    assert_number(&calculation, 1, 100_057.0, 0.0);
}

#[test]
fn discrete_cdf_endpoints_bypass_the_iteration_budget() {
    let calculation = calculate_workbook(
        &workbook_with_formulas(&[
            (1, 1, "BINOM.DIST(1000000,1000000,0.5,TRUE)"),
            (1, 2, "BINOM.DIST(3,1000000,0,TRUE)"),
            (1, 3, "BINOM.DIST.RANGE(1E20,1,1E20)"),
            (1, 4, "NEGBINOM.DIST(1E20,4,0,TRUE)"),
            (1, 5, "NEGBINOM.DIST(1000000,4,1,TRUE)"),
            (1, 6, "HYPGEOM.DIST(1000,1000,1000,2000,TRUE)"),
            (1, 7, "HYPGEOM.DIST(1E20,1E20,1E20,1E20,TRUE)"),
        ]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(1)
                .expect("positive endpoint work limit"),
        ),
    );
    for (column, expected) in [
        (1, 1.0),
        (2, 1.0),
        (3, 1.0),
        (4, 0.0),
        (5, 1.0),
        (6, 1.0),
        (7, 1.0),
    ] {
        assert_number(&calculation, column, expected, 0.0);
    }
}

#[test]
fn binomial_inverse_respects_the_function_iteration_budget() {
    let calculation = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "BINOM.INV(1000000,0.5,0.6)")]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(10)
                .expect("positive search work limit"),
        ),
    );
    assert_issue(&calculation, 1, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn negative_binomial_cumulative_respects_the_function_iteration_budget() {
    let calculation = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "NEGBINOM.DIST(6,4,0.4,TRUE)")]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(1)
                .expect("positive incomplete-beta work limit"),
        ),
    );
    assert_issue(&calculation, 1, CalculationIssueCode::ResourceLimitExceeded);
}

#[test]
fn hypergeometric_family_matches_the_excel_oracle() {
    let workbook = workbook_with_formulas(&[
        (1, 1, "HYPGEOM.DIST(1,4,8,20,TRUE)"),
        (1, 2, "HYPGEOM.DIST(\"2\",4,8,20,TRUE)"),
        (1, 3, "HYPGEOM.DIST(-1,4,8,20,TRUE)"),
        (1, 4, "HYPGEOMDIST(1,4,8,20)"),
        (1, 5, "HYPGEOMDIST(\"2\",4,8,20)"),
        (1, 6, "HYPGEOMDIST(-1,4,8,20)"),
        // Every argument is truncated to an integer before the mass is taken.
        (1, 7, "HYPGEOM.DIST(1.9,4,8,20,FALSE)"),
        // The legacy four-argument name adapts onto the same kernel, so its
        // results are bit-identical to the mass branch of the modern name.
        (1, 8, "HYPGEOMDIST(1,4,8,20)=HYPGEOM.DIST(1,4,8,20,FALSE)"),
        (1, 9, "HYPGEOMDIST(2,4,8,20)=HYPGEOM.DIST(2,4,8,20,FALSE)"),
        (1, 10, "HYPGEOMDIST(4,6,8,10)=HYPGEOM.DIST(4,6,8,10,FALSE)"),
    ]);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    // Oracle-pinned values (both Excel profiles agree) within ~1e-9 relative.
    // The oracle prints 0.84685242518059867, 0.36326109391124861 and
    // 0.38142414860681101; each is the same f64 as the shortest form below.
    for (column, expected, tolerance) in [
        (1, 0.465_428_276_573_787_27, 4.7e-10),
        (2, 0.846_852_425_180_598_7, 8.5e-10),
        (4, 0.363_261_093_911_248_6, 3.7e-10),
        (5, 0.381_424_148_606_811, 3.9e-10),
        (7, 0.363_261_093_911_248_6, 3.7e-10),
    ] {
        assert_number(&calculation, column, expected, tolerance);
    }
    for column in [3, 6] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Number
            ))),
            "sample_s below the support must be #NUM! in column {column}",
        );
    }
    for column in [8, 9, 10] {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Logical(true))),
            "HYPGEOMDIST must be bit-identical to the mass branch in column {column}",
        );
    }
}

#[test]
fn hypergeometric_family_rejects_domain_violations_with_excel_errors() {
    let workbook = workbook_with_formulas(&[
        // sample_s above the lesser of number_sample and population_s.
        (1, 1, "HYPGEOM.DIST(5,4,8,20,TRUE)"),
        (1, 2, "HYPGEOM.DIST(9,20,8,20,FALSE)"),
        // sample_s below max(0, number_sample - number_pop + population_s).
        (1, 3, "HYPGEOM.DIST(3,6,8,10,FALSE)"),
        // number_sample outside (0, number_pop].
        (1, 4, "HYPGEOM.DIST(1,0,8,20,TRUE)"),
        (1, 5, "HYPGEOM.DIST(1,21,8,20,TRUE)"),
        // population_s outside (0, number_pop].
        (1, 6, "HYPGEOM.DIST(1,4,0,20,TRUE)"),
        (1, 7, "HYPGEOM.DIST(1,4,21,20,TRUE)"),
        // number_pop at or below zero.
        (1, 8, "HYPGEOM.DIST(1,4,8,0,TRUE)"),
        (1, 9, "HYPGEOM.DIST(1,4,8,-20,TRUE)"),
        // The legacy name enforces the identical conditions.
        (1, 10, "HYPGEOMDIST(5,4,8,20)"),
        (1, 11, "HYPGEOMDIST(3,6,8,10)"),
        (1, 12, "HYPGEOMDIST(1,4,8,0)"),
        // Non-numeric arguments are coercion failures, not domain failures.
        (1, 13, "HYPGEOM.DIST(\"abc\",4,8,20,TRUE)"),
        (1, 14, "HYPGEOM.DIST(1,4,8,20,\"abc\")"),
        (1, 15, "HYPGEOMDIST(\"abc\",4,8,20)"),
    ]);

    assert!(scan_formula_capabilities(&workbook).is_supported());
    let calculation = calculate_workbook(&workbook, CalculationOptions::default());

    for column in 1..=12 {
        assert_eq!(
            calculation.cell(cell_id(column)),
            Some(&CalculationCellResult::Value(CellValue::Error(
                ExcelError::Number
            ))),
            "unexpected domain result in column {column}",
        );
    }
    for column in [13, 14, 15] {
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
fn hypergeometric_cumulative_distribution_respects_the_function_iteration_budget() {
    let calculation = calculate_workbook(
        &workbook_with_formulas(&[(1, 1, "HYPGEOM.DIST(400,1000,40000,100000,TRUE)")]),
        CalculationOptions::default().with_limits(
            CalculationLimits::default()
                .with_max_function_iterations(2)
                .expect("positive summation work limit"),
        ),
    );
    // The 401-term summation must fail closed: no partial sum is installed.
    assert_issue(&calculation, 1, CalculationIssueCode::ResourceLimitExceeded);
}
