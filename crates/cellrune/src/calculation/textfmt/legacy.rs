//! The single-section subset of number-format codes `TEXT` formatted in 0.1.26.
//!
//! A formula that returned a value in 0.1.26 keeps that value, so the codes of this subset keep
//! its rules, including the ones Excel does not share: `#NUM!` from 2^53 rounded units, an integer
//! `0` kept unless decimals follow (`TEXT(0,"#")` is `0` where Excel shows nothing), and the
//! decimal point omitted when no decimal digit is shown (`TEXT(1,"0.#")` is `1` where Excel shows
//! `1.`). Every other code follows Excel's documented or measured output or is unsupported.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Prefix,
    Integer,
    Fraction,
    Suffix,
}

/// Whether the 0.1.26 subset formatted `format`: one section of `0` and `#` placeholders with at
/// most one decimal point, `,` only between an integer placeholder and the next placeholder, `%`,
/// and quoted, escaped, or plain literal text before or after the digits, without the letters of
/// date, time, `General`, and exponent codes or `?`, `*`, `_`, `[`, `]`, `;`, and `@`.
pub(super) fn accepts(format: &str) -> bool {
    let mut part = Part::Prefix;
    let mut has_placeholder = false;
    let mut characters = format.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '0' | '#' => match part {
                Part::Prefix => part = Part::Integer,
                Part::Integer | Part::Fraction => {}
                Part::Suffix => return false,
            },
            '.' => match part {
                Part::Prefix | Part::Integer => part = Part::Fraction,
                Part::Fraction | Part::Suffix => return false,
            },
            ',' if part == Part::Integer && matches!(characters.peek(), Some('0' | '#')) => {}
            ',' | ';' | '@' | '?' | '*' | '_' | '[' | ']' => return false,
            _ if "dmyhsegDMYHSEG".contains(character) => return false,
            '"' if !characters.by_ref().any(|inner| inner == '"') => return false,
            '\\' if characters.next().is_none() => return false,
            _ => {
                if part != Part::Prefix {
                    part = Part::Suffix;
                }
            }
        }
        has_placeholder |= matches!(character, '0' | '#');
    }
    has_placeholder
}

#[cfg(test)]
mod tests {
    use super::accepts;

    #[test]
    fn accepts_the_single_section_digit_codes_of_0_1_26() {
        for format in [
            "0",
            "#,##0.00",
            "$#,##0.00",
            "\"$\"#,##0",
            ".00",
            "0.0%",
            "%0",
            "0/2",
            "0 a/p",
            "\"000\"#",
            "#.0#",
        ] {
            assert!(accepts(format), "{format}");
        }
        for format in [
            "", "@", "\"x\"", "0;0", "0.0.0", "0\"x\"0", "0,", "#,", "?", "0_)", "*0", "[Red]0",
            "0.00E+00", "General", "yyyy", "0 e", "\"x", "0\\", "0\\.0",
        ] {
            assert!(!accepts(format), "{format}");
        }
    }
}
