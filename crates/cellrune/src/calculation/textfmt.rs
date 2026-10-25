//! Excel number-format codes: the engine behind `TEXT`, `DOLLAR`, and `FIXED`.
//!
//! A code has up to four `;`-separated sections for positive numbers, negative numbers, zero,
//! and text. Each section is a number layout (digit placeholders, scientific notation, or a
//! fraction), a date and time layout, `General`, a text layout around `@`, or plain literal text.
//! Constructs whose Excel output has not been established produce [`ErrorKind::Unsupported`]
//! rather than a guess.

mod datetime;
mod lexer;
mod number;
#[cfg(test)]
mod tests;

use lexer::{Placeholder, RawSection, Token};

use super::value::{ErrorKind, number_to_general_text};
use crate::{DateSystem, Locale};

/// The workbook facts a format code reads besides the value.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FormatContext {
    pub(crate) date_system: DateSystem,
    /// The regional format whose month, weekday, and `AM`/`PM` names date sections show.
    pub(crate) locale: Locale,
    /// Whether the code formats a `TEXT`, `DOLLAR`, or `FIXED` result rather than a cell's
    /// display text. `TEXT` shows a `*` fill as nothing (`TEXT(1.5,"*x0")` is `2`; excel-measure
    /// RESULTS.md, section B), where a cell repeats the character across its column, whose width
    /// is not an input, so the display text of a code with a fill is unsupported. `TEXT` also
    /// rejects `General` for text (`TEXT("abc","General")` is `#VALUE!`, Mac Excel 16.114), which
    /// a cell shows as written.
    pub(crate) formula: bool,
    /// Whether numbers round by their fifteen-significant-digit decimal, as Excel shows them,
    /// rather than by their binary value.
    pub(crate) decimal: bool,
}

/// The most UTF-16 code units a formatted result holds: `TEXT(1E+254,"0")` shows 255 digits,
/// `TEXT(1E+255,"0")` and `TEXT(-1E+254,"0")` are `#VALUE!`, and `TEXT(REPT("😀",127),"@")` holds
/// 254 units while 128 emoji are `#VALUE!` (Mac Excel 16.114).
const MAX_FORMATTED_UNITS: usize = 255;

/// The most decimal placeholders a section holds: `TEXT(1,"0."&REPT("0",128))` is `#VALUE!` in
/// any section and in scientific notation (Mac Excel 16.114).
const MAX_DECIMALS: usize = 127;

/// Rejects a result longer than Excel formats.
fn within_limit(text: String) -> Result<String, ErrorKind> {
    if text.encode_utf16().count() > MAX_FORMATTED_UNITS {
        Err(ErrorKind::Value)
    } else {
        Ok(text)
    }
}

/// Formats a number with `format`, as `TEXT` does.
pub(crate) fn format_number(
    value: f64,
    format: &str,
    context: FormatContext,
) -> Result<String, ErrorKind> {
    let sections = lexer::lex(format)?;
    check_code(&sections)?;
    let chosen = choose_number_section(&sections, value)?;
    // A number past what the engine renders, such as a fraction of 2^53 units, is unsupported.
    let text = render_number_section(&chosen.section.tokens, chosen, context).map_err(|kind| {
        if kind == ErrorKind::Num {
            ErrorKind::Unsupported
        } else {
            kind
        }
    })?;
    within_limit(text)
}

/// Formats text with `format`, as Mac Excel 16.114 formats text and logical values in `TEXT`: the
/// fourth section places the text at each `@` and rejects digits, `@` in another section, or a
/// condition; with fewer sections, a last section holding `@` places the text and an earlier one
/// rejects the code (`TEXT("abc","0;""x""@")` is `xabc`, `TEXT("abc","@;0")` is `#VALUE!`); a code
/// without a text section shows the text unchanged, whatever conditions its number sections hold.
pub(crate) fn format_text(
    text: &str,
    format: &str,
    context: FormatContext,
) -> Result<String, ErrorKind> {
    let sections = lexer::lex(format)?;
    check_code(&sections)?;
    if context.formula
        && sections
            .iter()
            .any(|section| section.tokens.contains(&Token::General))
    {
        return Err(ErrorKind::Value);
    }
    let holds_text = |section: &RawSection| section.tokens.contains(&Token::TextValue);
    let (earlier, last) = sections.split_at(sections.len() - 1);
    let section = &last[0];
    if earlier.iter().any(holds_text) {
        return Err(ErrorKind::Value);
    }
    if sections.len() == 4 {
        if section.condition.is_some() || shows_digits(&section.tokens) {
            return Err(ErrorKind::Value);
        }
    } else if !holds_text(section) {
        return Ok(text.to_owned());
    } else if section.condition.is_some() || shows_digits(&section.tokens) {
        // A conditional text section and digits beside `@` have not been measured.
        return Err(ErrorKind::Unsupported);
    }
    let mut output = String::new();
    for token in &section.tokens {
        match token {
            Token::TextValue => output.push_str(text),
            Token::Fill if context.formula => {}
            _ => output.push_str(&literal_text(token)?),
        }
    }
    within_limit(output)
}

