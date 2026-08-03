use super::super::super::ast::Expr;
use super::super::super::coerce::to_logical;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::array_common::poll_cancellation;
use super::super::special_functions::{
    DomainPolicy, invert_monotone_cdf, ln_gamma, regularized_gamma_p, signed_gamma,
};
use super::super::util::required_number;
use super::{finite, quantile_solver_error};

pub(super) fn gamma(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [argument] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let x = match required_number(engine, context, argument) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    match signed_gamma(x) {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn log_gamma_precise(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    let [argument] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let x = match required_number(engine, context, argument) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    match ln_gamma(x) {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn gamma_distribution(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 4 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match required_number(engine, context, &args[0]) {
        Ok(value) if value >= 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let alpha = match required_number(engine, context, &args[1]) {
        Ok(value) if value > 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let beta = match required_number(engine, context, &args[2]) {
        Ok(value) if value > 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let cumulative = match to_logical(&engine.eval_scalar(context, &args[3])) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    if cumulative {
        match regularized_gamma_p(alpha, x / beta, || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        }) {
            Ok(value) => finite(value),
            Err(kind) => Value::Error(kind),
        }
    } else {
        density(x, alpha, beta)
    }
}

fn density(x: f64, alpha: f64, beta: f64) -> Value {
    let scaled = x / beta;
    if scaled == 0.0 {
        // Microsoft's GAMMA.DIST contract at the origin: the density is a pole
        // for alpha < 1, 1/beta for alpha = 1, and zero for alpha > 1.
        return if alpha < 1.0 {
            Value::Error(ErrorKind::Num)
        } else if alpha == 1.0 {
            finite(1.0 / beta)
        } else {
            Value::Number(0.0)
        };
    }
    let ln_gamma_alpha = match ln_gamma(alpha) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let log_density = (alpha - 1.0) * scaled.ln() - scaled - ln_gamma_alpha - beta.ln();
    finite(log_density.exp())
}

pub(super) fn gamma_inverse(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let probability = match required_number(engine, context, &args[0]) {
        Ok(value) if (0.0..=1.0).contains(&value) => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let alpha = match required_number(engine, context, &args[1]) {
        Ok(value) if value > 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let beta = match required_number(engine, context, &args[2]) {
        Ok(value) if value > 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    if probability == 0.0 {
        return Value::Number(0.0);
    }
    if probability == 1.0 {
        // The upper quantile is infinite and cannot materialize as a number.
        return Value::Error(ErrorKind::Num);
    }
    let solved = invert_monotone_cdf(
        |x| {
            regularized_gamma_p(alpha, x, || {
                poll_cancellation(context)?;
                engine.charge_function_iterations(context, 1)
            })
        },
        probability,
        DomainPolicy::PositiveHalfLine {
            initial_guess: alpha,
        },
        || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        },
    );
    match solved {
        Ok(value) => finite(beta * value),
        Err(kind) => Value::Error(quantile_solver_error(kind)),
    }
}
