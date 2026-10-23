use super::ast::{
    Expr, Reference, SheetPrefix, StructuredColumns, StructuredReference,
    structured_column_needs_grouping, structured_simple_column_needs_grouping,
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
    /// Renames and deletes several sheets at once, so each reference is rewritten exactly once.
    Sheets(SheetChanges<'a>),
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

/// Sheets renamed and deleted together, every name as the formulas spell it before the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SheetChanges<'a> {
    /// `(old, new)` names of renamed sheets.
    pub(crate) renames: &'a [(&'a str, &'a str)],
    /// Names of deleted sheets.
    pub(crate) deleted: &'a [&'a str],
    /// Every tab name in tab order, deleted tabs and tabs that are not worksheets included. A 3-D
    /// reference whose end sheet is deleted moves that end along this order.
    pub(crate) tabs: &'a [&'a str],
    /// Names of the tabs that are not worksheets, such as chartsheets, which a moved 3-D end
    /// passes over as Excel does.
    pub(crate) non_worksheets: &'a [&'a str],
    /// Display names of the tables on deleted sheets.
    pub(crate) deleted_tables: &'a [&'a str],
}

impl SheetChanges<'_> {
    fn renamed(&self, name: &str) -> Option<&str> {
        self.renames
            .iter()
            .find(|(old_name, _)| case_insensitive_eq(name, old_name))
            .map(|(_, new_name)| *new_name)
    }

    fn is_deleted(&self, name: &str) -> bool {
        self.deleted
            .iter()
            .any(|deleted| case_insensitive_eq(name, deleted))
    }

    fn is_deleted_table(&self, name: &str) -> bool {
        self.deleted_tables
            .iter()
            .any(|table| case_insensitive_eq(name, table))
    }

    fn tab_position(&self, name: &str) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| case_insensitive_eq(name, tab))
    }

    /// Resolves the sheets a reference from `start` through `end` keeps: each deleted end moves to
    /// the nearest remaining worksheet toward the other end. `None` means no worksheet remains, or
    /// a deleted end's span cannot be placed in the tab order, and the reference becomes `#REF!`.
    fn remaining_span<'name>(
        &'name self,
        start: &'name str,
        end: &'name str,
    ) -> Option<(&'name str, &'name str)> {
        if !self.is_deleted(start) && !self.is_deleted(end) {
            return Some((start, end));
        }
        let first = self.tab_position(start)?;
        let last = self.tab_position(end)?;
        let nearest_remaining = |from: usize, to: usize| {
            let mut position = from;
            if !self.is_deleted(self.tabs[from]) {
                return Some(self.tabs[from]);
            }
            loop {
                let tab = self.tabs[position];
                if !self.is_deleted(tab)
                    && !self
                        .non_worksheets
                        .iter()
                        .any(|other| case_insensitive_eq(tab, other))
                {
                    return Some(tab);
                }
                if position == to {
                    return None;
                }
                position = if from < to {
                    position + 1
                } else {
                    position - 1
                };
            }
        };
        Some((
            nearest_remaining(first, last)?,
            nearest_remaining(last, first)?,
        ))
    }
}

