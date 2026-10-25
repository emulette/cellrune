use super::super::ast::Expr;
use super::super::coerce::{logical_text, to_logical, values_equal};
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::elementwise::Pick;
use super::kernel::LogicalFunction;
use super::util::{collect_argument_values, fifteen_digit_rules};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: LogicalFunction,
    args: &[Expr],
) -> Value {
    match function {
        LogicalFunction::True => constant(args, true),
        LogicalFunction::False => constant(args, false),
        LogicalFunction::Not => not(engine, context, args),
        LogicalFunction::Or => logical_aggregate(engine, context, args, false),
        LogicalFunction::Xor => logical_aggregate(engine, context, args, true),
        LogicalFunction::IfNa => ifna(engine, context, args),
        LogicalFunction::Ifs => ifs(engine, context, args),
        LogicalFunction::Switch => switch(engine, context, args),
    }
}

fn constant(args: &[Expr], value: bool) -> Value {
    if args.is_empty() {
        Value::Logical(value)
    } else {
        Value::Error(ErrorKind::Value)
    }
}

fn not(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match to_logical(&engine.eval_scalar(context, &args[0])) {
        Ok(value) => Value::Logical(!value),
        Err(kind) => Value::Error(kind),
    }
}

fn logical_aggregate(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    exclusive: bool,
) -> Value {
    let maximum_arguments = if exclusive { 254 } else { 255 };
    if args.is_empty() || args.len() > maximum_arguments {
        return Value::Error(ErrorKind::Value);
    }
    let mut participants = 0_u64;
    let mut true_count = 0_u64;
    let values = match collect_argument_values(engine, context, args) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    for item in values {
        match item.value {
            Value::Number(number) => {
                participants += 1;
                true_count += u64::from(number != 0.0);
            }
            Value::Logical(logical) => {
                participants += 1;
                true_count += u64::from(logical);
            }
            // Text other than `TRUE` and `FALSE` takes no part, as in `AND`.
            Value::Text(text) if !item.from_collection => {
                if let Some(logical) = logical_text(&text) {
                    participants += 1;
                    true_count += u64::from(logical);
                }
            }
            Value::Blank if !item.from_collection => participants += 1,
            Value::Error(kind) => return Value::Error(kind),
            Value::Blank | Value::Text(_) => {}
        }
    }
    if participants == 0 {
        Value::Error(ErrorKind::Value)
    } else if exclusive {
        Value::Logical(!true_count.is_multiple_of(2))
    } else {
        Value::Logical(true_count > 0)
    }
}

fn ifna(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    match engine.eval_scalar(context, &args[0]) {
        Value::Error(kind) if kind.is_engine_issue() => Value::Error(kind),
        Value::Error(ErrorKind::NA) => engine.eval_scalar(context, &args[1]),
        value => value,
    }
}

/// `IFS` in an array context: conditions that hold arrays pick per element, and the picked values
/// are arrays.
pub(super) fn ifs_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if args.len() < 2 || !args.len().is_multiple_of(2) {
        return Err(ErrorKind::Value);
    }
    let conditions = args
        .chunks_exact(2)
        .map(|pair| evaluated_array(engine, context, &pair[0]))
        .collect::<Result<Vec<_>, _>>()?;
    super::elementwise::select(engine, context, args, &conditions, |elements| {
        for (pair, condition) in elements.iter().enumerate() {
            match to_logical(condition) {
                Ok(true) => return Pick::Argument(2 * pair + 1),
                Ok(false) => {}
                Err(kind) => return Pick::Value(Value::Error(kind)),
            }
        }
        Pick::Value(Value::Error(ErrorKind::NA))
    })
}

/// `SWITCH` in an array context: an expression or values that hold arrays pick per element, and
/// the picked results are arrays.
pub(super) fn switch_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if args.len() < 3 {
        return Err(ErrorKind::Value);
    }
    let pair_end = if args.len().is_multiple_of(2) {
        args.len() - 1
    } else {
        args.len()
    };
    let mut selectors = vec![evaluated_array(engine, context, &args[0])?];
    for pair in args[1..pair_end].chunks_exact(2) {
        selectors.push(evaluated_array(engine, context, &pair[0])?);
    }
    let fifteen_digits = fifteen_digit_rules(engine);
    super::elementwise::select(engine, context, args, &selectors, |elements| {
        let target = elements[0];
        if let Value::Error(kind) = target {
            return Pick::Value(Value::Error(*kind));
        }
        for (pair, candidate) in elements[1..].iter().enumerate() {
            match values_equal(target, candidate, fifteen_digits) {
                Ok(true) => return Pick::Argument(2 * pair + 2),
                Ok(false) => {}
                Err(kind) => return Pick::Value(Value::Error(kind)),
            }
        }
        if pair_end < args.len() {
            Pick::Argument(pair_end)
        } else {
            Pick::Value(Value::Error(ErrorKind::NA))
        }
    })
}

/// The array `expr` evaluates to; its own error is a one-cell array.
fn evaluated_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<Array, ErrorKind> {
    match engine.eval_array(context, expr) {
        Ok(array) => Ok(array),
        Err(kind) if kind.is_engine_issue() => Err(kind),
        Err(kind) => Ok(Array::scalar(Value::Error(kind))),
    }
}

fn ifs(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || !args.len().is_multiple_of(2) {
        return Value::Error(ErrorKind::Value);
    }
    for pair in args.chunks_exact(2) {
        match to_logical(&engine.eval_scalar(context, &pair[0])) {
            Ok(true) => return engine.eval_scalar(context, &pair[1]),
            Ok(false) => {}
            Err(kind) => return Value::Error(kind),
        }
    }
    Value::Error(ErrorKind::NA)
}

fn switch(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 3 {
        return Value::Error(ErrorKind::Value);
    }
    let target = engine.eval_scalar(context, &args[0]);
    if let Value::Error(kind) = target {
        return Value::Error(kind);
    }
    let pair_end = if args.len().is_multiple_of(2) {
        args.len() - 1
    } else {
        args.len()
    };
    for pair in args[1..pair_end].chunks_exact(2) {
        let candidate = engine.eval_scalar(context, &pair[0]);
        match values_equal(&target, &candidate, fifteen_digit_rules(engine)) {
            Ok(true) => return engine.eval_scalar(context, &pair[1]),
            Ok(false) => {}
            Err(kind) => return Value::Error(kind),
        }
    }
    if pair_end < args.len() {
        engine.eval_scalar(context, &args[pair_end])
    } else {
        Value::Error(ErrorKind::NA)
    }
}
