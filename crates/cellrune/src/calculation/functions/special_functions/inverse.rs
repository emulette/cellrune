use super::super::super::value::ErrorKind;
use super::{
    MAX_BRACKET_STEPS, MAX_SOLVER_ITERATIONS, SOLVER_P_ABSOLUTE_TOLERANCE,
    SOLVER_P_RELATIVE_TOLERANCE, SOLVER_X_ABSOLUTE_TOLERANCE, SOLVER_X_RELATIVE_TOLERANCE,
};

/// Bracketing policy for [`invert_monotone_cdf`]. Each distribution support
/// keeps its own transform: the beta step adds a finite-interval variant
/// beside this one instead of widening it.
#[derive(Debug, Clone, Copy)]
pub(in crate::calculation::functions) enum DomainPolicy {
    /// Support (0, ∞): grow or shrink a bracket geometrically from a positive
    /// initial guess until the target probability is enclosed.
    PositiveHalfLine { initial_guess: f64 },
}

/// Solves cdf(x) = probability for a non-decreasing CDF on the policy domain.
///
/// Safeguarded refinement: secant steps accelerate but the candidate never
/// leaves the bracket, and every other step bisects so the bracket provably
/// halves. Convergence requires BOTH the bracket-width and the
/// probability-residual tolerances (absolute + relative each). Bracket
/// failure, a non-finite CDF value, and refinement that cannot converge all
/// surface as typed errors; `on_iteration` is charged before every bracket
/// step and every refinement step, and its error (the engine budget) passes
/// through unchanged.
pub(in crate::calculation::functions) fn invert_monotone_cdf(
    mut cdf: impl FnMut(f64) -> Result<f64, ErrorKind>,
    probability: f64,
    domain: DomainPolicy,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    if !probability.is_finite() || probability <= 0.0 || probability >= 1.0 {
        return Err(ErrorKind::Num);
    }
    let bracket = match domain {
        DomainPolicy::PositiveHalfLine { initial_guess } => {
            bracket_positive_half_line(&mut cdf, probability, initial_guess, &mut on_iteration)?
        }
    };
    refine(&mut cdf, probability, bracket, &mut on_iteration)
}

/// Invariant: residual(low) ≤ 0 ≤ residual(high) with low < high.
struct Bracket {
    low: f64,
    low_residual: f64,
    high: f64,
    high_residual: f64,
}

