use super::super::super::value::ErrorKind;
use super::bounded_probability;
use super::log_gamma::ln_gamma;
use super::{CONVERGENCE_EPSILON, LENTZ_TINY, LN_UNDERFLOW_LIMIT, MAX_REFINEMENT_ITERATIONS};

/// Regularized incomplete beta I_x(a, b) for finite a > 0, b > 0, x ∈ [0, 1].
///
/// Branch policy: the modified-Lentz continued fraction converges fastest
/// below the switch point (a + 1)/(a + b + 2), so past it the kernel applies
/// the symmetry I_x(a, b) = 1 − I_{1−x}(b, a) and evaluates the fraction on
/// the reflected arguments. The directly computed quantity is always the
/// small tail, so the complement subtraction never cancels. `on_iteration`
/// is charged before every continued-fraction step.
///
/// Measured limitation: at extreme equal shapes (a = b ≳ 5e6) accuracy at
/// the symmetry seam is floored by the ULP of the Lanczos lnΓ terms, so the
/// two branches can disagree — and order — by that margin there, and tails
/// whose log-space prefactor drops below ln(f64::MIN_POSITIVE) are reported
/// as exact 0/1 even where a subnormal would be representable. Both are
/// accepted fail-closed policy: refinement that cannot converge surfaces as
/// a typed error, never a wrong number.
pub(in crate::calculation::functions) fn regularized_incomplete_beta(
    a: f64,
    b: f64,
    x: f64,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    if !a.is_finite() || a <= 0.0 || !b.is_finite() || b <= 0.0 || !(0.0..=1.0).contains(&x) {
        return Err(ErrorKind::Num);
    }
    if x == 0.0 {
        return Ok(0.0);
    }
    if x == 1.0 {
        return Ok(1.0);
    }
    if x < (a + 1.0) / (a + b + 2.0) {
        direct_tail(a, b, x, &mut on_iteration)
    } else {
        Ok(1.0 - direct_tail(b, a, 1.0 - x, &mut on_iteration)?)
    }
}

/// ln B(a, b) = lnΓ(a) + lnΓ(b) − lnΓ(a + b) for finite a > 0, b > 0; shared
/// with the density evaluators so they cannot drift from this kernel.
pub(in crate::calculation::functions) fn ln_beta(a: f64, b: f64) -> Result<f64, ErrorKind> {
    Ok(ln_gamma(a)? + ln_gamma(b)? - ln_gamma(a + b)?)
}

fn direct_tail(
    a: f64,
    b: f64,
    x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let prefactor = log_prefactor(a, b, x)?;
    if prefactor < LN_UNDERFLOW_LIMIT {
        return Ok(0.0);
    }
    let fraction = continued_fraction(a, b, x, on_iteration)?;
    bounded_probability(prefactor.exp() * fraction / a)
}

/// ln of the prefactor exp(a·ln x + b·ln(1−x) − lnB(a, b)); staying in log
/// space is what protects the tails from spurious overflow.
fn log_prefactor(a: f64, b: f64, x: f64) -> Result<f64, ErrorKind> {
    let value = a * x.ln() + b * (1.0 - x).ln() - ln_beta(a, b)?;
    if value.is_nan() {
        Err(ErrorKind::Num)
    } else {
        Ok(value)
    }
}