impl FormulaRewriteRequest<'_> {
    fn renamed_sheet(&self, name: &str) -> Option<&str> {
        match self {
            Self::Sheet { old_name, new_name } => {
                case_insensitive_eq(name, old_name).then_some(*new_name)
            }
            Self::Sheets(changes) => changes.renamed(name),
            Self::Table { .. } | Self::TableColumn { .. } => None,
        }
    }

    const fn sheet_changes(&self) -> Option<&SheetChanges<'_>> {
        match self {
            Self::Sheets(changes) => Some(changes),
            Self::Sheet { .. } | Self::Table { .. } | Self::TableColumn { .. } => None,
        }
    }

    /// Reports whether a deletion turns `expr`, read through parentheses, into `#REF!` as a whole:
    /// a reference or qualified name on a deleted sheet, a 3-D reference no tab of which remains,
    /// or a structured reference to a table on a deleted sheet.
    fn deletes_reference(&self, mut expr: &Expr) -> bool {
        let Some(changes) = self.sheet_changes() else {
            return false;
        };
        while let Expr::Paren(inner) = expr {
            expr = inner;
        }
        match expr {
            Expr::Ref(Reference {
                sheet: Some(prefix),
                ..
            })
            | Expr::QualifiedName { sheet: prefix, .. } => changes
                .remaining_span(
                    &prefix.name,
                    prefix.end_name.as_deref().unwrap_or(&prefix.name),
                )
                .is_none(),
            Expr::StructuredRef(reference) => reference
                .table
                .as_deref()
                .is_some_and(|table| changes.is_deleted_table(table)),
            _ => false,
        }
    }
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
        if replaced_by_deletion(expr, request) {
            budget.charge_edit()?;
            edits.push(SourceEdit::new(source_node.span(), DELETED_REFERENCE));
            continue;
        }
        match expr {
            Expr::Ref(reference) => {
                if let Some(prefix) = &reference.sheet {
                    plan_sheet_prefix_edits(
                        source,
                        prefix,
                        source_node.components(),
                        request,
                        &mut edits,
                        budget,
                    )?;
                }
            }
            Expr::QualifiedName { sheet, .. } => {
                plan_sheet_prefix_edits(
                    source,
                    sheet,
                    source_node.components(),
                    request,
                    &mut edits,
                    budget,
                )?;
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
                FormulaRewriteRequest::Sheet { .. } | FormulaRewriteRequest::Sheets(_) => {}
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

pub(super) fn push_children<'a>(
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
        | Expr::BuiltinCallable(_)
        | Expr::Missing => Vec::new(),
    };
    debug_assert_eq!(expressions.len(), source_node.children().len());
    expressions
        .drain(..)
        .zip(source_node.children())
        .rev()
        .for_each(|pair| stack.push(pair));
}

/// The text a reference to a deleted sheet or table becomes, as Excel saves it.
const DELETED_REFERENCE: &str = "#REF!";

/// Reports whether a sheet deletion replaces `expr` as a whole with `#REF!`: a reference to a
/// deleted sheet or table, or a range, spill reference, or invocation whose reference operand is
/// one, since `#REF!` cannot stand in that operand's place alone.
fn replaced_by_deletion(expr: &Expr, request: &FormulaRewriteRequest<'_>) -> bool {
    match expr {
        Expr::Ref(_) | Expr::QualifiedName { .. } | Expr::StructuredRef(_) => {
            request.deletes_reference(expr)
        }
        Expr::Range { start, end } => {
            request.deletes_reference(start) || request.deletes_reference(end)
        }
        Expr::SpillRef(operand) => request.deletes_reference(operand),
        Expr::Invoke { callee, .. } => request.deletes_reference(callee),
        _ => false,
    }
}

