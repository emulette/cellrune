//! Splits an Excel number-format code into sections of tokens.
//!
//! The lexer only recognizes the syntax; whether a token is a date code, a literal, or an
//! unsupported construct depends on the section it ends up in and is decided by
//! [`super::section`].

use super::super::value::ErrorKind;

/// A digit placeholder: `0` pads with zeros, `#` shows only significant digits, and `?` pads
/// with spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Placeholder {
    Zero,
    Hash,
    Question,
}

/// How an `AM/PM` or `A/P` marker writes the half of the day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Meridiem {
    /// `AM/PM` written in capitals, which shows `AM` or `PM`.
    Full,
    /// `A/P` in any case, which shows the letter as written.
    Letter { morning: char, evening: char },
    /// `AM/PM` written in another case; its output case is unmeasured.
    UnmeasuredCase,
}

/// A comparison that selects a section, such as `[<=100]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Condition {
    pub(super) operator: Comparison,
    pub(super) operand: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Comparison {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
    NotEqual,
}

impl Condition {
    pub(super) fn holds(self, value: f64) -> bool {
        match self.operator {
            Comparison::Less => value < self.operand,
            Comparison::LessOrEqual => value <= self.operand,
            Comparison::Greater => value > self.operand,
            Comparison::GreaterOrEqual => value >= self.operand,
            Comparison::Equal => value == self.operand,
            Comparison::NotEqual => value != self.operand,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Token {
    /// Text shown as written: quoted text, an escaped character, or a plain literal character.
    Literal(String),
    Placeholder(Placeholder),
    DecimalPoint,
    Comma,
    Percent,
    /// `E+`, `E-`, `e+`, or `e-`.
    Exponent {
        uppercase: bool,
        always_signed: bool,
    },
    Slash,
    /// `@`, the place of a text value.
    TextValue,
    /// `_x`, a space as wide as `x`.
    Skip,
    /// `*x`, which repeats `x` to fill the cell.
    Fill,
    General,
    /// A run of one letter, compared without case; `text` keeps the letters as written.
    Letters {
        letter: char,
        count: usize,
        text: String,
    },
    Meridiem(Meridiem),
    /// `[h]`, `[mm]`, or `[ss]`: elapsed hours, minutes, or seconds.
    Elapsed {
        unit: char,
        width: usize,
    },
}

/// One `;`-separated part of a format code.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct RawSection {
    pub(super) tokens: Vec<Token>,
    pub(super) condition: Option<Condition>,
    /// The locale identifier of a `[$-xxxx]` or `[$symbol-xxxx]` code.
    pub(super) locale_id: Option<u32>,
}

const COLOR_NAMES: [&str; 8] = [
    "black", "blue", "cyan", "green", "magenta", "red", "white", "yellow",
];

/// Splits `format` into at most four sections.
pub(super) fn lex(format: &str) -> Result<Vec<RawSection>, ErrorKind> {
    let characters = format.chars().collect::<Vec<_>>();
    let mut sections = vec![RawSection::default()];
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        index += 1;
        let section = sections
            .last_mut()
            .expect("lexing always has a current section");
        match character {
            ';' => {
                if sections.len() == 4 {
                    return Err(ErrorKind::Unsupported);
                }
                sections.push(RawSection::default());
            }
            '"' => {
                let mut literal = String::new();
                loop {
                    let Some(&inner) = characters.get(index) else {
                        return Err(ErrorKind::Unsupported);
                    };
                    index += 1;
                    if inner == '"' {
                        break;
                    }
                    literal.push(inner);
                }
                section.tokens.push(Token::Literal(literal));
            }
            '\\' | '_' | '*' => {
                let Some(&next) = characters.get(index) else {
                    return Err(ErrorKind::Unsupported);
                };
                index += 1;
                section.tokens.push(match character {
                    '\\' => Token::Literal(next.to_string()),
                    '_' => Token::Skip,
                    _ => Token::Fill,
                });
            }
            '[' => {
                let start = index;
                while characters.get(index).is_some_and(|inner| *inner != ']') {
                    index += 1;
                }
                if index == characters.len() {
                    return Err(ErrorKind::Unsupported);
                }
                let content = characters[start..index].iter().collect::<String>();
                index += 1;
                lex_bracket(&content, section)?;
            }
            '0' => section.tokens.push(Token::Placeholder(Placeholder::Zero)),
            '#' => section.tokens.push(Token::Placeholder(Placeholder::Hash)),
            '?' => section
                .tokens
                .push(Token::Placeholder(Placeholder::Question)),
            '.' => section.tokens.push(Token::DecimalPoint),
            ',' => section.tokens.push(Token::Comma),
            '%' => section.tokens.push(Token::Percent),
            '/' => section.tokens.push(Token::Slash),
            '@' => section.tokens.push(Token::TextValue),
            'E' | 'e' if matches!(characters.get(index), Some('+' | '-')) => {
                section.tokens.push(Token::Exponent {
                    uppercase: character == 'E',
                    always_signed: characters[index] == '+',
                });
                index += 1;
            }
            _ if starts_with_ignoring_case(&characters[index - 1..], "general") => {
                index += "general".len() - 1;
                section.tokens.push(Token::General);
            }
            _ if starts_with_ignoring_case(&characters[index - 1..], "am/pm") => {
                let written = characters[index - 1..index + 4].iter().collect::<String>();
                index += "am/pm".len() - 1;
                section.tokens.push(Token::Meridiem(if written == "AM/PM" {
                    Meridiem::Full
                } else {
                    Meridiem::UnmeasuredCase
                }));
            }
            _ if starts_with_ignoring_case(&characters[index - 1..], "a/p") => {
                section.tokens.push(Token::Meridiem(Meridiem::Letter {
                    morning: character,
                    evening: characters[index + 1],
                }));
                index += "a/p".len() - 1;
            }
            _ if character.is_alphabetic() => {
                let mut text = character.to_string();
                while characters
                    .get(index)
                    .is_some_and(|next| same_letter(*next, character))
                {
                    text.push(characters[index]);
                    index += 1;
                }
                section.tokens.push(Token::Letters {
                    letter: lowercase(character),
                    count: text.chars().count(),
                    text,
                });
            }
            _ => section.tokens.push(Token::Literal(character.to_string())),
        }
    }
    Ok(sections)
}

fn lex_bracket(content: &str, section: &mut RawSection) -> Result<(), ErrorKind> {
    if let Some(currency) = content.strip_prefix('$') {
        let (symbol, locale) = match currency.split_once('-') {
            Some((symbol, locale)) => {
                let locale_id =
                    u32::from_str_radix(locale, 16).map_err(|_| ErrorKind::Unsupported)?;
                // Values from 0xF000 select a system date or time format, and values above
                // 0xFFFF also select a calendar or a numeral system.
                if locale_id >= 0xF000 || section.locale_id.is_some() {
                    return Err(ErrorKind::Unsupported);
                }
                (symbol, Some(locale_id))
            }
            None => (currency, None),
        };
        section.locale_id = section.locale_id.or(locale);
        if !symbol.is_empty() {
            section.tokens.push(Token::Literal(symbol.to_owned()));
        }
        return Ok(());
    }
    let mut letters = content.chars();
    if let Some(unit) = letters.next().map(lowercase)
        && matches!(unit, 'h' | 'm' | 's')
        && letters.all(|letter| lowercase(letter) == unit)
    {
        section.tokens.push(Token::Elapsed {
            unit,
            width: content.chars().count(),
        });
        return Ok(());
    }
    if COLOR_NAMES
        .iter()
        .any(|name| content.eq_ignore_ascii_case(name))
    {
        return Ok(());
    }
    let condition = parse_condition(content).ok_or(ErrorKind::Unsupported)?;
    if section.condition.is_some() {
        return Err(ErrorKind::Unsupported);
    }
    section.condition = Some(condition);
    Ok(())
}

fn parse_condition(content: &str) -> Option<Condition> {
    let (operator, operand) = [
        ("<=", Comparison::LessOrEqual),
        (">=", Comparison::GreaterOrEqual),
        ("<>", Comparison::NotEqual),
        ("<", Comparison::Less),
        (">", Comparison::Greater),
        ("=", Comparison::Equal),
    ]
    .into_iter()
    .find_map(|(prefix, operator)| {
        content
            .strip_prefix(prefix)
            .map(|operand| (operator, operand))
    })?;
    let operand = operand.trim().parse::<f64>().ok()?;
    operand
        .is_finite()
        .then_some(Condition { operator, operand })
}

fn starts_with_ignoring_case(characters: &[char], expected: &str) -> bool {
    characters.len() >= expected.len()
        && characters
            .iter()
            .zip(expected.chars())
            .all(|(actual, expected)| lowercase(*actual) == expected)
}

fn same_letter(left: char, right: char) -> bool {
    lowercase(left) == lowercase(right)
}

fn lowercase(character: char) -> char {
    character.to_lowercase().next().unwrap_or(character)
}
