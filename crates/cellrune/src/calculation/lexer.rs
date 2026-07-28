use super::CalculationLimitKind;
use super::ast::NumberLiteral;
use super::error::{ERROR_LEX_EXTERNAL_REFERENCE, ERROR_LEX_UNTERMINATED_STRUCTURED_REF};
use super::value::ErrorKind;
use super::{
    ERROR_LEX_UNEXPECTED_CHARACTER, ERROR_LEX_UNKNOWN_ERROR_LITERAL,
    ERROR_LEX_UNTERMINATED_SHEET_NAME, ERROR_LEX_UNTERMINATED_STRING,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(NumberLiteral),
    Str(String),
    Ident(String),
    QuotedSheet(String),
    /// One opaque structured table reference, original spelling preserved, such as
    /// `Table1[Amount]` or `[@Amount]`. 0.1.10 replaces this with a typed selector model.
    StructuredRef(String),
    ErrorLit(ErrorKind),
    Dollar,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Semicolon,
    Colon,
    Bang,
    At,
    Amp,
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Percent,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub position: usize,
    pub message: &'static str,
    pub limit: Option<CalculationLimitKind>,
}

/// Consumes one balanced bracket group starting at `open` (which must index a `[`) and
/// returns the position just past the matching `]`.
///
/// Inside structured-reference brackets a single quote escapes the next character
/// (`'[`, `']`, `'#`, `''`), so escaped brackets do not affect the balance.
fn consume_balanced_brackets(characters: &[char], open: usize) -> Result<usize, LexError> {
    let mut depth = 0_usize;
    let mut cursor = open;
    while cursor < characters.len() {
        match characters[cursor] {
            '\'' => {
                if cursor + 1 >= characters.len() {
                    break;
                }
                cursor += 2;
            }
            '[' => {
                depth += 1;
                cursor += 1;
            }
            ']' => {
                depth = depth.saturating_sub(1);
                cursor += 1;
                if depth == 0 {
                    return Ok(cursor);
                }
            }
            _ => {
                cursor += 1;
            }
        }
    }
    Err(LexError {
        position: open,
        message: ERROR_LEX_UNTERMINATED_STRUCTURED_REF,
        limit: None,
    })
}

/// Reports whether the text after a closing bracket continues as an external workbook
/// link: `!` directly (`[1]!Name`), or a sheet name followed by `!` (`[1]Sheet1!A1`,
/// `[Book1.xlsx]'My Sheet'!A1`).
fn external_link_follows(characters: &[char], mut cursor: usize) -> bool {
    match characters.get(cursor) {
        Some('!') => true,
        Some('\'') => {
            cursor += 1;
            while cursor < characters.len() {
                if characters[cursor] == '\'' {
                    if characters.get(cursor + 1) == Some(&'\'') {
                        cursor += 2;
                    } else {
                        cursor += 1;
                        break;
                    }
                } else {
                    cursor += 1;
                }
            }
            characters.get(cursor) == Some(&'!')
        }
        Some(&character) if is_ident_start(character) => {
            while cursor < characters.len() && is_ident_continue(characters[cursor]) {
                cursor += 1;
            }
            characters.get(cursor) == Some(&'!')
        }
        _ => false,
    }
}

fn is_ident_start(character: char) -> bool {
    character.is_alphabetic() || character == '_'
}

fn is_ident_continue(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '.'
}

