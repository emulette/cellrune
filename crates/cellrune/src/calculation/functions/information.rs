use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::kernel::{InformationArrayFunction, InformationFunction};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: InformationFunction,
    args: &[Expr],
) -> Value {
    if function == InformationFunction::FormulaText {
        return super::reference_introspection::formula_text(engine, context, args);
    }
    if function == InformationFunction::IsFormula {
        return super::reference_introspection::is_formula(engine, context, args);
    }
    if function == InformationFunction::Na {
        return if args.is_empty() {
            Value::Error(ErrorKind::NA)
        } else {
            Value::Error(ErrorKind::Value)
        };
    }
    if function == InformationFunction::IsRef {
        return if args.len() == 1 {
            match engine.resolve_reference_value_expr(context, &args[0]) {
                Ok(reference) => Value::Logical(
                    !matches!(&reference, super::super::runtime::ReferenceValue::Empty)
                        && !reference.has_sheet_span(),
                ),
                Err(kind) if kind.is_engine_issue() => Value::Error(kind),
                Err(_) => Value::Logical(false),
            }
        } else {
            Value::Error(ErrorKind::Value)
        };
    }
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let value = if function == InformationFunction::T {
        match engine.resolve_rect_expr(context, &args[0]) {
            Ok(rect) => engine
                .read_reference_cell(context, (rect.sheet, rect.row_start, rect.col_start))
                .unwrap_or_else(Value::Error),
            Err(_) => engine.eval_scalar(context, &args[0]),
        }
    } else {
        engine.eval_scalar(context, &args[0])
    };
    if matches!(value, Value::Error(kind) if kind.is_engine_issue()) {
        return value;
    }
    apply(function, value)
}

pub(super) fn call_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: InformationArrayFunction,
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
            .map(|value| apply(function.scalar_function(), value))
            .collect(),
    }))
}

fn apply(function: InformationFunction, value: Value) -> Value {
    if matches!(value, Value::Error(kind) if kind.is_engine_issue()) {
        return value;
    }
    match function {
        InformationFunction::IsBlank => Value::Logical(matches!(value, Value::Blank)),
        InformationFunction::IsErr => Value::Logical(matches!(
            value,
            Value::Error(kind) if kind != ErrorKind::NA
        )),
        InformationFunction::IsError => Value::Logical(matches!(value, Value::Error(_))),
        InformationFunction::IsNa => Value::Logical(matches!(value, Value::Error(ErrorKind::NA))),
        InformationFunction::IsLogical => Value::Logical(matches!(value, Value::Logical(_))),
        InformationFunction::IsNonText => Value::Logical(!matches!(value, Value::Text(_))),
        InformationFunction::IsNumber => Value::Logical(matches!(value, Value::Number(_))),
        InformationFunction::IsText => Value::Logical(matches!(value, Value::Text(_))),
        InformationFunction::IsEven => parity(value, false),
        InformationFunction::IsOdd => parity(value, true),
        InformationFunction::N => n(value),
        InformationFunction::T => t(value),
        InformationFunction::Type => value_type(value),
        InformationFunction::ErrorType => error_type(value),
        InformationFunction::FormulaText
        | InformationFunction::IsFormula
        | InformationFunction::Na
        | InformationFunction::IsRef => {
            unreachable!("reference metadata and NA return before value classification")
        }
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
