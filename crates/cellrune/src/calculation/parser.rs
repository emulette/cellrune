use std::sync::Arc;

use super::ast::{
    BinaryOp, CellRef, ColRef, Expr, ExternalReferenceTarget, ExternalWorkbookReference, RefBody,
    Reference, RowRef, SheetPrefix, UnaryOp, column_number,
};
use super::error::ParseErrorCode;
use super::functions::{builtin_callable, function_argument_is_callable, storage_builtin_callable};
use super::lexer::{LexError, SpannedToken, Token, lex_spanned};
use super::limits::SAFE_FORMULA_NESTING_DEPTH;
use super::structured_reference::parse_structured_reference;
use super::syntax::{
    FormulaSourceMap, NodeSpanTree, ParsedFormula, PendingNodeSource, SourceComponent,
    SourceComponentKind, SourceSpan,
};
use super::value::ErrorKind;
use super::{CalculationLimitKind, CalculationLimits, EXCEL_MAX_ROWS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub code: ParseErrorCode,
    pub span: SourceSpan,
    pub limit: Option<CalculationLimitKind>,
}

impl From<LexError> for ParseError {
    fn from(error: LexError) -> Self {
        Self {
            code: error.code,
            span: error.span,
            limit: error.limit,
        }
    }
}

#[derive(Debug)]
struct SpannedExpr {
    expr: Expr,
    span: SourceSpan,
}

impl SpannedExpr {
    fn boxed(self) -> Box<Expr> {
        Box::new(self.expr)
    }
}

pub(super) fn parse_formula_with_limits(
    input: &str,
    limits: CalculationLimits,
) -> Result<ParsedFormula, ParseError> {
    if input.len() as u64 > limits.max_formula_source_bytes() {
        return Err(limit_error(
            SourceSpan::new(0, input.len()),
            CalculationLimitKind::FormulaSourceBytes,
        ));
    }
    let lexed = lex_spanned(input, limits.max_formula_tokens())?;
    let token_spans = lexed.tokens.iter().map(|token| token.span).collect();
    let mut parser = Parser {
        input,
        tokens: lexed.tokens,
        cursor: 0,
        parse_depth: 0,
        max_parse_depth: limits
            .max_formula_nesting_depth()
            .min(SAFE_FORMULA_NESTING_DEPTH),
        max_ast_depth: limits
            .max_formula_nesting_depth()
            .min(SAFE_FORMULA_NESTING_DEPTH),
        max_ast_nodes: limits.max_formula_ast_nodes(),
        input_len: input.len(),
        node_sources: Vec::new(),
    };
    let leading_prefix = parser.eat(&Token::Plus);
    let mut root = parser.parse_expr(0, true)?;
    if let Some(prefix) = leading_prefix {
        root.span = prefix.merge(root.span);
        if let Some(root_source) = parser.node_sources.last_mut() {
            root_source.span = root.span;
        }
    }
    if parser.cursor != parser.tokens.len() {
        return Err(parser.error(ParseErrorCode::UnexpectedToken));
    }
    validate_ast_limits(&root.expr, root.span, limits)?;
    let mut node_sources = parser.node_sources.iter().rev();
    let node_tree =
        NodeSpanTree::from_postorder(&root.expr, &mut node_sources).ok_or(ParseError {
            code: ParseErrorCode::UnexpectedToken,
            span: root.span,
            limit: None,
        })?;
    if node_sources.next().is_some() {
        return Err(ParseError {
            code: ParseErrorCode::UnexpectedToken,
            span: root.span,
            limit: None,
        });
    }
    Ok(ParsedFormula::new(
        Arc::from(input),
        root.expr,
        FormulaSourceMap::new(token_spans, lexed.trivia_spans, node_tree),
    ))
}

fn validate_ast_limits(
    expr: &Expr,
    span: SourceSpan,
    limits: CalculationLimits,
) -> Result<(), ParseError> {
    let mut nodes = 0_u64;
    let mut pending = vec![(expr, 1_u64)];
    while let Some((current, depth)) = pending.pop() {
        nodes += 1;
        if nodes > limits.max_formula_ast_nodes() {
            return Err(limit_error(span, CalculationLimitKind::FormulaAstNodes));
        }
        if depth > limits.max_formula_nesting_depth() {
            return Err(limit_error(span, CalculationLimitKind::FormulaNestingDepth));
        }
        match current {
            Expr::Call { args, .. } => {
                pending.extend(args.iter().map(|arg| (arg, depth + 1)));
            }
            Expr::Invoke { callee, args } => {
                pending.push((callee, depth + 1));
                pending.extend(args.iter().map(|arg| (arg, depth + 1)));
            }
            Expr::ImplicitIntersection(operand)
            | Expr::SpillRef(operand)
            | Expr::Unary { operand, .. }
            | Expr::Paren(operand) => pending.push((operand, depth + 1)),
            Expr::Binary { left, right, .. }
            | Expr::ReferenceUnion { left, right }
            | Expr::ReferenceIntersection { left, right } => {
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
            | Expr::BuiltinCallable(_)
            | Expr::StructuredRef(_)
            | Expr::ExternalReference(_)
            | Expr::QualifiedName { .. }
            | Expr::Missing => {}
        }
    }
    Ok(())
}

fn limit_error(span: SourceSpan, limit: CalculationLimitKind) -> ParseError {
    ParseError {
        code: ParseErrorCode::UnexpectedToken,
        span,
        limit: Some(limit),
    }
}

#[derive(Debug, Clone, Copy)]
enum Endpoint {
    Cell(CellRef),
    Column(ColRef),
    Row(RowRef),
}

struct Parser<'a> {
    input: &'a str,
    tokens: Vec<SpannedToken>,
    cursor: usize,
    parse_depth: u64,
    max_parse_depth: u64,
    max_ast_depth: u64,
    max_ast_nodes: u64,
    input_len: usize,
    node_sources: Vec<PendingNodeSource>,
}

fn split_cell_ident(ident: &str) -> Option<(String, Option<u32>)> {
    let letters: String = ident
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect();
    if letters.is_empty() {
        return None;
    }
    let rest = &ident[letters.len()..];
    if rest.is_empty() {
        return Some((letters, None));
    }
    if !rest.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let row = rest.parse::<u32>().ok()?;
    (row > 0 && row <= EXCEL_MAX_ROWS).then_some((letters, Some(row)))
}

fn is_lambda_name(name: &str) -> bool {
    super::functions::function_evaluator(name)
        == Some(super::functions::Evaluator::Dynamic(
            super::functions::DynamicFunction::Lambda,
        ))
}

fn can_be_reference_expression(expr: &Expr) -> bool {
    match expr {
        Expr::Ref(_)
        | Expr::StructuredRef(_)
        | Expr::ReferenceUnion { .. }
        | Expr::ReferenceIntersection { .. }
        | Expr::SpillRef(_)
        | Expr::ExternalReference(_)
        | Expr::QualifiedName { .. }
        | Expr::Range { .. }
        | Expr::Name(_)
        | Expr::Call { .. }
        | Expr::Invoke { .. }
        | Expr::ErrorLit(ErrorKind::Ref) => true,
        Expr::Paren(inner) | Expr::ImplicitIntersection(inner) => {
            can_be_reference_expression(inner)
        }
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Logical(_)
        | Expr::BuiltinCallable(_)
        | Expr::ErrorLit(_)
        | Expr::Unary { .. }
        | Expr::Binary { .. }
        | Expr::Array(_)
        | Expr::Missing => false,
    }
}

