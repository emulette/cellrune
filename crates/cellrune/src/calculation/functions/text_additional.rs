use super::super::ast::Expr;
use super::super::coerce::to_logical;
use super::super::eval::{Engine, EvalContext};
use super::super::excel_digits::{Rounding, round_to_digits};
use super::super::limits::CalculationLimitKind;
use super::super::textfmt::format_number;
use super::super::value::{ErrorKind, Value};
use super::kernel::TextAdditionalFunction;
use super::util::{fifteen_digit_rules, required_number, required_text};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: TextAdditionalFunction,
    args: &[Expr],
    written: &[Expr],
) -> Value {
    match function {
        TextAdditionalFunction::BahtText => super::baht_text::baht_text(engine, context, args),
        TextAdditionalFunction::Char => character(engine, context, args),
        TextAdditionalFunction::Code => code(engine, context, args),
        TextAdditionalFunction::NumberValue => {
            super::number_value::number_value(engine, context, args)
        }
        TextAdditionalFunction::Clean => clean(engine, context, args),
        TextAdditionalFunction::Concatenate => concatenate(engine, context, args),
        TextAdditionalFunction::Dollar => dollar(engine, context, args, written),
        TextAdditionalFunction::EncodeUrl => super::url_encoding::encode_url(engine, context, args),
        TextAdditionalFunction::Fixed => fixed(engine, context, args),
        TextAdditionalFunction::UniChar => unichar(engine, context, args),
        TextAdditionalFunction::Unicode => unicode(engine, context, args),
        TextAdditionalFunction::TextBefore => text_boundary(engine, context, args, false),
        TextAdditionalFunction::TextAfter => text_boundary(engine, context, args, true),
        TextAdditionalFunction::Value => value(engine, context, args),
        TextAdditionalFunction::ValueToText => value_to_text(engine, context, args),
    }
}

fn character(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let code = match required_number(engine, context, &args[0]) {
        Ok(number) if (1.0..=255.0).contains(&number) => number.trunc() as u8,
        Ok(_) => return Value::Error(ErrorKind::Value),
        Err(kind) => return Value::Error(kind),
    };
    cp1252_character(code).map_or(Value::Error(ErrorKind::Value), |character| {
        engine.bounded_text(character.to_string())
    })
}

fn code(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [argument] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let text = match required_text(engine, context, argument) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let Some(character) = text.chars().next() else {
        return Value::Error(ErrorKind::Value);
    };
    let point = u32::from(character);
    if point < 128 || (160..=255).contains(&point) {
        return Value::Number(f64::from(point));
    }
    (128_u8..=159)
        .find(|code| cp1252_character(*code) == Some(character))
        .map_or(Value::Error(ErrorKind::Value), |code| {
            Value::Number(f64::from(code))
        })
}

/// `DOLLAR`; an omitted `decimals` is 2, which `written`, the arguments as the formula wrote them,
/// tells apart from an explicit value. The locale chooses the currency symbol; Excel 16.114 with
/// Korean regional settings writes `DOLLAR(1.5)` as `₩1.50` and `DOLLAR(-1.5)` as `(₩1.50)`.
fn dollar(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr], written: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let locale = engine.locale_profile().locale();
    let decimals = written.get(1).filter(|expr| !matches!(expr, Expr::Missing));
    let (rounded, decimals) = match rounded_to_decimals(engine, context, &args[0], decimals, 2) {
        Ok(rounded) => rounded,
        Err(kind) => return Value::Error(kind),
    };
    let format = format!(
        "\"{}\"#,##0{}",
        locale.currency_symbol(),
        decimal_places(decimals)
    );
    let formatted = match format_number(rounded.abs(), &format, engine.format_context()) {
        Ok(formatted) => formatted,
        Err(kind) => return Value::Error(kind),
    };
    let output = if rounded.is_sign_negative() && rounded != 0.0 {
        format!("({formatted})")
    } else {
        formatted
    };
    engine.bounded_text(output)
}

