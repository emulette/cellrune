//! CONFIDENCE.NORM (also spelled CONFIDENCE) and CONFIDENCE.T: the half-width of a two-sided
//! confidence interval for a population mean, the critical value at significance alpha scaled by
//! the standard error standard_dev/√size.

use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::array_common::poll_cancellation;
use super::super::special_functions::standard_normal_inverse;
use super::super::util::required_number;
use super::t::two_tailed_critical_value;
use super::{finite, quantile_solver_error};

pub(super) fn confidence_normal(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    let result = interval_arguments(engine, context, args).and_then(|(alpha, deviation, size)| {
        // The lower quantile at alpha/2 is the negated critical value; solving it directly keeps
        // small significance levels that 1 − alpha/2 would round away.
        let critical = -standard_normal_inverse(alpha / 2.0, || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        })?;
        Ok(critical * deviation / size.sqrt())
    });
    result.map_or_else(Value::Error, finite)
}

pub(super) fn confidence_t(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let result = interval_arguments(engine, context, args).and_then(|(alpha, deviation, size)| {
        // One observation leaves no degrees of freedom for the sample deviation.
        if size == 1.0 {
            return Err(ErrorKind::Div0);
        }
        let critical = two_tailed_critical_value(alpha, size - 1.0, || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        })
        .map_err(quantile_solver_error)?;
        Ok(critical * deviation / size.sqrt())
    });
    result.map_or_else(Value::Error, finite)
}

/// Validates (alpha, standard_dev, size): 0 < alpha < 1, standard_dev > 0, and size truncated to
/// an integer of at least one.
fn interval_arguments(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<(f64, f64, f64), ErrorKind> {
    let [alpha, deviation, size] = args else {
        return Err(ErrorKind::Value);
    };
    let alpha = required_number(engine, context, alpha)?;
    if !(alpha > 0.0 && alpha < 1.0) {
        return Err(ErrorKind::Num);
    }
    let deviation = required_number(engine, context, deviation)?;
    if deviation <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let size = required_number(engine, context, size)?.trunc();
    if size < 1.0 {
        return Err(ErrorKind::Num);
    }
    Ok((alpha, deviation, size))
}