/// Rejects a code Mac Excel 16.114 answers with `#VALUE!` whichever section formats the value:
/// a letter it reads as no code (`TEXT(5,"0;0n")`, `TEXT(1,"nn")`), more than 127 decimal
/// placeholders, or digit placeholders beside date, time, era, or weekday codes
/// (`TEXT(5,"0;0 d")`, `TEXT("abc","0 g")`, `TEXT(0.5,"ss.##")`).
fn check_code(sections: &[RawSection]) -> Result<(), ErrorKind> {
    let mut unsupported = false;
    for section in sections {
        let digits = shows_digits(&section.tokens);
        if decimal_places(&section.tokens) > MAX_DECIMALS {
            return Err(ErrorKind::Value);
        }
        for token in &section.tokens {
            let checked = match token {
                Token::Invalid(_) | Token::Letters { letter: 'n', .. } => Err(ErrorKind::Value),
                Token::Elapsed { .. } if digits => Err(ErrorKind::Value),
                Token::Letters { letter, count, .. } if digits => {
                    number::letters_beside_digits(*letter, *count)
                }
                _ => Ok(()),
            };
            match checked {
                Err(ErrorKind::Value) => return Err(ErrorKind::Value),
                Err(_) => unsupported = true,
                Ok(()) => {}
            }
        }
    }
    if unsupported {
        Err(ErrorKind::Unsupported)
    } else {
        Ok(())
    }
}

/// The digit placeholders between a section's first decimal point and its exponent.
fn decimal_places(tokens: &[Token]) -> usize {
    tokens
        .iter()
        .skip_while(|token| **token != Token::DecimalPoint)
        .take_while(|token| !matches!(token, Token::Exponent { .. }))
        .filter(|token| matches!(token, Token::Placeholder(_)))
        .count()
}

/// Whether a section shows digits: a digit placeholder other than the `0`s of `ss.00`.
fn shows_digits(tokens: &[Token]) -> bool {
    let mut sub_second = false;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::DecimalPoint => {
                sub_second = index.checked_sub(1).is_some_and(|before| {
                    matches!(
                        tokens[before],
                        Token::Letters { letter: 's', .. } | Token::Elapsed { unit: 's', .. }
                    )
                });
            }
            Token::Placeholder(Placeholder::Zero) if sub_second => {}
            Token::Placeholder(_) => return true,
            _ => sub_second = false,
        }
    }
    false
}

/// The section a number selected and the value that section renders.
#[derive(Debug, Clone, Copy)]
struct ChosenSection<'a> {
    section: &'a RawSection,
    value: f64,
    /// Whether a negative value shows its own minus sign: true only when one section formats
    /// every number.
    signed: bool,
    locale_id: Option<u32>,
}

fn choose_number_section(
    sections: &[RawSection],
    value: f64,
) -> Result<ChosenSection<'_>, ErrorKind> {
    // A fourth section formats text only.
    let numeric = &sections[..sections.len().min(3)];
    let chosen = |index: usize, value: f64, signed: bool| ChosenSection {
        section: &numeric[index],
        value,
        signed,
        locale_id: numeric[index].locale_id,
    };
    if numeric.iter().all(|section| section.condition.is_none()) {
        return Ok(match numeric.len() {
            1 => chosen(0, value, true),
            2 if value < 0.0 => chosen(1, -value, false),
            2 => chosen(0, value, false),
            _ if value > 0.0 => chosen(0, value, false),
            _ if value < 0.0 => chosen(1, -value, false),
            _ => chosen(2, value, false),
        });
    }
    // How a conditional section shows a negative number has not been measured.
    if value < 0.0
        || numeric[0].condition.is_none()
        || numeric
            .get(2)
            .is_some_and(|section| section.condition.is_some())
    {
        return Err(ErrorKind::Unsupported);
    }
    let first = numeric[0].condition.expect("checked above");
    if first.holds(value) {
        return Ok(chosen(0, value, false));
    }
    match numeric.get(1).map(|second| second.condition) {
        // Microsoft documents a second section without a condition as the one for the numbers the
        // first condition rejects, as in `[<=9999999]###-####;(###) ###-####`.
        Some(None) if numeric.len() == 2 => Ok(chosen(1, value, false)),
        Some(Some(condition)) if condition.holds(value) => Ok(chosen(1, value, false)),
        Some(Some(_)) if numeric.len() == 3 => Ok(chosen(2, value, false)),
        // Which numbers a third section takes after a second section without a condition has
        // not been established.
        _ => Err(ErrorKind::Unsupported),
    }
}

