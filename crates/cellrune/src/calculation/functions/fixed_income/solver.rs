/// Safeguarded fixed-income root solver.
///
/// Yield inverses (`YIELD(N>1)` and `ODDFYIELD`) share one Newton/bracket primitive. Newton runs
/// while the residual and derivative are finite and the step stays inside the `q > 0` domain; a
/// bad derivative or an out-of-domain step falls back to bracket expansion followed by bisection.
/// Every evaluation and bracket expansion charges the evaluation budget and observes cancellation,
/// so a cancelled or exhausted solve never installs a partial iterate.
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::limits::CalculationLimitKind;
use super::super::super::value::ErrorKind;

#[derive(Debug, Clone, Copy)]
pub(super) struct SolverPolicy {
    pub(super) max_iterations: u64,
    pub(super) initial_guess: f64,
    pub(super) tolerance: f64,
}

pub(super) const EXCEL_YIELD_POLICY: SolverPolicy = SolverPolicy {
    max_iterations: 100,
    initial_guess: 0.1,
    tolerance: 1e-7,
};

pub(super) const EXTENDED_YIELD_POLICY: SolverPolicy = SolverPolicy {
    max_iterations: 1_000,
    initial_guess: 0.1,
    tolerance: 1e-12,
};

const DERIVATIVE_FLOOR: f64 = 1e-14;
const MAX_BRACKET_EXPANSIONS: u64 = 64;

pub(super) fn solve(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    lower_bound: f64,
    work_per_iteration: usize,
    policy: SolverPolicy,
    residual: &impl Fn(f64) -> Result<(f64, f64), ErrorKind>,
) -> Result<f64, ErrorKind> {
    solve_with_charge(
        lower_bound,
        work_per_iteration,
        policy,
        residual,
        &mut |work| charge_work(engine, context, work),
    )
}

pub(super) fn solve_with_charge(
    lower_bound: f64,
    work_per_iteration: usize,
    policy: SolverPolicy,
    residual: &impl Fn(f64) -> Result<(f64, f64), ErrorKind>,
    charge: &mut impl FnMut(usize) -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let mut evaluations = 0_u64;
    let mut guess = if policy.initial_guess > lower_bound {
        policy.initial_guess
    } else {
        lower_bound + 1.0
    };

    while evaluations < policy.max_iterations {
        let (value, derivative) = evaluate(
            work_per_iteration,
            policy,
            &mut evaluations,
            residual,
            guess,
            charge,
        )?;
        if value.abs() <= policy.tolerance {
            return Ok(guess);
        }
        let newton_usable =
            value.is_finite() && derivative.is_finite() && derivative.abs() >= DERIVATIVE_FLOOR;
        if newton_usable {
            let next = guess - value / derivative;
            if next.is_finite() && next > lower_bound {
                if (next - guess).abs() <= policy.tolerance {
                    let (next_value, _) = evaluate(
                        work_per_iteration,
                        policy,
                        &mut evaluations,
                        residual,
                        next,
                        charge,
                    )?;
                    if next_value.abs() <= policy.tolerance {
                        return Ok(next);
                    }
                }
                guess = next;
                continue;
            }
        }
        return bracket_solve(
            lower_bound,
            work_per_iteration,
            policy,
            residual,
            guess,
            &mut evaluations,
            charge,
        );
    }
    Err(ErrorKind::Num)
}

fn bracket_solve(
    lower_bound: f64,
    work_per_iteration: usize,
    policy: SolverPolicy,
    residual: &impl Fn(f64) -> Result<(f64, f64), ErrorKind>,
    hint: f64,
    evaluations: &mut u64,
    charge: &mut impl FnMut(usize) -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let mut low = lower_bound + 1e-9;
    let mut high = if hint.is_finite() && hint > low {
        hint
    } else {
        low + 1.0
    };

    let mut low_value = evaluate(
        work_per_iteration,
        policy,
        evaluations,
        residual,
        low,
        charge,
    )?
    .0;
    let mut high_value = evaluate(
        work_per_iteration,
        policy,
        evaluations,
        residual,
        high,
        charge,
    )?
    .0;

    for _ in 0..MAX_BRACKET_EXPANSIONS {
        if !low_value.is_finite() {
            low = lower_bound + (low - lower_bound) * 0.5;
            low_value = evaluate(
                work_per_iteration,
                policy,
                evaluations,
                residual,
                low,
                charge,
            )?
            .0;
            continue;
        }
        if !high_value.is_finite() {
            high = (low + high) * 0.5;
            high_value = evaluate(
                work_per_iteration,
                policy,
                evaluations,
                residual,
                high,
                charge,
            )?
            .0;
            continue;
        }
        if low_value * high_value <= 0.0 {
            break;
        }
        high = lower_bound + (high - lower_bound) * 2.0;
        high_value = evaluate(
            work_per_iteration,
            policy,
            evaluations,
            residual,
            high,
            charge,
        )?
        .0;
    }

    if low_value.is_finite() && high_value.is_finite() && low_value * high_value > 0.0 {
        return Err(ErrorKind::Num);
    }

    while *evaluations < policy.max_iterations {
        let midpoint = (low + high) * 0.5;
        let midpoint_value = evaluate(
            work_per_iteration,
            policy,
            evaluations,
            residual,
            midpoint,
            charge,
        )?
        .0;
        if !midpoint_value.is_finite() {
            return Err(ErrorKind::Num);
        }
        if midpoint_value.abs() <= policy.tolerance {
            return Ok(midpoint);
        }
        if low_value * midpoint_value <= 0.0 {
            high = midpoint;
        } else {
            low = midpoint;
            low_value = midpoint_value;
        }
    }

    Err(ErrorKind::Num)
}

