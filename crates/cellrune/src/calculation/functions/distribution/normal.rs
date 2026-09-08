use super::super::super::ast::Expr;
use super::super::super::coerce::to_logical;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::array_common::poll_cancellation;
use super::super::special_functions::{standard_normal_inverse, standard_normal_lower};
use super::super::util::required_number;
use super::finite;

pub(super) fn normal_inverse(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    scaled_quantile(engine, context, args).map_or_else(Value::Error, finite)
}

pub(super) fn normal_standard_inverse(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    let [probability] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let result = required_probability(engine, context, probability)
        .and_then(|p| quantile(engine, context, p));
    result.map_or_else(Value::Error, finite)
}

pub(super) fn lognormal_inverse(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    scaled_quantile(engine, context, args).map_or_else(Value::Error, |value| finite(value.exp()))
}

pub(super) fn lognormal_distribution(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    legacy: bool,
) -> Value {
    let result = (|| {
        if args.len() != if legacy { 3 } else { 4 } {
            return Err(ErrorKind::Value);
        }
        let x = required_number(engine, context, &args[0])?;
        if x <= 0.0 {
            return Err(ErrorKind::Num);
        }
        let mean = required_number(engine, context, &args[1])?;
        let sigma = required_sigma(engine, context, &args[2])?;
        let cumulative = legacy || to_logical(&engine.eval_scalar(context, &args[3]))?;
        let log_x = x.ln();
        let z = (log_x - mean) / sigma;
        Ok(if cumulative {
            standard_normal_lower(z)
        } else {
            // Log-space density avoids overflowing x*sigma, or losing a tiny
            // normal density before division by a small x restores its scale.
            (-0.5 * z * z - log_x - sigma.ln() - 0.5 * (2.0 * std::f64::consts::PI).ln()).exp()
        })
    })();
    result.map_or_else(Value::Error, finite)
}

fn scaled_quantile(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<f64, ErrorKind> {
    let [probability, mean, sigma] = args else {
        return Err(ErrorKind::Value);
    };
    let probability = required_probability(engine, context, probability)?;
    let mean = required_number(engine, context, mean)?;
    let sigma = required_sigma(engine, context, sigma)?;
    // Fused scaling permits a finite result even when z*sigma alone overflows.
    Ok(quantile(engine, context, probability)?.mul_add(sigma, mean))
}

fn required_probability(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    argument: &Expr,
) -> Result<f64, ErrorKind> {
    let value = required_number(engine, context, argument)?;
    if value > 0.0 && value < 1.0 {
        Ok(value)
    } else {
        Err(ErrorKind::Num)
    }
}

fn required_sigma(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    argument: &Expr,
) -> Result<f64, ErrorKind> {
    let value = required_number(engine, context, argument)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(ErrorKind::Num)
    }
}

fn quantile(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    probability: f64,
) -> Result<f64, ErrorKind> {
    standard_normal_inverse(probability, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    })
}
