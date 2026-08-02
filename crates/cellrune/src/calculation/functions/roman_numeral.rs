use super::super::ast::Expr;
use super::super::coerce::to_number;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::kernel::RomanFunction;
use super::util::required_number;

const MAX_ARABIC_TEXT_CHARS: usize = 255;
const MAX_ARABIC_MAGNITUDE: i32 = 255_000;

#[derive(Debug, Clone, Copy)]
struct RomanSymbol {
    symbol: char,
    value: i32,
}

const ROMAN_SYMBOLS: [RomanSymbol; 7] = [
    RomanSymbol {
        symbol: 'M',
        value: 1000,
    },
    RomanSymbol {
        symbol: 'D',
        value: 500,
    },
    RomanSymbol {
        symbol: 'C',
        value: 100,
    },
    RomanSymbol {
        symbol: 'L',
        value: 50,
    },
    RomanSymbol {
        symbol: 'X',
        value: 10,
    },
    RomanSymbol {
        symbol: 'V',
        value: 5,
    },
    RomanSymbol {
        symbol: 'I',
        value: 1,
    },
];

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: RomanFunction,
    args: &[Expr],
) -> Value {
    match function {
        RomanFunction::Arabic => arabic(engine, context, args),
        RomanFunction::Roman => roman(engine, context, args),
    }
}

fn arabic(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [argument] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let text = match engine.eval_scalar(context, argument) {
        Value::Text(text) => text,
        Value::Error(kind) => return Value::Error(kind),
        Value::Blank | Value::Number(_) | Value::Logical(_) => {
            return Value::Error(ErrorKind::Value);
        }
    };
    let character_count = text.chars().count();
    if let Err(kind) = engine.charge_function_iterations(context, character_count as u64) {
        return Value::Error(kind);
    }
    let parsed = match parse_arabic(&text) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    Value::Number(f64::from(parsed))
}

fn roman(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_number(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    if !number.is_finite() || !(0.0..=3999.0).contains(&number) {
        return Value::Error(ErrorKind::Value);
    }
    let form = match roman_form(engine, context, args.get(1)) {
        Ok(form) => form,
        Err(kind) => return Value::Error(kind),
    };
    let number = number.trunc() as i32;
    let text = match format_roman(number, form) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    if let Err(kind) = engine.ensure_text_bytes(text.len()) {
        return Value::Error(kind);
    }
    if let Err(kind) = engine.charge_function_iterations(context, text.len() as u64) {
        return Value::Error(kind);
    }
    Value::Text(text)
}

fn roman_form(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    argument: Option<&Expr>,
) -> Result<u8, ErrorKind> {
    let Some(argument) = argument.filter(|argument| !matches!(argument, Expr::Missing)) else {
        return Ok(0);
    };
    let value = engine.eval_scalar(context, argument);
    let form = match value {
        Value::Logical(true) => 0.0,
        Value::Logical(false) => 4.0,
        value => to_number(&value)?,
    };
    let form = form.trunc();
    if !form.is_finite() || !(0.0..=4.0).contains(&form) {
        return Err(ErrorKind::Value);
    }
    Ok(form as u8)
}

fn parse_arabic(input: &str) -> Result<i32, ErrorKind> {
    if input.chars().count() > MAX_ARABIC_TEXT_CHARS {
        return Err(ErrorKind::Value);
    }
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(0);
    }
    let (negative, numeral) = match trimmed.strip_prefix('-') {
        Some(numeral) => (true, numeral),
        None => (false, trimmed),
    };
    if numeral.is_empty() {
        return Err(ErrorKind::Value);
    }
    let mut magnitude = 0_i32;
    let mut maximum_to_right = 0_i32;
    for character in numeral.chars().rev() {
        let value = symbol_value(character.to_ascii_uppercase()).ok_or(ErrorKind::Value)?;
        magnitude = if value < maximum_to_right {
            magnitude.checked_sub(value)
        } else {
            maximum_to_right = value;
            magnitude.checked_add(value)
        }
        .ok_or(ErrorKind::Value)?;
    }
    if magnitude > MAX_ARABIC_MAGNITUDE {
        return Err(ErrorKind::Value);
    }
    Ok(if negative { -magnitude } else { magnitude })
}

fn symbol_value(symbol: char) -> Option<i32> {
    ROMAN_SYMBOLS
        .iter()
        .find(|candidate| candidate.symbol == symbol)
        .map(|candidate| candidate.value)
}

fn format_roman(number: i32, form: u8) -> Result<String, ErrorKind> {
    if !(0..=3999).contains(&number) || form > 4 {
        return Err(ErrorKind::Value);
    }
    let mut remaining = number;
    let mut result = String::new();
    for place in 0..=3 {
        let mut symbol_index = place * 2;
        let unit = ROMAN_SYMBOLS[symbol_index].value;
        let digit = remaining / unit;
        if digit % 5 == 4 {
            let target_index = symbol_index
                .checked_sub(if digit == 4 { 1 } else { 2 })
                .ok_or(ErrorKind::Value)?;
            let mut steps = 0_u8;
            while steps < form && symbol_index + 1 < ROMAN_SYMBOLS.len() {
                steps += 1;
                let concise_value =
                    ROMAN_SYMBOLS[target_index].value - ROMAN_SYMBOLS[symbol_index + 1].value;
                if concise_value <= remaining {
                    symbol_index += 1;
                } else {
                    break;
                }
            }
            result.push(ROMAN_SYMBOLS[symbol_index].symbol);
            result.push(ROMAN_SYMBOLS[target_index].symbol);
            remaining += ROMAN_SYMBOLS[symbol_index].value;
            remaining -= ROMAN_SYMBOLS[target_index].value;
        } else {
            if digit > 4 {
                result.push(
                    ROMAN_SYMBOLS
                        .get(symbol_index.checked_sub(1).ok_or(ErrorKind::Value)?)
                        .ok_or(ErrorKind::Value)?
                        .symbol,
                );
            }
            for _ in 0..(digit % 5) {
                result.push(ROMAN_SYMBOLS[symbol_index].symbol);
            }
            remaining %= unit;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{format_roman, parse_arabic};
    use crate::calculation::value::ErrorKind;

    #[test]
    fn formatter_covers_every_excel_conciseness_form() {
        let expected = ["CDXCIX", "LDVLIV", "XDIX", "VDIV", "ID"];
        for (form, expected) in expected.into_iter().enumerate() {
            assert_eq!(format_roman(499, form as u8).unwrap(), expected);
        }
        assert_eq!(format_roman(0, 0).unwrap(), "");
        assert_eq!(format_roman(3999, 0).unwrap(), "MMMCMXCIX");
    }

    #[test]
    fn parser_handles_case_space_negative_and_wide_excel_domain() {
        assert_eq!(parse_arabic("  mxmvii  "), Ok(1997));
        assert_eq!(parse_arabic("-MMXI"), Ok(-2011));
        assert_eq!(parse_arabic(" "), Ok(0));
        assert_eq!(parse_arabic(&"M".repeat(255)), Ok(255_000));
        assert_eq!(parse_arabic(&"M".repeat(256)), Err(ErrorKind::Value));
        assert_eq!(parse_arabic("not-roman"), Err(ErrorKind::Value));
    }

    #[test]
    fn every_formatter_form_round_trips_through_the_parser() {
        for number in 0..=3999 {
            for form in 0..=4 {
                let formatted = format_roman(number, form).unwrap();
                assert_eq!(
                    parse_arabic(&formatted),
                    Ok(number),
                    "{number}, form {form}"
                );
            }
        }
    }
}
