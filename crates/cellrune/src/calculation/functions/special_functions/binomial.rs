use super::super::super::value::ErrorKind;
use super::LN_UNDERFLOW_LIMIT;
use super::bounded_probability;
use super::log_binomial::ln_binomial;

/// Probability mass of Binomial(trials, probability) at `successes`.
///
/// The mass is evaluated per point in log space through [`ln_binomial`], so
/// large combinations never form factorial products and no recurrence can
/// propagate an underflowed leading term across the support. The degenerate
/// probabilities keep Excel's conventions without evaluating ln(0): p = 0
/// concentrates all mass at zero successes and p = 1 at `trials`. A log-space
/// mass below the exp() underflow limit is reported as exactly zero.
pub(in crate::calculation::functions) fn binomial_pmf(
    trials: f64,
    successes: f64,
    probability: f64,
) -> Result<f64, ErrorKind> {
    validate_count_pair(trials, successes)?;
    validate_probability(probability)?;
    pmf_unchecked(trials, successes, probability)
}

/// Sum of Binomial(trials, probability) masses for successes in
/// [first, last]. `on_iteration` is charged before every added term, so the
/// engine budget and cancellation govern long summations; each term is an
/// independent log-space evaluation, keeping the sum stable for large trials.
pub(in crate::calculation::functions) fn binomial_pmf_sum(
    trials: f64,
    probability: f64,
    first: f64,
    last: f64,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    validate_count_pair(trials, first)?;
    validate_count_pair(trials, last)?;
    if first > last {
        return Err(ErrorKind::Num);
    }
    validate_probability(probability)?;
    let mut total = 0.0;
    for successes in (first as u64)..=(last as u64) {
        on_iteration()?;
        total += pmf_unchecked(trials, successes as f64, probability)?;
    }
    bounded_probability(total)
}

/// Smallest integer k with CDF(k) ≥ alpha for Binomial(trials, probability).
///
/// Monotone integer bisection over [0, trials]; no continuous solver runs.
/// The full-support mass CDF(trials) is taken as exactly 1 by definition,
/// which keeps targets near alpha = 1 out of floating-point rounding traps,
/// and the accepted k is verified explicitly against
/// CDF(k−1) < alpha ≤ CDF(k) before it is returned (k = 0 needs only the
/// upper half). Degenerate corners resolve without summation: alpha = 0 and
/// p = 0 pin k = 0, while alpha = 1 and p = 1 (with alpha > 0) pin
/// k = trials. Every bisection step and every summed CDF term charges
/// `on_iteration` first. Minimality is exact against this module's f64 CDF;
/// when alpha sits within the CDF's own log-space noise (~ULP of the lnΓ
/// magnitude per term) of an exact-arithmetic CDF value, the returned k can
/// differ from the infinite-precision minimal k by one step for each
/// noise-crossed CDF value — usually one, occasionally more at extreme alpha.
pub(in crate::calculation::functions) fn smallest_binomial_quantile(
    trials: f64,
    probability: f64,
    alpha: f64,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    validate_count_pair(trials, trials)?;
    validate_probability(probability)?;
    validate_probability(alpha)?;
    if probability == 0.0 || alpha == 0.0 {
        return Ok(0.0);
    }
    if probability == 1.0 || alpha == 1.0 {
        return Ok(trials);
    }
    let support_end = trials as u64;
    let mut low = 0_u64;
    let mut high = support_end;
    while low < high {
        on_iteration()?;
        let midpoint = low + (high - low) / 2;
        if lower_cdf(
            trials,
            midpoint,
            support_end,
            probability,
            &mut on_iteration,
        )? >= alpha
        {
            high = midpoint;
        } else {
            low = midpoint + 1;
        }
    }
    // Final explicit verification of the minimal-k contract. Both halves are
    // recomputed sums, not values remembered from the search.
    if lower_cdf(trials, low, support_end, probability, &mut on_iteration)? < alpha {
        return Err(ErrorKind::Num);
    }
    if low > 0 && lower_cdf(trials, low - 1, support_end, probability, &mut on_iteration)? >= alpha
    {
        return Err(ErrorKind::Num);
    }
    Ok(low as f64)
}

/// Probability mass of NegativeBinomial(successes, probability) at
/// `failures`: C(failures + successes − 1, failures) · p^s · (1−p)^f in log
/// space. p = 1 places all mass at zero failures; p = 0 has zero mass
/// everywhere because the final success can never arrive.
pub(in crate::calculation::functions) fn negative_binomial_pmf(
    failures: f64,
    successes: f64,
    probability: f64,
) -> Result<f64, ErrorKind> {
    validate_failure_success_pair(failures, successes)?;
    validate_probability(probability)?;
    negative_binomial_pmf_unchecked(failures, successes, probability)
}