fn fixed(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let (rounded, decimals) = match rounded_to_decimals(engine, context, &args[0], args.get(1), 2) {
        Ok(rounded) => rounded,
        Err(kind) => return Value::Error(kind),
    };
    let no_commas = match args.get(2) {
        None | Some(Expr::Missing) => false,
        Some(expr) => match to_logical(&engine.eval_scalar(context, expr)) {
            Ok(no_commas) => no_commas,
            Err(kind) => return Value::Error(kind),
        },
    };
    let grouping = if no_commas { "0" } else { "#,##0" };
    let format = format!("{grouping}{}", decimal_places(decimals));
    match format_number(rounded, &format, engine.format_context()) {
        Ok(text) => engine.bounded_text(text),
        Err(kind) => Value::Error(kind),
    }
}

/// Reads the number and decimals `DOLLAR` and `FIXED` share and rounds the number as `ROUND` does:
/// numeric text is read as `VALUE` reads it, decimals truncate and take `default_decimals` when
/// omitted, and negative decimals round to the left of the decimal point. A negative number that
/// rounds to zero therefore shows no sign (`FIXED(-0.001,2)` is `0.00`). Returns the rounded number
/// and the count of decimals to show.
fn rounded_to_decimals(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    number: &Expr,
    decimals: Option<&Expr>,
    default_decimals: i32,
) -> Result<(f64, usize), ErrorKind> {
    let number = required_number(engine, context, number)?;
    let decimals = match decimals {
        None | Some(Expr::Missing) => default_decimals,
        Some(expr) => required_number(engine, context, expr)?.trunc() as i32,
    };
    if !(-308..=127).contains(&decimals) {
        return Err(ErrorKind::Value);
    }
    let rounded = round_to_digits(
        number,
        decimals,
        Rounding::Nearest,
        fifteen_digit_rules(engine),
    )
    .ok_or(ErrorKind::Num)?;
    Ok((rounded, decimals.max(0) as usize))
}

fn decimal_places(decimals: usize) -> String {
    if decimals == 0 {
        String::new()
    } else {
        format!(".{}", "0".repeat(decimals))
    }
}

fn value(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let text = match required_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    engine
        .numeric_text()
        .number(&text)
        .map_or(Value::Error(ErrorKind::Value), Value::Number)
}

fn clean(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match required_text(engine, context, &args[0]) {
        Ok(text) => engine.bounded_text(
            text.chars()
                .filter(|character| !matches!(*character as u32, 0..=31))
                .collect(),
        ),
        Err(kind) => Value::Error(kind),
    }
}

fn concatenate(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 255 {
        return Value::Error(ErrorKind::Value);
    }
    let mut result = String::new();
    for arg in args {
        let text = match required_text(engine, context, arg) {
            Ok(text) => text,
            Err(kind) => return Value::Error(kind),
        };
        let Some(output_bytes) = result.len().checked_add(text.len()) else {
            return Value::Error(ErrorKind::ResourceLimit(CalculationLimitKind::TextBytes));
        };
        if let Err(kind) = engine.ensure_text_bytes(output_bytes) {
            return Value::Error(kind);
        }
        result.push_str(&text);
    }
    Value::Text(result)
}

fn unichar(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let code = match required_number(engine, context, &args[0]) {
        Ok(number) if number >= 1.0 && number <= char::MAX as u32 as f64 => number.trunc() as u32,
        Ok(_) => return Value::Error(ErrorKind::Value),
        Err(kind) => return Value::Error(kind),
    };
    char::from_u32(code).map_or(Value::Error(ErrorKind::Value), |character| {
        engine.bounded_text(character.to_string())
    })
}

fn unicode(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match required_text(engine, context, &args[0]) {
        Ok(text) => text
            .chars()
            .next()
            .map(|character| Value::Number(character as u32 as f64))
            .unwrap_or(Value::Error(ErrorKind::Value)),
        Err(kind) => Value::Error(kind),
    }
}

