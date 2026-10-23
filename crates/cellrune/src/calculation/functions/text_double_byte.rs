//! The byte-counting text functions (`LENB`, `LEFTB`, `RIGHTB`, `MIDB`, `FINDB`, `SEARCHB`, and
//! `REPLACEB`) and the width conversions `ASC` and `DBCS`.
//!
//! Microsoft documents that the byte functions count two bytes for a double-byte character only
//! when the locale uses a double-byte character set, and count characters like their non-byte
//! forms otherwise. Korean is such a locale: its code page 949 stores ASCII in one byte and Hangul
//! syllables, Hangul compatibility jamo, full-width ASCII forms, and the ideographic space in two.
//! Other characters, and positions that would split a double-byte character, are unsupported
//! until Excel's handling of them is established.

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::kernel::TextFunction;
use super::text::{find_from, replace_characters};
use super::util::{required_number, required_text};
use crate::Locale;

/// The distance between an ASCII character and its full-width form.
const FULL_WIDTH_OFFSET: u32 = 0xFEE0;
const IDEOGRAPHIC_SPACE: char = '\u{3000}';

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: TextFunction,
    args: &[Expr],
) -> Value {
    let locale = engine.locale_profile().locale();
    let result = match function {
        TextFunction::LenB => length(engine, context, args, locale),
        TextFunction::LeftB => edge(engine, context, args, locale, false),
        TextFunction::RightB => edge(engine, context, args, locale, true),
        TextFunction::MidB => middle(engine, context, args, locale),
        TextFunction::FindB => find(engine, context, args, locale, true),
        TextFunction::SearchB => find(engine, context, args, locale, false),
        TextFunction::ReplaceB => replace(engine, context, args, locale),
        TextFunction::Asc => convert(engine, context, args, locale, half_width),
        TextFunction::Dbcs => convert(engine, context, args, locale, full_width),
        _ => unreachable!("the text dispatcher routes only double-byte functions here"),
    };
    result.unwrap_or_else(Value::Error)
}

/// The byte width of each character of `text`.
fn widths(text: &str, locale: Locale) -> Result<Vec<usize>, ErrorKind> {
    text.chars()
        .map(|character| match locale {
            Locale::EnUs => Ok(1),
            Locale::KoKr if character.is_ascii() => Ok(1),
            Locale::KoKr if is_korean_double_byte(character) => Ok(2),
            Locale::KoKr => Err(ErrorKind::Unsupported),
        })
        .collect()
}

fn is_korean_double_byte(character: char) -> bool {
    matches!(
        character,
        '\u{AC00}'..='\u{D7A3}' | '\u{3131}'..='\u{318E}' | '\u{FF01}'..='\u{FF5E}' | IDEOGRAPHIC_SPACE
    )
}

/// The count of leading characters within `bytes`; splitting a double-byte character is
/// unsupported.
fn leading_characters(widths: &[usize], bytes: usize) -> Result<usize, ErrorKind> {
    let mut used = 0_usize;
    for (index, width) in widths.iter().enumerate() {
        if used + width > bytes {
            return if used == bytes {
                Ok(index)
            } else {
                Err(ErrorKind::Unsupported)
            };
        }
        used += width;
    }
    Ok(widths.len())
}

/// The character index that starts at the zero-based byte `offset`, the character count when
/// `offset` is the end of the text, or `None` past the end; an offset inside a double-byte
/// character is unsupported.
fn character_at_byte(widths: &[usize], offset: usize) -> Result<Option<usize>, ErrorKind> {
    let mut used = 0_usize;
    for (index, width) in widths.iter().enumerate() {
        if used == offset {
            return Ok(Some(index));
        }
        used += width;
        if used > offset {
            return Err(ErrorKind::Unsupported);
        }
    }
    Ok((used == offset).then_some(widths.len()))
}

