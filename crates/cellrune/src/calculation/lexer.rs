use super::CalculationLimitKind;
use super::ast::NumberLiteral;
use super::error::ParseErrorCode;
use super::syntax::SourceSpan;
use super::value::ErrorKind;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(NumberLiteral),
    Str(String),
    Ident(String),
    QuotedSheet(String),
    StructuredRef(String),
    ExternalWorkbook(String),
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
    Hash,
    Intersection,
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

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SpannedToken {
    pub token: Token,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LexedFormula {
    pub tokens: Vec<SpannedToken>,
    pub trivia_spans: Vec<SourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub span: SourceSpan,
    pub code: ParseErrorCode,
    pub limit: Option<CalculationLimitKind>,
}

fn is_ident_start(character: char) -> bool {
    character.is_alphabetic() || matches!(character, '_' | '\\')
}

fn is_ident_continue(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | '.')
}

fn byte_at(offsets: &[usize], input_len: usize, character: usize) -> usize {
    offsets.get(character).copied().unwrap_or(input_len)
}

fn error(
    offsets: &[usize],
    input_len: usize,
    start: usize,
    end: usize,
    code: ParseErrorCode,
) -> LexError {
    LexError {
        span: SourceSpan::new(
            byte_at(offsets, input_len, start),
            byte_at(offsets, input_len, end),
        ),
        code,
        limit: None,
    }
}

fn consume_balanced_brackets(
    characters: &[char],
    offsets: &[usize],
    input_len: usize,
    open: usize,
) -> Result<usize, LexError> {
    let mut depth = 0_u32;
    let mut cursor = open;
    while cursor < characters.len() {
        match characters[cursor] {
            '\'' if cursor + 1 < characters.len() => cursor += 2,
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
            _ => cursor += 1,
        }
    }
    Err(error(
        offsets,
        input_len,
        open,
        characters.len(),
        ParseErrorCode::UnterminatedStructuredReference,
    ))
}

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
            if characters.get(cursor) == Some(&':') {
                cursor += 1;
                let Some(&first) = characters.get(cursor) else {
                    return false;
                };
                if !is_ident_start(first) {
                    return false;
                }
                cursor += 1;
                while cursor < characters.len() && is_ident_continue(characters[cursor]) {
                    cursor += 1;
                }
            }
            characters.get(cursor) == Some(&'!')
        }
        _ => false,
    }
}

fn may_end_reference(token: &Token) -> bool {
    matches!(
        token,
        Token::Ident(_)
            | Token::QuotedSheet(_)
            | Token::StructuredRef(_)
            | Token::Number(_)
            | Token::ErrorLit(ErrorKind::Ref)
            | Token::RParen
            | Token::Hash
    )
}

fn looks_like_cell_reference(ident: &str) -> bool {
    let letters_end = ident
        .char_indices()
        .find_map(|(offset, character)| (!character.is_ascii_alphabetic()).then_some(offset))
        .unwrap_or(ident.len());
    if letters_end == 0 || letters_end == ident.len() {
        return false;
    }
    let (column, row) = ident.split_at(letters_end);
    super::ast::column_number(column).is_some()
        && row
            .parse::<u32>()
            .is_ok_and(|row| row > 0 && row <= super::EXCEL_MAX_ROWS)
}

fn ends_whole_column_reference(tokens: &[SpannedToken]) -> bool {
    let Some(SpannedToken {
        token: Token::Ident(column),
        ..
    }) = tokens.last()
    else {
        return false;
    };
    if super::ast::column_number(column).is_none() {
        return false;
    }
    let mut cursor = tokens.len().saturating_sub(1);
    if cursor > 0 && tokens[cursor - 1].token == Token::Dollar {
        cursor -= 1;
    }
    cursor > 0 && tokens[cursor - 1].token == Token::Colon
}

fn may_start_reference(previous: &[SpannedToken], token: &Token) -> bool {
    let Some(previous_token) = previous.last().map(|token| &token.token) else {
        return false;
    };
    match token {
        Token::Ident(_)
        | Token::QuotedSheet(_)
        | Token::StructuredRef(_)
        | Token::ExternalWorkbook(_)
        | Token::Number(_)
        | Token::Dollar
        | Token::At
        | Token::ErrorLit(ErrorKind::Ref) => true,
        Token::LParen => {
            (match previous_token {
                Token::Ident(ident) => looks_like_cell_reference(ident),
                Token::Number(_) | Token::StructuredRef(_) | Token::RParen | Token::Hash => true,
                _ => false,
            }) || ends_whole_column_reference(previous)
        }
        _ => false,
    }
}