fn plan_sheet_prefix_edits(
    source: &str,
    prefix: &SheetPrefix,
    components: &[SourceComponent],
    request: &FormulaRewriteRequest<'_>,
    edits: &mut Vec<SourceEdit>,
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<(), FormulaRewriteError> {
    let Some(qualifier) = component(components, SourceComponentKind::SheetQualifier) else {
        return Ok(());
    };
    let Some(end_name) = prefix.end_name.as_deref() else {
        if let Some(new_name) = request.renamed_sheet(&prefix.name) {
            budget.charge_edit()?;
            edits.push(SourceEdit::new(
                qualifier.span(),
                replacement_sheet_name(source, qualifier.span(), new_name),
            ));
        }
        return Ok(());
    };
    let Some(range_end) = component(components, SourceComponentKind::SheetRangeEnd) else {
        return Ok(());
    };
    // A deleted end of a 3-D reference moves to the nearest remaining tab, spelled with its new
    // name when that tab was renamed as well.
    let deleted = |name: &str| {
        request
            .sheet_changes()
            .is_some_and(|changes| changes.is_deleted(name))
    };
    let (start, end) = request
        .sheet_changes()
        .and_then(|changes| changes.remaining_span(&prefix.name, end_name))
        .unwrap_or((&prefix.name, end_name));
    let collapsed = (deleted(&prefix.name) || deleted(end_name)) && case_insensitive_eq(start, end);
    let bytes = source.as_bytes();
    let opening = qualifier
        .span()
        .start
        .checked_sub(1)
        .filter(|offset| bytes.get(*offset) == Some(&b'\''));
    let closing =
        (bytes.get(range_end.span().end) == Some(&b'\'')).then_some(range_end.span().end + 1);
    // The whole prefix, with the quotes around either end.
    let prefix_span = SourceSpan::new(
        opening.unwrap_or(qualifier.span().start),
        closing.unwrap_or(range_end.span().end),
    );
    if !collapsed {
        let new_name = |original: &str, remaining| {
            if deleted(original) {
                Some(request.renamed_sheet(remaining).unwrap_or(remaining))
            } else {
                request.renamed_sheet(original)
            }
        };
        let new_start = new_name(&prefix.name, start);
        let new_end = new_name(end_name, end);
        if new_start.is_none() && new_end.is_none() {
            return Ok(());
        }
        let final_start = new_start.unwrap_or(&prefix.name);
        let final_end = new_end.unwrap_or(end_name);
        // MS-XLSX quotes a sheet range as one token, so `My Data:S3` is written `'My Data:S3'`
        // rather than `'My Data':S3`.
        let wholly_quoted = opening.is_some()
            && closing.is_some()
            && source.get(qualifier.span().end..range_end.span().start) == Some(":");
        if !wholly_quoted
            && (sheet_name_needs_quotes(final_start) || sheet_name_needs_quotes(final_end))
        {
            budget.charge_edit()?;
            edits.push(SourceEdit::new(
                prefix_span,
                format!(
                    "'{}:{}'",
                    final_start.replace('\'', "''"),
                    final_end.replace('\'', "''")
                ),
            ));
            return Ok(());
        }
        for (component, new_name) in [(qualifier, new_start), (range_end, new_end)] {
            if let Some(new_name) = new_name {
                budget.charge_edit()?;
                edits.push(SourceEdit::new(
                    component.span(),
                    replacement_sheet_name(source, component.span(), new_name),
                ));
            }
        }
        return Ok(());
    }
    // `'S2:S3'!A1` keeps one sheet once `S2` is deleted and becomes `'S3'!A1`.
    let name = request.renamed_sheet(start).unwrap_or(start);
    let escaped = name.replace('\'', "''");
    let quoted = opening.is_some() || closing.is_some() || sheet_name_needs_quotes(name);
    budget.charge_edit()?;
    edits.push(SourceEdit::new(
        prefix_span,
        if quoted {
            format!("'{escaped}'")
        } else {
            escaped
        },
    ));
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
                let simple = reference.table.is_some() && reference.items.is_empty();
                edits.push(column_edit(*component, new_name, simple));
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
                edits.push(column_edit(*component, new_name, false));
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
                edits.push(column_edit(*component, new_name, false));
            }
        }
        _ => {}
    }
    Ok(())
}

/// `simple` marks a table-qualified reference to one column, such as `Table1[unit_price]`, which
/// Excel saves with fewer names bracketed than a selector list needs.
fn column_edit(component: SourceComponent, new_name: &str, simple: bool) -> SourceEdit {
    let grouped = match component.kind() {
        SourceComponentKind::StructuredColumn { grouped }
        | SourceComponentKind::StructuredColumnStart { grouped }
        | SourceComponentKind::StructuredColumnEnd { grouped } => grouped,
        _ => false,
    };
    let needs_grouping = if simple {
        structured_simple_column_needs_grouping(new_name)
    } else {
        structured_column_needs_grouping(new_name)
    };
    let replacement = if grouped {
        escape_structured_name(new_name, false)
    } else if needs_grouping {
        format!("[{}]", escape_structured_name(new_name, false))
    } else {
        // Excel saves `Table1[a@b]` with a bare `@`; a leading one would mean this row.
        escape_structured_name(new_name, simple)
    };
    SourceEdit::new(component.span(), replacement)
}

/// Escapes `[`, `]`, `#`, `'`, and `@` with `'`, leaving an `@` after the first character bare
/// when `bare_inner_at` is set.
fn escape_structured_name(name: &str, bare_inner_at: bool) -> String {
    let mut escaped = String::with_capacity(name.len());
    for (index, character) in name.char_indices() {
        let bare = bare_inner_at && index > 0 && character == '@';
        if matches!(character, '[' | ']' | '#' | '\'' | '@') && !bare {
            escaped.push('\'');
        }
        escaped.push(character);
    }
    escaped
}

