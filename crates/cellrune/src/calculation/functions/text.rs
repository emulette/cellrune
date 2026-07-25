use super::super::ast::Expr;
use super::super::coerce::{to_logical, to_text};
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::value::{ErrorKind, Value};
use super::util::{collect_argument_values, required_number, required_text};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    match name {
        "LEFT" => left(engine, context, args),
        "RIGHT" => right(engine, context, args),
        "MID" => mid(engine, context, args),
        "FIND" => find(engine, context, args, true),
        "SEARCH" => find(engine, context, args, false),
        "SUBSTITUTE" => substitute(engine, context, args),
        "LEN" => unary_text(
            engine,
            context,
            args,
            |text| text.chars().count().to_string(),
            true,
        ),
        "TRIM" => unary_text(engine, context, args, trim_excel, false),
        "UPPER" => unary_text(engine, context, args, |text| text.to_uppercase(), false),
        "PROPER" => unary_text(engine, context, args, proper, false),
        "EXACT" => exact(engine, context, args),
        "REPLACE" => replace(engine, context, args),
        "REPT" => rept(engine, context, args),
        "CONCAT" => concat(engine, context, args),
        "TEXTJOIN" => textjoin(engine, context, args),
        _ => Value::Error(ErrorKind::Unsupported),
    }
}

fn right(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let text = match required_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let count = match args.get(1) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) if number >= 0.0 => number.trunc() as usize,
            Ok(_) => return Value::Error(ErrorKind::Value),
            Err(kind) => return Value::Error(kind),
        },
        None => 1,
    };
    let character_count = text.chars().count();
    engine.bounded_text(
        text.chars()
            .skip(character_count.saturating_sub(count))
            .collect(),
    )
}

fn left(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let text = match required_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let count = match args.get(1) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) if number >= 0.0 => number.trunc() as usize,
            Ok(_) => return Value::Error(ErrorKind::Value),
            Err(kind) => return Value::Error(kind),
        },
        None => 1,
    };
    engine.bounded_text(text.chars().take(count).collect())
}

fn mid(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let text = match required_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let start = match required_number(engine, context, &args[1]) {
        Ok(number) if number >= 1.0 => number.trunc() as usize - 1,
        Ok(_) => return Value::Error(ErrorKind::Value),
        Err(kind) => return Value::Error(kind),
    };
    let count = match required_number(engine, context, &args[2]) {
        Ok(number) if number >= 0.0 => number.trunc() as usize,
        Ok(_) => return Value::Error(ErrorKind::Value),
        Err(kind) => return Value::Error(kind),
    };
    engine.bounded_text(text.chars().skip(start).take(count).collect())
}

fn find(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    case_sensitive: bool,
) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let needle = match required_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let haystack = match required_text(engine, context, &args[1]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    if let Err(kind) = engine.ensure_text_bytes(needle.len().max(haystack.len())) {
        return Value::Error(kind);
    }
    let start = match args.get(2) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) if number >= 1.0 => number.trunc() as usize - 1,
            Ok(_) => return Value::Error(ErrorKind::Value),
            Err(kind) => return Value::Error(kind),
        },
        None => 0,
    };
    let (source, origins) = comparison_form(&haystack, case_sensitive);
    let (target, _) = comparison_form(&needle, case_sensitive);
    let source_length = haystack.chars().count();
    if start > source_length {
        return Value::Error(ErrorKind::Value);
    }
    if target.is_empty() {
        return Value::Number((start + 1) as f64);
    }
    // `start` indexes the original text, and case folding may have changed the
    // character count before it, so translate it into the folded sequence.
    let folded_start = origins.partition_point(|origin| *origin < start);
    source[folded_start..]
        .windows(target.len())
        .position(|window| window == target)
        .map_or(Value::Error(ErrorKind::Value), |offset| {
            Value::Number((origins[folded_start + offset] + 1) as f64)
        })
}

