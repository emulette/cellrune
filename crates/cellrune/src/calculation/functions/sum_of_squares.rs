use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::util::{excel_numeric_arguments, excel_sum};

const MAX_EXCEL_ARGUMENTS: usize = 255;

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    match name {
        "SUMSQ" => sumsq(engine, context, args),
        "SUMX2MY2" => paired(engine, context, args, |left, right| {
            left * left - right * right
        }),
        "SUMX2PY2" => paired(engine, context, args, |left, right| {
            left * left + right * right
        }),
        "SUMXMY2" => paired(engine, context, args, |left, right| {
            (left - right) * (left - right)
        }),
        _ => Value::Error(ErrorKind::Unsupported),
    }
}

fn sumsq(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > MAX_EXCEL_ARGUMENTS {
        return Value::Error(ErrorKind::Value);
    }
    let numbers = match excel_numeric_arguments(engine, context, args) {
        Ok(numbers) => numbers,
        Err(kind) => return Value::Error(kind),
    };
    finite(excel_sum(numbers.into_iter().map(|number| number * number)))
}

fn paired(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: impl Fn(f64, f64) -> f64,
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let left = match engine.eval_array(context, &args[0]) {
        Ok(array) => array,
        Err(kind) => return Value::Error(kind),
    };
    let right = match engine.eval_array(context, &args[1]) {
        Ok(array) => array,
        Err(kind) => return Value::Error(kind),
    };
    if left.data.len() != right.data.len() {
        return Value::Error(ErrorKind::NA);
    }
    let mut result = 0.0;
    let mut pairs = 0_u64;
    for (left, right) in left.data.into_iter().zip(right.data) {
        match (left, right) {
            (Value::Error(kind), _) | (_, Value::Error(kind)) => return Value::Error(kind),
            (Value::Number(left), Value::Number(right)) => {
                result += operation(left, right);
                pairs += 1;
            }
            _ => {}
        }
    }
    if pairs == 0 {
        return Value::Error(ErrorKind::Div0);
    }
    finite(result)
}

fn finite(number: f64) -> Value {
    if number.is_finite() {
        Value::Number(number)
    } else {
        Value::Error(ErrorKind::Num)
    }
}
