use super::super::super::value::ErrorKind;
use super::bounded_probability;
use super::log_gamma::ln_gamma;
use super::{CONVERGENCE_EPSILON, LENTZ_TINY, LN_UNDERFLOW_LIMIT, MAX_REFINEMENT_ITERATIONS};

/// Regularized lower incomplete gamma P(a, x) for finite a > 0, x ≥ 0.
///
/// Branch policy: the power series converges fastest for x < a + 1 and the
/// modified-Lentz continued fraction for x ≥ a + 1. The small tail is always
/// the directly computed quantity, so the complement subtraction never
/// cancels: P is small only on the series branch and Q only on the fraction
/// branch. `on_iteration` is charged before every refinement step.
pub(in crate::calculation::functions) fn regularized_gamma_p(
    a: f64,
    x: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let (value, branch) = regularized_gamma(a, x, on_iteration)?;
    Ok(match branch {
        Branch::LowerSeries => value,
        Branch::UpperContinuedFraction => 1.0 - value,
    })
}

/// Regularized lower incomplete gamma P(a, x) when the caller has x in log
/// space. This preserves meaningful lower tails when a finite ratio such as
/// `value / scale` lies below the smallest subnormal, and resolves ratios
/// above f64::MAX to the exact limiting CDF without materializing infinity.
pub(in crate::calculation::functions) fn regularized_gamma_p_from_log(
    a: f64,
    log_x: f64,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    if !a.is_finite() || a <= 0.0 || !log_x.is_finite() {
        return Err(ErrorKind::Num);
    }
    if log_x > f64::MAX.ln() {
        return Ok(1.0);
    }
    let x = log_x.exp();
    let (value, branch) = regularized_gamma_positive(a, x, log_x, &mut on_iteration)?;
    Ok(match branch {
        Branch::LowerSeries => value,
        Branch::UpperContinuedFraction => 1.0 - value,
    })
}

/// Complement Q(a, x) = 1 − P(a, x), computed on the same branch policy.
/// Test-gated until a production consumer lands — the statistical wave's
/// right-tail distributions are the planned first consumer. The branch tests
/// below exercise it directly.
#[cfg(test)]
pub(super) fn regularized_gamma_q(
    a: f64,
    x: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let (value, branch) = regularized_gamma(a, x, on_iteration)?;
    Ok(match branch {
        Branch::LowerSeries => 1.0 - value,
        Branch::UpperContinuedFraction => value,
    })
}

#[derive(Debug, Clone, Copy)]
enum Branch {
    LowerSeries,
    UpperContinuedFraction,
}