/// Returns the characters to compare and, for each of them, the index of the
/// original character it came from.
///
/// Case folding is applied one character at a time so that the mapping stays
/// exact. Folding the whole string at once can emit a different number of
/// characters than it consumed, for example `İ` lowercasing to `i` followed by a
/// combining dot, which would shift every position reported after it.
fn comparison_form(text: &str, case_sensitive: bool) -> (Vec<char>, Vec<usize>) {
    let mut characters = Vec::new();
    let mut origins = Vec::new();
    for (index, character) in text.chars().enumerate() {
        if case_sensitive {
            characters.push(character);
            origins.push(index);
        } else {
            for lowered in character.to_lowercase() {
                characters.push(lowered);
                origins.push(index);
            }
        }
    }
    (characters, origins)
}

fn substitute(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 3 || args.len() > 4 {
        return Value::Error(ErrorKind::Value);
    }
    let text = match required_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let old = match required_text(engine, context, &args[1]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let new = match required_text(engine, context, &args[2]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    if old.is_empty() {
        return engine.bounded_text(text);
    }
    let Some(instance_expr) = args.get(3) else {
        let replacements = text.matches(&old).count();
        let Some(output_bytes) =
            replacement_output_bytes(text.len(), old.len(), new.len(), replacements)
        else {
            return Value::Error(ErrorKind::ResourceLimit(CalculationLimitKind::TextBytes));
        };
        if let Err(kind) = engine.ensure_text_bytes(output_bytes) {
            return Value::Error(kind);
        }
        return Value::Text(text.replace(&old, &new));
    };
    let instance = match required_number(engine, context, instance_expr) {
        Ok(number) if number >= 1.0 => number.trunc() as usize,
        Ok(_) => return Value::Error(ErrorKind::Value),
        Err(kind) => return Value::Error(kind),
    };
    let Some((start, _)) = text.match_indices(&old).nth(instance - 1) else {
        return engine.bounded_text(text);
    };
    let Some(output_bytes) = replacement_output_bytes(text.len(), old.len(), new.len(), 1) else {
        return Value::Error(ErrorKind::ResourceLimit(CalculationLimitKind::TextBytes));
    };
    if let Err(kind) = engine.ensure_text_bytes(output_bytes) {
        return Value::Error(kind);
    }
    let mut result = text;
    result.replace_range(start..start + old.len(), &new);
    Value::Text(result)
}

fn replacement_output_bytes(
    source_bytes: usize,
    old_bytes: usize,
    new_bytes: usize,
    replacements: usize,
) -> Option<usize> {
    source_bytes
        .checked_sub(old_bytes.checked_mul(replacements)?)?
        .checked_add(new_bytes.checked_mul(replacements)?)
}

fn unary_text(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: impl FnOnce(&str) -> String,
    numeric_result: bool,
) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    match required_text(engine, context, &args[0]) {
        Ok(text) if numeric_result => operation(&text)
            .parse::<f64>()
            .map(Value::Number)
            .unwrap_or(Value::Error(ErrorKind::Value)),
        Ok(text) => engine.bounded_text(operation(&text)),
        Err(kind) => Value::Error(kind),
    }
}

fn trim_excel(text: &str) -> String {
    text.split(' ')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn proper(text: &str) -> String {
    let mut capitalize = true;
    text.chars()
        .map(|character| {
            if character.is_alphanumeric() {
                let mapped = if capitalize {
                    character.to_uppercase().collect::<String>()
                } else {
                    character.to_lowercase().collect::<String>()
                };
                capitalize = false;
                mapped
            } else {
                capitalize = true;
                character.to_string()
            }
        })
        .collect()
}

fn exact(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    match (
        required_text(engine, context, &args[0]),
        required_text(engine, context, &args[1]),
    ) {
        (Ok(left), Ok(right)) => Value::Logical(left == right),
        (Err(kind), _) | (_, Err(kind)) => Value::Error(kind),
    }
}

fn replace(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 4 {
        return Value::Error(ErrorKind::Value);
    }
    let text = match required_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let start = match required_number(engine, context, &args[1]) {
        Ok(number) if number >= 1.0 => number.trunc() as usize - 1,
        Ok(_) => return Value::Error(ErrorKind::Value),
        Err(kind) => return Value::Error(kind),
    };
    let count = match required_number(engine, context, &args[2]) {
        Ok(number) if number >= 0.0 => number.trunc() as usize,
        Ok(_) => return Value::Error(ErrorKind::Value),
        Err(kind) => return Value::Error(kind),
    };
    let replacement = match required_text(engine, context, &args[3]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let char_count = text.chars().count();
    if start > char_count {
        return Value::Error(ErrorKind::Value);
    }
    let end = start.saturating_add(count).min(char_count);
    let byte_start = text
        .char_indices()
        .nth(start)
        .map_or(text.len(), |(index, _)| index);
    let byte_end = text
        .char_indices()
        .nth(end)
        .map_or(text.len(), |(index, _)| index);
    let Some(output_bytes) = text
        .len()
        .checked_sub(byte_end - byte_start)
        .and_then(|bytes| bytes.checked_add(replacement.len()))
    else {
        return Value::Error(ErrorKind::ResourceLimit(CalculationLimitKind::TextBytes));
    };
    if let Err(kind) = engine.ensure_text_bytes(output_bytes) {
        return Value::Error(kind);
    }
    let mut result = text;
    result.replace_range(byte_start..byte_end, &replacement);
    Value::Text(result)
}

fn rept(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let text = match required_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let count = match required_number(engine, context, &args[1]) {
        Ok(number) if number >= 0.0 => number.trunc() as usize,
        Ok(_) => return Value::Error(ErrorKind::Value),
        Err(kind) => return Value::Error(kind),
    };
    let Some(output_bytes) = text.len().checked_mul(count) else {
        return Value::Error(ErrorKind::Value);
    };
    if output_bytes > 32_767 {
        return Value::Error(ErrorKind::Value);
    }
    if let Err(kind) = engine.ensure_text_bytes(output_bytes) {
        return Value::Error(kind);
    }
    Value::Text(text.repeat(count))
}

fn concat(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    let values = match collect_argument_values(engine, context, args) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let mut result = String::new();
    for item in values {
        match to_text(&item.value) {
            Ok(text) => {
                let Some(output_bytes) = result.len().checked_add(text.len()) else {
                    return Value::Error(ErrorKind::ResourceLimit(CalculationLimitKind::TextBytes));
                };
                if let Err(kind) = engine.ensure_text_bytes(output_bytes) {
                    return Value::Error(kind);
                }
                result.push_str(&text);
            }
            Err(kind) => return Value::Error(kind),
        }
    }
    Value::Text(result)
}

fn textjoin(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 3 {
        return Value::Error(ErrorKind::Value);
    }
    let delimiter = match required_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let ignore_empty = match to_logical(&engine.eval_scalar(context, &args[1])) {
        Ok(logical) => logical,
        Err(kind) => return Value::Error(kind),
    };
    let values = match collect_argument_values(engine, context, &args[2..]) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let mut result = String::new();
    let mut has_part = false;
    for item in values {
        match to_text(&item.value) {
            Ok(text) if ignore_empty && text.is_empty() => {}
            Ok(text) => {
                let delimiter_bytes = if has_part { delimiter.len() } else { 0 };
                let Some(output_bytes) = result
                    .len()
                    .checked_add(delimiter_bytes)
                    .and_then(|bytes| bytes.checked_add(text.len()))
                else {
                    return Value::Error(ErrorKind::ResourceLimit(CalculationLimitKind::TextBytes));
                };
                if let Err(kind) = engine.ensure_text_bytes(output_bytes) {
                    return Value::Error(kind);
                }
                if has_part {
                    result.push_str(&delimiter);
                }
                result.push_str(&text);
                has_part = true;
            }
            Err(kind) => return Value::Error(kind),
        }
    }
    Value::Text(result)
}
