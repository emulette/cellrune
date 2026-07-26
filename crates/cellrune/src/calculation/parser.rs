use super::ast::{
    BinaryOp, CellRef, ColRef, Expr, RefBody, Reference, RowRef, SheetPrefix, UnaryOp,
    column_number,
};
use super::lexer::{LexError, Token, lex};
use super::{
    CalculationLimitKind, CalculationLimits, ERROR_PARSE_INVALID_REFERENCE,
    ERROR_PARSE_MISMATCHED_RANGE, ERROR_PARSE_UNEXPECTED_END, ERROR_PARSE_UNEXPECTED_TOKEN,
    EXCEL_MAX_ROWS,
};

/// Where a formula failed, in the only unit that is meaningful for that failure.
///
/// Lexing runs before any token stream exists, so a lex failure can only be located by character
/// offset. Parsing consumes tokens, so a parse failure is located by token index. Reporting both
/// as one number under a single label is wrong for one of the two cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorPosition {
    /// Zero-based character offset into the formula text.
    Character(usize),
    /// Zero-based index into the lexed token stream.
    Token(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub position: ErrorPosition,
    pub message: &'static str,
    pub limit: Option<CalculationLimitKind>,
}

impl From<LexError> for ParseError {
    fn from(error: LexError) -> Self {
        ParseError {
            position: ErrorPosition::Character(error.position),
            message: error.message,
            limit: error.limit,
        }
    }
}

pub(super) fn parse_formula_with_limits(
    input: &str,
    limits: CalculationLimits,
) -> Result<Expr, ParseError> {
    let tokens = lex(input, limits.max_formula_tokens())?;
    let mut parser = Parser {
        tokens,
        cursor: 0,
        parse_depth: 0,
        max_parse_depth: limits.max_formula_nesting_depth(),
    };
    // Excel accepts a leading `+` as a legacy formula-entry prefix. At the formula root it is a
    // no-op even when the expression returns text; nested `+` tokens retain unary coercion.
    parser.eat(&Token::Plus);
    let expr = parser.parse_expr(0)?;
    if parser.cursor != parser.tokens.len() {
        return Err(parser.error(ERROR_PARSE_UNEXPECTED_TOKEN));
    }
    validate_ast_limits(&expr, parser.cursor, limits)?;
    Ok(expr)
}

fn validate_ast_limits(
    expr: &Expr,
    token_index: usize,
    limits: CalculationLimits,
) -> Result<(), ParseError> {
    let mut nodes = 0_u64;
    let mut pending = vec![(expr, 1_u64)];
    while let Some((current, depth)) = pending.pop() {
        nodes += 1;
        if nodes > limits.max_formula_ast_nodes() {
            return Err(limit_error(
                token_index,
                CalculationLimitKind::FormulaAstNodes,
            ));
        }
        if depth > limits.max_formula_nesting_depth() {
            return Err(limit_error(
                token_index,
                CalculationLimitKind::FormulaNestingDepth,
            ));
        }
        match current {
            Expr::Call { args, .. } => {
                pending.extend(args.iter().map(|arg| (arg, depth + 1)));
            }
            Expr::ImplicitIntersection(operand)
            | Expr::Unary { operand, .. }
            | Expr::Paren(operand) => {
                pending.push((operand, depth + 1));
            }
            Expr::Binary { left, right, .. } => {
                pending.push((left, depth + 1));
                pending.push((right, depth + 1));
            }
            Expr::Range { start, end } => {
                pending.push((start, depth + 1));
                pending.push((end, depth + 1));
            }
            Expr::Array(rows) => {
                pending.extend(rows.iter().flatten().map(|item| (item, depth + 1)));
            }
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::Ref(_)
            | Expr::Name(_)
            | Expr::Missing => {}
        }
    }
    Ok(())
}

fn limit_error(token_index: usize, limit: CalculationLimitKind) -> ParseError {
    ParseError {
        position: ErrorPosition::Token(token_index),
        message: ERROR_PARSE_UNEXPECTED_TOKEN,
        limit: Some(limit),
    }
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
    parse_depth: u64,
    max_parse_depth: u64,
}

#[derive(Debug, Clone, Copy)]
enum Endpoint {
    Cell(CellRef),
    Column(ColRef),
    Row(RowRef),
}