pub(crate) fn render_unqualified_structured_column(name: &str) -> String {
    let escaped = escape_structured_name(name, false);
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
        FormulaRewriteRequest::Sheet { .. } | FormulaRewriteRequest::Sheets(_) => {
            let targeted = |name: &str| {
                request.renamed_sheet(name).is_some()
                    || request
                        .sheet_changes()
                        .is_some_and(|changes| changes.is_deleted(name))
            };
            let deleted_table = |raw: &str| {
                request.sheet_changes().is_some_and(|changes| {
                    match parse_structured_reference(raw) {
                        Ok(parsed) => parsed
                            .reference
                            .table
                            .is_some_and(|table| changes.is_deleted_table(&table)),
                        Err(_) => changes
                            .deleted_tables
                            .iter()
                            .any(|table| raw_structured_targets_table(raw, table, false)),
                    }
                })
            };
            lexed
                .tokens
                .iter()
                .enumerate()
                .any(|(index, token)| match &token.token {
                    Token::StructuredRef(raw) => {
                        deleted_table(raw) && !structured_token_is_external(&lexed.tokens, index)
                    }
                    _ => false,
                })
                || lexed.tokens.windows(2).enumerate().any(|(index, tokens)| {
                    let (Token::Ident(name) | Token::QuotedSheet(name), Token::Bang) =
                        (&tokens[0].token, &tokens[1].token)
                    else {
                        return false;
                    };
                    !sheet_token_is_external(&lexed.tokens, index)
                        && quoted_or_single_sheet_targets(name, targeted)
                })
                || lexed.tokens.windows(4).enumerate().any(|(index, tokens)| {
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
                            && (targeted(name) || targeted(end))
                    )
                })
        }
    };
    budget.check_cancelled()?;
    Ok(result)
}

