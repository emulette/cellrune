//! Excel number-format codes: the engine behind `TEXT`, `DOLLAR`, and `FIXED`.
//!
//! A code has up to four `;`-separated sections for positive numbers, negative numbers, zero,
//! and text. Each section is a number layout (digit placeholders, scientific notation, or a
//! fraction), a date and time layout, `General`, a text layout around `@`, or plain literal text.
//! Constructs whose Excel output has not been established produce [`ErrorKind::Unsupported`]
//! rather than a guess.

mod datetime;
mod legacy;
mod lexer;
mod number;
#[cfg(test)]
mod tests;

use lexer::{RawSection, Token};

use super::value::{ErrorKind, number_to_general_text};
use crate::{DateSystem, Locale};

/// The workbook facts a format code reads besides the value.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FormatContext {
    pub(crate) date_system: DateSystem,
    /// The regional format whose month, weekday, and `AM`/`PM` names date sections show.
    pub(crate) locale: Locale,
}

/// Formats a number with `format`, as `TEXT` does.
pub(crate) fn format_number(
    value: f64,
    format: &str,
    context: FormatContext,
) -> Result<String, ErrorKind> {
    let sections = lexer::lex(format)?;
    let chosen = choose_number_section(&sections, value)?;
    let earlier_subset = legacy::accepts(format);
    render_number_section(&chosen.section.tokens, chosen, context, earlier_subset).map_err(|kind| {
        // Only the 0.1.26 subset reported a number past the engine's digits as `#NUM!`; Excel
        // still shows such a number, so other codes report it as unsupported, as `FIXED` does.
        if kind == ErrorKind::Num && !earlier_subset {
            ErrorKind::Unsupported
        } else {
            kind
        }
    })
}

/// Formats text with `format`: the fourth section, or a lone section holding `@`, places the text
/// at each `@`; a code without a text section shows the text unchanged.
pub(crate) fn format_text(text: &str, format: &str) -> Result<String, ErrorKind> {
    let sections = lexer::lex(format)?;
    let holds_text = |section: &RawSection| {
        section
            .tokens
            .iter()
            .any(|token| matches!(token, Token::TextValue))
    };
    // How a conditional code formats text has not been measured.
    if sections.iter().any(|section| section.condition.is_some()) {
        return Err(ErrorKind::Unsupported);
    }
    let section = match sections.as_slice() {
        [.., text_section] if sections.len() == 4 => text_section,
        [only] if holds_text(only) => only,
        _ if sections.iter().any(holds_text) => return Err(ErrorKind::Unsupported),
        _ => return Ok(text.to_owned()),
    };
    let mut output = String::new();
    for token in &section.tokens {
        match token {
            Token::TextValue => output.push_str(text),
            _ => output.push_str(&literal_text(token)?),
        }
    }
    Ok(output)
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
    match (numeric.get(1), numeric.len()) {
        (Some(second), _)
            if second
                .condition
                .is_none_or(|condition| condition.holds(value)) =>
        {
            Ok(chosen(1, value, false))
        }
        (Some(_), 3) => Ok(chosen(2, value, false)),
        _ => Err(ErrorKind::Unsupported),
    }
}

/// Renders the chosen section; `earlier_subset` tells whether the code is one the 0.1.26 subset
/// formatted, whose rules it keeps.
fn render_number_section(
    tokens: &[Token],
    chosen: ChosenSection<'_>,
    context: FormatContext,
    earlier_subset: bool,
) -> Result<String, ErrorKind> {
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
        return number::render(tokens, chosen.value, chosen.signed, earlier_subset);
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
/// number or writes it in scientific notation (`TEXT(PI(),"General")` is `3.141592654` and
/// `TEXT(1E+20,"General")` is `1E+20`), which is not reproduced.
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