pub(super) fn lex_spanned(input: &str, max_tokens: u64) -> Result<LexedFormula, LexError> {
    let characters: Vec<char> = input.chars().collect();
    let offsets: Vec<usize> = input.char_indices().map(|(offset, _)| offset).collect();
    let mut tokens = Vec::new();
    let mut trivia_spans = Vec::new();
    let mut index = 0_usize;

    macro_rules! push_token {
        ($token:expr, $start:expr, $end:expr $(,)?) => {
            tokens.push(SpannedToken {
                token: $token,
                span: SourceSpan::new(
                    byte_at(&offsets, input.len(), $start),
                    byte_at(&offsets, input.len(), $end),
                ),
            })
        };
    }

    while index < characters.len() {
        let start = index;
        match characters[index] {
            character if character.is_whitespace() => {
                index += 1;
                while index < characters.len() && characters[index].is_whitespace() {
                    index += 1;
                }
                trivia_spans.push(SourceSpan::new(
                    byte_at(&offsets, input.len(), start),
                    byte_at(&offsets, input.len(), index),
                ));
            }
            '"' => {
                let mut text = String::new();
                index += 1;
                loop {
                    if index >= characters.len() {
                        return Err(error(
                            &offsets,
                            input.len(),
                            start,
                            characters.len(),
                            ParseErrorCode::UnterminatedString,
                        ));
                    }
                    if characters[index] == '"' {
                        if characters.get(index + 1) == Some(&'"') {
                            text.push('"');
                            index += 2;
                        } else {
                            index += 1;
                            break;
                        }
                    } else {
                        text.push(characters[index]);
                        index += 1;
                    }
                }
                push_token!(Token::Str(text), start, index);
            }
            '\'' => {
                let mut name = String::new();
                index += 1;
                loop {
                    if index >= characters.len() {
                        return Err(error(
                            &offsets,
                            input.len(),
                            start,
                            characters.len(),
                            ParseErrorCode::UnterminatedSheetName,
                        ));
                    }
                    if characters[index] == '\'' {
                        if characters.get(index + 1) == Some(&'\'') {
                            name.push('\'');
                            index += 2;
                        } else {
                            index += 1;
                            break;
                        }
                    } else {
                        name.push(characters[index]);
                        index += 1;
                    }
                }
                push_token!(Token::QuotedSheet(name), start, index);
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
                let rest = &input[byte_at(&offsets, input.len(), index)..];
                let upper = rest.to_ascii_uppercase();
                if let Some(literal) = ERROR_LITERALS
                    .iter()
                    .find(|literal| upper.starts_with(**literal))
                {
                    let kind = ErrorKind::parse(literal).expect("known error literal");
                    index += literal.chars().count();
                    push_token!(Token::ErrorLit(kind), start, index);
                } else if characters.get(index + 1).is_some_and(|c| c.is_alphabetic()) {
                    return Err(error(
                        &offsets,
                        input.len(),
                        start,
                        (index + 1).min(characters.len()),
                        ParseErrorCode::UnknownErrorLiteral,
                    ));
                } else {
                    index += 1;
                    push_token!(Token::Hash, start, index);
                }
            }
            '0'..='9' | '.' => {
                let mut cursor = index;
                let mut seen_digit = false;
                let mut seen_dot = false;
                while cursor < characters.len() {
                    if characters[cursor].is_ascii_digit() {
                        seen_digit = true;
                        cursor += 1;
                    } else if characters[cursor] == '.' && !seen_dot {
                        seen_dot = true;
                        cursor += 1;
                    } else {
                        break;
                    }
                }
                if !seen_digit {
                    return Err(error(
                        &offsets,
                        input.len(),
                        start,
                        start + 1,
                        ParseErrorCode::UnexpectedCharacter,
                    ));
                }
                if matches!(characters.get(cursor), Some('e' | 'E')) {
                    let mut exponent = cursor + 1;
                    if matches!(characters.get(exponent), Some('+' | '-')) {
                        exponent += 1;
                    }
                    let digits = exponent;
                    while characters.get(exponent).is_some_and(char::is_ascii_digit) {
                        exponent += 1;
                    }
                    if exponent > digits {
                        cursor = exponent;
                    }
                }
                let literal = &input
                    [byte_at(&offsets, input.len(), start)..byte_at(&offsets, input.len(), cursor)];
                let number = literal.parse::<f64>().map_err(|_| {
                    error(
                        &offsets,
                        input.len(),
                        start,
                        cursor,
                        ParseErrorCode::UnexpectedCharacter,
                    )
                })?;
                index = cursor;
                push_token!(
                    if number.is_finite() {
                        Token::Number(NumberLiteral::from_literal(number, literal))
                    } else {
                        Token::ErrorLit(ErrorKind::Num)
                    },
                    start,
                    index,
                );
            }
            '[' => {
                let close = consume_balanced_brackets(&characters, &offsets, input.len(), index)?;
                let raw = input
                    [byte_at(&offsets, input.len(), index)..byte_at(&offsets, input.len(), close)]
                    .to_owned();
                index = close;
                if external_link_follows(&characters, close) {
                    push_token!(Token::ExternalWorkbook(raw), start, index);
                } else {
                    push_token!(Token::StructuredRef(raw), start, index);
                }
            }
            character if is_ident_start(character) => {
                index += 1;
                while index < characters.len() && is_ident_continue(characters[index]) {
                    index += 1;
                }
                if characters.get(index) == Some(&'[') {
                    let close =
                        consume_balanced_brackets(&characters, &offsets, input.len(), index)?;
                    let raw = input[byte_at(&offsets, input.len(), start)
                        ..byte_at(&offsets, input.len(), close)]
                        .to_owned();
                    index = close;
                    push_token!(Token::StructuredRef(raw), start, index);
                } else {
                    let ident = input[byte_at(&offsets, input.len(), start)
                        ..byte_at(&offsets, input.len(), index)]
                        .to_owned();
                    push_token!(Token::Ident(ident), start, index);
                }
            }
            character => {
                let (token, width) = match character {
                    '$' => (Token::Dollar, 1),
                    '(' => (Token::LParen, 1),
                    ')' => (Token::RParen, 1),
                    '{' => (Token::LBrace, 1),
                    '}' => (Token::RBrace, 1),
                    ',' => (Token::Comma, 1),
                    ';' => (Token::Semicolon, 1),
                    ':' => (Token::Colon, 1),
                    '!' => (Token::Bang, 1),
                    '@' => (Token::At, 1),
                    '&' => (Token::Amp, 1),
                    '+' => (Token::Plus, 1),
                    '-' => (Token::Minus, 1),
                    '*' => (Token::Star, 1),
                    '/' => (Token::Slash, 1),
                    '^' => (Token::Caret, 1),
                    '%' => (Token::Percent, 1),
                    '=' => (Token::Eq, 1),
                    '<' if characters.get(index + 1) == Some(&'>') => (Token::Ne, 2),
                    '<' if characters.get(index + 1) == Some(&'=') => (Token::Le, 2),
                    '<' => (Token::Lt, 1),
                    '>' if characters.get(index + 1) == Some(&'=') => (Token::Ge, 2),
                    '>' => (Token::Gt, 1),
                    _ => {
                        return Err(error(
                            &offsets,
                            input.len(),
                            start,
                            start + 1,
                            ParseErrorCode::UnexpectedCharacter,
                        ));
                    }
                };
                index += width;
                push_token!(token, start, index);
            }
        }
        if tokens.len() as u64 > max_tokens {
            return Err(LexError {
                span: SourceSpan::empty(byte_at(&offsets, input.len(), index)),
                code: ParseErrorCode::UnexpectedToken,
                limit: Some(CalculationLimitKind::FormulaTokens),
            });
        }
    }

    let mut semantic: Vec<SpannedToken> = Vec::with_capacity(tokens.len());
    for token in tokens {
        if let Some(previous) = semantic.last()
            && previous.span.end < token.span.start
            && may_end_reference(&previous.token)
            && may_start_reference(&semantic, &token.token)
        {
            semantic.push(SpannedToken {
                token: Token::Intersection,
                span: SourceSpan::new(previous.span.end, token.span.start),
            });
        }
        semantic.push(token);
    }
    if semantic.len() as u64 > max_tokens {
        return Err(LexError {
            span: SourceSpan::empty(input.len()),
            code: ParseErrorCode::UnexpectedToken,
            limit: Some(CalculationLimitKind::FormulaTokens),
        });
    }
    Ok(LexedFormula {
        tokens: semantic,
        trivia_spans,
    })
}