/// Lower CDF of NegativeBinomial(successes, probability) at `failures`,
/// summed term by term with `on_iteration` charged before every term.
pub(in crate::calculation::functions) fn negative_binomial_cdf(
    failures: f64,
    successes: f64,
    probability: f64,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    validate_failure_success_pair(failures, successes)?;
    validate_probability(probability)?;
    let mut total = 0.0;
    for index in 0..=(failures as u64) {
        on_iteration()?;
        total += negative_binomial_pmf_unchecked(index as f64, successes, probability)?;
    }
    bounded_probability(total)
}

/// Lower binomial CDF used by the quantile search; the full support returns
/// exactly 1 by definition instead of a rounded floating-point sum.
fn lower_cdf(
    trials: f64,
    successes: u64,
    support_end: u64,
    probability: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    if successes >= support_end {
        return Ok(1.0);
    }
    let mut total = 0.0;
    for index in 0..=successes {
        on_iteration()?;
        total += pmf_unchecked(trials, index as f64, probability)?;
    }
    bounded_probability(total)
}

fn pmf_unchecked(trials: f64, successes: f64, probability: f64) -> Result<f64, ErrorKind> {
    if probability == 0.0 {
        return Ok(if successes == 0.0 { 1.0 } else { 0.0 });
    }
    if probability == 1.0 {
        return Ok(if successes == trials { 1.0 } else { 0.0 });
    }
    let log_mass = ln_binomial(trials, successes)?
        + successes * probability.ln()
        + (trials - successes) * (1.0 - probability).ln();
    if log_mass < LN_UNDERFLOW_LIMIT {
        return Ok(0.0);
    }
    bounded_probability(log_mass.exp())
}

fn negative_binomial_pmf_unchecked(
    failures: f64,
    successes: f64,
    probability: f64,
) -> Result<f64, ErrorKind> {
    if probability == 1.0 {
        return Ok(if failures == 0.0 { 1.0 } else { 0.0 });
    }
    if probability == 0.0 {
        return Ok(0.0);
    }
    let log_mass = ln_binomial(failures + successes - 1.0, failures)?
        + successes * probability.ln()
        + failures * (1.0 - probability).ln();
    if log_mass < LN_UNDERFLOW_LIMIT {
        return Ok(0.0);
    }
    bounded_probability(log_mass.exp())
}

fn validate_count_pair(total: f64, part: f64) -> Result<(), ErrorKind> {
    if total.is_finite()
        && total == total.trunc()
        && part == part.trunc()
        && part >= 0.0
        && part <= total
    {
        Ok(())
    } else {
        Err(ErrorKind::Num)
    }
}

fn validate_failure_success_pair(failures: f64, successes: f64) -> Result<(), ErrorKind> {
    if failures.is_finite()
        && failures == failures.trunc()
        && failures >= 0.0
        && successes.is_finite()
        && successes == successes.trunc()
        && successes >= 1.0
    {
        Ok(())
    } else {
        Err(ErrorKind::Num)
    }
}