fn byte_count(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<usize, ErrorKind> {
    match required_number(engine, context, expr)? {
        number if number >= 0.0 => Ok(number.trunc() as usize),
        _ => Err(ErrorKind::Value),
    }
}

/// A one-based byte position as a zero-based byte offset.
fn byte_offset(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<usize, ErrorKind> {
    match required_number(engine, context, expr)? {
        number if number >= 1.0 => Ok(number.trunc() as usize - 1),
        _ => Err(ErrorKind::Value),
    }
}

fn length(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    locale: Locale,
) -> Result<Value, ErrorKind> {
    let [text] = args else {
        return Err(ErrorKind::Value);
    };
    let text = required_text(engine, context, text)?;
    Ok(Value::Number(
        widths(&text, locale)?.iter().sum::<usize>() as f64
    ))
}

/// `LEFTB` or, from the end, `RIGHTB`.
fn edge(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    locale: Locale,
    from_end: bool,
) -> Result<Value, ErrorKind> {
    if args.is_empty() || args.len() > 2 {
        return Err(ErrorKind::Value);
    }
    let text = required_text(engine, context, &args[0])?;
    let bytes = match args.get(1) {
        Some(expr) => byte_count(engine, context, expr)?,
        None => 1,
    };
    let mut widths = widths(&text, locale)?;
    if from_end {
        widths.reverse();
    }
    let count = leading_characters(&widths, bytes)?;
    let characters = text.chars();
    Ok(engine.bounded_text(if from_end {
        characters.skip(widths.len() - count).collect()
    } else {
        characters.take(count).collect()
    }))
}

fn middle(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    locale: Locale,
) -> Result<Value, ErrorKind> {
    let [text, start, bytes] = args else {
        return Err(ErrorKind::Value);
    };
    let text = required_text(engine, context, text)?;
    let start = byte_offset(engine, context, start)?;
    let bytes = byte_count(engine, context, bytes)?;
    let widths = widths(&text, locale)?;
    let Some(first) = character_at_byte(&widths, start)? else {
        return Ok(Value::Text(String::new()));
    };
    let count = leading_characters(&widths[first..], bytes)?;
    Ok(engine.bounded_text(text.chars().skip(first).take(count).collect()))
}

/// `FINDB` (case-sensitive) or `SEARCHB`.
fn find(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    locale: Locale,
    case_sensitive: bool,
) -> Result<Value, ErrorKind> {
    if args.len() < 2 || args.len() > 3 {
        return Err(ErrorKind::Value);
    }
    let needle = required_text(engine, context, &args[0])?;
    let haystack = required_text(engine, context, &args[1])?;
    engine.ensure_text_bytes(needle.len().max(haystack.len()))?;
    let start = match args.get(2) {
        Some(expr) => byte_offset(engine, context, expr)?,
        None => 0,
    };
    let widths = widths(&haystack, locale)?;
    let start = character_at_byte(&widths, start)?.ok_or(ErrorKind::Value)?;
    let index = find_from(engine, context, &needle, &haystack, start, case_sensitive)?;
    Ok(Value::Number(
        (widths[..index].iter().sum::<usize>() + 1) as f64,
    ))
}

fn replace(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    locale: Locale,
) -> Result<Value, ErrorKind> {
    let [text, start, bytes, replacement] = args else {
        return Err(ErrorKind::Value);
    };
    let text = required_text(engine, context, text)?;
    let start = byte_offset(engine, context, start)?;
    let bytes = byte_count(engine, context, bytes)?;
    let replacement = required_text(engine, context, replacement)?;
    let widths = widths(&text, locale)?;
    let first = character_at_byte(&widths, start)?.ok_or(ErrorKind::Value)?;
    let count = leading_characters(&widths[first..], bytes)?;
    Ok(replace_characters(engine, text, first, count, &replacement))
}

/// `ASC` or `DBCS`: converts each character of the text in a double-byte locale.
fn convert(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    locale: Locale,
    character: fn(char) -> Option<char>,
) -> Result<Value, ErrorKind> {
    let [text] = args else {
        return Err(ErrorKind::Value);
    };
    let text = required_text(engine, context, text)?;
    // Whether a locale without a double-byte character set converts has not been established.
    if locale != Locale::KoKr {
        return Err(ErrorKind::Unsupported);
    }
    let converted = text
        .chars()
        .map(|input| character(input).ok_or(ErrorKind::Unsupported))
        .collect::<Result<String, _>>()?;
    Ok(engine.bounded_text(converted))
}

/// `ASC`: full-width ASCII forms and the ideographic space become their one-byte forms; ASCII and
/// Hangul syllables, which have no other form, stay.
fn half_width(character: char) -> Option<char> {
    match character {
        IDEOGRAPHIC_SPACE => Some(' '),
        '\u{FF01}'..='\u{FF5E}' => char::from_u32(u32::from(character) - FULL_WIDTH_OFFSET),
        '\u{AC00}'..='\u{D7A3}' => Some(character),
        _ if character.is_ascii() => Some(character),
        _ => None,
    }
}

/// `DBCS`: printable ASCII and the space become their full-width forms; double-byte characters
/// stay.
fn full_width(character: char) -> Option<char> {
    match character {
        ' ' => Some(IDEOGRAPHIC_SPACE),
        '!'..='~' => char::from_u32(u32::from(character) + FULL_WIDTH_OFFSET),
        _ if is_korean_double_byte(character) => Some(character),
        _ => None,
    }
}