fn text_boundary(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    after: bool,
) -> Value {
    if args.len() < 2 || args.len() > 6 {
        return Value::Error(ErrorKind::Value);
    }
    let text = match required_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let delimiter = match required_text(engine, context, &args[1]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let instance = match args.get(2) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) if number.trunc() != 0.0 => number.trunc() as i64,
            Ok(_) => return Value::Error(ErrorKind::Value),
            Err(kind) => return Value::Error(kind),
        },
        None => 1,
    };
    let case_insensitive = match args.get(3) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) if number.trunc() == 0.0 => false,
            Ok(number) if number.trunc() == 1.0 => true,
            Ok(_) => return Value::Error(ErrorKind::Value),
            Err(kind) => return Value::Error(kind),
        },
        None => false,
    };
    let match_end = match args.get(4) {
        Some(expr) => match to_logical(&engine.eval_scalar(context, expr)) {
            Ok(value) => value,
            Err(kind) => return Value::Error(kind),
        },
        None => false,
    };
    match find_text_boundary(&text, &delimiter, instance, case_insensitive, match_end) {
        Some((start, end)) => {
            let result = if after {
                text.get(end..)
            } else {
                text.get(..start)
            };
            result.map_or(Value::Error(ErrorKind::Value), |result| {
                engine.bounded_text(result.to_owned())
            })
        }
        None => args.get(5).map_or(Value::Error(ErrorKind::NA), |fallback| {
            engine.eval_scalar(context, fallback)
        }),
    }
}

fn find_text_boundary(
    text: &str,
    delimiter: &str,
    instance: i64,
    case_insensitive: bool,
    match_end: bool,
) -> Option<(usize, usize)> {
    if delimiter.is_empty() {
        return Some(if instance > 0 {
            (0, 0)
        } else {
            (text.len(), text.len())
        });
    }
    let mut matches = text_matches(text, delimiter, case_insensitive);
    if match_end {
        if instance > 0 {
            matches.push((text.len(), text.len()));
        } else {
            matches.insert(0, (0, 0));
        }
    }
    let index = if instance > 0 {
        usize::try_from(instance - 1).ok()?
    } else {
        matches
            .len()
            .checked_sub(usize::try_from(instance.unsigned_abs()).ok()?)?
    };
    matches.get(index).copied()
}

fn text_matches(text: &str, delimiter: &str, case_insensitive: bool) -> Vec<(usize, usize)> {
    if !case_insensitive {
        return text
            .match_indices(delimiter)
            .map(|(start, matched)| (start, start + matched.len()))
            .collect();
    }
    let boundaries: Vec<usize> = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect();
    let delimiter_chars = delimiter.chars().count();
    let folded_delimiter = delimiter.to_lowercase();
    let mut matches = Vec::new();
    let mut index = 0;
    while index + delimiter_chars < boundaries.len() {
        let start = boundaries[index];
        let end = boundaries[index + delimiter_chars];
        if text[start..end].to_lowercase() == folded_delimiter {
            matches.push((start, end));
            index += delimiter_chars;
        } else {
            index += 1;
        }
    }
    matches
}

fn value_to_text(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let strict = match args.get(1) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) if number.trunc() == 0.0 => false,
            Ok(number) if number.trunc() == 1.0 => true,
            Ok(_) => return Value::Error(ErrorKind::Value),
            Err(kind) => return Value::Error(kind),
        },
        None => false,
    };
    let text =
        match super::text_common::value_to_text(&engine.eval_scalar(context, &args[0]), strict) {
            Ok(text) => text,
            Err(kind) => return Value::Error(kind),
        };
    engine.bounded_text(text)
}

fn cp1252_character(code: u8) -> Option<char> {
    let character = match code {
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8A => 'Š',
        0x8B => '‹',
        0x8C => 'Œ',
        0x8E => 'Ž',
        0x91 => '‘',
        0x92 => '’',
        0x93 => '“',
        0x94 => '”',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9A => 'š',
        0x9B => '›',
        0x9C => 'œ',
        0x9E => 'ž',
        0x9F => 'Ÿ',
        0x81 | 0x8D | 0x8F | 0x90 | 0x9D => return None,
        _ => char::from_u32(u32::from(code))?,
    };
    Some(character)
}

#[cfg(test)]
mod tests {
    use super::{cp1252_character, text_matches};

    #[test]
    fn case_insensitive_boundaries_keep_original_utf8_offsets() {
        assert_eq!(text_matches("Ä-one-ä", "ä", true), vec![(0, 2), (7, 9)]);
    }

    #[test]
    fn char_uses_the_deterministic_windows_1252_mapping() {
        assert_eq!(cp1252_character(65), Some('A'));
        assert_eq!(cp1252_character(128), Some('€'));
        assert_eq!(cp1252_character(129), None);
    }
}
