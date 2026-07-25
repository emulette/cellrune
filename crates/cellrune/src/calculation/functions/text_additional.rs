use super::super::ast::Expr;
use super::super::coerce::{to_logical, to_number, to_text};
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::textfmt::format_number;
use super::super::value::{ErrorKind, Value};
use super::util::{required_number, required_text};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    match name {
        "CHAR" => character(engine, context, args),
        "CLEAN" => clean(engine, context, args),
        "CONCATENATE" => concatenate(engine, context, args),
        "DOLLAR" => dollar(engine, context, args),
        "UNICHAR" => unichar(engine, context, args),
        "UNICODE" => unicode(engine, context, args),
        "TEXTBEFORE" => text_boundary(engine, context, args, false),
        "TEXTAFTER" => text_boundary(engine, context, args, true),
        "VALUE" => value(engine, context, args),
        "VALUETOTEXT" => value_to_text(engine, context, args),
        _ => Value::Error(ErrorKind::Unsupported),
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

fn dollar(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let number_value = engine.eval_scalar(context, &args[0]);
    let number = match to_number(&number_value) {
        Ok(number) => number,
        Err(ErrorKind::Value) => match &number_value {
            Value::Text(text) => match parse_value_number(text) {
                Some(number) => number,
                None => return Value::Error(ErrorKind::Value),
            },
            _ => return Value::Error(ErrorKind::Value),
        },
        Err(kind) => return Value::Error(kind),
    };
    let decimals = match args.get(1) {
        None | Some(Expr::Missing) => 2_i32,
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number.trunc() as i32,
            Err(kind) => return Value::Error(kind),
        },
    };
    if !(-308..=127).contains(&decimals) {
        return Value::Error(ErrorKind::Value);
    }
    let rounded = if decimals < 0 {
        let scale = 10_f64.powi(-decimals);
        (number / scale).round() * scale
    } else {
        number
    };
    if !rounded.is_finite() {
        return Value::Error(ErrorKind::Num);
    }
    let visible_decimals = decimals.max(0) as usize;
    let mut format = String::from("$#,##0");
    if visible_decimals > 0 {
        format.push('.');
        format.push_str(&"0".repeat(visible_decimals));
    }
    let formatted = match format_number(rounded.abs(), &format) {
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

fn value(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let text = match required_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    parse_value_number(&text).map_or(Value::Error(ErrorKind::Value), Value::Number)
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
    let value = engine.eval_scalar(context, &args[0]);
    if let Value::Error(kind) = value
        && kind.is_engine_issue()
    {
        return Value::Error(kind);
    }
    let text = match value {
        Value::Text(text) if strict => format!("\"{}\"", text.replace('"', "\"\"")),
        Value::Text(text) => text,
        Value::Error(kind) => kind.as_str().to_owned(),
        other => match to_text(&other) {
            Ok(text) => text,
            Err(kind) => return Value::Error(kind),
        },
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

fn parse_value_number(text: &str) -> Option<f64> {
    let mut value = text.trim();
    if value.is_empty() {
        return None;
    }
    let parenthesized = value.starts_with('(') && value.ends_with(')');
    if parenthesized {
        value = value.get(1..value.len().checked_sub(1)?)?.trim();
    } else if value.starts_with('(') || value.ends_with(')') {
        return None;
    }
    let percent = value.ends_with('%');
    if percent {
        value = value.get(..value.len().checked_sub(1)?)?.trim_end();
    }
    if value
        .chars()
        .next()
        .is_some_and(|character| "$₩€£¥".contains(character))
    {
        value = value.get(value.chars().next()?.len_utf8()..)?.trim_start();
    }
    let normalized = normalize_grouped_number(value)?;
    let mut number = normalized.parse::<f64>().ok()?;
    if !number.is_finite() {
        return None;
    }
    if parenthesized {
        number = -number;
    }
    if percent {
        number /= 100.0;
    }
    Some(number)
}

fn normalize_grouped_number(value: &str) -> Option<String> {
    if !value.contains(',') {
        return Some(value.to_owned());
    }
    let exponent_at = value
        .char_indices()
        .find_map(|(index, character)| matches!(character, 'e' | 'E').then_some(index));
    let (mantissa, exponent) = exponent_at.map_or((value, ""), |index| value.split_at(index));
    let (integer, fraction) = mantissa
        .split_once('.')
        .map_or((mantissa, None), |(integer, fraction)| {
            (integer, Some(fraction))
        });
    let (sign, unsigned_integer) = integer.strip_prefix('+').map_or_else(
        || {
            integer
                .strip_prefix('-')
                .map_or(("", integer), |digits| ("-", digits))
        },
        |digits| ("+", digits),
    );
    let groups = unsigned_integer.split(',').collect::<Vec<_>>();
    if groups.len() < 2
        || groups[0].is_empty()
        || groups[0].len() > 3
        || !groups
            .iter()
            .all(|group| group.bytes().all(|byte| byte.is_ascii_digit()))
        || groups[1..].iter().any(|group| group.len() != 3)
        || fraction.is_some_and(|fraction| {
            fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return None;
    }
    let mut normalized = String::from(sign);
    for group in groups {
        normalized.push_str(group);
    }
    if let Some(fraction) = fraction {
        normalized.push('.');
        normalized.push_str(fraction);
    }
    normalized.push_str(exponent);
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::{cp1252_character, parse_value_number, text_matches};

    #[test]
    fn case_insensitive_boundaries_keep_original_utf8_offsets() {
        assert_eq!(text_matches("Ä-one-ä", "ä", true), vec![(0, 2), (7, 9)]);
    }

    #[test]
    fn invariant_value_parser_handles_currency_grouping_percent_and_parentheses() {
        assert_eq!(parse_value_number("$1,000.50"), Some(1_000.5));
        assert_eq!(parse_value_number("12.5%"), Some(0.125));
        assert_eq!(parse_value_number("(₩1,234)"), Some(-1_234.0));
        assert_eq!(parse_value_number("1,00"), None);
        assert_eq!(parse_value_number("NaN"), None);
    }

    #[test]
    fn char_uses_the_deterministic_windows_1252_mapping() {
        assert_eq!(cp1252_character(65), Some('A'));
        assert_eq!(cp1252_character(128), Some('€'));
        assert_eq!(cp1252_character(129), None);
    }
}
