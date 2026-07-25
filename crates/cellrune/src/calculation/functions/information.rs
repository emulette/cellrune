use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    if name == "NA" {
        return if args.is_empty() {
            Value::Error(ErrorKind::NA)
        } else {
            Value::Error(ErrorKind::Value)
        };
    }
    if name == "ISREF" {
        return if args.len() == 1 {
            Value::Logical(engine.resolve_rect_expr(context, &args[0]).is_ok())
        } else {
            Value::Error(ErrorKind::Value)
        };
    }
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let value = if name == "T" {
        match engine.resolve_rect_expr(context, &args[0]) {
            Ok(rect) => engine.cell_value((rect.sheet, rect.row_start, rect.col_start)),
            Err(_) => engine.eval_scalar(context, &args[0]),
        }
    } else {
        engine.eval_scalar(context, &args[0])
    };
    if matches!(value, Value::Error(kind) if kind.is_engine_issue()) {
        return value;
    }
    apply(name, value)
}

pub(super) fn call_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Option<Result<Array, ErrorKind>> {
    if args.len() != 1 {
        return None;
    }
    let array = match engine.eval_array(context, &args[0]) {
        Ok(array) if !array.is_scalar() => array,
        Ok(_) => return None,
        Err(kind) => return Some(Err(kind)),
    };
    Some(Ok(Array {
        rows: array.rows,
        cols: array.cols,
        data: array
            .data
            .into_iter()
            .map(|value| apply(name, value))
            .collect(),
    }))
}

fn apply(name: &str, value: Value) -> Value {
    if matches!(value, Value::Error(kind) if kind.is_engine_issue()) {
        return value;
    }
    match name {
        "ISBLANK" => Value::Logical(matches!(value, Value::Blank)),
        "ISERR" => Value::Logical(matches!(
            value,
            Value::Error(kind) if kind != ErrorKind::NA
        )),
        "ISERROR" => Value::Logical(matches!(value, Value::Error(_))),
        "ISNA" => Value::Logical(matches!(value, Value::Error(ErrorKind::NA))),
        "ISLOGICAL" => Value::Logical(matches!(value, Value::Logical(_))),
        "ISNONTEXT" => Value::Logical(!matches!(value, Value::Text(_))),
        "ISNUMBER" => Value::Logical(matches!(value, Value::Number(_))),
        "ISTEXT" => Value::Logical(matches!(value, Value::Text(_))),
        "ISEVEN" => parity(value, false),
        "ISODD" => parity(value, true),
        "N" => n(value),
        "T" => t(value),
        "TYPE" => value_type(value),
        "ERROR.TYPE" => error_type(value),
        _ => Value::Error(ErrorKind::Unsupported),
    }
}

fn parity(value: Value, odd: bool) -> Value {
    match value {
        Value::Number(number) => {
            let is_odd = (number.abs().trunc() as i64) % 2 == 1;
            Value::Logical(is_odd == odd)
        }
        Value::Error(kind) => Value::Error(kind),
        Value::Blank | Value::Text(_) | Value::Logical(_) => Value::Error(ErrorKind::Value),
    }
}

fn n(value: Value) -> Value {
    match value {
        Value::Number(_) | Value::Error(_) => value,
        Value::Logical(logical) => Value::Number(if logical { 1.0 } else { 0.0 }),
        Value::Blank | Value::Text(_) => Value::Number(0.0),
    }
}

fn t(value: Value) -> Value {
    match value {
        Value::Text(_) | Value::Error(_) => value,
        Value::Blank | Value::Number(_) | Value::Logical(_) => Value::Text(String::new()),
    }
}

fn value_type(value: Value) -> Value {
    Value::Number(match value {
        Value::Number(_) | Value::Blank => 1.0,
        Value::Text(_) => 2.0,
        Value::Logical(_) => 4.0,
        Value::Error(_) => 16.0,
    })
}

fn error_type(value: Value) -> Value {
    match value {
        Value::Error(ErrorKind::Null) => Value::Number(1.0),
        Value::Error(ErrorKind::Div0) => Value::Number(2.0),
        Value::Error(ErrorKind::Value) => Value::Number(3.0),
        Value::Error(ErrorKind::Ref) => Value::Number(4.0),
        Value::Error(ErrorKind::Name) => Value::Number(5.0),
        Value::Error(ErrorKind::Num) => Value::Number(6.0),
        Value::Error(ErrorKind::NA) => Value::Number(7.0),
        Value::Error(kind) => Value::Error(kind),
        Value::Blank | Value::Number(_) | Value::Text(_) | Value::Logical(_) => {
            Value::Error(ErrorKind::NA)
        }
    }
}