fn direct_child_count(expr: &Expr) -> usize {
    match expr {
        Expr::Call { args, .. } => args.len(),
        Expr::Invoke { args, .. } => args.len() + 1,
        Expr::ImplicitIntersection(_) | Expr::SpillRef(_) | Expr::Unary { .. } | Expr::Paren(_) => {
            1
        }
        Expr::Binary { .. }
        | Expr::ReferenceUnion { .. }
        | Expr::ReferenceIntersection { .. }
        | Expr::Range { .. } => 2,
        Expr::Array(rows) => rows.iter().map(Vec::len).sum(),
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Logical(_)
        | Expr::ErrorLit(_)
        | Expr::Ref(_)
        | Expr::StructuredRef(_)
        | Expr::ExternalReference(_)
        | Expr::QualifiedName { .. }
        | Expr::Name(_)
        | Expr::BuiltinCallable(_)
        | Expr::Missing => 0,
    }
}

impl Parser<'_> {
    fn track(&mut self, expr: Expr, span: SourceSpan) -> Result<SpannedExpr, ParseError> {
        self.track_with_components(expr, span, Vec::new())
    }

    fn track_with_components(
        &mut self,
        expr: Expr,
        span: SourceSpan,
        components: Vec<SourceComponent>,
    ) -> Result<SpannedExpr, ParseError> {
        if self.node_sources.len() as u64 >= self.max_ast_nodes {
            return Err(limit_error(span, CalculationLimitKind::FormulaAstNodes));
        }
        let child_count = direct_child_count(&expr);
        let mut remaining = self.node_sources.len();
        let mut child_nodes = 0_usize;
        let mut max_child_depth = 0_u64;
        for _ in 0..child_count {
            let Some(root_index) = remaining.checked_sub(1) else {
                return Err(ParseError {
                    code: ParseErrorCode::UnexpectedToken,
                    span,
                    limit: None,
                });
            };
            let child = &self.node_sources[root_index];
            max_child_depth = max_child_depth.max(child.depth);
            let Some(next_remaining) = remaining.checked_sub(child.subtree_nodes) else {
                return Err(ParseError {
                    code: ParseErrorCode::UnexpectedToken,
                    span,
                    limit: None,
                });
            };
            remaining = next_remaining;
            child_nodes = child_nodes.saturating_add(child.subtree_nodes);
        }
        let depth = max_child_depth.saturating_add(1);
        if depth > self.max_ast_depth {
            return Err(limit_error(span, CalculationLimitKind::FormulaNestingDepth));
        }
        self.node_sources.push(PendingNodeSource::new(
            span,
            components,
            depth,
            child_nodes.saturating_add(1),
        ));
        Ok(SpannedExpr { expr, span })
    }

    fn track_missing(&mut self, span: SourceSpan) -> Result<(), ParseError> {
        self.track(Expr::Missing, span).map(|_| ())
    }

    fn strip_parens(&mut self, mut expr: Expr) -> Expr {
        while let Expr::Paren(inner) = expr {
            let removed = self.node_sources.pop();
            debug_assert!(removed.is_some());
            expr = *inner;
        }
        expr
    }

    fn error(&self, code: ParseErrorCode) -> ParseError {
        ParseError {
            code,
            span: self
                .tokens
                .get(self.cursor)
                .map_or(SourceSpan::empty(self.input_len), |token| token.span),
            limit: None,
        }
    }

    fn token(&self, offset: usize) -> Option<&Token> {
        self.tokens
            .get(self.cursor + offset)
            .map(|spanned| &spanned.token)
    }

    fn span(&self, offset: usize) -> SourceSpan {
        self.tokens
            .get(self.cursor + offset)
            .map_or(SourceSpan::empty(self.input_len), |token| token.span)
    }

    fn advance(&mut self) -> Option<SpannedToken> {
        let token = self.tokens.get(self.cursor).cloned();
        if token.is_some() {
            self.cursor += 1;
        }
        token
    }

    fn eat(&mut self, expected: &Token) -> Option<SourceSpan> {
        if self.token(0) == Some(expected) {
            let span = self.span(0);
            self.cursor += 1;
            Some(span)
        } else {
            None
        }
    }

    fn expect(&mut self, expected: &Token) -> Result<SourceSpan, ParseError> {
        self.eat(expected)
            .ok_or_else(|| self.error(ParseErrorCode::UnexpectedToken))
    }

    fn peek_sheet_range_end(&self) -> Option<(String, bool, SourceSpan)> {
        if self.token(1) != Some(&Token::Colon) || self.token(3) != Some(&Token::Bang) {
            return None;
        }
        match self.token(2) {
            Some(Token::Ident(end)) => Some((end.clone(), false, self.span(2))),
            Some(Token::QuotedSheet(end)) => Some((end.clone(), true, self.span(2))),
            _ => None,
        }
    }

    fn parse_expr(&mut self, min_bp: u8, allow_union: bool) -> Result<SpannedExpr, ParseError> {
        if self.parse_depth >= self.max_parse_depth {
            return Err(limit_error(
                self.span(0),
                CalculationLimitKind::FormulaNestingDepth,
            ));
        }
        self.parse_depth += 1;
        let result = self.parse_expr_inner(min_bp, allow_union);
        self.parse_depth -= 1;
        result
    }

    fn parse_expr_inner(
        &mut self,
        min_bp: u8,
        allow_union: bool,
    ) -> Result<SpannedExpr, ParseError> {
        let mut left = self.parse_prefix(allow_union)?;
        loop {
            if self.token(0) == Some(&Token::Hash) && 90 >= min_bp {
                let end = self.expect(&Token::Hash)?;
                if !can_be_reference_expression(&left.expr) {
                    return Err(ParseError {
                        code: ParseErrorCode::UnexpectedToken,
                        span: end,
                        limit: None,
                    });
                }
                let span = left.span.merge(end);
                let anchor = self.strip_parens(left.expr);
                if matches!(anchor, Expr::SpillRef(_)) {
                    return Err(ParseError {
                        code: ParseErrorCode::UnexpectedToken,
                        span: end,
                        limit: None,
                    });
                }
                left = self.track(Expr::SpillRef(Box::new(anchor)), span)?;
                continue;
            }
            if self.token(0) == Some(&Token::Percent) && 60 >= min_bp {
                let end = self.expect(&Token::Percent)?;
                let span = left.span.merge(end);
                left = self.track(
                    Expr::Unary {
                        op: UnaryOp::Percent,
                        operand: left.boxed(),
                    },
                    span,
                )?;
                continue;
            }
            if self.token(0) == Some(&Token::LParen) && 90 >= min_bp {
                if !can_be_postfix_invoked(&left.expr) {
                    break;
                }
                self.cursor += 1;
                let (args, end) = self.parse_call_args()?;
                let span = left.span.merge(end);
                left = self.track(
                    Expr::Invoke {
                        callee: left.boxed(),
                        args,
                    },
                    span,
                )?;
                continue;
            }
            if self.token(0) == Some(&Token::Colon) && 80 >= min_bp {
                let operator = self.span(0);
                self.cursor += 1;
                let end = self.parse_expr(81, allow_union)?;
                if !can_be_reference_expression(&left.expr)
                    || !can_be_reference_expression(&end.expr)
                {
                    return Err(ParseError {
                        code: ParseErrorCode::UnexpectedToken,
                        span: operator,
                        limit: None,
                    });
                }
                let span = left.span.merge(end.span);
                left = self.track(
                    Expr::Range {
                        start: left.boxed(),
                        end: end.boxed(),
                    },
                    span,
                )?;
                continue;
            }
            if self.token(0) == Some(&Token::Intersection) && 75 >= min_bp {
                let operator = self.span(0);
                self.cursor += 1;
                let right = self.parse_expr(76, allow_union)?;
                if !can_be_reference_expression(&left.expr)
                    || !can_be_reference_expression(&right.expr)
                {
                    return Err(ParseError {
                        code: ParseErrorCode::UnexpectedToken,
                        span: operator,
                        limit: None,
                    });
                }
                let span = left.span.merge(right.span);
                left = self.track(
                    Expr::ReferenceIntersection {
                        left: left.boxed(),
                        right: right.boxed(),
                    },
                    span,
                )?;
                continue;
            }
            if allow_union && self.token(0) == Some(&Token::Comma) && 65 >= min_bp {
                let operator = self.span(0);
                self.cursor += 1;
                let right = self.parse_expr(66, true)?;
                if !can_be_reference_expression(&left.expr)
                    || !can_be_reference_expression(&right.expr)
                {
                    return Err(ParseError {
                        code: ParseErrorCode::UnexpectedToken,
                        span: operator,
                        limit: None,
                    });
                }
                let span = left.span.merge(right.span);
                left = self.track(
                    Expr::ReferenceUnion {
                        left: left.boxed(),
                        right: right.boxed(),
                    },
                    span,
                )?;
                continue;
            }
            let (op, bp) = match self.token(0) {
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
            let right = self.parse_expr(bp + 1, allow_union)?;
            let span = left.span.merge(right.span);
            left = self.track(
                Expr::Binary {
                    op,
                    left: left.boxed(),
                    right: right.boxed(),
                },
                span,
            )?;
        }
        Ok(left)
    }

    fn parse_prefix(&mut self, allow_union: bool) -> Result<SpannedExpr, ParseError> {
        match self.token(0) {
            Some(Token::Minus) | Some(Token::Plus) | Some(Token::At) => {
                let token = self.advance().expect("peeked token");
                let operand = self.parse_expr(61, allow_union)?;
                let span = token.span.merge(operand.span);
                let expr = match token.token {
                    Token::Minus => Expr::Unary {
                        op: UnaryOp::Negate,
                        operand: operand.boxed(),
                    },
                    Token::Plus => Expr::Unary {
                        op: UnaryOp::Plus,
                        operand: operand.boxed(),
                    },
                    Token::At => Expr::ImplicitIntersection(operand.boxed()),
                    _ => unreachable!("matched prefix"),
                };
                self.track(expr, span)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<SpannedExpr, ParseError> {
        let Some(current) = self.tokens.get(self.cursor).cloned() else {
            return Err(self.error(ParseErrorCode::UnexpectedEnd));
        };
        match current.token {
            Token::Number(number) => {
                if self.token(1) == Some(&Token::Colon)
                    && matches!(self.token(2), Some(Token::Number(_) | Token::Dollar))
                {
                    let (body, span) = self.parse_ref_body()?;
                    return self.track_with_components(
                        Expr::Ref(Reference { sheet: None, body }),
                        span,
                        vec![SourceComponent::new(
                            SourceComponentKind::ReferenceBody,
                            span,
                        )],
                    );
                }
                self.cursor += 1;
                self.track(Expr::Number(number), current.span)
            }
            Token::Str(text) => {
                self.cursor += 1;
                self.track(Expr::Text(text), current.span)
            }
            Token::ErrorLit(kind) => {
                self.cursor += 1;
                self.track(Expr::ErrorLit(kind), current.span)
            }
            Token::StructuredRef(raw) => {
                self.cursor += 1;
                let parsed = parse_structured_reference(&raw).map_err(|_| ParseError {
                    code: ParseErrorCode::InvalidStructuredReference,
                    span: current.span,
                    limit: None,
                })?;
                let components = parsed
                    .components
                    .into_iter()
                    .map(|component| component.shifted(current.span.start))
                    .collect();
                self.track_with_components(
                    Expr::StructuredRef(parsed.reference),
                    current.span,
                    components,
                )
            }
            Token::ExternalWorkbook(workbook) => {
                self.cursor += 1;
                self.parse_external(workbook, None, None, false, false, current.span)
            }
            Token::LParen => {
                self.cursor += 1;
                let inner = self.parse_expr(0, true)?;
                let end = self.expect(&Token::RParen)?;
                let span = current.span.merge(end);
                self.track(Expr::Paren(inner.boxed()), span)
            }
            Token::LBrace => self.parse_array_literal(),
            Token::QuotedSheet(name) => self.parse_quoted_prefix(name, current.span),
            Token::Dollar => {
                let (body, span) = self.parse_ref_body()?;
                self.track_with_components(
                    Expr::Ref(Reference { sheet: None, body }),
                    span,
                    vec![SourceComponent::new(
                        SourceComponentKind::ReferenceBody,
                        span,
                    )],
                )
            }
            Token::Ident(ident) => self.parse_ident(ident, current.span),
            _ => Err(self.error(ParseErrorCode::UnexpectedToken)),
        }
    }

    fn parse_quoted_prefix(
        &mut self,
        name: String,
        start: SourceSpan,
    ) -> Result<SpannedExpr, ParseError> {
        if let Some((end_name, end_quoted, end_sheet_span)) = self.peek_sheet_range_end() {
            self.cursor += 4;
            let (body, body_span) = self.parse_ref_body()?;
            let mut components = self.quoted_sheet_prefix_components(start);
            components.push(SourceComponent::new(
                SourceComponentKind::SheetRangeEnd,
                self.sheet_token_content_span(end_sheet_span, end_quoted),
            ));
            components.push(SourceComponent::new(
                SourceComponentKind::ReferenceBody,
                body_span,
            ));
            return self.track_with_components(
                Expr::Ref(Reference {
                    sheet: Some(SheetPrefix {
                        name,
                        end_name: Some(end_name),
                        quoted: true,
                    }),
                    body,
                }),
                start.merge(body_span),
                components,
            );
        }
        self.cursor += 1;
        self.expect(&Token::Bang)?;
        if let Some(close) = name.rfind(']') {
            let workbook = name[..=close].to_owned();
            let (sheet, sheet_end) = match name[close + 1..].split_once(':') {
                Some((first, last)) => (
                    Some(first.to_owned().into_boxed_str()),
                    Some(last.to_owned().into_boxed_str()),
                ),
                None if !name[close + 1..].is_empty() => {
                    (Some(name[close + 1..].to_owned().into_boxed_str()), None)
                }
                None => (None, None),
            };
            return self.parse_external(workbook, sheet, sheet_end, false, true, start);
        }
        let (name, end_name) = match name.split_once(':') {
            Some((first, last)) => (first.to_owned(), Some(last.to_owned())),
            None => (name, None),
        };
        self.parse_qualified_target(
            SheetPrefix {
                name,
                end_name,
                quoted: true,
            },
            start,
            self.quoted_sheet_prefix_components(start),
        )
    }

    fn parse_ident(&mut self, ident: String, start: SourceSpan) -> Result<SpannedExpr, ParseError> {
        if self.token(1) == Some(&Token::Bang) {
            self.cursor += 2;
            return self.parse_qualified_target(
                SheetPrefix {
                    name: ident,
                    end_name: None,
                    quoted: false,
                },
                start,
                vec![SourceComponent::new(
                    SourceComponentKind::SheetQualifier,
                    start,
                )],
            );
        }
        if let Some((end_name, end_quoted, end_sheet_span)) = self.peek_sheet_range_end() {
            self.cursor += 4;
            let (body, body_span) = self.parse_ref_body()?;
            return self.track_with_components(
                Expr::Ref(Reference {
                    sheet: Some(SheetPrefix {
                        name: ident,
                        end_name: Some(end_name),
                        quoted: end_quoted,
                    }),
                    body,
                }),
                start.merge(body_span),
                vec![
                    SourceComponent::new(SourceComponentKind::SheetQualifier, start),
                    SourceComponent::new(
                        SourceComponentKind::SheetRangeEnd,
                        self.sheet_token_content_span(end_sheet_span, end_quoted),
                    ),
                    SourceComponent::new(SourceComponentKind::ReferenceBody, body_span),
                ],
            );
        }
        if let Some(callable) = storage_builtin_callable(&ident) {
            self.cursor += 1;
            return self.track(Expr::BuiltinCallable(callable), start);
        }
        if self.token(1) == Some(&Token::LParen) {
            self.cursor += 2;
            let (mut args, end) = self.parse_call_args()?;
            let span = start.merge(end);
            if ident.eq_ignore_ascii_case("_xlfn.SINGLE") && args.len() == 1 {
                let operand = self.strip_parens(args.remove(0));
                return self.track(Expr::ImplicitIntersection(Box::new(operand)), span);
            }
            if ident.eq_ignore_ascii_case("_xlfn.ANCHORARRAY") && args.len() == 1 {
                let anchor = self.strip_parens(args.remove(0));
                if matches!(anchor, Expr::SpillRef(_)) || !can_be_reference_expression(&anchor) {
                    return Err(ParseError {
                        code: ParseErrorCode::UnexpectedToken,
                        span: start,
                        limit: None,
                    });
                }
                return self.track(Expr::SpillRef(Box::new(anchor)), span);
            }
            let argument_count = args.len();
            for (index, arg) in args.iter_mut().enumerate() {
                if !function_argument_is_callable(&ident, index, argument_count) {
                    continue;
                }
                normalize_callable_argument(arg);
            }
            return self.track(Expr::Call { name: ident, args }, span);
        }
        let snapshot = self.cursor;
        if let Ok((body, span)) = self.parse_ref_body() {
            return self.track_with_components(
                Expr::Ref(Reference { sheet: None, body }),
                span,
                vec![SourceComponent::new(
                    SourceComponentKind::ReferenceBody,
                    span,
                )],
            );
        }
        self.cursor = snapshot + 1;
        if ident.eq_ignore_ascii_case("TRUE") {
            self.track(Expr::Logical(true), start)
        } else if ident.eq_ignore_ascii_case("FALSE") {
            self.track(Expr::Logical(false), start)
        } else {
            self.track_with_components(
                Expr::Name(ident),
                start,
                vec![SourceComponent::new(
                    SourceComponentKind::DefinedName,
                    start,
                )],
            )
        }
    }

    fn parse_qualified_target(
        &mut self,
        sheet: SheetPrefix,
        start: SourceSpan,
        mut components: Vec<SourceComponent>,
    ) -> Result<SpannedExpr, ParseError> {
        let snapshot = self.cursor;
        if let Ok((body, body_span)) = self.parse_ref_body() {
            components.push(SourceComponent::new(
                SourceComponentKind::ReferenceBody,
                body_span,
            ));
            return self.track_with_components(
                Expr::Ref(Reference {
                    sheet: Some(sheet),
                    body,
                }),
                start.merge(body_span),
                components,
            );
        }
        self.cursor = snapshot;
        let invalid_span = self.span(0);
        let Some(SpannedToken {
            token: Token::Ident(name),
            span,
        }) = self.advance()
        else {
            return Err(ParseError {
                code: ParseErrorCode::InvalidReference,
                span: invalid_span,
                limit: None,
            });
        };
        components.push(SourceComponent::new(SourceComponentKind::DefinedName, span));
        self.track_with_components(
            Expr::QualifiedName {
                sheet,
                name: name.into_boxed_str(),
            },
            start.merge(span),
            components,
        )
    }

    fn quoted_sheet_prefix_components(&self, span: SourceSpan) -> Vec<SourceComponent> {
        let source = &self.input[span.start..span.end];
        let inner_start = span.start + usize::from(source.starts_with('\''));
        let inner_end = span.end.saturating_sub(usize::from(source.ends_with('\'')));
        let inner = &self.input[inner_start..inner_end];
        if let Some(colon) = inner.find(':') {
            vec![
                SourceComponent::new(
                    SourceComponentKind::SheetQualifier,
                    SourceSpan::new(inner_start, inner_start + colon),
                ),
                SourceComponent::new(
                    SourceComponentKind::SheetRangeEnd,
                    SourceSpan::new(inner_start + colon + 1, inner_end),
                ),
            ]
        } else {
            vec![SourceComponent::new(
                SourceComponentKind::SheetQualifier,
                SourceSpan::new(inner_start, inner_end),
            )]
        }
    }

    fn sheet_token_content_span(&self, span: SourceSpan, quoted: bool) -> SourceSpan {
        if quoted {
            SourceSpan::new(span.start.saturating_add(1), span.end.saturating_sub(1))
        } else {
            span
        }
    }

    fn parse_external(
        &mut self,
        workbook: String,
        supplied_sheet: Option<Box<str>>,
        supplied_sheet_end: Option<Box<str>>,
        supplied_sheet_quoted: bool,
        quoted: bool,
        start: SourceSpan,
    ) -> Result<SpannedExpr, ParseError> {
        let mut components = if quoted {
            self.quoted_external_prefix_components(start)
        } else {
            vec![SourceComponent::new(
                SourceComponentKind::ExternalWorkbook,
                start,
            )]
        };
        let (sheet, sheet_end, sheet_quoted) = if supplied_sheet.is_some() {
            (supplied_sheet, supplied_sheet_end, supplied_sheet_quoted)
        } else {
            match (
                self.token(0).cloned(),
                self.token(1),
                self.token(2).cloned(),
                self.token(3),
            ) {
                (
                    Some(Token::Ident(name)),
                    Some(Token::Colon),
                    Some(Token::Ident(end_name)),
                    Some(Token::Bang),
                ) => {
                    components.push(SourceComponent::new(
                        SourceComponentKind::ExternalSheet,
                        self.span(0),
                    ));
                    components.push(SourceComponent::new(
                        SourceComponentKind::ExternalSheetRangeEnd,
                        self.span(2),
                    ));
                    self.cursor += 3;
                    (
                        Some(name.into_boxed_str()),
                        Some(end_name.into_boxed_str()),
                        false,
                    )
                }
                (Some(Token::Ident(name)), Some(Token::Bang), _, _) => {
                    components.push(SourceComponent::new(
                        SourceComponentKind::ExternalSheet,
                        self.span(0),
                    ));
                    self.cursor += 1;
                    (Some(name.into_boxed_str()), None, false)
                }
                (Some(Token::QuotedSheet(name)), Some(Token::Bang), _, _) => {
                    let content_span = SourceSpan::new(
                        self.span(0).start.saturating_add(1),
                        self.span(0).end.saturating_sub(1),
                    );
                    let (name, end_name) = match name.split_once(':') {
                        Some((first, last)) => {
                            let source = &self.input[content_span.start..content_span.end];
                            let colon = source.find(':').unwrap_or(source.len());
                            components.push(SourceComponent::new(
                                SourceComponentKind::ExternalSheet,
                                SourceSpan::new(content_span.start, content_span.start + colon),
                            ));
                            components.push(SourceComponent::new(
                                SourceComponentKind::ExternalSheetRangeEnd,
                                SourceSpan::new(content_span.start + colon + 1, content_span.end),
                            ));
                            (
                                first.to_owned().into_boxed_str(),
                                Some(last.to_owned().into_boxed_str()),
                            )
                        }
                        None => {
                            components.push(SourceComponent::new(
                                SourceComponentKind::ExternalSheet,
                                content_span,
                            ));
                            (name.into_boxed_str(), None)
                        }
                    };
                    self.cursor += 1;
                    (Some(name), end_name, true)
                }
                _ => (None, None, false),
            }
        };
        if !quoted {
            self.expect(&Token::Bang)?;
        }
        let snapshot = self.cursor;
        let (target, end, target_components) = if let Ok((body, span)) = self.parse_ref_body() {
            (
                ExternalReferenceTarget::Reference(body),
                span,
                vec![SourceComponent::new(
                    SourceComponentKind::ReferenceBody,
                    span,
                )],
            )
        } else {
            self.cursor = snapshot;
            let invalid_span = self.span(0);
            match self.advance() {
                Some(SpannedToken {
                    token: Token::Ident(name),
                    span,
                }) => (
                    ExternalReferenceTarget::DefinedName(name.into_boxed_str()),
                    span,
                    vec![SourceComponent::new(SourceComponentKind::DefinedName, span)],
                ),
                Some(SpannedToken {
                    token: Token::StructuredRef(raw),
                    span,
                }) => {
                    let parsed = parse_structured_reference(&raw).map_err(|_| ParseError {
                        code: ParseErrorCode::InvalidStructuredReference,
                        span,
                        limit: None,
                    })?;
                    if parsed.reference.table.is_none() {
                        return Err(ParseError {
                            code: ParseErrorCode::InvalidExternalReference,
                            span,
                            limit: None,
                        });
                    }
                    let structured_components = parsed
                        .components
                        .into_iter()
                        .map(|component| component.shifted(span.start))
                        .collect();
                    (
                        ExternalReferenceTarget::StructuredReference(parsed.reference),
                        span,
                        structured_components,
                    )
                }
                _ => {
                    return Err(ParseError {
                        code: ParseErrorCode::InvalidExternalReference,
                        span: invalid_span,
                        limit: None,
                    });
                }
            }
        };
        components.extend(target_components);
        self.track_with_components(
            Expr::ExternalReference(ExternalWorkbookReference {
                workbook: workbook.into_boxed_str(),
                sheet,
                sheet_end,
                sheet_quoted,
                quoted,
                target,
            }),
            start.merge(end),
            components,
        )
    }

    fn quoted_external_prefix_components(&self, span: SourceSpan) -> Vec<SourceComponent> {
        let source = &self.input[span.start..span.end];
        let inner_start = usize::from(source.starts_with('\''));
        let inner_end = source
            .len()
            .saturating_sub(usize::from(source.ends_with('\'')));
        let inner = &source[inner_start..inner_end];
        let Some(workbook_close) = inner.rfind(']') else {
            return vec![SourceComponent::new(
                SourceComponentKind::ExternalWorkbook,
                span,
            )];
        };
        let base = span.start + inner_start;
        let mut components = vec![SourceComponent::new(
            SourceComponentKind::ExternalWorkbook,
            SourceSpan::new(base, base + workbook_close + 1),
        )];
        let sheet_start = workbook_close + 1;
        if sheet_start == inner.len() {
            return components;
        }
        if let Some(relative_colon) = inner[sheet_start..].find(':') {
            let colon = sheet_start + relative_colon;
            components.push(SourceComponent::new(
                SourceComponentKind::ExternalSheet,
                SourceSpan::new(base + sheet_start, base + colon),
            ));
            components.push(SourceComponent::new(
                SourceComponentKind::ExternalSheetRangeEnd,
                SourceSpan::new(base + colon + 1, base + inner.len()),
            ));
        } else {
            components.push(SourceComponent::new(
                SourceComponentKind::ExternalSheet,
                SourceSpan::new(base + sheet_start, base + inner.len()),
            ));
        }
        components
    }

    fn parse_call_args(&mut self) -> Result<(Vec<Expr>, SourceSpan), ParseError> {
        let mut args = Vec::new();
        if let Some(end) = self.eat(&Token::RParen) {
            return Ok((args, end));
        }
        loop {
            if let Some(span) = self.eat(&Token::Comma) {
                self.track_missing(span)?;
                args.push(Expr::Missing);
                continue;
            }
            if let Some(end) = self.eat(&Token::RParen) {
                self.track_missing(end)?;
                args.push(Expr::Missing);
                return Ok((args, end));
            }
            args.push(self.parse_expr(0, false)?.expr);
            if self.eat(&Token::Comma).is_some() {
                continue;
            }
            let end = self.expect(&Token::RParen)?;
            return Ok((args, end));
        }
    }

    fn parse_array_literal(&mut self) -> Result<SpannedExpr, ParseError> {
        let start = self.expect(&Token::LBrace)?;
        let mut rows = Vec::new();
        let mut current = Vec::new();
        loop {
            current.push(self.parse_expr(0, false)?.expr);
            match self.advance() {
                Some(SpannedToken {
                    token: Token::Comma,
                    ..
                }) => {}
                Some(SpannedToken {
                    token: Token::Semicolon,
                    ..
                }) => rows.push(std::mem::take(&mut current)),
                Some(SpannedToken {
                    token: Token::RBrace,
                    span,
                }) => {
                    rows.push(current);
                    return self.track(Expr::Array(rows), start.merge(span));
                }
                Some(token) => {
                    return Err(ParseError {
                        code: ParseErrorCode::UnexpectedToken,
                        span: token.span,
                        limit: None,
                    });
                }
                None => return Err(self.error(ParseErrorCode::UnexpectedToken)),
            }
        }
    }

    fn parse_endpoint(&mut self) -> Result<(Endpoint, SourceSpan), ParseError> {
        let leading = self.eat(&Token::Dollar);
        let start = leading.unwrap_or_else(|| self.span(0));
        match self.token(0).cloned() {
            Some(Token::Number(number)) => {
                let value = number.value();
                let row = value as u32;
                if value.fract() != 0.0 || row == 0 || row > EXCEL_MAX_ROWS {
                    return Err(self.error(ParseErrorCode::InvalidReference));
                }
                let end = self.span(0);
                self.cursor += 1;
                Ok((
                    Endpoint::Row(RowRef {
                        row,
                        absolute: leading.is_some(),
                    }),
                    start.merge(end),
                ))
            }
            Some(Token::Ident(ident)) => {
                let Some((letters, row)) = split_cell_ident(&ident) else {
                    return Err(self.error(ParseErrorCode::InvalidReference));
                };
                let Some(column) = column_number(&letters) else {
                    return Err(self.error(ParseErrorCode::InvalidReference));
                };
                let ident_span = self.span(0);
                self.cursor += 1;
                if let Some(row) = row {
                    return Ok((
                        Endpoint::Cell(CellRef {
                            column,
                            row,
                            column_absolute: leading.is_some(),
                            row_absolute: false,
                        }),
                        start.merge(ident_span),
                    ));
                }
                if let Some(dollar) = self.eat(&Token::Dollar)
                    && let Some(Token::Number(number)) = self.token(0).cloned()
                {
                    let number_span = self.span(0);
                    self.cursor += 1;
                    let value = number.value();
                    let row = value as u32;
                    if value.fract() != 0.0 || row == 0 || row > EXCEL_MAX_ROWS {
                        return Err(ParseError {
                            code: ParseErrorCode::InvalidReference,
                            span: number_span,
                            limit: None,
                        });
                    }
                    return Ok((
                        Endpoint::Cell(CellRef {
                            column,
                            row,
                            column_absolute: leading.is_some(),
                            row_absolute: true,
                        }),
                        start.merge(dollar).merge(number_span),
                    ));
                }
                Ok((
                    Endpoint::Column(ColRef {
                        column,
                        absolute: leading.is_some(),
                    }),
                    start.merge(ident_span),
                ))
            }
            _ => Err(self.error(ParseErrorCode::InvalidReference)),
        }
    }

    fn parse_ref_body(&mut self) -> Result<(RefBody, SourceSpan), ParseError> {
        let (first, first_span) = self.parse_endpoint()?;
        let colon_cursor = self.cursor;
        if self.eat(&Token::Colon).is_none() {
            return match first {
                Endpoint::Cell(cell) => Ok((RefBody::Cell(cell), first_span)),
                _ => Err(self.error(ParseErrorCode::InvalidReference)),
            };
        }
        let (second, second_span) = match self.parse_endpoint() {
            Ok(second) => second,
            Err(error) => {
                if let Endpoint::Cell(cell) = first {
                    self.cursor = colon_cursor;
                    return Ok((RefBody::Cell(cell), first_span));
                }
                return Err(error);
            }
        };
        let span = first_span.merge(second_span);
        match (first, second) {
            (Endpoint::Cell(start), Endpoint::Cell(end)) => Ok((RefBody::Area(start, end), span)),
            (Endpoint::Column(start), Endpoint::Column(end)) => {
                Ok((RefBody::Columns(start, end), span))
            }
            (Endpoint::Row(start), Endpoint::Row(end)) => Ok((RefBody::Rows(start, end), span)),
            (Endpoint::Cell(cell), _) => {
                self.cursor = colon_cursor;
                Ok((RefBody::Cell(cell), first_span))
            }
            _ => Err(ParseError {
                code: ParseErrorCode::MismatchedRange,
                span,
                limit: None,
            }),
        }
    }
}

fn normalize_callable_argument(expr: &mut Expr) {
    match expr {
        Expr::Name(name) => {
            if let Some(callable) = builtin_callable(name) {
                *expr = Expr::BuiltinCallable(callable);
            }
        }
        Expr::Paren(inner) => normalize_callable_argument(inner),
        _ => {}
    }
}

fn can_be_postfix_invoked(expr: &Expr) -> bool {
    match expr {
        Expr::Call { name, .. } => is_lambda_name(name),
        Expr::BuiltinCallable(_) | Expr::QualifiedName { .. } => true,
        Expr::ExternalReference(ExternalWorkbookReference {
            target: ExternalReferenceTarget::DefinedName(_),
            ..
        }) => true,
        Expr::Paren(inner) => can_be_postfix_invoked(inner),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_formula_with_limits;
    use crate::calculation::CalculationLimits;
    use crate::calculation::ast::{
        Expr, FormulaDisplayMode, StructuredColumns, StructuredItem, UnaryOp,
    };
    use crate::calculation::error::ParseErrorCode;
    use crate::calculation::syntax::{SourceComponentKind, SourceSpan};

    fn parse(input: &str) -> crate::calculation::syntax::ParsedFormula {
        parse_formula_with_limits(input, CalculationLimits::default()).expect(input)
    }

    #[test]
    fn keeps_original_and_utf8_source_map() {
        let parsed = parse("표[금액] \t B1");
        assert_eq!(parsed.original(), "표[금액] \t B1");
        assert_eq!(
            parsed.source_map().trivia_spans(),
            [SourceSpan::new(11, 14)]
        );
        assert_eq!(
            parsed.source_map().node_tree().span(),
            SourceSpan::new(0, 16)
        );
        assert_eq!(parsed.source_map().node_tree().children().len(), 2);
        assert_eq!(
            parsed.source_map().token_spans().first(),
            Some(&SourceSpan::new(0, 11))
        );
    }

    #[test]
    fn source_map_tracks_utf8_rename_targets_without_reparsing_text() {
        let parsed = parse("표[[#Headers],[금액]]");
        let components = parsed.source_map().node_tree().components();
        assert_eq!(components.len(), 3);
        assert_eq!(components[0].kind(), SourceComponentKind::StructuredTable);
        assert_eq!(components[0].span(), SourceSpan::new(0, 3));
        assert_eq!(components[1].kind(), SourceComponentKind::StructuredItem(0));
        assert_eq!(components[1].span(), SourceSpan::new(5, 13));
        assert_eq!(
            components[2].kind(),
            SourceComponentKind::StructuredColumn { grouped: true }
        );
        assert_eq!(components[2].span(), SourceSpan::new(16, 22));
        for component in components {
            assert!(parsed.original().is_char_boundary(component.span().start));
            assert!(parsed.original().is_char_boundary(component.span().end));
        }

        let qualified = parse("'My Sheet'!TaxRate");
        let components = qualified.source_map().node_tree().components();
        assert_eq!(
            components
                .iter()
                .map(|component| component.kind())
                .collect::<Vec<_>>(),
            [
                SourceComponentKind::SheetQualifier,
                SourceComponentKind::DefinedName
            ]
        );
        assert_eq!(components[0].span(), SourceSpan::new(1, 9));
        assert_eq!(components[1].span(), SourceSpan::new(11, 18));

        let sheet_range_source = "'표''1:끝''2'!A1";
        let sheet_range = parse(sheet_range_source);
        let components = sheet_range.source_map().node_tree().components();
        assert_eq!(
            components
                .iter()
                .map(|component| component.kind())
                .collect::<Vec<_>>(),
            [
                SourceComponentKind::SheetQualifier,
                SourceComponentKind::SheetRangeEnd,
                SourceComponentKind::ReferenceBody,
            ]
        );
        assert_eq!(
            &sheet_range_source[components[0].span().start..components[0].span().end],
            "표''1"
        );
        assert_eq!(
            &sheet_range_source[components[1].span().start..components[1].span().end],
            "끝''2"
        );

        let external_source = "'C:\\Dir]\\[Book.xlsx]Sheet1'!A1";
        let external = parse(external_source);
        let components = external.source_map().node_tree().components();
        assert_eq!(
            &external_source[components[0].span().start..components[0].span().end],
            "C:\\Dir]\\[Book.xlsx]"
        );
        assert_eq!(
            &external_source[components[1].span().start..components[1].span().end],
            "Sheet1"
        );
    }

    #[test]
    fn parses_structured_reference_semantics() {
        let parsed = parse("Table1[[#Headers],[Amount]]");
        let Expr::StructuredRef(reference) = parsed.root() else {
            panic!("typed structured reference expected");
        };
        assert_eq!(reference.table.as_deref(), Some("Table1"));
        assert_eq!(reference.items, [StructuredItem::Headers]);
        assert_eq!(
            reference.columns,
            Some(StructuredColumns::Single("Amount".into()))
        );

        assert_eq!(
            parse("[@Amount]").root(),
            parse("[[#This Row],[Amount]]").root()
        );
        let escaped = parse("['#Headers]");
        let Expr::StructuredRef(escaped_header) = escaped.root() else {
            panic!("escaped header column expected");
        };
        assert!(escaped_header.items.is_empty());
        assert_eq!(
            escaped_header.columns,
            Some(StructuredColumns::Single("#Headers".into()))
        );
    }

    #[test]
    fn distinguishes_union_intersection_and_argument_commas() {
        assert!(matches!(parse("A1,B1").root(), Expr::ReferenceUnion { .. }));
        assert!(matches!(
            parse("A1 B1").root(),
            Expr::ReferenceIntersection { .. }
        ));
        assert!(matches!(
            parse("A1 (B1)").root(),
            Expr::ReferenceIntersection { .. }
        ));
        for formula in [
            "A$1 B1",
            "$A$1 B1",
            "Sheet1!$A$1 B1",
            "1:2 B1",
            "A1 1:2",
            "$A$1 (B1)",
            "A:A (B1)",
        ] {
            assert!(
                matches!(parse(formula).root(), Expr::ReferenceIntersection { .. }),
                "{formula}"
            );
        }
        let call = parse("SUM(A1,B1)");
        let Expr::Call { args, .. } = call.root() else {
            panic!("call expected");
        };
        assert_eq!(args.len(), 2);

        let negated_union = parse("-A1,B1");
        let Expr::Unary { operand, .. } = negated_union.root() else {
            panic!("negation should bind outside the reference union");
        };
        assert!(matches!(operand.as_ref(), Expr::ReferenceUnion { .. }));
        let negated_percent = parse("-A1%");
        let Expr::Unary {
            op: UnaryOp::Percent,
            operand,
        } = negated_percent.root()
        else {
            panic!("percent should bind outside negation");
        };
        assert!(matches!(
            operand.as_ref(),
            Expr::Unary {
                op: UnaryOp::Negate,
                ..
            }
        ));

        let precedence = parse("A1:B2 C2:D3,E1");
        let Expr::ReferenceUnion { left, right } = precedence.root() else {
            panic!("union should have the lowest reference precedence");
        };
        assert!(matches!(left.as_ref(), Expr::ReferenceIntersection { .. }));
        assert!(matches!(right.as_ref(), Expr::Ref(_)));

        let nested = parse("SUM((A1,B1),C1)");
        let Expr::Call { args, .. } = nested.root() else {
            panic!("call expected");
        };
        assert!(matches!(args[0], Expr::Paren(_)));
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn builtin_callable_names_normalize_only_in_callable_positions() {
        let authored = parse("BYROW({1,2;3,4},SUM)");
        let Expr::Call { args, .. } = authored.root() else {
            panic!("BYROW call expected");
        };
        assert!(matches!(args[1], Expr::BuiltinCallable(_)));
        assert_eq!(authored.root().to_string(), "BYROW({1,2;3,4},SUM)");

        let storage = authored
            .display_with_mode(FormulaDisplayMode::Storage)
            .to_string();
        assert_eq!(storage, "BYROW({1,2;3,4},_xleta.SUM)");
        assert_eq!(authored.root(), parse(&storage).root());

        assert!(matches!(parse("SUM").root(), Expr::Name(_)));
        assert!(matches!(
            parse("_xleta.SUM").root(),
            Expr::BuiltinCallable(_)
        ));
        assert!(matches!(parse("SUM(1)").root(), Expr::Call { .. }));
        assert!(matches!(parse("BYROW({1},ABS)").root(), Expr::Call { .. }));

        let parenthesized = parse("BYROW({1},(SUM))");
        let Expr::Call { args, .. } = parenthesized.root() else {
            panic!("BYROW call expected");
        };
        assert!(matches!(
            args[1],
            Expr::Paren(ref inner) if matches!(inner.as_ref(), Expr::BuiltinCallable(_))
        ));

        let invoked = parse("_xleta.SUM(1,2)");
        assert!(matches!(invoked.root(), Expr::Invoke { .. }));
        assert_eq!(invoked.root().to_string(), "SUM(1,2)");
        assert_eq!(
            invoked
                .display_with_mode(FormulaDisplayMode::Storage)
                .to_string(),
            "_xleta.SUM(1,2)"
        );

        let prefixed = parse("_xlfn._xlws.BYROW({1},_xleta.SUM)");
        assert_eq!(
            prefixed
                .display_with_mode(FormulaDisplayMode::Storage)
                .to_string(),
            "_xlfn._xlws.BYROW({1},_xleta.SUM)"
        );
    }

    #[test]
    fn spill_spellings_share_one_semantic_node() {
        let postfix = parse("A1#");
        let storage = parse("_xlfn.ANCHORARRAY(A1)");
        assert_eq!(postfix.root(), storage.root());
        assert_eq!(postfix.root().to_string(), "A1#");
        assert_eq!(
            postfix
                .display_with_mode(FormulaDisplayMode::Storage)
                .to_string(),
            "_xlfn.ANCHORARRAY(A1)"
        );

        let implicit = parse("_xlfn.SINGLE(A1)");
        assert_eq!(implicit.root().to_string(), "@A1");
        assert_eq!(
            implicit
                .display_with_mode(FormulaDisplayMode::Storage)
                .to_string(),
            "_xlfn.SINGLE(A1)"
        );

        for storage in ["_xlfn.ANCHORARRAY(A1 B1)", "_xlfn.ANCHORARRAY(@A1)"] {
            let parsed = parse(storage);
            let authored = parsed.root().to_string();
            assert_eq!(
                parsed.root(),
                parse(&authored).root(),
                "{storage} -> {authored}"
            );
        }

        let union_spill = parse("(A1,B1)#");
        let storage = union_spill
            .display_with_mode(FormulaDisplayMode::Storage)
            .to_string();
        assert_eq!(storage, "_xlfn.ANCHORARRAY((A1,B1))");
        assert_eq!(union_spill.root(), parse(&storage).root());

        let union_intersection = parse("@A1,B1");
        let storage = union_intersection
            .display_with_mode(FormulaDisplayMode::Storage)
            .to_string();
        assert_eq!(storage, "_xlfn.SINGLE((A1,B1))");
        assert_eq!(union_intersection.root(), parse(&storage).root());

        for input in ["@-A1,B1", "@+A1,B1"] {
            let parsed = parse(input);
            let storage = parsed
                .display_with_mode(FormulaDisplayMode::Storage)
                .to_string();
            assert_eq!(
                parsed.root(),
                parse(&storage).root(),
                "{input} -> {storage}"
            );
        }
    }

    #[test]
    fn external_and_qualified_names_are_typed() {
        assert!(matches!(
            parse("[Book.xlsx]Sheet1!A1").root(),
            Expr::ExternalReference(_)
        ));
        assert!(matches!(
            parse("Sheet1!TaxRate").root(),
            Expr::QualifiedName { .. }
        ));
        let quoted = parse("[Book.xlsx]'My Sheet'!A1");
        let displayed = quoted.root().to_string();
        assert_eq!(displayed, "[Book.xlsx]'My Sheet'!A1");
        assert_eq!(quoted.root(), parse(&displayed).root());

        for input in [
            "[Book.xlsx]Sheet1:Sheet3!A1",
            "'[Book.xlsx]Sheet1:Sheet3'!A1",
            "[1]!DataTable[Amount]",
        ] {
            let parsed = parse(input);
            assert!(
                matches!(parsed.root(), Expr::ExternalReference(_)),
                "{input}"
            );
            let displayed = parsed.root().to_string();
            assert_eq!(
                parsed.root(),
                parse(&displayed).root(),
                "{input} -> {displayed}"
            );
        }
        assert!(matches!(
            parse("Sheet1!MyLambda(1)").root(),
            Expr::Invoke { .. }
        ));
        assert!(matches!(
            parse("[1]!MyLambda(1)").root(),
            Expr::Invoke { .. }
        ));

        let mixed_quoted_range = parse("Sheet1:'Sheet 3'!A1");
        assert_eq!(mixed_quoted_range.root().to_string(), "'Sheet1:Sheet 3'!A1");
        assert_eq!(
            mixed_quoted_range.root(),
            parse(&mixed_quoted_range.root().to_string()).root()
        );
    }

    #[test]
    fn invalid_structured_reference_has_stable_code_and_byte_span() {
        let error = parse_formula_with_limits("[]", CalculationLimits::default())
            .expect_err("invalid structured reference");
        assert_eq!(error.code, ParseErrorCode::InvalidStructuredReference);
        assert_eq!(error.span, SourceSpan::new(0, 2));

        for invalid in [
            "[]",
            "Table1[[#Bogus],[Amount]]",
            "Table1[[A],[B]]",
            "Table1[[#Headers]:[Amount]]",
            "Table1[@#Headers]",
            "Table1[A:B]",
        ] {
            let error = parse_formula_with_limits(invalid, CalculationLimits::default())
                .expect_err(invalid);
            assert_eq!(
                error.code,
                ParseErrorCode::InvalidStructuredReference,
                "{invalid}"
            );
        }
    }

    #[test]
    fn invalid_reference_operators_have_stable_operator_spans() {
        for (formula, span) in [
            ("1#", SourceSpan::new(1, 2)),
            ("A1##", SourceSpan::new(3, 4)),
            ("A1:TRUE", SourceSpan::new(2, 3)),
            ("1,A1", SourceSpan::new(1, 2)),
            ("1 B1", SourceSpan::new(1, 2)),
        ] {
            let error = parse_formula_with_limits(formula, CalculationLimits::default())
                .expect_err(formula);
            assert_eq!(error.code, ParseErrorCode::UnexpectedToken, "{formula}");
            assert_eq!(error.span, span, "{formula}");
        }

        for formula in ["A1:#REF!", "#REF!:B2", "#REF!,A1"] {
            parse(formula);
        }
        let error = parse_formula_with_limits("A1:#VALUE!", CalculationLimits::default())
            .expect_err("non-reference errors are not range endpoints");
        assert_eq!(error.span, SourceSpan::new(2, 3));
    }

    #[test]
    fn invalid_qualified_targets_report_the_offending_token() {
        for (formula, expected_code, expected_span) in [
            (
                "[1]!@",
                ParseErrorCode::InvalidExternalReference,
                SourceSpan::new(4, 5),
            ),
            (
                "Sheet1!@",
                ParseErrorCode::InvalidReference,
                SourceSpan::new(7, 8),
            ),
            (
                "[1]![Amount]",
                ParseErrorCode::InvalidExternalReference,
                SourceSpan::new(4, 12),
            ),
            (
                "[1]![@Amount]",
                ParseErrorCode::InvalidExternalReference,
                SourceSpan::new(4, 13),
            ),
        ] {
            let error = parse_formula_with_limits(formula, CalculationLimits::default())
                .expect_err(formula);
            assert_eq!(error.code, expected_code, "{formula}");
            assert_eq!(error.span, expected_span, "{formula}");
        }
    }

    #[test]
    fn consumed_invalid_tokens_keep_their_original_error_spans() {
        for (formula, code, span) in [
            (
                "{1 2}",
                ParseErrorCode::UnexpectedToken,
                SourceSpan::new(2, 3),
            ),
            (
                "$A$0",
                ParseErrorCode::InvalidReference,
                SourceSpan::new(3, 4),
            ),
        ] {
            let error = parse_formula_with_limits(formula, CalculationLimits::default())
                .expect_err(formula);
            assert_eq!(error.code, code, "{formula}");
            assert_eq!(error.span, span, "{formula}");
        }
    }

    #[test]
    fn spill_normalization_removes_redundant_parentheses_and_rejects_nested_spills() {
        for input in ["((A1))#", "_xlfn.ANCHORARRAY((A1))"] {
            let parsed = parse(input);
            assert_eq!(parsed.root(), parse("A1#").root(), "{input}");
            assert_eq!(parsed.root(), parse(&parsed.root().to_string()).root());
        }
        for invalid in ["(A1#)#", "_xlfn.ANCHORARRAY(A1#)", "_xlfn.ANCHORARRAY(1)"] {
            parse_formula_with_limits(invalid, CalculationLimits::default()).expect_err(invalid);
        }
    }

    #[test]
    fn source_and_ast_budgets_fail_before_unbounded_parser_allocation() {
        let source_limits = CalculationLimits::default()
            .with_max_formula_source_bytes(3)
            .expect("nonzero source limit");
        let source_error =
            parse_formula_with_limits("표A", source_limits).expect_err("four UTF-8 source bytes");
        assert_eq!(
            source_error.limit,
            Some(crate::calculation::CalculationLimitKind::FormulaSourceBytes)
        );
        assert_eq!(source_error.span, SourceSpan::new(0, 4));

        let ast_limits = CalculationLimits::default()
            .with_max_formula_ast_nodes(2)
            .expect("nonzero AST limit");
        let ast_error =
            parse_formula_with_limits("1+2", ast_limits).expect_err("third node exceeds budget");
        assert_eq!(
            ast_error.limit,
            Some(crate::calculation::CalculationLimitKind::FormulaAstNodes)
        );

        let deep_chain = std::iter::repeat_n("1", 300).collect::<Vec<_>>().join("+");
        let deep_limits = CalculationLimits::default()
            .with_max_formula_tokens(1_000)
            .expect("token limit")
            .with_max_formula_ast_nodes(1_000)
            .expect("AST limit")
            .with_max_formula_nesting_depth(1_000)
            .expect("compatible caller configuration");
        let depth_error = parse_formula_with_limits(&deep_chain, deep_limits)
            .expect_err("internal safe AST depth is charged during construction");
        assert_eq!(
            depth_error.limit,
            Some(crate::calculation::CalculationLimitKind::FormulaNestingDepth)
        );
    }

    #[test]
    fn canonical_display_round_trips_semantics() {
        for input in [
            "Table1[[#Headers],[Amount]]",
            "Table1[[A:B]]",
            "Table1[A|B]",
            "Table1[😀]",
            "Table1[ [Sales]:[Region] ]",
            "Table1[[#Headers], [#Data], [Amount]]",
            "Table1[@[January]:[December]]",
            "(A1:B2,C1:D2)",
            "A1 B1",
            "Sheet1!TaxRate",
            "[Book.xlsx]Sheet1!A1",
            "[Book.xlsx]'My Sheet'!A1",
            "[Book.xlsx]Sheet1:Sheet3!A1",
            "[1]!DataTable[Amount]",
            "Table1[]",
        ] {
            let first = parse(input);
            let displayed = first.root().to_string();
            let second = parse(&displayed);
            assert_eq!(first.root(), second.root(), "{input} -> {displayed}");
        }
    }
}
