//! Pure log-space kernels for the hypergeometric mass and its running sum.
//!
//! The mass has two branches. Below [`FALLING_FACTORIAL_SAMPLE_LIMIT`] it is
//! evaluated as a product of individually moderate factors, which is what keeps
//! huge populations accurate; above it the lnΓ form trades accuracy for a
//! constant cost per term. Every unbounded loop charges a caller-supplied
//! callback before it does more work.

use super::super::super::super::value::ErrorKind;
use super::super::super::special_functions::bounded_probability;
use super::super::super::special_functions::ln_gamma;
use super::{Parameters, support_floor};

/// Largest `number_sample` that still takes the falling-factorial branch. The
/// lnΓ form differences values of size lnΓ(N) ≈ N·ln N, so its absolute error
/// in log space is about N·ln(N)·f64::EPSILON — already 4e-6 at N = 1e9, which
/// would dominate the result. The falling-factorial branch instead multiplies
/// `number_sample` moderate factors and stays accurate to a few ULP, at a cost
/// of 2·number_sample charged factors per mass evaluation. The threshold caps
/// that cost while covering every sample size a spreadsheet realistically draws.
pub(super) const FALLING_FACTORIAL_SAMPLE_LIMIT: f64 = 10_000.0;

pub(super) fn probability_mass(
    parameters: Parameters,
    on_factor: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let log_mass = if parameters.sample <= FALLING_FACTORIAL_SAMPLE_LIMIT {
        falling_factorial_log_mass(parameters, on_factor)?
    } else {
        log_gamma_log_mass(parameters)?
    };
    bounded_probability(log_mass.exp())
}

/// Sums the mass from the support floor up to `sample_successes`. Every term
/// charges `on_iteration` once before any factor work, so the engine's
/// function-iteration limit bounds the summation even when the truncated
/// support is astronomically wide.
pub(super) fn cumulative_probability(
    parameters: Parameters,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let floor = support_floor(parameters);
    let ceiling = parameters.sample.min(parameters.population_successes);
    // The CDF over the complete finite support is exactly one. This also
    // covers every one-point (degenerate) support without evaluating a mass or
    // converting its potentially huge f64 count into a loop index.
    if parameters.sample_successes == ceiling {
        return Ok(1.0);
    }
    let span = parameters.sample_successes - floor;
    let mut total = 0.0;
    // An integer step counter always advances; adding 1.0 to the success count
    // itself would stall once the count reaches 2^53 and re-add one term until
    // the budget ran out.
    let mut step = 0_u64;
    while (step as f64) <= span {
        on_iteration()?;
        total += probability_mass(
            Parameters {
                sample_successes: floor + step as f64,
                ..parameters
            },
            &mut on_iteration,
        )?;
        step += 1;
    }
    bounded_probability(total)
}

/// C(n,k)·(M)_k·(N−M)_(n−k)/(N)_n, the falling-factorial rearrangement of
/// C(M,k)·C(N−M,n−k)/C(N,n). Every factor is a moderate number, so no two large
/// lnΓ values are ever differenced and the huge-population cancellation that
/// afflicts the lnΓ form cannot arise.
fn falling_factorial_log_mass(
    parameters: Parameters,
    on_factor: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let Parameters {
        sample_successes,
        sample,
        population_successes,
        population,
    } = parameters;
    let drawn_successes = ln_falling_factorial(population_successes, sample_successes, on_factor)?;
    let drawn_failures = ln_falling_factorial(
        population - population_successes,
        sample - sample_successes,
        on_factor,
    )?;
    let draws = ln_falling_factorial(population, sample, on_factor)?;
    Ok(ln_binomial(sample, sample_successes)? + drawn_successes + drawn_failures - draws)
}

/// ln[(value)(value−1)···(value−length+1)], charging one factor per step.
/// Callers guarantee 0 ≤ length ≤ value, so every factor is at least one.
fn ln_falling_factorial(
    value: f64,
    length: f64,
    on_factor: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let mut total = 0.0;
    let mut index = 0_u64;
    while (index as f64) < length {
        on_factor()?;
        total += (value - index as f64).ln();
        index += 1;
    }
    Ok(total)
}

/// Constant-cost fallback above the falling-factorial threshold. Contract: the
/// relative error grows with the population as N·ln(N)·f64::EPSILON, because
/// the binomials are differences of lnΓ values of that size.
fn log_gamma_log_mass(parameters: Parameters) -> Result<f64, ErrorKind> {
    let Parameters {
        sample_successes,
        sample,
        population_successes,
        population,
    } = parameters;
    // Log space keeps every intermediate binomial inside the f64 range; the
    // direct C(M,k)·C(N−M,n−k)/C(N,n) product overflows well before the
    // quotient does.
    Ok(ln_binomial(population_successes, sample_successes)?
        + ln_binomial(population - population_successes, sample - sample_successes)?
        - ln_binomial(population, sample)?)
}