fn quoted_or_single_sheet_targets(name: &str, renamed: impl Fn(&str) -> bool) -> bool {
    match name.split_once(':') {
        Some((start, end)) => renamed(start) || renamed(end),
        None => renamed(name),
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
        FormulaRewriteRequest, SheetChanges, raw_structured_targets_table, rewrite_formula,
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
                "SUM(Sales[Gross.Amount],Other[Amount],[Amount],Sales[[Gross.Amount]:[Tax]])"
                    .to_owned()
            )
        );

        let request = FormulaRewriteRequest::TableColumn {
            table_name: "Sales",
            old_name: "Amount",
            new_name: "unit_price",
            owner_is_target_table: true,
        };
        assert_eq!(
            rewrite(
                "Sales[Amount]+[Amount]+Sales[[#Data],Amount]+Sales[@Amount]",
                request
            ),
            Some(
                "Sales[unit_price]+[[unit_price]]+Sales[[#Data],[unit_price]]+Sales[@[unit_price]]"
                    .to_owned()
            )
        );

        // Only a leading `@`, which would mean this row, keeps its escape in a simple reference.
        for (new_name, rewritten) in [
            ("a@b", "Sales[a@b]+Sales[[a'@b]]+Sales[[#Data],[a'@b]]"),
            ("@b", "Sales['@b]+Sales[['@b]]+Sales[[#Data],['@b]]"),
        ] {
            let request = FormulaRewriteRequest::TableColumn {
                table_name: "Sales",
                old_name: "Amount",
                new_name,
                owner_is_target_table: false,
            };
            assert_eq!(
                rewrite(
                    "Sales[Amount]+Sales[[Amount]]+Sales[[#Data],Amount]",
                    request
                ),
                Some(rewritten.to_owned()),
                "{new_name}"
            );
        }
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
            Some("SUM( 'New Sheet'!A1 , 'New Sheet'!Tax , 'New Sheet:End'!B2 )".to_owned())
        );
    }

    #[test]
    fn rewrites_simultaneous_sheet_renames_once() {
        let renames = [("Old", "New"), ("New", "Old")];
        assert_eq!(
            rewrite(
                "Old!A1+'new'!B2+Old:New!C3",
                FormulaRewriteRequest::Sheets(SheetChanges {
                    renames: &renames,
                    deleted: &[],
                    tabs: &[],
                    non_worksheets: &[],
                    deleted_tables: &[],
                })
            ),
            Some("New!A1+'Old'!B2+New:Old!C3".to_owned())
        );
    }

    const TABS: &[&str] = &["S1", "S2", "S3", "S4"];

    fn deletion<'a>(deleted: &'a [&'a str]) -> FormulaRewriteRequest<'a> {
        FormulaRewriteRequest::Sheets(SheetChanges {
            renames: &[],
            deleted,
            tabs: TABS,
            non_worksheets: &[],
            deleted_tables: &["Things"],
        })
    }

    #[test]
    fn deleted_sheet_references_become_ref_errors_as_excel_saves_them() {
        for (source, rewritten) in [
            ("'S2'!A1", "#REF!"),
            ("SUM('S2'!A1:A3)", "SUM(#REF!)"),
            ("'S2'!A1+'S3'!A1", "#REF!+'S3'!A1"),
            ("'S2'!LocalS2", "#REF!"),
            ("s2!$A$1*2", "#REF!*2"),
            ("SUM(Things[val])", "SUM(#REF!)"),
            ("SUM(things[[#Totals],[val]])", "SUM(#REF!)"),
            ("SUM('S2'!A1:'S2'!B2)", "SUM(#REF!)"),
            ("SUM(A1:S2!B2)", "SUM(#REF!)"),
            ("SUM(('S2'!A1):B2)", "SUM(#REF!)"),
            ("'S2'!A1#", "#REF!"),
            ("_xlfn.ANCHORARRAY('S2'!A1)", "#REF!"),
            ("@'S2'!A1:A3", "@#REF!"),
            ("_xlfn.SINGLE('S2'!A1:A3)", "_xlfn.SINGLE(#REF!)"),
            ("SUM(('S2'!A1,'S3'!A1))", "SUM((#REF!,'S3'!A1))"),
            ("SUM('S2'!A1:A3 'S3'!A1:A3)", "SUM(#REF! 'S3'!A1:A3)"),
            ("-('S2'!A1)", "-(#REF!)"),
            ("'S2'!Lambda(1)", "#REF!"),
        ] {
            assert_eq!(
                rewrite(source, deletion(&["S2"])),
                Some(rewritten.to_owned()),
                "{source}"
            );
        }
        for unchanged in [
            "INDIRECT(\"'S2'!A1\")",
            "[Book.xlsx]S2!A1",
            "'[Book.xlsx]S2'!A1",
            "'S3'!A1+Other[val]+[@val]",
            "SUM('S1:S3'!A1)",
            "S22!A1",
        ] {
            assert_eq!(rewrite(unchanged, deletion(&["S2"])), None, "{unchanged}");
        }
    }

    #[test]
    fn deleted_ends_of_3d_references_move_to_the_nearest_remaining_tab() {
        for (deleted, source, rewritten) in [
            (&["S2"][..], "SUM('S2:S4'!A1)", "SUM('S3:S4'!A1)"),
            (&["S2"], "SUM('S2:S3'!A1)", "SUM('S3'!A1)"),
            (&["S2"], "SUM(S1:S2!A1)", "SUM(S1!A1)"),
            (&["S2"], "SUM(S4:S2!A1)", "SUM(S4:S3!A1)"),
            (&["S2"], "SUM('S2:S2'!A1)", "SUM(#REF!)"),
            (&["S2"], "SUM(S2:Missing!A1)", "SUM(#REF!)"),
            (&["S3"], "SUM('S1:S3'!A1)", "SUM('S1:S2'!A1)"),
            (&["S3"], "SUM(S1:'S3'!A1)", "SUM(S1:'S2'!A1)"),
            (&["S4"], "SUM('S2:S4'!A1)", "SUM('S2:S3'!A1)"),
            (&["S1"], "SUM('S1:S3'!A1)", "SUM('S2:S3'!A1)"),
            (&["S2", "S3"], "SUM(S2:S4!A1)", "SUM(S4!A1)"),
            (&["S2", "S3"], "SUM('S2:S3'!A1)", "SUM(#REF!)"),
            (&["S2", "S4"], "SUM(S1:S4!A1)", "SUM(S1:S3!A1)"),
        ] {
            assert_eq!(
                rewrite(source, deletion(deleted)),
                Some(rewritten.to_owned()),
                "{deleted:?} {source}"
            );
        }
    }

    #[test]
    fn sheet_changes_rename_and_delete_in_one_rewrite() {
        let changes = |deleted, tabs| {
            FormulaRewriteRequest::Sheets(SheetChanges {
                renames: &[("S3", "New Name"), ("S1", "S2")],
                deleted,
                tabs,
                non_worksheets: &[],
                deleted_tables: &[],
            })
        };
        for (source, rewritten) in [
            ("SUM('S2:S3'!A1)", "SUM('New Name'!A1)"),
            ("SUM(S2:S4!A1)", "SUM('New Name:S4'!A1)"),
            ("SUM('S2:S4'!A1)", "SUM('New Name:S4'!A1)"),
            ("SUM(S1:S4!A1)", "SUM(S2:S4!A1)"),
            ("S2!A1+S1!A1+S3!A1", "#REF!+S2!A1+'New Name'!A1"),
        ] {
            assert_eq!(
                rewrite(source, changes(&["S2"], TABS)),
                Some(rewritten.to_owned()),
                "{source}"
            );
        }
    }

    #[test]
    fn a_3d_prefix_whose_end_needs_quotes_is_quoted_as_a_whole() {
        // MS-XLSX quotes a sheet range as one token, `'My Data:S4'!A1`, never per end.
        let tabs = &["S1", "S2", "My Data", "S4"];
        for (source, rewritten) in [
            ("SUM(S2:S4!A1)", "SUM('My Data:S4'!A1)"),
            ("SUM('S2:S4'!A1)", "SUM('My Data:S4'!A1)"),
            ("SUM(S1:S2!A1)", "SUM(S1!A1)"),
            ("SUM(S1:'S2'!A1)", "SUM('S1'!A1)"),
            ("SUM(S4:S2!A1)", "SUM('S4:My Data'!A1)"),
        ] {
            assert_eq!(
                rewrite(
                    source,
                    FormulaRewriteRequest::Sheets(SheetChanges {
                        renames: &[],
                        deleted: &["S2"],
                        tabs,
                        non_worksheets: &[],
                        deleted_tables: &[],
                    })
                ),
                Some(rewritten.to_owned()),
                "{source}"
            );
        }
        assert_eq!(
            rewrite(
                "SUM(S1:'S2'!A1)",
                FormulaRewriteRequest::Sheet {
                    old_name: "S1",
                    new_name: "First Sheet",
                }
            ),
            Some("SUM('First Sheet:S2'!A1)".to_owned())
        );
        assert_eq!(
            rewrite(
                "SUM('S1:S2'!A1)",
                FormulaRewriteRequest::Sheet {
                    old_name: "S1",
                    new_name: "O'Hara",
                }
            ),
            Some("SUM('O''Hara:S2'!A1)".to_owned())
        );
    }

    #[test]
    fn deleted_3d_ends_skip_tabs_that_are_not_worksheets() {
        // Excel 16.114 with tabs S1, S2, a chartsheet, S3: deleting S2 turns `'S2:S3'!A1` into
        // `'S3'!A1`, since a 3-D end moves to the nearest worksheet.
        for (deleted, source, rewritten) in [
            ("S2", "SUM('S2:S3'!A1)", "SUM('S3'!A1)"),
            ("S2", "SUM(S2:S4!A1)", "SUM(S3:S4!A1)"),
            ("S3", "SUM(S1:S3!A1)", "SUM(S1:S2!A1)"),
            ("S3", "SUM(Chart1:S3!A1)", "SUM(#REF!)"),
            ("S4", "SUM(Chart1:S4!A1)", "SUM(Chart1:S3!A1)"),
        ] {
            assert_eq!(
                rewrite(
                    source,
                    FormulaRewriteRequest::Sheets(SheetChanges {
                        renames: &[],
                        deleted: &[deleted],
                        tabs: &["S1", "S2", "Chart1", "S3", "S4"],
                        non_worksheets: &["Chart1"],
                        deleted_tables: &[],
                    })
                ),
                Some(rewritten.to_owned()),
                "{deleted} {source}"
            );
        }
    }

    #[test]
    fn malformed_formulas_naming_a_deleted_sheet_or_table_fail_closed() {
        for source in ["'S2'!A1+", "S2:S4!A1+", "Things[val]+", "SUM(Things[[val]"] {
            let mut budget = FormulaRewriteBudget::new(FormulaRewriteLimits::UNBOUNDED, &|| false);
            assert!(
                matches!(
                    rewrite_formula(source, &deletion(&["S2"]), &mut budget),
                    Err(FormulaRewriteError::Parse { .. })
                ),
                "{source}"
            );
        }
        for source in ["'S3'!A1+", "Other[val]+", "[Book.xlsx]S2!A1+"] {
            let mut budget = FormulaRewriteBudget::new(FormulaRewriteLimits::UNBOUNDED, &|| false);
            assert_eq!(
                rewrite_formula(source, &deletion(&["S2"]), &mut budget).expect(source),
                None,
                "{source}"
            );
        }
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