fn split_cell_ident(ident: &str) -> Option<(String, Option<u32>)> {
    let letters: String = ident
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    if letters.is_empty() {
        return None;
    }
    let rest = &ident[letters.len()..];
    if rest.is_empty() {
        return Some((letters, None));
    }
    if !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let row = rest.parse::<u32>().ok()?;
    if row == 0 || row > EXCEL_MAX_ROWS {
        return None;
    }
    Some((letters, Some(row)))
}

impl Parser {
    fn error(&self, message: &'static str) -> ParseError {
        ParseError {
            position: ErrorPosition::Token(self.cursor),
            message,
            limit: None,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.cursor)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.cursor + offset)
    }

    /// Returns the end sheet name when the cursor sits on the start sheet of a
    /// `Start:End!` 3-D sheet-range prefix, without consuming any tokens.
    fn peek_sheet_range_end(&self) -> Option<String> {
        if self.peek_at(1) != Some(&Token::Colon) || self.peek_at(3) != Some(&Token::Bang) {
            return None;
        }
        match self.peek_at(2) {
            Some(Token::Ident(end) | Token::QuotedSheet(end)) => Some(end.clone()),
            _ => None,
        }
    }

    fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.cursor).cloned();
        if token.is_some() {
            self.cursor += 1;
        }
        token
    }

    fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: &Token) -> Result<(), ParseError> {
        if self.eat(token) {
            Ok(())
        } else {
            Err(self.error(ERROR_PARSE_UNEXPECTED_TOKEN))
        }
    }

    fn parse_expr(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
        if self.parse_depth >= self.max_parse_depth {
            return Err(limit_error(
                self.cursor,
                CalculationLimitKind::FormulaNestingDepth,
            ));
        }
        self.parse_depth += 1;
        let result = self.parse_expr_inner(min_bp);
        self.parse_depth -= 1;
        result
    }

    fn parse_expr_inner(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
        let mut left = self.parse_prefix()?;
        loop {
            if self.peek() == Some(&Token::Percent) && 60 >= min_bp {
                self.cursor += 1;
                left = Expr::Unary {
                    op: UnaryOp::Percent,
                    operand: Box::new(left),
                };
                continue;
            }
            if self.peek() == Some(&Token::Colon) && 80 >= min_bp {
                self.cursor += 1;
                let end = self.parse_expr(81)?;
                left = Expr::Range {
                    start: Box::new(left),
                    end: Box::new(end),
                };
                continue;
            }
            let (op, bp) = match self.peek() {
                Some(Token::Eq) => (BinaryOp::Eq, 10),
                Some(Token::Ne) => (BinaryOp::Ne, 10),
                Some(Token::Lt) => (BinaryOp::Lt, 10),
                Some(Token::Le) => (BinaryOp::Le, 10),
                Some(Token::Gt) => (BinaryOp::Gt, 10),
                Some(Token::Ge) => (BinaryOp::Ge, 10),
                Some(Token::Amp) => (BinaryOp::Concat, 20),
                Some(Token::Plus) => (BinaryOp::Add, 30),
                Some(Token::Minus) => (BinaryOp::Subtract, 30),
                Some(Token::Star) => (BinaryOp::Multiply, 40),
                Some(Token::Slash) => (BinaryOp::Divide, 40),
                Some(Token::Caret) => (BinaryOp::Power, 50),
                _ => break,
            };
            if bp < min_bp {
                break;
            }
            self.cursor += 1;
            let right = self.parse_expr(bp + 1)?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_prefix(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            Some(Token::Minus) => {
                self.cursor += 1;
                let operand = self.parse_expr(70)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Negate,
                    operand: Box::new(operand),
                })
            }
            Some(Token::Plus) => {
                self.cursor += 1;
                let operand = self.parse_expr(70)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Plus,
                    operand: Box::new(operand),
                })
            }
            Some(Token::At) => {
                self.cursor += 1;
                let operand = self.parse_expr(70)?;
                Ok(Expr::ImplicitIntersection(Box::new(operand)))
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().cloned() {
            None => Err(self.error(ERROR_PARSE_UNEXPECTED_END)),
            Some(Token::Number(number)) => {
                if self.peek_at(1) == Some(&Token::Colon)
                    && matches!(self.peek_at(2), Some(Token::Number(_) | Token::Dollar))
                {
                    let body = self.parse_ref_body()?;
                    return Ok(Expr::Ref(Reference { sheet: None, body }));
                }
                self.cursor += 1;
                Ok(Expr::Number(number))
            }
            Some(Token::Str(text)) => {
                self.cursor += 1;
                Ok(Expr::Text(text))
            }
            Some(Token::ErrorLit(kind)) => {
                self.cursor += 1;
                Ok(Expr::ErrorLit(kind))
            }
            Some(Token::LParen) => {
                self.cursor += 1;
                let inner = self.parse_expr(0)?;
                self.expect(&Token::RParen)?;
                Ok(Expr::Paren(Box::new(inner)))
            }
            Some(Token::LBrace) => self.parse_array_literal(),
            Some(Token::QuotedSheet(name)) => {
                if let Some(end_name) = self.peek_sheet_range_end() {
                    self.cursor += 4;
                    let body = self.parse_ref_body()?;
                    return Ok(Expr::Ref(Reference {
                        sheet: Some(SheetPrefix {
                            name,
                            end_name: Some(end_name),
                            quoted: true,
                        }),
                        body,
                    }));
                }
                self.cursor += 1;
                self.expect(&Token::Bang)?;
                // Sheet names cannot contain ':', so a quoted colon marks a 3-D sheet range
                // stored as one token ('Sheet1:Sheet3'!A1). An external-workbook prefix is the
                // exception: Excel writes the saved path into that token, so
                // 'C:\Reports\[Q1.xlsx]Sheet1'!A1 carries a drive colon that names no end sheet.
                // Splitting it would drop the drive letter from the reported detail and claim a
                // 3-D range the formula does not contain.
                let external_workbook = name.contains('[');
                let (name, end_name) = match name.split_once(':') {
                    Some((start, end)) if !external_workbook => {
                        (start.to_owned(), Some(end.to_owned()))
                    }
                    _ => (name, None),
                };
                let body = self.parse_ref_body()?;
                Ok(Expr::Ref(Reference {
                    sheet: Some(SheetPrefix {
                        name,
                        end_name,
                        quoted: true,
                    }),
                    body,
                }))
            }
            Some(Token::Dollar) => {
                let body = self.parse_ref_body()?;
                Ok(Expr::Ref(Reference { sheet: None, body }))
            }
            Some(Token::Ident(ident)) => {
                if self.peek_at(1) == Some(&Token::Bang) {
                    self.cursor += 2;
                    let body = self.parse_ref_body()?;
                    return Ok(Expr::Ref(Reference {
                        sheet: Some(SheetPrefix {
                            name: ident,
                            end_name: None,
                            quoted: false,
                        }),
                        body,
                    }));
                }
                if let Some(end_name) = self.peek_sheet_range_end() {
                    self.cursor += 4;
                    let body = self.parse_ref_body()?;
                    return Ok(Expr::Ref(Reference {
                        sheet: Some(SheetPrefix {
                            name: ident,
                            end_name: Some(end_name),
                            quoted: false,
                        }),
                        body,
                    }));
                }
                if self.peek_at(1) == Some(&Token::LParen) {
                    self.cursor += 2;
                    let args = self.parse_call_args()?;
                    return if ident.eq_ignore_ascii_case("_xlfn.SINGLE") && args.len() == 1 {
                        Ok(Expr::ImplicitIntersection(Box::new(
                            args.into_iter().next().expect("single argument checked"),
                        )))
                    } else {
                        Ok(Expr::Call { name: ident, args })
                    };
                }
                let snapshot = self.cursor;
                if let Ok(body) = self.parse_ref_body() {
                    return Ok(Expr::Ref(Reference { sheet: None, body }));
                }
                self.cursor = snapshot;
                self.cursor += 1;
                if ident.eq_ignore_ascii_case("TRUE") {
                    Ok(Expr::Logical(true))
                } else if ident.eq_ignore_ascii_case("FALSE") {
                    Ok(Expr::Logical(false))
                } else {
                    Ok(Expr::Name(ident))
                }
            }
            Some(_) => Err(self.error(ERROR_PARSE_UNEXPECTED_TOKEN)),
        }
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut args = Vec::new();
        if self.eat(&Token::RParen) {
            return Ok(args);
        }
        loop {
            if self.peek() == Some(&Token::Comma) {
                args.push(Expr::Missing);
                self.cursor += 1;
                continue;
            }
            if self.peek() == Some(&Token::RParen) {
                args.push(Expr::Missing);
                self.cursor += 1;
                return Ok(args);
            }
            let arg = self.parse_expr(0)?;
            args.push(arg);
            if self.eat(&Token::Comma) {
                continue;
            }
            self.expect(&Token::RParen)?;
            return Ok(args);
        }
    }

    fn parse_array_literal(&mut self) -> Result<Expr, ParseError> {
        self.expect(&Token::LBrace)?;
        let mut rows = Vec::new();
        let mut current = Vec::new();
        loop {
            let element = self.parse_expr(0)?;
            current.push(element);
            match self.advance() {
                Some(Token::Comma) => {}
                Some(Token::Semicolon) => {
                    rows.push(std::mem::take(&mut current));
                }
                Some(Token::RBrace) => {
                    rows.push(current);
                    return Ok(Expr::Array(rows));
                }
                _ => return Err(self.error(ERROR_PARSE_UNEXPECTED_TOKEN)),
            }
        }
    }

    fn parse_endpoint(&mut self) -> Result<Endpoint, ParseError> {
        let leading_absolute = self.eat(&Token::Dollar);
        match self.peek().cloned() {
            Some(Token::Number(number)) => {
                let number = number.value();
                let row = number as u32;
                if number.fract() != 0.0 || row == 0 || row > EXCEL_MAX_ROWS {
                    return Err(self.error(ERROR_PARSE_INVALID_REFERENCE));
                }
                self.cursor += 1;
                Ok(Endpoint::Row(RowRef {
                    row,
                    absolute: leading_absolute,
                }))
            }
            Some(Token::Ident(ident)) => {
                let Some((letters, row)) = split_cell_ident(&ident) else {
                    return Err(self.error(ERROR_PARSE_INVALID_REFERENCE));
                };
                let Some(column) = column_number(&letters) else {
                    return Err(self.error(ERROR_PARSE_INVALID_REFERENCE));
                };
                self.cursor += 1;
                if let Some(row) = row {
                    return Ok(Endpoint::Cell(CellRef {
                        column,
                        row,
                        column_absolute: leading_absolute,
                        row_absolute: false,
                    }));
                }
                if self.peek() == Some(&Token::Dollar)
                    && matches!(self.peek_at(1), Some(Token::Number(_)))
                {
                    self.cursor += 1;
                    let Some(Token::Number(number)) = self.advance() else {
                        return Err(self.error(ERROR_PARSE_INVALID_REFERENCE));
                    };
                    let number = number.value();
                    let row = number as u32;
                    if number.fract() != 0.0 || row == 0 || row > EXCEL_MAX_ROWS {
                        return Err(self.error(ERROR_PARSE_INVALID_REFERENCE));
                    }
                    return Ok(Endpoint::Cell(CellRef {
                        column,
                        row,
                        column_absolute: leading_absolute,
                        row_absolute: true,
                    }));
                }
                Ok(Endpoint::Column(ColRef {
                    column,
                    absolute: leading_absolute,
                }))
            }
            _ => Err(self.error(ERROR_PARSE_INVALID_REFERENCE)),
        }
    }

    fn parse_ref_body(&mut self) -> Result<RefBody, ParseError> {
        let first = self.parse_endpoint()?;
        let colon_cursor = self.cursor;
        if !self.eat(&Token::Colon) {
            return match first {
                Endpoint::Cell(cell) => Ok(RefBody::Cell(cell)),
                _ => Err(self.error(ERROR_PARSE_INVALID_REFERENCE)),
            };
        }
        let second = match self.parse_endpoint() {
            Ok(second) => second,
            Err(error) => {
                if let Endpoint::Cell(cell) = first {
                    self.cursor = colon_cursor;
                    return Ok(RefBody::Cell(cell));
                }
                return Err(error);
            }
        };
        match (first, second) {
            (Endpoint::Cell(start), Endpoint::Cell(end)) => Ok(RefBody::Area(start, end)),
            (Endpoint::Column(start), Endpoint::Column(end)) => Ok(RefBody::Columns(start, end)),
            (Endpoint::Row(start), Endpoint::Row(end)) => Ok(RefBody::Rows(start, end)),
            _ => Err(self.error(ERROR_PARSE_MISMATCHED_RANGE)),
        }
    }
}