/// ln C(n, k). Callers validate 0 ≤ k ≤ n, so all three lnΓ arguments are ≥ 1.
fn ln_binomial(n: f64, k: f64) -> Result<f64, ErrorKind> {
    Ok(ln_gamma(n + 1.0)? - ln_gamma(k + 1.0)? - ln_gamma(n - k + 1.0)?)
}

#[cfg(test)]
mod tests {
    use super::super::validated;
    use super::{
        FALLING_FACTORIAL_SAMPLE_LIMIT, cumulative_probability, probability_mass, support_floor,
    };
    use crate::calculation::limits::CalculationLimitKind;
    use crate::calculation::value::ErrorKind;

    // reference: mpmath 1.4.1, mp.dps = 30, mass via binomial ratios and the
    // distribution summed from the support floor —
    // (sample_s, number_sample, population_s, number_pop, mass, cumulative).
    // Covers the oracle-pinned inputs, both support endpoints (rows 6 and 8 sit
    // on a floor of 4), a degenerate two-item population, and a large
    // population where only log space stays in range.
    #[rustfmt::skip]
    const REFERENCES: [(u32, u32, u32, u32, f64, f64); 16] = [
        (0, 4, 8, 20, 0.1021671826625387, 0.1021671826625387),
        (1, 4, 8, 20, 0.3632610939112487, 0.46542827657378744),
        (2, 4, 8, 20, 0.3814241486068111, 0.8468524251805986),
        (3, 4, 8, 20, 0.1386996904024768, 0.9855521155830753),
        (4, 4, 8, 20, 0.014447884416924664, 1.0),
        (4, 6, 8, 10, 0.3333333333333333, 0.3333333333333333),
        (5, 6, 8, 10, 0.5333333333333333, 0.8666666666666667),
        (6, 6, 8, 10, 0.13333333333333333, 1.0),
        (0, 5, 3, 10, 0.08333333333333333, 0.08333333333333333),
        (3, 5, 3, 10, 0.08333333333333333, 1.0),
        (1, 1, 1, 2, 0.5, 1.0),
        (50, 100, 500, 1000, 0.08389209209281304, 0.5419460460464065),
        (380, 1000, 40000, 100000, 0.011198326931073884, 0.10268258107833424),
        (400, 1000, 40000, 100000, 0.025874515752789578, 0.5137817804521811),
        (450, 1000, 40000, 100000, 0.0001407695280203671, 0.9994398502998244),
        // Above FALLING_FACTORIAL_SAMPLE_LIMIT, so this row pins the lnΓ branch.
        (6000, 20000, 30000, 100000, 0.006882294924727216, 0.5037164218769691),
    ];

    fn arguments(x: u32, n: u32, m: u32, big_n: u32) -> [f64; 4] {
        [f64::from(x), f64::from(n), f64::from(m), f64::from(big_n)]
    }

    fn mass(arguments: [f64; 4]) -> f64 {
        let parameters = validated(arguments).expect("documented domain");
        probability_mass(parameters, &mut || Ok(())).expect("valid domain")
    }

    fn cumulative(arguments: [f64; 4]) -> f64 {
        let parameters = validated(arguments).expect("documented domain");
        cumulative_probability(parameters, || Ok(())).expect("valid domain")
    }

    /// The falling-factorial branch is limited by lnΓ(number_sample), whose ULP
    /// reaches ~1e-12 at the largest sample here. The lnΓ branch is limited by
    /// lnΓ(number_pop) instead, two orders looser at this population.
    fn relative_tolerance(sample: u32) -> f64 {
        if f64::from(sample) > FALLING_FACTORIAL_SAMPLE_LIMIT {
            1e-9
        } else {
            1e-11
        }
    }

    #[test]
    fn probability_mass_matches_mpmath() {
        for (x, n, m, big_n, expected, _) in REFERENCES {
            let actual = mass(arguments(x, n, m, big_n));
            assert!(
                (actual - expected).abs() <= 1e-15 + relative_tolerance(n) * expected,
                "mass({x}, {n}, {m}, {big_n}): {actual} vs {expected}",
            );
        }
    }

    #[test]
    fn cumulative_probability_matches_mpmath() {
        for (x, n, m, big_n, _, expected) in REFERENCES {
            let actual = cumulative(arguments(x, n, m, big_n));
            assert!(
                (actual - expected).abs() <= 1e-15 + relative_tolerance(n) * expected,
                "cumulative({x}, {n}, {m}, {big_n}): {actual} vs {expected}",
            );
        }
    }