fn validate_probability(probability: f64) -> Result<(), ErrorKind> {
    if (0.0..=1.0).contains(&probability) {
        Ok(())
    } else {
        Err(ErrorKind::Num)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        binomial_pmf, binomial_pmf_sum, negative_binomial_cdf, negative_binomial_pmf,
        smallest_binomial_quantile,
    };
    use crate::calculation::limits::CalculationLimitKind;
    use crate::calculation::value::ErrorKind;

    fn cdf(trials: f64, successes: f64, probability: f64) -> f64 {
        binomial_pmf_sum(trials, probability, 0.0, successes, || Ok(())).expect("valid domain")
    }

    // reference: mpmath 1.4.1, mp.dps = 30 — binomial(n, k)·p^k·(1−p)^(n−k).
    // The n = 100000 rows exercise pure log-space evaluation: the leading
    // term (1−p)^n underflows f64, so any recurrence seeded there dies.
    // Large-n tolerances are set by the ULP of lnΓ near 1e6 (≈ 4e-10 of
    // absolute log-space noise), not by machine precision.
    const PMF_REFERENCES: [(f64, f64, f64, f64, f64); 8] = [
        (10.0, 3.0, 0.4, 0.21499084799999998, 1e-12),
        (10.0, 0.0, 0.4, 0.006046617599999997, 1e-12),
        (10.0, 10.0, 0.4, 0.00010485760000000006, 1e-12),
        (1000.0, 500.0, 0.5, 0.0252250181783608, 1e-11),
        (100000.0, 40000.0, 0.4, 0.002575154551265565, 5e-9),
        (100000.0, 39500.0, 0.4, 1.401504909902546e-5, 5e-9),
        (100000.0, 41000.0, 0.4, 2.4305003234107736e-12, 5e-9),
        (100000.0, 38000.0, 0.4, 1.0246962963389867e-39, 5e-9),
    ];

    // reference: mpmath 1.4.1, mp.dps = 30 — partial sums of the same mass.
    const CDF_REFERENCES: [(f64, f64, f64, f64, f64); 6] = [
        (10.0, 3.0, 0.4, 0.38228060159999994, 1e-12),
        (10.0, 6.0, 0.4, 0.9452381183999999, 1e-12),
        (1000.0, 500.0, 0.5, 0.5126125090891804, 1e-11),
        (100000.0, 40000.0, 0.4, 0.50137341504853, 5e-9),
        (100000.0, 39690.0, 0.4, 0.022833224629698272, 5e-9),
        (100000.0, 40310.0, 0.4, 0.9774449076136642, 5e-9),
    ];

    // reference: mpmath 1.4.1, mp.dps = 30 — sums over [first, last].
    const RANGE_REFERENCES: [(f64, f64, f64, f64, f64, f64); 3] = [
        (10.0, 0.4, 3.0, 6.0, 0.7779483648000001, 1e-12),
        (10.0, 0.4, 4.0, 4.0, 0.250822656, 1e-12),
        (100000.0, 0.4, 39800.0, 40200.0, 0.8044115403910748, 5e-9),
    ];

    // reference: mpmath 1.4.1, mp.dps = 30 — C(f+s−1, f)·p^s·(1−p)^f and its
    // partial sums, including large failure counts.
    const NEGATIVE_BINOMIAL_PMF_REFERENCES: [(f64, f64, f64, f64, f64); 5] = [
        (6.0, 4.0, 0.4, 0.1003290624, 1e-12),
        (2.0, 4.0, 0.4, 0.09216000000000002, 1e-12),
        (0.0, 4.0, 0.4, 0.025600000000000005, 1e-12),
        (5000.0, 5.0, 0.001, 0.00017537926031330198, 1e-10),
        (10000.0, 5.0, 0.001, 1.8841056306648148e-5, 1e-10),
    ];

    const NEGATIVE_BINOMIAL_CDF_REFERENCES: [(f64, f64, f64, f64, f64); 4] = [
        (6.0, 4.0, 0.4, 0.6177193984, 1e-12),
        (2.0, 4.0, 0.4, 0.17920000000000003, 1e-12),
        (5000.0, 5.0, 0.001, 0.560471745232959, 1e-10),
        (10000.0, 5.0, 0.001, 0.9708983683734362, 1e-10),
    ];

    #[test]
    fn pmf_matches_mpmath_including_large_n_log_space() {
        for (trials, successes, probability, expected, tolerance) in PMF_REFERENCES {
            let actual = binomial_pmf(trials, successes, probability).expect("valid domain");
            assert!(
                (actual - expected).abs() <= tolerance * expected.abs().max(f64::MIN_POSITIVE),
                "pmf({trials}, {successes}, {probability}): {actual} vs {expected}",
            );
        }
        // Far past the tail the log-space mass drops below the exp()
        // underflow limit and is reported as exactly zero.
        assert_eq!(binomial_pmf(100000.0, 250.0, 0.4), Ok(0.0));
    }

    #[test]
    fn cdf_and_range_sums_match_mpmath() {
        for (trials, successes, probability, expected, tolerance) in CDF_REFERENCES {
            let actual = cdf(trials, successes, probability);
            assert!(
                (actual - expected).abs() <= tolerance * expected.abs().max(1e-3),
                "cdf({trials}, {successes}, {probability}): {actual} vs {expected}",
            );
        }
        for (trials, probability, first, last, expected, tolerance) in RANGE_REFERENCES {
            let actual = binomial_pmf_sum(trials, probability, first, last, || Ok(()))
                .expect("valid domain");
            assert!(
                (actual - expected).abs() <= tolerance * expected.abs(),
                "sum({trials}, {probability}, {first}..{last}): {actual} vs {expected}",
            );
        }
    }

    #[test]
    fn degenerate_probabilities_follow_excel_conventions_exactly() {
        assert_eq!(binomial_pmf(10.0, 0.0, 0.0), Ok(1.0));
        assert_eq!(binomial_pmf(10.0, 3.0, 0.0), Ok(0.0));
        assert_eq!(binomial_pmf(10.0, 10.0, 1.0), Ok(1.0));
        assert_eq!(binomial_pmf(10.0, 3.0, 1.0), Ok(0.0));
        assert_eq!(binomial_pmf(0.0, 0.0, 0.4), Ok(1.0));
        assert_eq!(cdf(10.0, 3.0, 0.0), 1.0);
        assert_eq!(cdf(10.0, 3.0, 1.0), 0.0);
        assert_eq!(cdf(10.0, 10.0, 1.0), 1.0);
        assert_eq!(negative_binomial_pmf(0.0, 4.0, 1.0), Ok(1.0));
        assert_eq!(negative_binomial_pmf(2.0, 4.0, 1.0), Ok(0.0));
        assert_eq!(negative_binomial_pmf(2.0, 4.0, 0.0), Ok(0.0));
        assert_eq!(negative_binomial_cdf(5.0, 4.0, 1.0, || Ok(())), Ok(1.0));
        assert_eq!(negative_binomial_cdf(5.0, 4.0, 0.0, || Ok(())), Ok(0.0));
    }

    #[test]
    fn pmf_sums_to_one_across_the_support() {
        for (trials, probability) in [
            (0.0, 0.4),
            (1.0, 0.25),
            (10.0, 0.4),
            (60.0, 0.75),
            (100.0, 0.01),
            (100.0, 0.99),
            (500.0, 0.5),
        ] {
            let total = binomial_pmf_sum(trials, probability, 0.0, trials, || Ok(()))
                .expect("valid domain");
            assert!(
                (total - 1.0).abs() <= 1e-12,
                "n={trials} p={probability}: total mass {total}",
            );
        }
    }

    #[test]
    fn cdf_is_monotone_in_the_success_count() {
        for (trials, probability) in [(10.0, 0.4), (60.0, 0.75), (50.0, 0.02)] {
            let mut previous = 0.0;
            let mut successes = 0.0;
            while successes <= trials {
                let value = cdf(trials, successes, probability);
                assert!(
                    value >= previous,
                    "n={trials} p={probability} k={successes}: {value} < {previous}",
                );
                previous = value;
                successes += 1.0;
            }
        }
    }

    #[test]
    fn quantile_satisfies_the_minimal_k_contract_across_alpha_grids() {
        // Alphas hug both ends of (0, 1) to hunt for off-by-one drift.
        let alphas = [
            1e-300,
            1e-12,
            0.001,
            0.1,
            0.25,
            0.5,
            0.75,
            0.9,
            0.999,
            1.0 - 1e-12,
            1.0 - 1e-16,
        ];
        for (trials, probability) in [(10.0, 0.4), (100.0, 0.5), (1000.0, 0.01), (50.0, 0.99)] {
            for alpha in alphas {
                let k = smallest_binomial_quantile(trials, probability, alpha, || Ok(()))
                    .expect("valid domain");
                assert!((0.0..=trials).contains(&k) && k == k.trunc());
                // CDF(trials) is 1 by definition; interior k use the sums.
                if k < trials {
                    assert!(
                        cdf(trials, k, probability) >= alpha,
                        "n={trials} p={probability} alpha={alpha}: cdf({k}) below alpha",
                    );
                }
                if k > 0.0 {
                    assert!(
                        cdf(trials, k - 1.0, probability) < alpha,
                        "n={trials} p={probability} alpha={alpha}: {k} is not minimal",
                    );
                }
            }
        }
    }

    #[test]
    fn quantile_lands_exactly_on_attained_cdf_values() {
        // alpha equal to an attained CDF value must return that k itself.
        for successes in [0.0, 3.0, 4.0, 9.0] {
            let alpha = cdf(10.0, successes, 0.4);
            let k = smallest_binomial_quantile(10.0, 0.4, alpha, || Ok(())).expect("valid domain");
            assert_eq!(k, successes, "alpha={alpha}");
        }
    }

    #[test]
    fn quantile_resolves_degenerate_corners_without_summation() {
        assert_eq!(
            smallest_binomial_quantile(10.0, 0.4, 0.0, || Ok(())),
            Ok(0.0)
        );
        assert_eq!(
            smallest_binomial_quantile(10.0, 0.4, 1.0, || Ok(())),
            Ok(10.0)
        );
        assert_eq!(
            smallest_binomial_quantile(10.0, 0.0, 0.7, || Ok(())),
            Ok(0.0)
        );
        assert_eq!(
            smallest_binomial_quantile(10.0, 0.0, 1.0, || Ok(())),
            Ok(0.0)
        );
        assert_eq!(
            smallest_binomial_quantile(10.0, 1.0, 0.7, || Ok(())),
            Ok(10.0)
        );
        assert_eq!(
            smallest_binomial_quantile(10.0, 1.0, 0.0, || Ok(())),
            Ok(0.0)
        );
        assert_eq!(
            smallest_binomial_quantile(0.0, 0.4, 0.6, || Ok(())),
            Ok(0.0)
        );
    }

    #[test]
    fn negative_binomial_matches_mpmath_including_large_failure_counts() {
        for (failures, successes, probability, expected, tolerance) in
            NEGATIVE_BINOMIAL_PMF_REFERENCES
        {
            let actual =
                negative_binomial_pmf(failures, successes, probability).expect("valid domain");
            assert!(
                (actual - expected).abs() <= tolerance * expected.abs().max(1e-6),
                "pmf({failures}, {successes}, {probability}): {actual} vs {expected}",
            );
        }
        for (failures, successes, probability, expected, tolerance) in
            NEGATIVE_BINOMIAL_CDF_REFERENCES
        {
            let actual = negative_binomial_cdf(failures, successes, probability, || Ok(()))
                .expect("valid domain");
            assert!(
                (actual - expected).abs() <= tolerance * expected.abs(),
                "cdf({failures}, {successes}, {probability}): {actual} vs {expected}",
            );
        }
    }

    #[test]
    fn invalid_domains_are_rejected() {
        for (trials, successes, probability) in [
            (10.0, -1.0, 0.4),
            (10.0, 11.0, 0.4),
            (-1.0, 0.0, 0.4),
            (10.5, 3.0, 0.4),
            (10.0, 2.5, 0.4),
            (10.0, 3.0, -0.1),
            (10.0, 3.0, 1.1),
            (10.0, 3.0, f64::NAN),
            (f64::INFINITY, 3.0, 0.4),
        ] {
            assert_eq!(
                binomial_pmf(trials, successes, probability),
                Err(ErrorKind::Num),
                "n={trials} k={successes} p={probability}",
            );
        }
        assert_eq!(
            binomial_pmf_sum(10.0, 0.4, 4.0, 3.0, || Ok(())),
            Err(ErrorKind::Num),
        );
        for (trials, probability, alpha) in [
            (-1.0, 0.4, 0.6),
            (10.0, 1.5, 0.6),
            (10.0, 0.4, -0.1),
            (10.0, 0.4, 1.5),
        ] {
            assert_eq!(
                smallest_binomial_quantile(trials, probability, alpha, || Ok(())),
                Err(ErrorKind::Num),
                "n={trials} p={probability} alpha={alpha}",
            );
        }
        for (failures, successes, probability) in [
            (-1.0, 4.0, 0.4),
            (6.0, 0.0, 0.4),
            (6.5, 4.0, 0.4),
            (6.0, 4.5, 0.4),
            (6.0, 4.0, -0.1),
            (6.0, 4.0, 1.1),
        ] {
            assert_eq!(
                negative_binomial_pmf(failures, successes, probability),
                Err(ErrorKind::Num),
                "f={failures} s={successes} p={probability}",
            );
        }
    }

    #[test]
    fn summations_and_the_search_charge_work_and_stop_on_callback_errors() {
        let budget_error = ErrorKind::ResourceLimit(CalculationLimitKind::FunctionIterations);
        let mut sum_calls = 0_u32;
        binomial_pmf_sum(10.0, 0.4, 0.0, 10.0, || {
            sum_calls += 1;
            Ok(())
        })
        .expect("valid domain");
        assert_eq!(sum_calls, 11);

        let mut search_calls = 0_u32;
        smallest_binomial_quantile(10.0, 0.4, 0.6, || {
            search_calls += 1;
            Ok(())
        })
        .expect("valid domain");
        assert!(search_calls > 0);

        for budget in [0_u32, 3] {
            let mut remaining = budget;
            let charge = |remaining: &mut u32| {
                if *remaining == 0 {
                    return Err(budget_error);
                }
                *remaining -= 1;
                Ok(())
            };
            assert_eq!(
                binomial_pmf_sum(10.0, 0.4, 0.0, 10.0, || charge(&mut remaining)),
                Err(budget_error),
            );
            let mut remaining = budget;
            assert_eq!(
                smallest_binomial_quantile(10.0, 0.4, 0.6, || charge(&mut remaining)),
                Err(budget_error),
            );
            let mut remaining = budget;
            assert_eq!(
                negative_binomial_cdf(6.0, 4.0, 0.4, || charge(&mut remaining)),
                Err(budget_error),
            );
        }
    }
}
