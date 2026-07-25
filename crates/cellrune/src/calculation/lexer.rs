use super::CalculationLimitKind;
use super::value::ErrorKind;
use super::{
    ERROR_LEX_UNEXPECTED_CHARACTER, ERROR_LEX_UNKNOWN_ERROR_LITERAL,
    ERROR_LEX_UNTERMINATED_SHEET_NAME, ERROR_LEX_UNTERMINATED_STRING,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(f64),
    Str(String),
    Ident(String),
    QuotedSheet(String),
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
                    Token::Number(number)
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
            _ if is_ident_start(character) => {
                let mut cursor = index + 1;
                while cursor < characters.len() && is_ident_continue(characters[cursor]) {
                    cursor += 1;
                }
                let ident: String = characters[index..cursor].iter().collect();
                tokens.push(Token::Ident(ident));
                index = cursor;
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
