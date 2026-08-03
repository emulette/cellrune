use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::kernel::DistributionFunction;

mod gamma;

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: DistributionFunction,
    args: &[Expr],
) -> Value {
    match function {
        DistributionFunction::Gamma => gamma::gamma(engine, context, args),
        DistributionFunction::GammaDist => gamma::gamma_distribution(engine, context, args),
        DistributionFunction::GammaInv => gamma::gamma_inverse(engine, context, args),
        DistributionFunction::GammaLnPrecise => gamma::log_gamma_precise(engine, context, args),
    }
}

/// Distribution kernels return untraced finite numbers or an Excel error;
/// NaN and infinity never escape as cell values.
fn finite(number: f64) -> Value {
    if number.is_finite() {
        Value::Number(number)
    } else {
        Value::Error(ErrorKind::Num)
    }
}

/// Microsoft documents #N/A when an inverse-distribution search fails to
/// converge (GAMMA.INV; BETA.INV shares the contract). Domain violations are
/// rejected before the solve, so any non-budget failure out of the quantile
/// driver is by construction a convergence failure; engine resource errors
/// pass through unchanged.
fn quantile_solver_error(kind: ErrorKind) -> ErrorKind {
    match kind {
        ErrorKind::Num => ErrorKind::NA,
        kind => kind,
    }
}