#[cfg(test)]
mod tests {
    use super::{Token, lex_spanned};
    use crate::calculation::error::ParseErrorCode;
    use crate::calculation::syntax::SourceSpan;

    #[test]
    fn preserves_utf8_byte_spans_and_whitespace_trivia() {
        let lexed = lex_spanned("표[A]  B1", 100).expect("lexed");
        assert_eq!(lexed.tokens[0].span, SourceSpan::new(0, 6));
        assert_eq!(lexed.trivia_spans, [SourceSpan::new(6, 8)]);
        assert!(matches!(lexed.tokens[1].token, Token::Intersection));
    }

    #[test]
    fn external_workbooks_are_typed_tokens() {
        for formula in ["[1]Sheet1!A1", "[Book.xlsx]Sheet1!A1", "[1]!Name"] {
            let lexed = lex_spanned(formula, 100).expect(formula);
            assert!(matches!(
                lexed.tokens.first().map(|token| &token.token),
                Some(Token::ExternalWorkbook(_))
            ));
        }
    }

    #[test]
    fn bracket_sweep_never_panics() {
        const ALPHABET: [char; 7] = ['[', ']', '\'', '!', 'a', '#', '@'];
        let mut inputs = vec![String::new()];
        for _ in 0..6 {
            let mut next = Vec::new();
            for input in &inputs {
                for character in ALPHABET {
                    let mut value = input.clone();
                    value.push(character);
                    next.push(value);
                }
            }
            for input in &next {
                if let Err(error) = lex_spanned(input, 64) {
                    assert!(!error.code.as_str().is_empty());
                }
                let _ = super::super::parser::parse_formula_with_limits(
                    input,
                    crate::calculation::CalculationLimits::default(),
                );
            }
            inputs = next;
        }
    }

    #[test]
    fn unterminated_utf8_token_reports_byte_span() {
        let error = lex_spanned("\"표", 100).expect_err("unterminated");
        assert_eq!(error.code, ParseErrorCode::UnterminatedString);
        assert_eq!(error.span, SourceSpan::new(0, 4));
    }
}