fn evaluate(
    work_per_iteration: usize,
    policy: SolverPolicy,
    evaluations: &mut u64,
    residual: &impl Fn(f64) -> Result<(f64, f64), ErrorKind>,
    point: f64,
    charge: &mut impl FnMut(usize) -> Result<(), ErrorKind>,
) -> Result<(f64, f64), ErrorKind> {
    if *evaluations >= policy.max_iterations {
        return Err(ErrorKind::Num);
    }
    charge(work_per_iteration)?;
    *evaluations += 1;
    residual(point)
}

fn charge_work(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    work_per_iteration: usize,
) -> Result<(), ErrorKind> {
    if context.is_cancelled() {
        return Err(ErrorKind::ResourceLimit(
            CalculationLimitKind::FunctionIterations,
        ));
    }
    engine.charge_function_iterations(context, work_per_iteration.max(1) as u64)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn excel_policy_has_one_exact_residual_evaluation_budget() {
        let calls = Cell::new(0_u64);
        let residual = |point: f64| {
            calls.set(calls.get() + 1);
            Ok((if point < 0.0 { -1.0 } else { 1.0 }, 0.0))
        };
        let result = solve_with_charge(-1.0, 1, EXCEL_YIELD_POLICY, &residual, &mut |_| Ok(()));
        assert_eq!(result, Err(ErrorKind::Num));
        assert_eq!(calls.get(), 100);
    }

    #[test]
    fn small_newton_step_is_not_accepted_without_residual_validation() {
        let calls = Cell::new(0_u64);
        let residual = |_point: f64| {
            calls.set(calls.get() + 1);
            Ok((1.0, 1e12))
        };
        let policy = SolverPolicy {
            max_iterations: 3,
            initial_guess: 0.1,
            tolerance: 1e-7,
        };
        assert_eq!(
            solve_with_charge(-1.0, 1, policy, &residual, &mut |_| Ok(())),
            Err(ErrorKind::Num)
        );
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn charging_failure_stops_before_an_unpaid_residual_call() {
        let residual_calls = Cell::new(0_u64);
        let charge_calls = Cell::new(0_u64);
        let residual = |point: f64| {
            residual_calls.set(residual_calls.get() + 1);
            Ok((point - 0.25, 1.0))
        };
        let mut charge = |_| {
            let next = charge_calls.get() + 1;
            charge_calls.set(next);
            if next > 1 {
                Err(ErrorKind::ResourceLimit(
                    CalculationLimitKind::FunctionIterations,
                ))
            } else {
                Ok(())
            }
        };
        assert_eq!(
            solve_with_charge(-1.0, 1, EXCEL_YIELD_POLICY, &residual, &mut charge),
            Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations
            ))
        );
        assert_eq!(charge_calls.get(), 2);
        assert_eq!(residual_calls.get(), 1);
    }

    #[test]
    fn negative_root_near_frequency_domain_boundary_converges() {
        let policy = SolverPolicy {
            max_iterations: 100,
            initial_guess: 0.1,
            tolerance: 1e-12,
        };
        let target = -1.999_999;
        let residual = |point: f64| Ok((point - target, 1.0));
        let root = solve_with_charge(-2.0, 1, policy, &residual, &mut |_| Ok(())).unwrap();
        assert!((root - target).abs() < 1e-12);
        assert!(root > -2.0);
    }

    #[test]
    fn initial_guess_selects_the_positive_root_of_a_two_root_residual() {
        let policy = SolverPolicy {
            max_iterations: 100,
            initial_guess: 0.1,
            tolerance: 1e-12,
        };
        let residual = |point: f64| {
            let value = (point + 0.5) * (point - 0.25);
            Ok((value, 2.0 * point + 0.25))
        };
        let root = solve_with_charge(-1.0, 1, policy, &residual, &mut |_| Ok(())).unwrap();
        assert!((root - 0.25).abs() < 1e-12);
    }

    #[test]
    fn cancellation_style_charge_failure_stops_immediately() {
        let residual_calls = Cell::new(0_u64);
        let residual = |point: f64| {
            residual_calls.set(residual_calls.get() + 1);
            Ok((point - 0.25, 1.0))
        };
        let mut charge = |_| {
            Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations,
            ))
        };
        assert_eq!(
            solve_with_charge(-1.0, 1, EXCEL_YIELD_POLICY, &residual, &mut charge),
            Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations
            ))
        );
        assert_eq!(residual_calls.get(), 0);
    }

    #[test]
    fn residual_cancellation_error_is_not_collapsed_to_num() {
        let result = solve_with_charge(
            -1.0,
            1,
            EXCEL_YIELD_POLICY,
            &|_| {
                Err(ErrorKind::ResourceLimit(
                    CalculationLimitKind::FunctionIterations,
                ))
            },
            &mut |_| Ok(()),
        );
        assert_eq!(
            result,
            Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations
            ))
        );
    }
}