/// Modified-Lentz evaluation of the incomplete-beta continued fraction; each
/// loop pass applies one even and one odd fraction step.
fn continued_fraction(
    a: f64,
    b: f64,
    x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let sum = a + b;
    let above = a + 1.0;
    let below = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - sum * x / above;
    if d.abs() < LENTZ_TINY {
        d = LENTZ_TINY;
    }
    d = 1.0 / d;
    let mut fraction = d;
    for m in 1..=MAX_REFINEMENT_ITERATIONS {
        on_iteration()?;
        let m = f64::from(m);
        let doubled = 2.0 * m;
        let even = m * (b - m) * x / ((below + doubled) * (a + doubled));
        d = 1.0 + even * d;
        if d.abs() < LENTZ_TINY {
            d = LENTZ_TINY;
        }
        c = 1.0 + even / c;
        if c.abs() < LENTZ_TINY {
            c = LENTZ_TINY;
        }
        d = 1.0 / d;
        fraction *= d * c;
        let odd = -(a + m) * (sum + m) * x / ((a + doubled) * (above + doubled));
        d = 1.0 + odd * d;
        if d.abs() < LENTZ_TINY {
            d = LENTZ_TINY;
        }
        c = 1.0 + odd / c;
        if c.abs() < LENTZ_TINY {
            c = LENTZ_TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        fraction *= delta;
        if (delta - 1.0).abs() < CONVERGENCE_EPSILON {
            return Ok(fraction);
        }
    }
    Err(ErrorKind::Num)
}

#[cfg(test)]
mod tests {
    use super::regularized_incomplete_beta;
    use crate::calculation::limits::CalculationLimitKind;
    use crate::calculation::value::ErrorKind;

    fn incomplete_beta(a: f64, b: f64, x: f64) -> f64 {
        regularized_incomplete_beta(a, b, x, || Ok(())).expect("valid domain")
    }

    // reference: mpmath 1.4.1, mp.dps = 30 — (a, b, x, I, tolerance) covering
    // both symmetry branches (the switch point is (a + 1)/(a + b + 2)), tiny
    // and large shape parameters, and both extreme tails. The two deep-tail
    // rows carry relative tolerances: at a = 500 the accuracy limit is the
    // ULP of the log-space exponent a·ln x (≈ 1e-13 relative), not machine
    // epsilon, and an absolute bound there would have no discriminating power.
    const GRID_REFERENCES: [(f64, f64, f64, f64, f64); 15] = [
        (0.001, 0.001, 1e-9, 0.4897457971281709, 1e-12),
        (0.001, 0.001, 0.5, 0.5, 1e-12),
        (0.001, 0.001, 0.999, 0.5034406643608944, 1e-12),
        (0.5, 0.5, 0.25, 0.3333333333333333, 1e-12),
        (1.0, 1.0, 0.3, 0.3, 1e-12),
        (1.0, 5.0, 0.2, 0.67232, 1e-12),
        (2.0, 3.0, 0.3, 0.3483, 1e-12),
        (2.0, 3.0, 0.6, 0.8208, 1e-12),
        (8.0, 10.0, 0.05, 6.313621707851916e-7, 1e-18),
        (8.0, 10.0, 0.6, 0.9081007458287615, 1e-12),
        (8.0, 10.0, 0.99, 0.9999999999999998, 1e-15),
        (500.0, 700.0, 0.28, 1.9508843506706205e-24, 2e-35),
        (500.0, 700.0, 0.35, 8.924518076445037e-7, 1e-17),
        (500.0, 700.0, 0.4, 0.12051628908943207, 2e-12),
        (500.0, 700.0, 0.5, 0.9999999964603646, 1e-12),
    ];

    #[test]
    fn incomplete_beta_matches_mpmath_on_both_branches() {
        for (a, b, x, expected, tolerance) in GRID_REFERENCES {
            let actual = incomplete_beta(a, b, x);
            assert!(
                (actual - expected).abs() <= tolerance,
                "I_{x}({a}, {b}): {actual} vs {expected}",
            );
        }
    }

    // reference: mpmath 1.4.1, mp.dps = 30 — one f64 step below, at, and
    // above the exact switch points 0.4 (for a=3, b=5) and 0.45 (for a=8,
    // b=10), so the branch selection itself is under test.
    const SWITCH_REFERENCES: [(f64, f64, f64, f64); 6] = [
        (3.0, 5.0, 0.39999999999999997, 0.580096),
        (3.0, 5.0, 0.4, 0.5800960000000001),
        (3.0, 5.0, 0.4000000000000001, 0.5800960000000002),
        (8.0, 10.0, 0.44999999999999996, 0.5256918104178496),
        (8.0, 10.0, 0.45, 0.5256918104178498),
        (8.0, 10.0, 0.45000000000000007, 0.52569181041785),
    ];

    #[test]
    fn both_branches_agree_across_the_symmetry_switch_point() {
        for (a, b, x, expected) in SWITCH_REFERENCES {
            let actual = incomplete_beta(a, b, x);
            assert!(
                (actual - expected).abs() <= 1e-14,
                "I_{x}({a}, {b}): {actual} vs {expected}",
            );
        }
    }

    #[test]
    fn tails_obey_the_reflection_identity_across_the_grid() {
        for (a, b) in [
            (0.001, 0.001),
            (0.5, 2.5),
            (2.0, 3.0),
            (8.0, 10.0),
            (500.0, 700.0),
        ] {
            for step in 1..16 {
                // Dyadic abscissae keep x and 1 − x exact in binary.
                let x = f64::from(step) / 16.0;
                let direct = incomplete_beta(a, b, x);
                let reflected = incomplete_beta(b, a, 1.0 - x);
                assert!(
                    (direct + reflected - 1.0).abs() <= 1e-12,
                    "a={a} b={b} x={x}: I + I' = {}",
                    direct + reflected,
                );
            }
        }
    }

    #[test]
    fn lower_tail_is_monotone_in_x_including_the_branch_boundary() {
        for (a, b) in [(0.5, 2.5), (2.0, 3.0), (8.0, 10.0)] {
            let mut previous = 0.0;
            for step in 0..=200 {
                let x = f64::from(step) / 200.0;
                let probability = incomplete_beta(a, b, x);
                assert!(
                    probability + 1e-13 >= previous,
                    "a={a} b={b} x={x}: {probability} dropped below {previous}",
                );
                previous = probability;
            }
        }
    }

    #[test]
    fn tails_below_the_underflow_cutoff_are_reported_as_exact_bounds() {
        // reference: mpmath 1.4.1 — I_0.05(500, 700) ≈ 1.11e-314 sits below
        // ln(f64::MIN_POSITIVE) in log space, so the documented policy reports
        // the tail as exactly zero (and its complement as exactly one).
        assert_eq!(incomplete_beta(500.0, 700.0, 0.05), 0.0);
        assert_eq!(incomplete_beta(500.0, 700.0, 0.9), 1.0);
        assert_eq!(incomplete_beta(2.0, 3.0, 0.0), 0.0);
        assert_eq!(incomplete_beta(2.0, 3.0, 1.0), 1.0);
    }

    #[test]
    fn invalid_domains_are_rejected() {
        for (a, b, x) in [
            (0.0, 1.0, 0.5),
            (-1.0, 1.0, 0.5),
            (1.0, 0.0, 0.5),
            (1.0, -2.0, 0.5),
            (1.0, 1.0, -0.1),
            (1.0, 1.0, 1.1),
            (f64::NAN, 1.0, 0.5),
            (1.0, f64::NAN, 0.5),
            (1.0, 1.0, f64::NAN),
            (f64::INFINITY, 1.0, 0.5),
            (1.0, f64::INFINITY, 0.5),
        ] {
            assert_eq!(
                regularized_incomplete_beta(a, b, x, || Ok(())),
                Err(ErrorKind::Num),
                "a={a} b={b} x={x}",
            );
        }
    }

    #[test]
    fn both_branches_charge_work_and_stop_on_callback_errors() {
        let budget_error = ErrorKind::ResourceLimit(CalculationLimitKind::FunctionIterations);
        for x in [0.2, 0.9] {
            let mut calls = 0_u32;
            regularized_incomplete_beta(2.0, 3.0, x, || {
                calls += 1;
                Ok(())
            })
            .expect("valid domain");
            assert!(calls > 0, "x={x} charged no work");

            let mut remaining = 1_u32;
            let result = regularized_incomplete_beta(2.0, 3.0, x, || {
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
