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
    residual: &impl Fn(f64) -> (f64, f64),
) -> Result<f64, ErrorKind> {
    let mut guess = if policy.initial_guess > lower_bound {
        policy.initial_guess
    } else {
        lower_bound + 1.0
    };

    for _ in 0..policy.max_iterations {
        charge_work(engine, context, work_per_iteration)?;
        let (value, derivative) = residual(guess);
        let newton_usable =
            value.is_finite() && derivative.is_finite() && derivative.abs() >= DERIVATIVE_FLOOR;
        if newton_usable {
            let next = guess - value / derivative;
            if next.is_finite() && next > lower_bound {
                if (next - guess).abs() <= policy.tolerance {
                    return Ok(next);
                }
                guess = next;
                continue;
            }
        }
        return bracket_solve(
            engine,
            context,
            lower_bound,
            work_per_iteration,
            policy,
            residual,
            guess,
        );
    }

    bracket_solve(
        engine,
        context,
        lower_bound,
        work_per_iteration,
        policy,
        residual,
        guess,
    )
}

fn bracket_solve(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    lower_bound: f64,
    work_per_iteration: usize,
    policy: SolverPolicy,
    residual: &impl Fn(f64) -> (f64, f64),
    hint: f64,
) -> Result<f64, ErrorKind> {
    let mut low = lower_bound + 1e-9;
    let mut high = if hint > low { hint * 2.0 } else { low + 1.0 };

    let mut low_value = residual(low).0;
    let mut high_value = residual(high).0;

    for _ in 0..MAX_BRACKET_EXPANSIONS {
        charge_work(engine, context, work_per_iteration)?;
        if !low_value.is_finite() {
            low = lower_bound + (low - lower_bound) * 0.5;
            low_value = residual(low).0;
            continue;
        }
        if !high_value.is_finite() {
            high = (low + high) * 0.5;
            high_value = residual(high).0;
            continue;
        }
        if low_value * high_value <= 0.0 {
            break;
        }
        high = lower_bound + (high - lower_bound) * 2.0;
        high_value = residual(high).0;
    }

    if low_value.is_finite() && high_value.is_finite() && low_value * high_value > 0.0 {
        return Err(ErrorKind::Num);
    }

    for _ in 0..policy.max_iterations {
        charge_work(engine, context, work_per_iteration)?;
        let midpoint = (low + high) * 0.5;
        let midpoint_value = residual(midpoint).0;
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