fn regularized_gamma(
    a: f64,
    x: f64,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<(f64, Branch), ErrorKind> {
    if !a.is_finite() || a <= 0.0 || !x.is_finite() || x < 0.0 {
        return Err(ErrorKind::Num);
    }
    if x == 0.0 {
        return Ok((0.0, Branch::LowerSeries));
    }
    regularized_gamma_positive(a, x, x.ln(), &mut on_iteration)
}

fn regularized_gamma_positive(
    a: f64,
    x: f64,
    log_x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<(f64, Branch), ErrorKind> {
    if x < a + 1.0 {
        Ok((
            lower_series(a, x, log_x, on_iteration)?,
            Branch::LowerSeries,
        ))
    } else {
        Ok((
            upper_continued_fraction(a, x, log_x, on_iteration)?,
            Branch::UpperContinuedFraction,
        ))
    }
}

fn lower_series(
    a: f64,
    x: f64,
    log_x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    // Factor 1/a out of the conventional series and fold it into
    // Gamma(a + 1) = a*Gamma(a). Besides avoiding `1/a` overflow, this lets
    // the leading lower tail be evaluated from log_x even when x itself is
    // below the smallest representable subnormal.
    let prefactor = a * log_x - x - ln_gamma(a + 1.0)?;
    if prefactor < LN_UNDERFLOW_LIMIT {
        return Ok(0.0);
    }
    let mut denominator = a;
    let mut term = 1.0;
    let mut sum = term;
    for _ in 0..MAX_REFINEMENT_ITERATIONS {
        on_iteration()?;
        denominator += 1.0;
        term *= x / denominator;
        sum += term;
        if term.abs() < sum.abs() * CONVERGENCE_EPSILON {
            return bounded_probability(prefactor.exp() * sum);
        }
    }
    Err(ErrorKind::Num)
}

fn upper_continued_fraction(
    a: f64,
    x: f64,
    log_x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let prefactor = log_prefactor(a, x, log_x)?;
    if prefactor < LN_UNDERFLOW_LIMIT {
        return Ok(0.0);
    }
    let mut b = x + 1.0 - a;
    let mut c = 1.0 / LENTZ_TINY;
    let mut d = 1.0 / b;
    let mut fraction = d;
    for i in 1..=MAX_REFINEMENT_ITERATIONS {
        on_iteration()?;
        let numerator = -f64::from(i) * (f64::from(i) - a);
        b += 2.0;
        d = numerator * d + b;
        if d.abs() < LENTZ_TINY {
            d = LENTZ_TINY;
        }
        c = b + numerator / c;
        if c.abs() < LENTZ_TINY {
            c = LENTZ_TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        fraction *= delta;
        if (delta - 1.0).abs() < CONVERGENCE_EPSILON {
            return bounded_probability(prefactor.exp() * fraction);
        }
    }
    Err(ErrorKind::Num)
}

/// ln of the shared prefactor exp(a·ln x − x − lnΓ(a)); keeping it in log
/// space is what protects both branches from spurious overflow.
fn log_prefactor(a: f64, x: f64, log_x: f64) -> Result<f64, ErrorKind> {
    let value = a * log_x - x - ln_gamma(a)?;
    if value.is_nan() {
        Err(ErrorKind::Num)
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{regularized_gamma_p, regularized_gamma_p_from_log, regularized_gamma_q};
    use crate::calculation::limits::CalculationLimitKind;
    use crate::calculation::value::ErrorKind;

    // reference: mpmath 1.4.1, mp.dps = 30 — (a, x, P, Q) covering the series
    // branch (x < a + 1), the continued-fraction branch (x ≥ a + 1), small and
    // large a, and both extreme tails.
    const PQ_REFERENCES: [(f64, f64, f64, f64); 15] = [
        (0.01, 0.02, 0.9669321313764189, 0.03306786862358118),
        (0.5, 0.1, 0.34527915398142295, 0.654720846018577),
        (0.5, 2.0, 0.9544997361036416, 0.04550026389635842),
        (1.0, 0.5, 0.3934693402873666, 0.6065306597126334),
        (2.0, 1.0, 0.26424111765711533, 0.7357588823428847),
        (2.0, 5.0, 0.9595723180054871, 0.040427681994512805),
        (3.0, 1e-8, 1.6666666541666667e-25, 1.0),
        (3.0, 50.0, 1.0, 2.509303552201057e-19),
        (
            3.0,
            1.3333333333333333,
            0.15063144384932484,
            0.8493685561506752,
        ),
        (10.0, 3.0, 0.0011024881301154798, 0.9988975118698845),
        (10.0, 30.0, 0.9999928782491372, 7.121750862815577e-6),
        (100.0, 80.0, 0.017108313035133115, 0.9828916869648668),
        (100.0, 120.0, 0.9721362601094793, 0.027863739890520663),
        (200.0, 180.0, 0.07485803498415958, 0.9251419650158405),
        (200.0, 220.0, 0.9181943116110617, 0.08180568838893833),
    ];

    #[test]
    fn log_scale_input_preserves_unrepresentable_ratios_and_upper_limits() {
        let log_x = 1e-308_f64.ln() - 1e308_f64.ln();
        let lower_tail =
            regularized_gamma_p_from_log(0.001, log_x, || Ok(())).expect("log-space lower tail");
        let expected = 0.242_242_491_462_598_63;
        assert!(
            (lower_tail - expected).abs() <= 1e-12 * expected,
            "P(0.001, exp({log_x})): {lower_tail} vs {expected}",
        );

        let log_overflow = 1e308_f64.ln() - 1e-308_f64.ln();
        assert_eq!(
            regularized_gamma_p_from_log(1.0, log_overflow, || Ok(())),
            Ok(1.0),
        );
    }

    #[test]
    fn regularized_gamma_matches_mpmath_on_both_branches() {
        for (a, x, p, q) in PQ_REFERENCES {
            let actual_p = regularized_gamma_p(a, x, || Ok(())).expect("valid domain");
            let actual_q = regularized_gamma_q(a, x, || Ok(())).expect("valid domain");
            assert!(
                (actual_p - p).abs() <= 1e-15 + 1e-12 * p,
                "P({a}, {x}): {actual_p} vs {p}",
            );
            assert!(
                (actual_q - q).abs() <= 1e-15 + 1e-12 * q,
                "Q({a}, {x}): {actual_q} vs {q}",
            );
        }
    }

    #[test]
    fn lower_and_upper_tails_sum_to_one_across_the_grid() {
        for a in [0.05, 0.5, 1.0, 2.5, 10.0, 100.0] {
            for x in [0.01, 0.5, 1.0, 2.0, 5.0, 20.0, 150.0] {
                let p = regularized_gamma_p(a, x, || Ok(())).expect("valid domain");
                let q = regularized_gamma_q(a, x, || Ok(())).expect("valid domain");
                assert!((p + q - 1.0).abs() <= 1e-12, "a={a} x={x}: P+Q={}", p + q);
            }
        }
    }

    #[test]
    fn lower_tail_is_monotone_in_x_including_the_branch_boundary() {
        for a in [0.5, 3.0, 50.0] {
            let mut previous = 0.0;
            for step in 0..200 {
                let x = f64::from(step) * (a + 10.0) / 100.0;
                let p = regularized_gamma_p(a, x, || Ok(())).expect("valid domain");
                assert!(
                    p + 1e-13 >= previous,
                    "a={a} x={x}: {p} dropped below {previous}",
                );
                previous = p;
            }
        }
    }

    #[test]
    fn large_alpha_series_converges_within_the_refinement_cap() {
        // x near a is the series branch's worst case (≈ 8.6·√a steps; ~12k
        // here). reference: mpmath 1.4.1, 40-digit series and continued
        // fraction agreeing to 1e-35. Accuracy at large alpha is bounded by
        // the ULP of the log-space exponent a·ln x (≈ 3.7e-9 at 2.9e7), not
        // by machine precision, so the tolerance is absolute at that scale.
        let p = regularized_gamma_p(2_000_000.0, 2_000_000.0, || Ok(())).expect("within coverage");
        let expected = 0.5000940315975192;
        assert!(
            (p - expected).abs() <= 1e-8,
            "P(2e6, 2e6): {p} vs {expected}",
        );
        // Beyond the ≈ 1.35e8 coverage bound the kernel fails closed.
        assert_eq!(
            regularized_gamma_p(1e9, 1e9, || Ok(())),
            Err(ErrorKind::Num),
        );
    }

    #[test]
    fn invalid_domains_are_rejected() {
        for (a, x) in [
            (0.0, 1.0),
            (-1.0, 1.0),
            (1.0, -0.5),
            (f64::NAN, 1.0),
            (1.0, f64::NAN),
            (f64::INFINITY, 1.0),
            (1.0, f64::INFINITY),
        ] {
            assert_eq!(
                regularized_gamma_p(a, x, || Ok(())),
                Err(ErrorKind::Num),
                "a={a} x={x}",
            );
        }
    }

    #[test]
    fn both_branches_charge_work_and_stop_on_callback_errors() {
        let budget_error = ErrorKind::ResourceLimit(CalculationLimitKind::FunctionIterations);
        for x in [3.0, 50.0] {
            let mut calls = 0_u32;
            regularized_gamma_p(3.0, x, || {
                calls += 1;
                Ok(())
            })
            .expect("valid domain");
            assert!(calls > 0, "x={x} charged no work");

            let mut remaining = 2_u32;
            let result = regularized_gamma_p(3.0, x, || {
                if remaining == 0 {
                    return Err(budget_error);
                }
                remaining -= 1;
                Ok(())
            });
            assert_eq!(result, Err(budget_error), "x={x}");
        }
    }
}