pub fn lex(input: &str, max_tokens: u64) -> Result<Vec<Token>, LexError> {
    let characters: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0_usize;
    while index < characters.len() {
        let character = characters[index];
        match character {
            ' ' | '\t' | '\r' | '\n' => {
                index += 1;
            }
            '"' => {
                let mut text = String::new();
                let mut cursor = index + 1;
                loop {
                    if cursor >= characters.len() {
                        return Err(LexError {
                            position: index,
                            message: ERROR_LEX_UNTERMINATED_STRING,
                            limit: None,
                        });
                    }
                    if characters[cursor] == '"' {
                        if cursor + 1 < characters.len() && characters[cursor + 1] == '"' {
                            text.push('"');
                            cursor += 2;
                        } else {
                            cursor += 1;
                            break;
                        }
                    } else {
                        text.push(characters[cursor]);
                        cursor += 1;
                    }
                }
                tokens.push(Token::Str(text));
                index = cursor;
            }
            '\'' => {
                let mut name = String::new();
                let mut cursor = index + 1;
                loop {
                    if cursor >= characters.len() {
                        return Err(LexError {
                            position: index,
                            message: ERROR_LEX_UNTERMINATED_SHEET_NAME,
                            limit: None,
                        });
                    }
                    if characters[cursor] == '\'' {
                        if cursor + 1 < characters.len() && characters[cursor + 1] == '\'' {
                            name.push('\'');
                            cursor += 2;
                        } else {
                            cursor += 1;
                            break;
                        }
                    } else {
                        name.push(characters[cursor]);
                        cursor += 1;
                    }
                }
                tokens.push(Token::QuotedSheet(name));
                index = cursor;
            }
            '#' => {
                const ERROR_LITERALS: [&str; 10] = [
                    "#UNSUPPORTED!",
                    "#SPILL!",
                    "#DIV/0!",
                    "#VALUE!",
                    "#NAME?",
                    "#NULL!",
                    "#CALC!",
                    "#NUM!",
                    "#REF!",
                    "#N/A",
                ];
                let rest: String = characters[index..].iter().collect();
                let upper = rest.to_ascii_uppercase();
                let matched = ERROR_LITERALS
                    .iter()
                    .find(|literal| upper.starts_with(*literal));
                match matched
                    .and_then(|literal| ErrorKind::parse(literal).map(|k| (k, literal.len())))
                {
                    Some((kind, length)) => {
                        tokens.push(Token::ErrorLit(kind));
                        index += length;
                    }
                    None => {
                        return Err(LexError {
                            position: index,
                            message: ERROR_LEX_UNKNOWN_ERROR_LITERAL,
                            limit: None,
                        });
                    }
                }
            }
            '0'..='9' | '.' => {
                let mut cursor = index;
                let mut seen_digit = false;
                let mut seen_dot = false;
                while cursor < characters.len() {
                    let candidate = characters[cursor];
                    if candidate.is_ascii_digit() {
                        seen_digit = true;
                        cursor += 1;
                    } else if candidate == '.' && !seen_dot {
                        seen_dot = true;
                        cursor += 1;
                    } else {
                        break;
                    }
                }
                if !seen_digit {
                    return Err(LexError {
                        position: index,
                        message: ERROR_LEX_UNEXPECTED_CHARACTER,
                        limit: None,
                    });
                }
                if cursor < characters.len()
                    && (characters[cursor] == 'e' || characters[cursor] == 'E')
                {
                    let mut exponent_cursor = cursor + 1;
                    if exponent_cursor < characters.len()
                        && (characters[exponent_cursor] == '+'
                            || characters[exponent_cursor] == '-')
                    {
                        exponent_cursor += 1;
                    }
                    if exponent_cursor < characters.len()
                        && characters[exponent_cursor].is_ascii_digit()
                    {
                        while exponent_cursor < characters.len()
                            && characters[exponent_cursor].is_ascii_digit()
                        {
                            exponent_cursor += 1;
                        }
                        cursor = exponent_cursor;
                    }
                }
                let literal: String = characters[index..cursor].iter().collect();
                let number = literal.parse::<f64>().map_err(|_| LexError {
                    position: index,
                    message: ERROR_LEX_UNEXPECTED_CHARACTER,
                    limit: None,
                })?;
                tokens.push(if number.is_finite() {
                    Token::Number(NumberLiteral::from_literal(number, &literal))
                } else {
                    Token::ErrorLit(ErrorKind::Num)
                });
                index = cursor;
            }
            '$' => {
                tokens.push(Token::Dollar);
                index += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                index += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                index += 1;
            }
            '{' => {
                tokens.push(Token::LBrace);
                index += 1;
            }
            '}' => {
                tokens.push(Token::RBrace);
                index += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                index += 1;
            }
            ';' => {
                tokens.push(Token::Semicolon);
                index += 1;
            }
            ':' => {
                tokens.push(Token::Colon);
                index += 1;
            }
            '!' => {
                tokens.push(Token::Bang);
                index += 1;
            }
            '@' => {
                tokens.push(Token::At);
                index += 1;
            }
            '&' => {
                tokens.push(Token::Amp);
                index += 1;
            }
            '+' => {
                tokens.push(Token::Plus);
                index += 1;
            }
            '-' => {
                tokens.push(Token::Minus);
                index += 1;
            }
            '*' => {
                tokens.push(Token::Star);
                index += 1;
            }
            '/' => {
                tokens.push(Token::Slash);
                index += 1;
            }
            '^' => {
                tokens.push(Token::Caret);
                index += 1;
            }
            '%' => {
                tokens.push(Token::Percent);
                index += 1;
            }
            '=' => {
                tokens.push(Token::Eq);
                index += 1;
            }
            '<' => {
                if index + 1 < characters.len() && characters[index + 1] == '>' {
                    tokens.push(Token::Ne);
                    index += 2;
                } else if index + 1 < characters.len() && characters[index + 1] == '=' {
                    tokens.push(Token::Le);
                    index += 2;
                } else {
                    tokens.push(Token::Lt);
                    index += 1;
                }
            }
            '>' => {
                if index + 1 < characters.len() && characters[index + 1] == '=' {
                    tokens.push(Token::Ge);
                    index += 2;
                } else {
                    tokens.push(Token::Gt);
                    index += 1;
                }
            }
            '[' => {
                let close = consume_balanced_brackets(&characters, index)?;
                // The leading-bracket spellings are ambiguous: `[1]Sheet1!A1` and
                // `[Book1.xlsx]Sheet1!A1` open external-workbook references while
                // `[@Amount]` and `[#Data]` are table-internal structured references.
                // The bracket contents cannot tell them apart - `[Book1.xlsx]` looks like
                // `[Amount]` - but what FOLLOWS the closing bracket can: an external link
                // always continues with `!` or a sheet name followed by `!`, and a
                // structured reference never does.
                if external_link_follows(&characters, close) {
                    return Err(LexError {
                        position: index,
                        message: ERROR_LEX_EXTERNAL_REFERENCE,
                        limit: None,
                    });
                }
                let text: String = characters[index..close].iter().collect();
                tokens.push(Token::StructuredRef(text));
                index = close;
            }
            _ if is_ident_start(character) => {
                let mut cursor = index + 1;
                while cursor < characters.len() && is_ident_continue(characters[cursor]) {
                    cursor += 1;
                }
                if cursor < characters.len() && characters[cursor] == '[' {
                    let close = consume_balanced_brackets(&characters, cursor)?;
                    let text: String = characters[index..close].iter().collect();
                    tokens.push(Token::StructuredRef(text));
                    index = close;
                } else {
                    let ident: String = characters[index..cursor].iter().collect();
                    tokens.push(Token::Ident(ident));
                    index = cursor;
                }
            }
            _ => {
                return Err(LexError {
                    position: index,
                    message: ERROR_LEX_UNEXPECTED_CHARACTER,
                    limit: None,
                });
            }
        }
        if tokens.len() as u64 > max_tokens {
            return Err(LexError {
                position: index,
                message: ERROR_LEX_UNEXPECTED_CHARACTER,
                limit: Some(CalculationLimitKind::FormulaTokens),
            });
        }
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::{Token, lex};
    use crate::calculation::error::{
        ERROR_LEX_EXTERNAL_REFERENCE, ERROR_LEX_UNEXPECTED_CHARACTER,
        ERROR_LEX_UNKNOWN_ERROR_LITERAL, ERROR_LEX_UNTERMINATED_SHEET_NAME,
        ERROR_LEX_UNTERMINATED_STRING, ERROR_LEX_UNTERMINATED_STRUCTURED_REF,
    };

    const SHARED_MESSAGES: [&str; 6] = [
        ERROR_LEX_UNEXPECTED_CHARACTER,
        ERROR_LEX_UNTERMINATED_STRING,
        ERROR_LEX_UNTERMINATED_SHEET_NAME,
        ERROR_LEX_UNKNOWN_ERROR_LITERAL,
        ERROR_LEX_UNTERMINATED_STRUCTURED_REF,
        ERROR_LEX_EXTERNAL_REFERENCE,
    ];

    #[test]
    fn structured_reference_spellings_lex_as_one_opaque_token() {
        for spelling in [
            "Table1[Amount]",
            "[@Amount]",
            "[#Data]",
            "Table1[[#Headers],[Amount]]",
            "Table1[[Col1]:[Col2]]",
            "Table1['[odd']name]",
            "[]",
        ] {
            let tokens = lex(spelling, 1_000).expect(spelling);
            assert_eq!(
                tokens,
                vec![Token::StructuredRef(spelling.to_owned())],
                "{spelling}"
            );
        }
    }

    #[test]
    fn external_workbook_spellings_stay_lex_errors() {
        for spelling in [
            "[1]Sheet1!A1",
            "[Book1.xlsx]Sheet1!A1",
            "[1]!Name",
            "[x]'My Sheet'!A1",
        ] {
            let error = lex(spelling, 1_000).expect_err(spelling);
            assert_eq!(error.message, ERROR_LEX_EXTERNAL_REFERENCE, "{spelling}");
        }
    }

    #[test]
    fn exhaustive_bracket_alphabet_sweep_never_panics_and_uses_shared_messages() {
        // Every string up to length 6 over a bracket-heavy alphabet. This is the
        // committed, deterministic form of the "fuzz must not panic on arbitrary
        // bracket text" acceptance criterion; the formula_calculation fuzz target
        // extends the same property to arbitrary UTF-8.
        const ALPHABET: [char; 7] = ['[', ']', '\'', '!', 'a', '#', '@'];
        let mut inputs = vec![String::new()];
        for _ in 0..6 {
            let mut next = Vec::with_capacity(inputs.len() * ALPHABET.len());
            for input in &inputs {
                for character in ALPHABET {
                    let mut extended = input.clone();
                    extended.push(character);
                    next.push(extended);
                }
            }
            for input in &next {
                if let Err(error) = lex(input, 64) {
                    assert!(
                        SHARED_MESSAGES.contains(&error.message),
                        "unshared lex message {:?} for {input:?}",
                        error.message
                    );
                }
            }
            inputs = next;
        }
    }
}
