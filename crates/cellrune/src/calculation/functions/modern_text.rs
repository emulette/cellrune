use super::super::ast::Expr;
use super::super::coerce::to_number;
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::array_common::{poll_cancellation, validate_array_input};
use super::kernel::{ModernTextArrayFunction, ModernTextFunction};

pub(super) fn call_scalar(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: ModernTextFunction,
    args: &[Expr],
) -> Value {
    match function {
        ModernTextFunction::ArrayToText => array_to_text(engine, context, args),
        ModernTextFunction::RegexExtract => {
            super::regex_text::extract_scalar(engine, context, args)
        }
        ModernTextFunction::RegexReplace => super::regex_text::replace(engine, context, args),
        ModernTextFunction::RegexTest => super::regex_text::test(engine, context, args),
        ModernTextFunction::TextSplit => super::text_split::call_scalar(engine, context, args),
    }
}

pub(super) fn call_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: ModernTextArrayFunction,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    match function {
        ModernTextArrayFunction::RegexExtract => {
            super::regex_text::extract_array(engine, context, args)
        }
        ModernTextArrayFunction::TextSplit => super::text_split::call_array(engine, context, args),
    }
}

fn array_to_text(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let array = match engine.eval_array(context, &args[0]) {
        Ok(array) => array,
        Err(kind) => return Value::Error(kind),
    };
    if let Err(kind) = validate_array_input(engine, context, &array) {
        return Value::Error(kind);
    }
    let strict = match args.get(1) {
        None | Some(Expr::Missing) => false,
        Some(expression) => match to_number(&engine.eval_scalar(context, expression)) {
            Ok(number) if number.trunc() == 0.0 => false,
            Ok(number) if number.trunc() == 1.0 => true,
            Ok(_) => return Value::Error(ErrorKind::Value),
            Err(kind) => return Value::Error(kind),
        },
    };

    let mut output = String::new();
    if strict && let Err(kind) = push_bounded(engine, &mut output, "{") {
        return Value::Error(kind);
    }
    for row in 0..array.rows {
        for column in 0..array.cols {
            if let Err(kind) = poll_cancellation(context) {
                return Value::Error(kind);
            }
            if row != 0 || column != 0 {
                let delimiter = if strict {
                    if column == 0 { ";" } else { "," }
                } else {
                    ", "
                };
                if let Err(kind) = push_bounded(engine, &mut output, delimiter) {
                    return Value::Error(kind);
                }
            }
            let text = match super::text_common::value_to_text(array.at(row, column), strict) {
                Ok(text) => text,
                Err(kind) => return Value::Error(kind),
            };
            if let Err(kind) = push_bounded(engine, &mut output, &text) {
                return Value::Error(kind);
            }
        }
    }
    if strict && let Err(kind) = push_bounded(engine, &mut output, "}") {
        return Value::Error(kind);
    }
    Value::Text(output)
}

pub(super) fn push_bounded(
    engine: &Engine<'_>,
    output: &mut String,
    text: &str,
) -> Result<(), ErrorKind> {
    let bytes = output
        .len()
        .checked_add(text.len())
        .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::TextBytes))?;
    engine.ensure_text_bytes(bytes)?;
    output.push_str(text);
    Ok(())
}