fn render_number_section(
    tokens: &[Token],
    chosen: ChosenSection<'_>,
    context: FormatContext,
) -> Result<String, ErrorKind> {
    let without_fill;
    let tokens = if context.formula && tokens.contains(&Token::Fill) {
        without_fill = tokens
            .iter()
            .filter(|token| **token != Token::Fill)
            .cloned()
            .collect::<Vec<_>>();
        &without_fill
    } else {
        tokens
    };
    if tokens.is_empty() {
        return Ok(String::new());
    }
    if tokens.iter().any(|token| matches!(token, Token::TextValue)) {
        return Err(ErrorKind::Unsupported);
    }
    if datetime::is_date_section(tokens) {
        if chosen.value < 0.0 {
            return Err(ErrorKind::Unsupported);
        }
        return datetime::render(tokens, chosen.value, chosen.locale_id, context);
    }
    if tokens.iter().any(|token| matches!(token, Token::General)) {
        return render_general(tokens, chosen);
    }
    if tokens
        .iter()
        .any(|token| matches!(token, Token::Placeholder(_)))
    {
        return number::render(tokens, chosen.value, chosen.signed, context.decimal);
    }
    render_literal_section(tokens, chosen)
}

/// Renders a section of literal text, such as `" "` or `"n/a"`.
fn render_literal_section(
    tokens: &[Token],
    chosen: ChosenSection<'_>,
) -> Result<String, ErrorKind> {
    // Whether a lone literal section shows a minus sign has not been measured.
    if chosen.signed && chosen.value < 0.0 {
        return Err(ErrorKind::Unsupported);
    }
    let mut output = String::new();
    for token in tokens {
        output.push_str(&literal_text(token)?);
    }
    Ok(output)
}

/// The most characters, minus sign included, a number shows in `General`. Excel rounds a longer
/// number or writes it in scientific notation (`TEXT(PI(),"General")` is `3.141592654`), which is
/// not reproduced.
const GENERAL_WIDTH: usize = 11;

/// The smallest magnitude `General` shows as plain decimals here; where Excel switches smaller
/// numbers to scientific notation has not been established.
const GENERAL_SMALLEST_DECIMAL: f64 = 0.001;

fn render_general(tokens: &[Token], chosen: ChosenSection<'_>) -> Result<String, ErrorKind> {
    let mut output = String::new();
    let negative = chosen.signed && chosen.value < 0.0;
    if negative {
        output.push('-');
    }
    let mut seen_general = false;
    for token in tokens {
        match token {
            Token::General if !seen_general => {
                seen_general = true;
                output.push_str(&general_number(chosen.value.abs(), negative)?);
            }
            _ => output.push_str(&literal_text(token)?),
        }
    }
    Ok(output)
}

/// The text `General` shows for `magnitude`: only numbers Excel shows as written, without
/// rounding or an exponent, are supported.
fn general_number(magnitude: f64, negative: bool) -> Result<String, ErrorKind> {
    let text = number_to_general_text(magnitude);
    if text.chars().count() + usize::from(negative) > GENERAL_WIDTH
        || (magnitude != 0.0 && magnitude < GENERAL_SMALLEST_DECIMAL)
    {
        return Err(ErrorKind::Unsupported);
    }
    Ok(text)
}

/// Returns the text a token shows in a section without digit placeholders. An unquoted letter is
/// unsupported, because some hosts read letters other than the documented codes as codes: Excel
/// Online stores `z1900ro` and Mac Excel 2021 `z3333o` for the section `zero`.
fn literal_text(token: &Token) -> Result<String, ErrorKind> {
    match token {
        Token::Literal(text) => Ok(text.clone()),
        Token::Skip => Ok(" ".to_owned()),
        Token::Comma => Ok(",".to_owned()),
        Token::DecimalPoint => Ok(".".to_owned()),
        Token::Slash => Ok("/".to_owned()),
        _ => Err(ErrorKind::Unsupported),
    }
}
