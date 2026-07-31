use super::ast::{
    Expr, SheetPrefix, StructuredColumns, StructuredReference, structured_column_needs_grouping,
};
use super::error::ParseErrorCode;
use super::lexer::{Token, lex_spanned};
use super::parser::parse_formula_with_limits;
use super::structured_reference::parse_structured_reference;
use super::syntax::{
    NodeSpanTree, ParsedFormula, SourceComponent, SourceComponentKind, SourceEdit, SourceEditError,
    SourceSpan, apply_source_edits,
};
use super::{CalculationLimitKind, CalculationLimits};
use crate::case_insensitive_eq;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormulaRewriteRequest<'a> {
    Sheet {
        old_name: &'a str,
        new_name: &'a str,
    },
    Table {
        old_name: &'a str,
        new_name: &'a str,
    },
    TableColumn {
        table_name: &'a str,
        old_name: &'a str,
        new_name: &'a str,
        owner_is_target_table: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FormulaRewriteLimits {
    pub max_formulas: usize,
    pub max_source_bytes: usize,
    pub max_ast_nodes: usize,
    pub max_source_edits: usize,
}

impl FormulaRewriteLimits {
    pub(crate) const UNBOUNDED: Self = Self {
        max_formulas: usize::MAX,
        max_source_bytes: usize::MAX,
        max_ast_nodes: usize::MAX,
        max_source_edits: usize::MAX,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormulaRewriteLimitKind {
    Formulas,
    SourceBytes,
    AstNodes,
    SourceEdits,
}

impl FormulaRewriteLimitKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Formulas => "formulas",
            Self::SourceBytes => "source_bytes",
            Self::AstNodes => "ast_nodes",
            Self::SourceEdits => "source_edits",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormulaRewriteError {
    Cancelled,
    LimitExceeded {
        kind: FormulaRewriteLimitKind,
        limit: usize,
        actual: usize,
    },
    Parse {
        code: ParseErrorCode,
        span: SourceSpan,
        owner: Option<String>,
    },
    SourceEdit(SourceEditError),
}

impl FormulaRewriteError {
    pub(crate) fn with_owner(self, owner: String) -> Self {
        match self {
            Self::Parse {
                code,
                span,
                owner: None,
            } => Self::Parse {
                code,
                span,
                owner: Some(owner),
            },
            other => other,
        }
    }
}

pub(crate) struct FormulaRewriteBudget<'a> {
    limits: FormulaRewriteLimits,
    formulas: usize,
    source_bytes: usize,
    ast_nodes: usize,
    source_edits: usize,
    cancelled: &'a dyn Fn() -> bool,
}

impl<'a> FormulaRewriteBudget<'a> {
    pub(crate) fn new(limits: FormulaRewriteLimits, cancelled: &'a dyn Fn() -> bool) -> Self {
        Self {
            limits,
            formulas: 0,
            source_bytes: 0,
            ast_nodes: 0,
            source_edits: 0,
            cancelled,
        }
    }

    pub(crate) fn check_cancelled(&self) -> Result<(), FormulaRewriteError> {
        if (self.cancelled)() {
            Err(FormulaRewriteError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn charge_formula(&mut self, source_bytes: usize) -> Result<(), FormulaRewriteError> {
        self.check_cancelled()?;
        self.formulas = charge(
            FormulaRewriteLimitKind::Formulas,
            self.formulas,
            1,
            self.limits.max_formulas,
        )?;
        self.source_bytes = charge(
            FormulaRewriteLimitKind::SourceBytes,
            self.source_bytes,
            source_bytes,
            self.limits.max_source_bytes,
        )?;
        Ok(())
    }

    fn charge_node(&mut self) -> Result<(), FormulaRewriteError> {
        self.check_cancelled()?;
        self.ast_nodes = charge(
            FormulaRewriteLimitKind::AstNodes,
            self.ast_nodes,
            1,
            self.limits.max_ast_nodes,
        )?;
        Ok(())
    }

    fn charge_edit(&mut self) -> Result<(), FormulaRewriteError> {
        self.check_cancelled()?;
        self.source_edits = charge(
            FormulaRewriteLimitKind::SourceEdits,
            self.source_edits,
            1,
            self.limits.max_source_edits,
        )?;
        Ok(())
    }

    fn parser_limits(&self) -> Result<(CalculationLimits, bool), FormulaRewriteError> {
        self.check_cancelled()?;
        let remaining = self.limits.max_ast_nodes.saturating_sub(self.ast_nodes);
        if remaining == 0 {
            return Err(FormulaRewriteError::LimitExceeded {
                kind: FormulaRewriteLimitKind::AstNodes,
                limit: self.limits.max_ast_nodes,
                actual: self.ast_nodes.saturating_add(1),
            });
        }
        let defaults = CalculationLimits::default();
        let parser_limit = remaining.min(defaults.max_formula_ast_nodes() as usize);
        let limits = defaults
            .with_max_formula_ast_nodes(parser_limit as u64)
            .expect("remaining AST budget is non-zero");
        Ok((
            limits,
            parser_limit < defaults.max_formula_ast_nodes() as usize,
        ))
    }

    fn parser_limit_error(&self) -> FormulaRewriteError {
        FormulaRewriteError::LimitExceeded {
            kind: FormulaRewriteLimitKind::AstNodes,
            limit: self.limits.max_ast_nodes,
            actual: self.limits.max_ast_nodes.saturating_add(1),
        }
    }
}

fn charge(
    kind: FormulaRewriteLimitKind,
    current: usize,
    amount: usize,
    limit: usize,
) -> Result<usize, FormulaRewriteError> {
    let actual = current.saturating_add(amount);
    if actual > limit {
        return Err(FormulaRewriteError::LimitExceeded {
            kind,
            limit,
            actual,
        });
    }
    Ok(actual)
}

pub(crate) fn rewrite_formula(
    source: &str,
    request: &FormulaRewriteRequest<'_>,
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<Option<String>, FormulaRewriteError> {
    budget.charge_formula(source.len())?;
    let (parser_limits, parser_is_budget_limited) = budget.parser_limits()?;
    let parsed = match parse_formula_with_limits(source, parser_limits) {
        Ok(parsed) => parsed,
        Err(error) => {
            if parser_is_budget_limited
                && error.limit == Some(CalculationLimitKind::FormulaAstNodes)
            {
                return Err(budget.parser_limit_error());
            }
            if formula_may_reference_target(source, request, budget)? {
                return Err(FormulaRewriteError::Parse {
                    code: error.code,
                    span: error.span,
                    owner: None,
                });
            }
            return Ok(None);
        }
    };
    let mut edits = Vec::new();
    let mut stack = vec![(parsed.root(), parsed.source_map().node_tree())];
    while let Some((expr, source_node)) = stack.pop() {
        budget.charge_node()?;
        match expr {
            Expr::Ref(reference) => {
                if let FormulaRewriteRequest::Sheet { old_name, new_name } = request
                    && let Some(prefix) = &reference.sheet
                {
                    plan_sheet_prefix_edits(
                        source,
                        prefix,
                        source_node.components(),
                        old_name,
                        new_name,
                        &mut edits,
                        budget,
                    )?;
                }
            }
            Expr::QualifiedName { sheet, .. } => {
                if let FormulaRewriteRequest::Sheet { old_name, new_name } = request {
                    plan_sheet_prefix_edits(
                        source,
                        sheet,
                        source_node.components(),
                        old_name,
                        new_name,
                        &mut edits,
                        budget,
                    )?;
                }
            }
            Expr::StructuredRef(reference) => match request {
                FormulaRewriteRequest::Table { old_name, new_name } => {
                    if reference
                        .table
                        .as_deref()
                        .is_some_and(|table| case_insensitive_eq(table, old_name))
                        && let Some(component) = component(
                            source_node.components(),
                            SourceComponentKind::StructuredTable,
                        )
                    {
                        budget.charge_edit()?;
                        edits.push(SourceEdit::new(component.span(), *new_name));
                    }
                }
                FormulaRewriteRequest::TableColumn {
                    table_name,
                    old_name,
                    new_name,
                    owner_is_target_table,
                } => {
                    let targets_table = reference
                        .table
                        .as_deref()
                        .map_or(*owner_is_target_table, |table| {
                            case_insensitive_eq(table, table_name)
                        });
                    if targets_table {
                        plan_column_edits(
                            reference,
                            source_node.components(),
                            old_name,
                            new_name,
                            &mut edits,
                            budget,
                        )?;
                    }
                }
                FormulaRewriteRequest::Sheet { .. } => {}
            },
            Expr::ExternalReference(_) => continue,
            _ => {}
        }
        push_children(expr, source_node, &mut stack);
    }
    if edits.is_empty() {
        return Ok(None);
    }
    let rewritten = apply_source_edits(source, &edits).map_err(FormulaRewriteError::SourceEdit)?;
    if rewritten == source {
        return Ok(None);
    }
    let (validation_limits, validation_is_budget_limited) = budget.parser_limits()?;
    let reparsed = parse_formula_with_limits(&rewritten, validation_limits).map_err(|error| {
        if validation_is_budget_limited
            && error.limit == Some(CalculationLimitKind::FormulaAstNodes)
        {
            budget.parser_limit_error()
        } else {
            FormulaRewriteError::Parse {
                code: error.code,
                span: error.span,
                owner: None,
            }
        }
    })?;
    charge_parsed_tree(&reparsed, budget)?;
    Ok(Some(rewritten))
}

fn charge_parsed_tree(
    parsed: &ParsedFormula,
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<(), FormulaRewriteError> {
    let mut stack = vec![(parsed.root(), parsed.source_map().node_tree())];
    while let Some((expr, source_node)) = stack.pop() {
        budget.charge_node()?;
        push_children(expr, source_node, &mut stack);
    }
    Ok(())
}

fn push_children<'a>(
    expr: &'a Expr,
    source_node: &'a NodeSpanTree,
    stack: &mut Vec<(&'a Expr, &'a NodeSpanTree)>,
) {
    let mut expressions: Vec<&Expr> = match expr {
        Expr::Call { args, .. } => args.iter().collect(),
        Expr::Invoke { callee, args } => {
            let mut children = Vec::with_capacity(args.len() + 1);
            children.push(callee.as_ref());
            children.extend(args.iter());
            children
        }
        Expr::ImplicitIntersection(child)
        | Expr::SpillRef(child)
        | Expr::Unary { operand: child, .. }
        | Expr::Paren(child) => vec![child],
        Expr::Binary { left, right, .. }
        | Expr::ReferenceUnion { left, right }
        | Expr::ReferenceIntersection { left, right } => vec![left, right],
        Expr::Range { start, end } => vec![start, end],
        Expr::Array(rows) => rows.iter().flatten().collect(),
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Logical(_)
        | Expr::ErrorLit(_)
        | Expr::Ref(_)
        | Expr::StructuredRef(_)
        | Expr::ExternalReference(_)
        | Expr::QualifiedName { .. }
        | Expr::Name(_)
        | Expr::Missing => Vec::new(),
    };
    debug_assert_eq!(expressions.len(), source_node.children().len());
    expressions
        .drain(..)
        .zip(source_node.children())
        .rev()
        .for_each(|pair| stack.push(pair));
}

fn plan_sheet_prefix_edits(
    source: &str,
    prefix: &SheetPrefix,
    components: &[SourceComponent],
    old_name: &str,
    new_name: &str,
    edits: &mut Vec<SourceEdit>,
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<(), FormulaRewriteError> {
    if case_insensitive_eq(&prefix.name, old_name)
        && let Some(component) = component(components, SourceComponentKind::SheetQualifier)
    {
        budget.charge_edit()?;
        edits.push(SourceEdit::new(
            component.span(),
            replacement_sheet_name(source, component.span(), new_name),
        ));
    }
    if prefix
        .end_name
        .as_deref()
        .is_some_and(|name| case_insensitive_eq(name, old_name))
        && let Some(component) = component(components, SourceComponentKind::SheetRangeEnd)
    {
        budget.charge_edit()?;
        edits.push(SourceEdit::new(
            component.span(),
            replacement_sheet_name(source, component.span(), new_name),
        ));
    }
    Ok(())
}

fn replacement_sheet_name(source: &str, span: SourceSpan, new_name: &str) -> String {
    let escaped = new_name.replace('\'', "''");
    let has_quoted_shell = span
        .start
        .checked_sub(1)
        .and_then(|offset| source.as_bytes().get(offset))
        == Some(&b'\'')
        || source.as_bytes().get(span.end) == Some(&b'\'');
    if has_quoted_shell || !sheet_name_needs_quotes(new_name) {
        escaped
    } else {
        format!("'{escaped}'")
    }
}

fn sheet_name_needs_quotes(name: &str) -> bool {
    name.is_empty()
        || !name
            .chars()
            .next()
            .is_some_and(|character| character.is_alphabetic() || matches!(character, '_' | '\\'))
        || name
            .chars()
            .skip(1)
            .any(|character| !(character.is_alphanumeric() || matches!(character, '_' | '.')))
}

fn plan_column_edits(
    reference: &StructuredReference,
    components: &[SourceComponent],
    old_name: &str,
    new_name: &str,
    edits: &mut Vec<SourceEdit>,
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<(), FormulaRewriteError> {
    match &reference.columns {
        Some(StructuredColumns::Single(column_name))
            if case_insensitive_eq(column_name, old_name) =>
        {
            if let Some(component) = components.iter().find(|component| {
                matches!(
                    component.kind(),
                    SourceComponentKind::StructuredColumn { .. }
                )
            }) {
                budget.charge_edit()?;
                edits.push(column_edit(*component, new_name));
            }
        }
        Some(StructuredColumns::Range { start, end }) => {
            if case_insensitive_eq(start, old_name)
                && let Some(component) = components.iter().find(|component| {
                    matches!(
                        component.kind(),
                        SourceComponentKind::StructuredColumnStart { .. }
                    )
                })
            {
                budget.charge_edit()?;
                edits.push(column_edit(*component, new_name));
            }
            if case_insensitive_eq(end, old_name)
                && let Some(component) = components.iter().find(|component| {
                    matches!(
                        component.kind(),
                        SourceComponentKind::StructuredColumnEnd { .. }
                    )
                })
            {
                budget.charge_edit()?;
                edits.push(column_edit(*component, new_name));
            }
        }
        _ => {}
    }
    Ok(())
}

fn column_edit(component: SourceComponent, new_name: &str) -> SourceEdit {
    let grouped = match component.kind() {
        SourceComponentKind::StructuredColumn { grouped }
        | SourceComponentKind::StructuredColumnStart { grouped }
        | SourceComponentKind::StructuredColumnEnd { grouped } => grouped,
        _ => false,
    };
    let escaped = escape_structured_name(new_name);
    let replacement = if !grouped && structured_column_needs_grouping(new_name) {
        format!("[{escaped}]")
    } else {
        escaped
    };
    SourceEdit::new(component.span(), replacement)
}

fn escape_structured_name(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for character in name.chars() {
        if matches!(character, '[' | ']' | '#' | '\'' | '@') {
            escaped.push('\'');
        }
        escaped.push(character);
    }
    escaped
}

pub(crate) fn render_unqualified_structured_column(name: &str) -> String {
    let escaped = escape_structured_name(name);
    if structured_column_needs_grouping(name) {
        format!("[[{escaped}]]")
    } else {
        format!("[{escaped}]")
    }
}

fn component(components: &[SourceComponent], kind: SourceComponentKind) -> Option<SourceComponent> {
    components
        .iter()
        .copied()
        .find(|component| component.kind() == kind)
}

fn formula_may_reference_target(
    source: &str,
    request: &FormulaRewriteRequest<'_>,
    budget: &FormulaRewriteBudget<'_>,
) -> Result<bool, FormulaRewriteError> {
    budget.check_cancelled()?;
    let defaults = CalculationLimits::default();
    if source.len() as u64 > defaults.max_formula_source_bytes() {
        return Ok(true);
    }
    let Ok(lexed) = lex_spanned(source, defaults.max_formula_tokens()) else {
        return Ok(true);
    };
    let result = match request {
        FormulaRewriteRequest::Table { old_name, .. } => {
            lexed
                .tokens
                .iter()
                .enumerate()
                .any(|(index, token)| match &token.token {
                    Token::StructuredRef(raw) => match parse_structured_reference(raw) {
                        Ok(parsed) => {
                            parsed
                                .reference
                                .table
                                .is_some_and(|table| case_insensitive_eq(&table, old_name))
                                && !structured_token_is_external(&lexed.tokens, index)
                        }
                        Err(_) => {
                            raw_structured_targets_table(raw, old_name, false)
                                && !structured_token_is_external(&lexed.tokens, index)
                        }
                    },
                    _ => false,
                })
        }
        FormulaRewriteRequest::TableColumn {
            table_name,
            old_name,
            owner_is_target_table,
            ..
        } => lexed
            .tokens
            .iter()
            .enumerate()
            .any(|(index, token)| match &token.token {
                Token::StructuredRef(raw) => match parse_structured_reference(raw) {
                    Ok(parsed) => {
                        let targets_table = parsed
                            .reference
                            .table
                            .as_deref()
                            .map_or(*owner_is_target_table, |table| {
                                case_insensitive_eq(table, table_name)
                            });
                        targets_table
                            && structured_reference_has_column(&parsed.reference, old_name)
                            && !structured_token_is_external(&lexed.tokens, index)
                    }
                    Err(_) => {
                        raw_structured_targets_table(raw, table_name, *owner_is_target_table)
                            && !structured_token_is_external(&lexed.tokens, index)
                    }
                },
                _ => false,
            }),
        FormulaRewriteRequest::Sheet { old_name, .. } => {
            lexed.tokens.windows(2).enumerate().any(|(index, tokens)| {
                let (Token::Ident(name) | Token::QuotedSheet(name), Token::Bang) =
                    (&tokens[0].token, &tokens[1].token)
                else {
                    return false;
                };
                !sheet_token_is_external(&lexed.tokens, index)
                    && quoted_or_single_sheet_targets(name, old_name)
            }) || lexed.tokens.windows(4).enumerate().any(|(index, tokens)| {
                matches!(
                    (
                        &tokens[0].token,
                        &tokens[1].token,
                        &tokens[2].token,
                        &tokens[3].token
                    ),
                    (
                        Token::Ident(name) | Token::QuotedSheet(name),
                        Token::Colon,
                        Token::Ident(end) | Token::QuotedSheet(end),
                        Token::Bang
                    ) if !sheet_token_is_external(&lexed.tokens, index)
                        && (case_insensitive_eq(name, old_name)
                            || case_insensitive_eq(end, old_name))
                )
            })
        }
    };
    budget.check_cancelled()?;
    Ok(result)
}

fn quoted_or_single_sheet_targets(name: &str, old_name: &str) -> bool {
    match name.split_once(':') {
        Some((start, end)) => {
            case_insensitive_eq(start, old_name) || case_insensitive_eq(end, old_name)
        }
        None => case_insensitive_eq(name, old_name),
    }
}

fn sheet_token_is_external(tokens: &[super::lexer::SpannedToken], index: usize) -> bool {
    matches!(
        tokens
            .get(index.saturating_sub(1))
            .map(|token| &token.token),
        Some(Token::ExternalWorkbook(_))
    ) || matches!(
        tokens.get(index).map(|token| &token.token),
        Some(Token::QuotedSheet(name)) if name.contains(']')
    )
}

fn structured_token_is_external(tokens: &[super::lexer::SpannedToken], index: usize) -> bool {
    let Some(previous_index) = index.checked_sub(1) else {
        return false;
    };
    if !matches!(tokens[previous_index].token, Token::Bang) {
        return false;
    }
    let start = index.saturating_sub(5);
    tokens[start..previous_index].iter().any(|token| {
        matches!(token.token, Token::ExternalWorkbook(_))
            || matches!(&token.token, Token::QuotedSheet(name) if name.contains(']'))
    })
}

fn raw_structured_targets_table(raw: &str, table_name: &str, owner_is_target: bool) -> bool {
    let Some((prefix, _)) = raw.split_once('[') else {
        return false;
    };
    let prefix = prefix.trim();
    if prefix.is_empty() {
        owner_is_target
    } else {
        case_insensitive_eq(prefix, table_name)
    }
}

fn structured_reference_has_column(reference: &StructuredReference, target: &str) -> bool {
    match &reference.columns {
        Some(StructuredColumns::Single(column)) => case_insensitive_eq(column, target),
        Some(StructuredColumns::Range { start, end }) => {
            case_insensitive_eq(start, target) || case_insensitive_eq(end, target)
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FormulaRewriteBudget, FormulaRewriteError, FormulaRewriteLimitKind, FormulaRewriteLimits,
        FormulaRewriteRequest, raw_structured_targets_table, rewrite_formula,
    };

    fn rewrite(source: &str, request: FormulaRewriteRequest<'_>) -> Option<String> {
        let mut budget = FormulaRewriteBudget::new(FormulaRewriteLimits::UNBOUNDED, &|| false);
        rewrite_formula(source, &request, &mut budget).expect(source)
    }

    #[test]
    fn rewrites_only_typed_table_and_column_components() {
        let request = FormulaRewriteRequest::Table {
            old_name: "Sales",
            new_name: "Orders",
        };
        assert_eq!(
            rewrite("SUM(Sales[Amount])+\"Sales[Amount]\"+A1", request),
            Some("SUM(Orders[Amount])+\"Sales[Amount]\"+A1".to_owned())
        );

        let request = FormulaRewriteRequest::TableColumn {
            table_name: "Sales",
            old_name: "Amount",
            new_name: "Gross.Amount",
            owner_is_target_table: false,
        };
        assert_eq!(
            rewrite(
                "SUM(Sales[Amount],Other[Amount],[Amount],Sales[[Amount]:[Tax]])",
                request
            ),
            Some(
                "SUM(Sales[[Gross.Amount]],Other[Amount],[Amount],Sales[[Gross.Amount]:[Tax]])"
                    .to_owned()
            )
        );
    }

    #[test]
    fn rewrites_owner_unqualified_and_preserves_special_escaping() {
        let request = FormulaRewriteRequest::TableColumn {
            table_name: "Sales",
            old_name: "Amount",
            new_name: "O'Brien]#",
            owner_is_target_table: true,
        };
        assert_eq!(
            rewrite("[@Amount]+[[#Data],[Amount]]", request),
            Some("[@[O''Brien']'#]]+[[#Data],[O''Brien']'#]]".to_owned())
        );
    }

    #[test]
    fn rewrites_sheet_components_without_normalizing_formula() {
        let request = FormulaRewriteRequest::Sheet {
            old_name: "Old",
            new_name: "New Sheet",
        };
        assert_eq!(
            rewrite("SUM( Old!A1 , 'Old'!Tax , Old:End!B2 )", request),
            Some("SUM( 'New Sheet'!A1 , 'New Sheet'!Tax , 'New Sheet':End!B2 )".to_owned())
        );
    }

    #[test]
    fn uses_the_workbook_unicode_case_insensitive_identity() {
        assert_eq!(
            rewrite(
                "SUM(äsales[ämount])",
                FormulaRewriteRequest::Table {
                    old_name: "ÄSales",
                    new_name: "Orders",
                }
            ),
            Some("SUM(Orders[ämount])".to_owned())
        );
        assert_eq!(
            rewrite(
                "Sales[ämount]",
                FormulaRewriteRequest::TableColumn {
                    table_name: "Sales",
                    old_name: "Ämount",
                    new_name: "Gross",
                    owner_is_target_table: false,
                }
            ),
            Some("Sales[Gross]".to_owned())
        );
        assert_eq!(
            rewrite(
                "ä!A1",
                FormulaRewriteRequest::Sheet {
                    old_name: "Ä",
                    new_name: "Data",
                }
            ),
            Some("Data!A1".to_owned())
        );
    }

    #[test]
    fn malformed_candidate_probe_handles_quoted_3d_and_ignores_external_targets() {
        let mut budget = FormulaRewriteBudget::new(FormulaRewriteLimits::UNBOUNDED, &|| false);
        let local = rewrite_formula(
            "'Old:End'!A1+",
            &FormulaRewriteRequest::Sheet {
                old_name: "Old",
                new_name: "New",
            },
            &mut budget,
        );
        assert!(matches!(local, Err(FormulaRewriteError::Parse { .. })));

        for source in [
            "[Book.xlsx]Old!A1+",
            "'[Book.xlsx]Old'!A1+",
            "[Book.xlsx]Sheet!Sales[Amount]+",
        ] {
            let request = if source.contains("Sales") {
                FormulaRewriteRequest::Table {
                    old_name: "Sales",
                    new_name: "Orders",
                }
            } else {
                FormulaRewriteRequest::Sheet {
                    old_name: "Old",
                    new_name: "New",
                }
            };
            let mut budget = FormulaRewriteBudget::new(FormulaRewriteLimits::UNBOUNDED, &|| false);
            assert_eq!(
                rewrite_formula(source, &request, &mut budget).expect(source),
                None
            );
        }
    }

    #[test]
    fn ast_and_source_edit_limits_stop_before_unbounded_planning() {
        let mut ast_budget = FormulaRewriteBudget::new(
            FormulaRewriteLimits {
                max_ast_nodes: 1,
                ..FormulaRewriteLimits::UNBOUNDED
            },
            &|| false,
        );
        assert!(matches!(
            rewrite_formula(
                "Old!A1+Old!A2",
                &FormulaRewriteRequest::Sheet {
                    old_name: "Old",
                    new_name: "New",
                },
                &mut ast_budget,
            ),
            Err(FormulaRewriteError::LimitExceeded {
                kind: FormulaRewriteLimitKind::AstNodes,
                ..
            })
        ));

        let mut edit_budget = FormulaRewriteBudget::new(
            FormulaRewriteLimits {
                max_source_edits: 1,
                ..FormulaRewriteLimits::UNBOUNDED
            },
            &|| false,
        );
        assert!(matches!(
            rewrite_formula(
                "Old!A1+Old!A2",
                &FormulaRewriteRequest::Sheet {
                    old_name: "Old",
                    new_name: "New",
                },
                &mut edit_budget,
            ),
            Err(FormulaRewriteError::LimitExceeded {
                kind: FormulaRewriteLimitKind::SourceEdits,
                ..
            })
        ));
    }

    #[test]
    fn malformed_structured_candidates_are_scoped_to_the_target_table() {
        assert!(raw_structured_targets_table(
            "Sales[[Amount]",
            "sales",
            false
        ));
        assert!(!raw_structured_targets_table(
            "Other[[Amount]",
            "Sales",
            false
        ));
        assert!(raw_structured_targets_table(
            "[[#Data],[Amount]",
            "Sales",
            true
        ));
        assert!(!raw_structured_targets_table(
            "[[#Data],[Amount]",
            "Sales",
            false
        ));
    }
}
