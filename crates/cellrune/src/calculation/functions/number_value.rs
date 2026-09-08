use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::array_common::poll_cancellation;
use super::util::required_text;

pub(super) fn number_value(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    // The call contract materializes the deterministic decimal/group defaults.
    let [text, decimal, group] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let result = (|| {
        let text = required_text(engine, context, text)?;
        let decimal = required_text(engine, context, decimal)?;
        let group = required_text(engine, context, group)?;
        let decimal = decimal.chars().next().ok_or(ErrorKind::Value)?;
        let group = group.chars().next();
        if Some(decimal) == group {
            return Err(ErrorKind::Value);
        }
        parse_number(&text, decimal, group, || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        })
    })();
    match result {
        Ok(number) => Value::Number(number),
        Err(kind) => Value::Error(kind),
    }
}

fn parse_number(
    text: &str,
    decimal: char,
    group: Option<char>,
    mut charge: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let mut normalized = String::with_capacity(text.len());
    let mut has_decimal = false;
    let mut has_exponent = false;
    let mut percents = 0_u32;
    for character in text.chars() {
        charge()?;
        if character == ' ' {
            continue;
        }
        if character == '%' {
            percents += 1;
            continue;
        }
        if percents != 0 {
            return Err(ErrorKind::Value);
        }
        if character == decimal {
            if has_decimal || has_exponent {
                return Err(ErrorKind::Value);
            }
            has_decimal = true;
            normalized.push('.');
        } else if Some(character) == group {
            if has_decimal || has_exponent {
                return Err(ErrorKind::Value);
            }
        } else if character.is_ascii_digit() || matches!(character, '+' | '-' | 'e' | 'E') {
            has_exponent |= matches!(character, 'e' | 'E');
            normalized.push(character);
        } else {
            return Err(ErrorKind::Value);
        }
    }
    if normalized.is_empty() && percents == 0 {
        return Ok(0.0);
    }
    let mut value = normalized.parse::<f64>().map_err(|_| ErrorKind::Value)?;
    if !value.is_finite() {
        return Err(ErrorKind::Value);
    }
    for _ in 0..percents {
        charge()?;
        value /= 100.0;
    }
    Ok(value)
}
