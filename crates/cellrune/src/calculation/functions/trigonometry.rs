use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::util::required_number;

const RECIPROCAL_INPUT_LIMIT: f64 = 134_217_728.0;

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    match name {
        "ACOS" => checked_unary(engine, context, args, |number| {
            (-1.0..=1.0).contains(&number).then(|| number.acos())
        }),
        "ACOSH" => checked_unary(engine, context, args, |number| {
            (number >= 1.0).then(|| number.acosh())
        }),
        "ACOT" => unary(engine, context, args, |number| 1.0_f64.atan2(number)),
        "ACOTH" => checked_unary(engine, context, args, |number| {
            (number.abs() > 1.0).then(|| 0.5 * ((number + 1.0) / (number - 1.0)).ln())
        }),
        "ASIN" => checked_unary(engine, context, args, |number| {
            (-1.0..=1.0).contains(&number).then(|| number.asin())
        }),
        "ASINH" => unary(engine, context, args, f64::asinh),
        "ATAN" => unary(engine, context, args, f64::atan),
        "ATAN2" => atan2(engine, context, args),
        "ATANH" => checked_unary(engine, context, args, |number| {
            (number.abs() < 1.0).then(|| number.atanh())
        }),
        "COS" => unary(engine, context, args, f64::cos),
        "COSH" => unary(engine, context, args, f64::cosh),
        "COT" => reciprocal_trig(engine, context, args, f64::tan),
        "COTH" => reciprocal_trig(engine, context, args, f64::tanh),
        "CSC" => reciprocal_trig(engine, context, args, f64::sin),
        "CSCH" => reciprocal_trig(engine, context, args, f64::sinh),
        "DEGREES" => unary(engine, context, args, f64::to_degrees),
        "RADIANS" => unary(engine, context, args, f64::to_radians),
        "SEC" => reciprocal_trig(engine, context, args, f64::cos),
        "SECH" => reciprocal_trig(engine, context, args, f64::cosh),
        "SIN" => unary(engine, context, args, f64::sin),
        "SINH" => unary(engine, context, args, f64::sinh),
        "TAN" => unary(engine, context, args, f64::tan),
        "TANH" => unary(engine, context, args, f64::tanh),
        _ => Value::Error(ErrorKind::Unsupported),
    }
}

fn unary(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: impl FnOnce(f64) -> f64,
) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match required_number(engine, context, &args[0]) {
        Ok(number) => finite(operation(number)),
        Err(kind) => Value::Error(kind),
    }
}

fn checked_unary(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: impl FnOnce(f64) -> Option<f64>,
) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match required_number(engine, context, &args[0]) {
        Ok(number) => operation(number).map_or(Value::Error(ErrorKind::Num), finite),
        Err(kind) => Value::Error(kind),
    }
}

fn atan2(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let y = match required_number(engine, context, &args[1]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    if x == 0.0 && y == 0.0 {
        Value::Error(ErrorKind::Div0)
    } else {
        finite(y.atan2(x))
    }
}

fn reciprocal_trig(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    denominator: impl FnOnce(f64) -> f64,
) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_number(engine, context, &args[0]) {
        Ok(number) if number.abs() < RECIPROCAL_INPUT_LIMIT => number,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let divisor = denominator(number);
    if divisor == 0.0 {
        Value::Error(ErrorKind::Div0)
    } else {
        finite(1.0 / divisor)
    }
}

fn finite(number: f64) -> Value {
    if number.is_finite() {
        Value::Number(number)
    } else {
        Value::Error(ErrorKind::Num)
    }
}

#[cfg(test)]
mod tests {
    use super::RECIPROCAL_INPUT_LIMIT;

    #[test]
    fn reciprocal_input_limit_is_exactly_two_to_the_twenty_seventh() {
        assert_eq!(RECIPROCAL_INPUT_LIMIT, 2_f64.powi(27));
    }
}