    /// Over each full support the mass is a distribution and the cumulative is
    /// its running total: non-decreasing, and reaching one at the support top.
    #[test]
    fn every_support_is_a_monotone_distribution_summing_to_one() {
        for (n, m, big_n) in [
            (4.0_f64, 8.0_f64, 20.0_f64),
            (6.0, 8.0, 10.0),
            (5.0, 3.0, 10.0),
            (1.0, 1.0, 2.0),
            (30.0, 40.0, 60.0),
            (100.0, 500.0, 1000.0),
            // A population far beyond the reach of the lnΓ form.
            (3.0, 2.0, 1_000_000_000.0),
        ] {
            let ceiling = n.min(m);
            let top = validated([ceiling, n, m, big_n]).expect("documented domain");
            let mut k = support_floor(top);
            let (mut total, mut previous) = (0.0, 0.0);
            while k <= ceiling {
                total += mass([k, n, m, big_n]);
                let running = cumulative([k, n, m, big_n]);
                assert!(running + 1e-15 >= previous, "({n},{m},{big_n}) k={k}");
                assert!((running - total).abs() <= 1e-12, "({n},{m},{big_n}) k={k}");
                previous = running;
                k += 1.0;
            }
            assert!((total - 1.0).abs() <= 1e-12, "({n},{m},{big_n}): {total}");
        }
    }

    // reference: mpmath 1.4.1, mp.dps = 50. The lnΓ form differences values of
    // size lnΓ(1e9 + 1) ≈ 1.97e10, whose ULP is ~3.8e-6, and returned
    // 0.9999961913099915 for the first case below; the falling-factorial branch
    // holds every digit.
    #[test]
    fn huge_population_masses_avoid_log_gamma_cancellation() {
        let top_of_support = cumulative([2.0, 3.0, 2.0, 1_000_000_000.0]);
        assert!(
            (top_of_support - 1.0).abs() <= 1e-12,
            "cumulative at the support top: {top_of_support}",
        );
        let expected = 0.999_999_994_000_000_1;
        let floor_of_support = cumulative([0.0, 3.0, 2.0, 1_000_000_000.0]);
        assert!(
            (floor_of_support - expected).abs() <= 1e-12 * expected,
            "cumulative at the support floor: {floor_of_support} vs {expected}",
        );
        let single = mass([1.0, 3.0, 2.0, 1_000_000_000.0]);
        let expected = 5.999_999_988e-9;
        assert!(
            (single - expected).abs() <= 1e-12 * expected,
            "mass at k = 1: {single} vs {expected}",
        );
    }

    #[test]
    fn full_and_degenerate_supports_bypass_the_cumulative_loop() {
        // 2^53 + 1 truncates onto 2^53. The support is the single point at that
        // value, so the exact full-support result must not depend on a lossy
        // float-to-integer conversion or consume iteration budget.
        let limit = 9_007_199_254_740_993.0;
        let degenerate = validated([limit, limit, limit, limit]).expect("documented domain");
        let mut calls = 0_u32;
        let total = cumulative_probability(degenerate, || {
            calls += 1;
            Ok(())
        })
        .expect("valid domain");
        assert_eq!(calls, 0, "a full support must not enter the summation");
        assert!((total - 1.0).abs() <= 1e-12, "one-point support: {total}");

        let ordinary = validated([4.0, 4.0, 8.0, 20.0]).expect("documented domain");
        assert_eq!(
            cumulative_probability(ordinary, || Err(ErrorKind::Num)),
            Ok(1.0),
        );
    }

    #[test]
    fn the_cumulative_branch_charges_work_and_stops_on_callback_errors() {
        let budget_error = ErrorKind::ResourceLimit(CalculationLimitKind::FunctionIterations);
        // Three terms below the threshold, each charging one iteration plus the
        // 2·number_sample falling factors.
        let narrow = validated([2.0, 4.0, 8.0, 20.0]).expect("documented domain");
        let mut calls = 0_u32;
        cumulative_probability(narrow, || {
            calls += 1;
            Ok(())
        })
        .expect("valid domain");
        assert_eq!(calls, 3 * (1 + 8));

        // Above the threshold the lnΓ form charges the per-term iteration only.
        let wide = validated([6000.0, 20000.0, 30000.0, 100000.0]).expect("documented domain");
        let mut calls = 0_u32;
        cumulative_probability(wide, || {
            calls += 1;
            Ok(())
        })
        .expect("valid domain");
        assert_eq!(calls, 6001, "one charge per summed term");

        let mut remaining = 2_u32;
        let result = cumulative_probability(wide, || {
            if remaining == 0 {
                return Err(budget_error);
            }
            remaining -= 1;
            Ok(())
        });
        assert_eq!(result, Err(budget_error));
    }
}