fn bracket_positive_half_line(
    cdf: &mut impl FnMut(f64) -> Result<f64, ErrorKind>,
    probability: f64,
    initial_guess: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<Bracket, ErrorKind> {
    if !initial_guess.is_finite() || initial_guess <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let mut point = initial_guess;
    let mut residual = residual_at(cdf, point, probability)?;
    if residual < 0.0 {
        for _ in 0..MAX_BRACKET_STEPS {
            on_iteration()?;
            let next = point * 2.0;
            if !next.is_finite() {
                return Err(ErrorKind::Num);
            }
            let next_residual = residual_at(cdf, next, probability)?;
            if next_residual >= 0.0 {
                return Ok(Bracket {
                    low: point,
                    low_residual: residual,
                    high: next,
                    high_residual: next_residual,
                });
            }
            point = next;
            residual = next_residual;
        }
    } else {
        for _ in 0..MAX_BRACKET_STEPS {
            on_iteration()?;
            let next = point / 2.0;
            let next_residual = residual_at(cdf, next, probability)?;
            if next_residual <= 0.0 {
                return Ok(Bracket {
                    low: next,
                    low_residual: next_residual,
                    high: point,
                    high_residual: residual,
                });
            }
            // Reached only when the CDF sits above the target even at the
            // origin (cdf(0) > p): the halving walk has exhausted the support
            // without a crossing, so no bracket exists.
            if next == 0.0 {
                return Err(ErrorKind::Num);
            }
            point = next;
            residual = next_residual;
        }
    }
    Err(ErrorKind::Num)
}

fn refine(
    cdf: &mut impl FnMut(f64) -> Result<f64, ErrorKind>,
    probability: f64,
    bracket: Bracket,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let Bracket {
        mut low,
        mut low_residual,
        mut high,
        mut high_residual,
    } = bracket;
    for iteration in 0..MAX_SOLVER_ITERATIONS {
        on_iteration()?;
        let midpoint = low + 0.5 * (high - low);
        let candidate = if iteration % 2 == 0 {
            midpoint
        } else {
            secant_step(low, low_residual, high, high_residual).unwrap_or(midpoint)
        };
        let candidate_residual = residual_at(cdf, candidate, probability)?;
        if candidate_residual <= 0.0 {
            low = candidate;
            low_residual = candidate_residual;
        } else {
            high = candidate;
            high_residual = candidate_residual;
        }
        let scale = low.abs().max(high.abs());
        let width_converged =
            high - low <= SOLVER_X_ABSOLUTE_TOLERANCE + SOLVER_X_RELATIVE_TOLERANCE * scale;
        let residual_converged = candidate_residual.abs()
            <= SOLVER_P_ABSOLUTE_TOLERANCE + SOLVER_P_RELATIVE_TOLERANCE * probability;
        if width_converged && residual_converged {
            return Ok(candidate);
        }
    }
    Err(ErrorKind::Num)
}

fn secant_step(low: f64, low_residual: f64, high: f64, high_residual: f64) -> Option<f64> {
    let denominator = high_residual - low_residual;
    if denominator <= 0.0 {
        return None;
    }
    let step = low - low_residual * (high - low) / denominator;
    (step.is_finite() && step > low && step < high).then_some(step)
}

fn residual_at(
    cdf: &mut impl FnMut(f64) -> Result<f64, ErrorKind>,
    x: f64,
    probability: f64,
) -> Result<f64, ErrorKind> {
    let residual = cdf(x)? - probability;
    if residual.is_finite() {
        Ok(residual)
    } else {
        Err(ErrorKind::Num)
    }
}

#[cfg(test)]
mod tests {
    use super::super::regularized_gamma_p;
    use super::{DomainPolicy, invert_monotone_cdf};
    use crate::calculation::limits::CalculationLimitKind;
    use crate::calculation::value::ErrorKind;

    fn gamma_inverse(a: f64, probability: f64) -> Result<f64, ErrorKind> {
        invert_monotone_cdf(
            |x| regularized_gamma_p(a, x, || Ok(())),
            probability,
            DomainPolicy::PositiveHalfLine { initial_guess: a },
            || Ok(()),
        )
    }

    // reference: mpmath 1.4.1, mp.dps = 30 — bisected to 1e-40 on the
    // regularized lower incomplete gamma for the binary value of each p,
    // independent of the Rust kernel. Tolerances are 1e-9 relative except the
    // p → 1 row: inverting through P quantizes the upper tail at half an ULP
    // of 1, and 5.6e-17 / pdf(x*) ≈ 5.9e-5 of x-noise is irreducible there, so
    // that row carries twice this bound as an absolute tolerance.
    const INVERSE_REFERENCES: [(f64, f64, f64, f64); 6] = [
        (3.0, 1e-12, 0.00018172031462637445, 1.9e-13),
        (3.0, 0.5, 2.6740603137235603, 2.7e-9),
        (3.0, 0.7, 3.6155676658659903, 3.7e-9),
        (3.0, 0.999999999999, 34.05239764948628, 1.2e-4),
        (0.5, 0.25, 0.050765522133810775, 5.1e-11),
        (50.0, 0.975, 64.7805985929183, 6.5e-8),
    ];

    #[test]
    fn inverse_matches_mpmath_including_extreme_tails() {
        for (a, probability, expected, tolerance) in INVERSE_REFERENCES {
            let actual = gamma_inverse(a, probability).expect("convergent target");
            assert!(
                (actual - expected).abs() <= tolerance,
                "a={a} p={probability}: {actual} vs {expected}",
            );
        }
    }

    #[test]
    fn inverse_round_trips_through_the_cdf_as_a_consistency_check() {
        for probability in [1e-6, 0.1, 0.5, 0.9, 1.0 - 1e-6] {
            let x = gamma_inverse(4.0, probability).expect("convergent target");
            let cdf = regularized_gamma_p(4.0, x, || Ok(())).expect("valid domain");
            assert!(
                (cdf - probability).abs() <= 1e-16 + 1e-9 * probability,
                "p={probability}: round trip gave {cdf}",
            );
        }
    }

    #[test]
    fn out_of_range_probabilities_and_guesses_are_rejected() {
        for probability in [0.0, 1.0, -0.1, 1.1, f64::NAN] {
            assert_eq!(
                gamma_inverse(3.0, probability),
                Err(ErrorKind::Num),
                "{probability}",
            );
        }
        for guess in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let result = invert_monotone_cdf(
                |x| regularized_gamma_p(3.0, x, || Ok(())),
                0.5,
                DomainPolicy::PositiveHalfLine {
                    initial_guess: guess,
                },
                || Ok(()),
            );
            assert_eq!(result, Err(ErrorKind::Num), "{guess}");
        }
    }

    #[test]
    fn bracket_failure_in_either_direction_is_a_typed_error() {
        // A flat CDF below the target can never enclose it upward…
        let stuck_low = invert_monotone_cdf(
            |_| Ok(0.25),
            0.5,
            DomainPolicy::PositiveHalfLine { initial_guess: 1.0 },
            || Ok(()),
        );
        assert_eq!(stuck_low, Err(ErrorKind::Num));
        // …and a flat CDF above the target walks the shrink direction down to
        // the origin. Counting the charged steps proves the support-exhausted
        // guard fires (1.0 reaches zero after 1 075 halvings) instead of the
        // step budget running out.
        let mut steps = 0_u32;
        let stuck_high = invert_monotone_cdf(
            |_| Ok(0.75),
            0.5,
            DomainPolicy::PositiveHalfLine { initial_guess: 1.0 },
            || {
                steps += 1;
                Ok(())
            },
        );
        assert_eq!(stuck_high, Err(ErrorKind::Num));
        assert!(
            steps < super::super::MAX_BRACKET_STEPS,
            "expected the origin guard to fire early, charged {steps} steps",
        );
    }

    #[test]
    fn non_finite_cdf_values_become_typed_errors() {
        let result = invert_monotone_cdf(
            |_| Ok(f64::NAN),
            0.5,
            DomainPolicy::PositiveHalfLine { initial_guess: 1.0 },
            || Ok(()),
        );
        assert_eq!(result, Err(ErrorKind::Num));
    }

    #[test]
    fn exhausted_iteration_budget_stops_the_solver_mid_flight() {
        let budget_error = ErrorKind::ResourceLimit(CalculationLimitKind::FunctionIterations);
        let mut remaining = 3_u32;
        let result = invert_monotone_cdf(
            |x| regularized_gamma_p(3.0, x, || Ok(())),
            0.999,
            DomainPolicy::PositiveHalfLine { initial_guess: 3.0 },
            || {
                if remaining == 0 {
                    return Err(budget_error);
                }
                remaining -= 1;
                Ok(())
            },
        );
        assert_eq!(result, Err(budget_error));
    }
}
