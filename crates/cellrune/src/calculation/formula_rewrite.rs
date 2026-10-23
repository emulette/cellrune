use super::ast::{
    Expr, ExternalReferenceTarget, Reference, SheetPrefix, StructuredColumns, StructuredReference,
    structured_column_needs_grouping, structured_simple_column_needs_grouping,
};
use super::error::ParseErrorCode;
use super::functions::function_evaluator;
use super::grid_rewrite::{
    CopyOffset, DELETED_REFERENCE, GridChange, NodePlan, plan_copy_node, plan_grid_node,
};
use super::lexer::{Token, lex_spanned};
use super::parser::parse_formula_with_limits;
pub(crate) use super::sheet_changes::SheetChanges;
use super::sheet_changes::{SheetOutcome, SpanOutcome};
use super::structured_reference::parse_structured_reference;
use super::syntax::{
    NodeSpanTree, ParsedFormula, SourceComponent, SourceComponentKind, SourceEdit, SourceEditError,
    SourceSpan, apply_source_edits,
};
use super::table_conversion::TableArea;
use super::{CalculationLimitKind, CalculationLimits};
use crate::case_insensitive_eq;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormulaRewriteRequest<'a> {
    Sheet {
        old_name: &'a str,
        new_name: &'a str,
    },
    /// Renames, deletes, moves, or copies sheets at once, so each reference is rewritten exactly
    /// once.
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
    /// Renames one defined name, rewriting only the references that resolve to it.
    DefinedName(NameRename<'a>),
    /// Removes columns from one table, so a structured reference that names one of them, alone or
    /// as an end of a column range, becomes `#REF!`.
    TableColumnsRemoved {
        table_name: &'a str,
        removed: &'a [&'a str],
        owner_is_target_table: bool,
    },
    /// Converts one table to a range, so its structured references become A1 references.
    TableToRange {
        area: &'a TableArea<'a>,
        /// Whether the formula belongs to the converted table, where an unqualified structured
        /// reference names it.
        owner_is_target_table: bool,
        /// The formula's row when it is on the table's sheet.
        this_row: Option<u32>,
    },
    /// Moves references through a structural edit of one sheet's grid.
    Grid(GridChange<'a>),
    /// Shifts the relative parts of every reference, as copying a formula does.
    Copy(CopyOffset),
}

/// A defined name renamed in one scope, with what decides which references resolve to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NameRename<'a> {
    pub(crate) old_name: &'a str,
    pub(crate) new_name: &'a str,
    /// The sheet the name is scoped to, as formulas spell it; `None` for workbook scope.
    pub(crate) scope_sheet: Option<&'a str>,
    /// Sheets with a name of their own spelled `old_name`, which a reference on such a sheet or
    /// qualified with it resolves to instead of a workbook name.
    pub(crate) shadowing_sheets: &'a [&'a str],
    /// The sheet the rewritten formula belongs to; `None` for a workbook name's definition or a
    /// chart.
    pub(crate) owner_sheet: Option<&'a str>,
}

impl NameRename<'_> {
    fn shadowed(&self, sheet: &str) -> bool {
        self.shadowing_sheets
            .iter()
            .any(|shadowing| case_insensitive_eq(sheet, shadowing))
    }

    /// Reports whether an unqualified use of `old_name` resolves to the renamed name: a sheet's
    /// own name wins over a workbook name on that sheet.
    fn resolves_unqualified(&self) -> bool {
        match self.scope_sheet {
            None => self.owner_sheet.is_none_or(|owner| !self.shadowed(owner)),
            Some(scope) => self
                .owner_sheet
                .is_some_and(|owner| case_insensitive_eq(owner, scope)),
        }
    }

    /// Reports whether `sheet!old_name` resolves to the renamed name: the sheet's own name, else
    /// the workbook name.
    fn resolves_qualified(&self, sheet: &str) -> bool {
        match self.scope_sheet {
            None => !self.shadowed(sheet),
            Some(scope) => case_insensitive_eq(sheet, scope),
        }
    }
}

impl FormulaRewriteRequest<'_> {
    /// Resolves a single-sheet qualifier.
    fn single_sheet(&self, name: &str) -> SheetOutcome<'_> {
        match self {
            Self::Sheet { old_name, new_name } if case_insensitive_eq(name, old_name) => {
                SheetOutcome::Renamed(new_name)
            }
            Self::Sheets(changes) => changes.single(name),
            Self::Sheet { .. }
            | Self::Table { .. }
            | Self::TableColumn { .. }
            | Self::DefinedName(_)
            | Self::TableColumnsRemoved { .. }
            | Self::TableToRange { .. }
            | Self::Grid(_)
            | Self::Copy(_) => SheetOutcome::Unchanged,
        }
    }

    /// Resolves the qualifier of a 3-D reference.
    fn sheet_span(&self, start: &str, end: &str) -> SpanOutcome<'_> {
        match self {
            Self::Sheet { old_name, new_name } => {
                let start = case_insensitive_eq(start, old_name).then_some(*new_name);
                let end = case_insensitive_eq(end, old_name).then_some(*new_name);
                if start.is_none() && end.is_none() {
                    SpanOutcome::Unchanged
                } else {
                    SpanOutcome::Ends { start, end }
                }
            }
            Self::Sheets(changes) => changes.span(start, end),
            Self::Table { .. }
            | Self::TableColumn { .. }
            | Self::DefinedName(_)
            | Self::TableColumnsRemoved { .. }
            | Self::TableToRange { .. }
            | Self::Grid(_)
            | Self::Copy(_) => SpanOutcome::Unchanged,
        }
    }

    const fn sheet_changes(&self) -> Option<&SheetChanges<'_>> {
        match self {
            Self::Sheets(changes) => Some(changes),
            Self::Sheet { .. }
            | Self::Table { .. }
            | Self::TableColumn { .. }
            | Self::DefinedName(_)
            | Self::TableColumnsRemoved { .. }
            | Self::TableToRange { .. }
            | Self::Grid(_)
            | Self::Copy(_) => None,
        }
    }

    /// Reports whether a deletion turns `expr`, read through parentheses, into `#REF!` as a whole:
    /// a reference or qualified name on a deleted sheet, a 3-D reference no worksheet of which
    /// remains, or a structured reference to a table on a deleted sheet.
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
            | Expr::QualifiedName { sheet: prefix, .. } => match prefix.end_name.as_deref() {
                None => changes.single(&prefix.name) == SheetOutcome::Deleted,
                Some(end) => changes.span(&prefix.name, end) == SpanOutcome::Deleted,
            },
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
    /// A range move would paste over cells this formula reads.
    OverwrittenReference {
        owner: Option<String>,
    },
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
            Self::OverwrittenReference { owner: None } => {
                Self::OverwrittenReference { owner: Some(owner) }
            }
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

    /// Returns a budget that observes the same cancellation but charges nothing to this one.
    pub(crate) fn unmetered(&self) -> FormulaRewriteBudget<'a> {
        Self::new(FormulaRewriteLimits::UNBOUNDED, self.cancelled)
    }

    /// Returns the cancellation check this budget polls.
    pub(crate) fn cancellation(&self) -> &'a dyn Fn() -> bool {
        self.cancelled
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
        let plan = match request {
            FormulaRewriteRequest::Grid(change) => {
                let before = edits.len();
                let plan = plan_grid_node(source, expr, source_node, change, &mut edits)
                    .map_err(|_| FormulaRewriteError::OverwrittenReference { owner: None })?;
                for _ in before..edits.len() {
                    budget.charge_edit()?;
                }
                Some(plan)
            }
            FormulaRewriteRequest::Copy(offset) => {
                let before = edits.len();
                let Ok(plan) = plan_copy_node(expr, source_node, *offset, &mut edits) else {
                    // A copied reference that leaves the grid turns the whole formula into
                    // `#REF!`, as Excel pastes it.
                    budget.charge_edit()?;
                    return Ok(Some(DELETED_REFERENCE.to_owned()));
                };
                for _ in before..edits.len() {
                    budget.charge_edit()?;
                }
                Some(plan)
            }
            _ => None,
        };
        if let Some(plan) = plan {
            if plan == NodePlan::Descend {
                push_children(expr, source_node, &mut stack);
            }
            continue;
        }
        if let Expr::ErrorLit(_) = expr
            && let Some(prefix) = qualified_error_prefix(source, source_node.components())
        {
            // `Sheet1!#REF!` keeps naming its sheet, so a rename or deletion changes it too.
            let deleted = request.sheet_changes().is_some_and(|changes| {
                changes.span(
                    &prefix.name,
                    prefix.end_name.as_deref().unwrap_or(&prefix.name),
                ) == SpanOutcome::Deleted
            });
            if deleted {
                budget.charge_edit()?;
                edits.push(SourceEdit::new(source_node.span(), DELETED_REFERENCE));
            } else {
                plan_sheet_prefix_edits(
                    source,
                    &prefix,
                    source_node.components(),
                    request,
                    &mut edits,
                    budget,
                )?;
            }
            continue;
        }
        if replaced_by_deletion(expr, request) {
            budget.charge_edit()?;
            edits.push(SourceEdit::new(source_node.span(), DELETED_REFERENCE));
            continue;
        }
        if let FormulaRewriteRequest::DefinedName(rename) = request {
            plan_name_edit(source, expr, source_node, rename, &mut edits, budget)?;
        }
        if let (
            FormulaRewriteRequest::TableColumnsRemoved {
                table_name,
                removed,
                owner_is_target_table,
            },
            Expr::StructuredRef(reference),
        ) = (request, expr)
        {
            let targets_table = reference
                .table
                .as_deref()
                .map_or(*owner_is_target_table, |table| {
                    case_insensitive_eq(table, table_name)
                });
            let removed_column = |name: &str| {
                removed
                    .iter()
                    .any(|removed| case_insensitive_eq(name, removed))
            };
            let names_removed = match &reference.columns {
                Some(StructuredColumns::Single(column)) => removed_column(column),
                Some(StructuredColumns::Range { start, end }) => {
                    removed_column(start) || removed_column(end)
                }
                None => false,
            };
            if targets_table && names_removed {
                budget.charge_edit()?;
                edits.push(SourceEdit::new(source_node.span(), DELETED_REFERENCE));
                continue;
            }
        }
        if let FormulaRewriteRequest::TableToRange {
            area,
            owner_is_target_table,
            this_row,
        } = request
        {
            // A bare table name means its data body, as `Things[]` does.
            let bare = StructuredReference {
                table: Some(area.display_name.into()),
                items: Vec::new(),
                columns: None,
            };
            let converted = match expr {
                Expr::StructuredRef(reference)
                    if reference
                        .table
                        .as_deref()
                        .map_or(*owner_is_target_table, |table| {
                            case_insensitive_eq(table, area.display_name)
                        }) =>
                {
                    Some(reference)
                }
                Expr::Name(name) if case_insensitive_eq(name, area.display_name) => Some(&bare),
                _ => None,
            };
            if let Some(reference) = converted {
                budget.charge_edit()?;
                edits.push(SourceEdit::new(
                    source_node.span(),
                    area.a1_text(reference, *this_row),
                ));
                continue;
            }
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
                FormulaRewriteRequest::Sheet { .. }
                | FormulaRewriteRequest::Sheets(_)
                | FormulaRewriteRequest::DefinedName(_)
                | FormulaRewriteRequest::TableColumnsRemoved { .. }
                | FormulaRewriteRequest::TableToRange { .. }
                | FormulaRewriteRequest::Grid(_)
                | FormulaRewriteRequest::Copy(_) => {}
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

/// Plans the edit of a name that resolves to the renamed name: an unqualified or sheet-qualified
/// name, the name of a LAMBDA called like a function, or a `[0]!Name` reference, the form charts
/// use for this workbook's names.
fn plan_name_edit(
    source: &str,
    expr: &Expr,
    source_node: &NodeSpanTree,
    rename: &NameRename<'_>,
    edits: &mut Vec<SourceEdit>,
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<(), FormulaRewriteError> {
    let resolves = match expr {
        Expr::Name(name) => {
            case_insensitive_eq(name, rename.old_name) && rename.resolves_unqualified()
        }
        Expr::QualifiedName { sheet, name } => {
            sheet.end_name.is_none()
                && case_insensitive_eq(name, rename.old_name)
                && rename.resolves_qualified(&sheet.name)
        }
        Expr::ExternalReference(reference) => {
            reference.workbook.as_ref() == "[0]"
                && reference.sheet_end.is_none()
                && matches!(
                    &reference.target,
                    ExternalReferenceTarget::DefinedName(name)
                        if case_insensitive_eq(name, rename.old_name)
                )
                && reference.sheet.as_deref().map_or_else(
                    || rename.scope_sheet.is_none(),
                    |sheet| rename.resolves_qualified(sheet),
                )
        }
        Expr::Call { name, .. } => {
            // A built-in function keeps its name; only a name defined as a LAMBDA is renamed.
            if case_insensitive_eq(name, rename.old_name)
                && function_evaluator(name).is_none()
                && rename.resolves_unqualified()
            {
                let start = source_node.span().start;
                let end = start + name.len();
                if source
                    .get(start..end)
                    .is_some_and(|spelled| case_insensitive_eq(spelled, name))
                {
                    budget.charge_edit()?;
                    edits.push(SourceEdit::new(
                        SourceSpan::new(start, end),
                        rename.new_name,
                    ));
                }
            }
            return Ok(());
        }
        _ => false,
    };
    if resolves
        && let Some(component) =
            component(source_node.components(), SourceComponentKind::DefinedName)
    {
        budget.charge_edit()?;
        edits.push(SourceEdit::new(component.span(), rename.new_name));
    }
    Ok(())
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
        if let SheetOutcome::Renamed(new_name) = request.single_sheet(&prefix.name) {
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
    match request.sheet_span(&prefix.name, end_name) {
        // A deleted span is replaced as a whole by `replaced_by_deletion`.
        SpanOutcome::Unchanged | SpanOutcome::Deleted => Ok(()),
        SpanOutcome::Ends {
            start: new_start,
            end: new_end,
        } => {
            let final_start = new_start.unwrap_or(&prefix.name);
            let final_end = new_end.unwrap_or(end_name);
            // MS-XLSX quotes a sheet range as one token, so `My Data:S3` is written
            // `'My Data:S3'` rather than `'My Data':S3`.
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
            Ok(())
        }
        // `'S2:S3'!A1` keeps one sheet once `S2` is deleted and becomes `'S3'!A1`.
        SpanOutcome::Collapsed(name) => {
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
    }
}

/// Returns the sheet a `Sheet1!#REF!` names, read from its qualifier components.
fn qualified_error_prefix(source: &str, components: &[SourceComponent]) -> Option<SheetPrefix> {
    let qualifier = component(components, SourceComponentKind::SheetQualifier)?;
    let text = |span: SourceSpan| {
        source
            .get(span.start..span.end)
            .map(|name| name.replace("''", "'"))
    };
    let quoted = qualifier
        .span()
        .start
        .checked_sub(1)
        .is_some_and(|offset| source.as_bytes().get(offset) == Some(&b'\''));
    Some(SheetPrefix {
        name: text(qualifier.span())?,
        end_name: component(components, SourceComponentKind::SheetRangeEnd)
            .and_then(|end| text(end.span())),
        quoted,
    })
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

/// Reports whether a sheet name needs quotes in a reference: when it is not a plain identifier, or
/// when it reads as a cell reference, such as `S2` or `R1C1`. Excel reads an unquoted `S2:S3!A1`
/// as a range from cell `S2`, so it writes such names quoted.
pub(crate) fn sheet_name_needs_quotes(name: &str) -> bool {
    name.is_empty()
        || !name
            .chars()
            .next()
            .is_some_and(|character| character.is_alphabetic() || matches!(character, '_' | '\\'))
        || name
            .chars()
            .skip(1)
            .any(|character| !(character.is_alphanumeric() || matches!(character, '_' | '.')))
        || reads_as_a1_cell(name)
        || reads_as_r1c1_reference(name)
}

/// `A1` through `XFD1048576`, letters in either case.
fn reads_as_a1_cell(name: &str) -> bool {
    let letters = name.bytes().take_while(u8::is_ascii_alphabetic).count();
    let digits = &name[letters..];
    (1..=3).contains(&letters)
        && !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && crate::CellAddress::from_a1(name).is_ok()
}

/// `R`, `C`, `RC`, and their numbered forms such as `R1C1`, `R2`, or `C3`, in either case.
fn reads_as_r1c1_reference(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let digits = |text: &str| text.bytes().all(|byte| byte.is_ascii_digit());
    match upper.strip_prefix('R') {
        Some(rest) => match rest.split_once('C') {
            Some((row, column)) => digits(row) && digits(column),
            None => digits(rest),
        },
        None => upper.strip_prefix('C').is_some_and(digits),
    }
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
        FormulaRewriteRequest::TableColumnsRemoved {
            table_name,
            owner_is_target_table,
            ..
        } => lexed
            .tokens
            .iter()
            .enumerate()
            .any(|(index, token)| match &token.token {
                Token::StructuredRef(raw) => {
                    raw_structured_targets_table(raw, table_name, *owner_is_target_table)
                        && !structured_token_is_external(&lexed.tokens, index)
                }
                _ => false,
            }),
        FormulaRewriteRequest::TableToRange {
            area,
            owner_is_target_table,
            ..
        } => lexed
            .tokens
            .iter()
            .enumerate()
            .any(|(index, token)| match &token.token {
                Token::StructuredRef(raw) => {
                    raw_structured_targets_table(raw, area.display_name, *owner_is_target_table)
                        && !structured_token_is_external(&lexed.tokens, index)
                }
                Token::Ident(name) => case_insensitive_eq(name, area.display_name),
                _ => false,
            }),
        FormulaRewriteRequest::DefinedName(rename) => lexed.tokens.iter().any(|token| {
            matches!(&token.token, Token::Ident(name) if case_insensitive_eq(name, rename.old_name))
        }),
        FormulaRewriteRequest::Grid(change) => {
            // An unqualified reference names the edited sheet in its own formulas, and anything
            // shaped like a cell, column, or row may be one.
            let unqualified = change.host_is_sheet
                && lexed.tokens.iter().any(|token| {
                    matches!(
                        &token.token,
                        Token::Dollar | Token::Number(_) | Token::Colon
                    ) || matches!(&token.token, Token::Ident(name) if name_may_be_reference(name))
                });
            unqualified
                || lexed.tokens.windows(2).enumerate().any(|(index, tokens)| {
                    let (Token::Ident(name) | Token::QuotedSheet(name), Token::Bang) =
                        (&tokens[0].token, &tokens[1].token)
                    else {
                        return false;
                    };
                    !sheet_token_is_external(&lexed.tokens, index)
                        && case_insensitive_eq(name, change.sheet)
                })
        }
        // A copy shifts every reference, so a formula that cannot be read cannot be copied.
        FormulaRewriteRequest::Copy(_) => true,
        FormulaRewriteRequest::Sheet { .. } | FormulaRewriteRequest::Sheets(_) => {
            let targeted = |name: &str| match request {
                FormulaRewriteRequest::Sheet { old_name, .. } => {
                    case_insensitive_eq(name, old_name)
                }
                FormulaRewriteRequest::Sheets(changes) => changes.targets(name),
                FormulaRewriteRequest::Table { .. }
                | FormulaRewriteRequest::TableColumn { .. }
                | FormulaRewriteRequest::DefinedName(_)
                | FormulaRewriteRequest::TableColumnsRemoved { .. }
                | FormulaRewriteRequest::TableToRange { .. }
                | FormulaRewriteRequest::Grid(_)
                | FormulaRewriteRequest::Copy(_) => false,
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

/// Reports whether a name token could be a cell reference such as `A1`.
fn name_may_be_reference(name: &str) -> bool {
    let letters = name.bytes().take_while(u8::is_ascii_alphabetic).count();
    let digits = &name.as_bytes()[letters..];
    (1..=3).contains(&letters) && !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
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
        FormulaRewriteRequest, NameRename, SheetChanges, raw_structured_targets_table,
        rewrite_formula,
    };
    use crate::SheetId;
    use crate::calculation::sheet_changes::{TabEntry, TabStep};
    use crate::calculation::table_conversion::TableArea;

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
                Changes::new(&["Old", "New"], &[], &renames, &[]).request(&[])
            ),
            Some("New!A1+'Old'!B2+New:Old!C3".to_owned())
        );
    }

    const TABS: &[&str] = &["S1", "S2", "S3", "S4"];

    /// Sheet changes over `tabs`, in tab order with IDs from 1, where a name in `charts` is a
    /// chartsheet: `renames` give new names, and the `deleted` sheets are deleted in turn.
    struct Changes {
        before: Vec<(&'static str, SheetId)>,
        after: Vec<(SheetId, &'static str)>,
        steps: Vec<TabStep>,
    }

    impl Changes {
        fn new(
            tabs: &[&'static str],
            charts: &[&str],
            renames: &[(&'static str, &'static str)],
            deleted: &[&str],
        ) -> Self {
            let before = tabs
                .iter()
                .zip(1..)
                .map(|(name, id)| (*name, SheetId::new(id).expect("sheet ID")))
                .collect::<Vec<_>>();
            let mut order = before
                .iter()
                .map(|(name, id)| TabEntry {
                    id: *id,
                    worksheet: !charts.contains(name),
                })
                .collect::<Vec<_>>();
            let mut steps = Vec::new();
            for name in deleted {
                let (_, id) = before
                    .iter()
                    .find(|(tab, _)| tab == name)
                    .expect("deleted tab");
                steps.push(TabStep::Delete {
                    tabs: order.clone().into(),
                    deleted: *id,
                });
                order.retain(|tab| tab.id != *id);
            }
            let after = before
                .iter()
                .filter(|(name, _)| !deleted.contains(name))
                .map(|(name, id)| {
                    let renamed = renames
                        .iter()
                        .find(|(old, _)| old == name)
                        .map_or(*name, |(_, new)| *new);
                    (*id, renamed)
                })
                .collect();
            Self {
                before,
                after,
                steps,
            }
        }

        fn request<'a>(&'a self, deleted_tables: &'a [&'a str]) -> FormulaRewriteRequest<'a> {
            FormulaRewriteRequest::Sheets(SheetChanges {
                before: &self.before,
                after: &self.after,
                steps: &self.steps,
                retarget: None,
                deleted_tables,
            })
        }
    }

    fn deletion(deleted: &[&str]) -> Changes {
        Changes::new(TABS, &[], &[], deleted)
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
                rewrite(source, deletion(&["S2"]).request(&["Things"])),
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
            assert_eq!(
                rewrite(unchanged, deletion(&["S2"]).request(&["Things"])),
                None,
                "{unchanged}"
            );
        }
    }

    #[test]
    fn deleted_ends_of_3d_references_move_to_the_nearest_remaining_tab() {
        for (deleted, source, rewritten) in [
            (&["S2"][..], "SUM('S2:S4'!A1)", "SUM('S3:S4'!A1)"),
            (&["S2"], "SUM('S2:S3'!A1)", "SUM('S3'!A1)"),
            (&["S2"], "SUM(S1:S2!A1)", "SUM('S1'!A1)"),
            (&["S2"], "SUM(S4:S2!A1)", "SUM('S4:S3'!A1)"),
            (&["S2"], "SUM('S2:S2'!A1)", "SUM(#REF!)"),
            (&["S2"], "SUM(S2:Missing!A1)", "SUM(#REF!)"),
            (&["S3"], "SUM('S1:S3'!A1)", "SUM('S1:S2'!A1)"),
            (&["S3"], "SUM(S1:'S3'!A1)", "SUM('S1:S2'!A1)"),
            (&["S4"], "SUM('S2:S4'!A1)", "SUM('S2:S3'!A1)"),
            (&["S1"], "SUM('S1:S3'!A1)", "SUM('S2:S3'!A1)"),
            (&["S2", "S3"], "SUM(S2:S4!A1)", "SUM('S4'!A1)"),
            (&["S2", "S3"], "SUM('S2:S3'!A1)", "SUM(#REF!)"),
            (&["S2", "S4"], "SUM(S1:S4!A1)", "SUM('S1:S3'!A1)"),
        ] {
            assert_eq!(
                rewrite(source, deletion(deleted).request(&["Things"])),
                Some(rewritten.to_owned()),
                "{deleted:?} {source}"
            );
        }
    }

    #[test]
    fn sheet_changes_rename_and_delete_in_one_rewrite() {
        let changes = Changes::new(TABS, &[], &[("S3", "New Name"), ("S1", "S2")], &["S2"]);
        for (source, rewritten) in [
            ("SUM('S2:S3'!A1)", "SUM('New Name'!A1)"),
            ("SUM(S2:S4!A1)", "SUM('New Name:S4'!A1)"),
            ("SUM('S2:S4'!A1)", "SUM('New Name:S4'!A1)"),
            ("SUM(S1:S4!A1)", "SUM('S2:S4'!A1)"),
            ("S2!A1+S1!A1+S3!A1", "#REF!+'S2'!A1+'New Name'!A1"),
        ] {
            assert_eq!(
                rewrite(source, changes.request(&[])),
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
            ("SUM(S1:S2!A1)", "SUM('S1'!A1)"),
            ("SUM(S1:'S2'!A1)", "SUM('S1'!A1)"),
            ("SUM(S4:S2!A1)", "SUM('S4:My Data'!A1)"),
        ] {
            assert_eq!(
                rewrite(source, Changes::new(tabs, &[], &[], &["S2"]).request(&[])),
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
    fn sheet_names_that_read_as_cell_references_are_quoted() {
        // Excel 16.114 reads an unquoted `S2:S3!A1` as a range from cell `S2`.
        for (new_name, rewritten) in [
            ("S2", "'S2'!A1+SUM('S2:End'!A1)"),
            ("xfd1048576", "'xfd1048576'!A1+SUM('xfd1048576:End'!A1)"),
            ("R1C1", "'R1C1'!A1+SUM('R1C1:End'!A1)"),
            ("rc", "'rc'!A1+SUM('rc:End'!A1)"),
            ("C3", "'C3'!A1+SUM('C3:End'!A1)"),
            ("XFE1", "XFE1!A1+SUM(XFE1:End!A1)"),
            ("Rates", "Rates!A1+SUM(Rates:End!A1)"),
        ] {
            assert_eq!(
                rewrite(
                    "Data!A1+SUM(Data:End!A1)",
                    FormulaRewriteRequest::Sheet {
                        old_name: "Data",
                        new_name,
                    }
                ),
                Some(rewritten.to_owned()),
                "{new_name}"
            );
        }
    }

    #[test]
    fn a_renamed_name_rewrites_only_the_references_that_resolve_to_it() {
        let workbook_rename = |owner_sheet| {
            FormulaRewriteRequest::DefinedName(NameRename {
                old_name: "Rate",
                new_name: "Tax",
                scope_sheet: None,
                shadowing_sheets: &["S2"],
                owner_sheet,
            })
        };
        // `S2` has its own `Rate`, which `'S2'!Rate` and an unqualified `Rate` on `S2` mean.
        assert_eq!(
            rewrite(
                "rate*2+S1!Rate+'S2'!Rate+S3!Rate+[0]!Rate+INDIRECT(\"Rate\")+Rates+[Book.xlsx]S1!Rate",
                workbook_rename(Some("S1"))
            ),
            Some(
                "Tax*2+S1!Tax+'S2'!Rate+S3!Tax+[0]!Tax+INDIRECT(\"Rate\")+Rates+[Book.xlsx]S1!Rate"
                    .to_owned()
            )
        );
        assert_eq!(
            rewrite("Rate+S1!Rate", workbook_rename(Some("S2"))),
            Some("Rate+S1!Tax".to_owned())
        );
        assert_eq!(
            rewrite("Rate*2", workbook_rename(None)),
            Some("Tax*2".to_owned())
        );
        let sheet_rename = |owner_sheet| {
            FormulaRewriteRequest::DefinedName(NameRename {
                old_name: "Rate",
                new_name: "Local",
                scope_sheet: Some("S2"),
                shadowing_sheets: &[],
                owner_sheet,
            })
        };
        assert_eq!(
            rewrite("Rate+'S2'!Rate+S1!Rate", sheet_rename(Some("S2"))),
            Some("Local+'S2'!Local+S1!Rate".to_owned())
        );
        assert_eq!(
            rewrite("Rate+s2!Rate", sheet_rename(Some("S1"))),
            Some("Rate+s2!Local".to_owned())
        );
        // A name defined as a LAMBDA is renamed where it is called; a built-in keeps its name.
        let lambda = FormulaRewriteRequest::DefinedName(NameRename {
            old_name: "Double",
            new_name: "Twice",
            scope_sheet: None,
            shadowing_sheets: &[],
            owner_sheet: Some("S1"),
        });
        assert_eq!(
            rewrite("Double(2)+SUM(Double(1))", lambda),
            Some("Twice(2)+SUM(Twice(1))".to_owned())
        );
        let builtin = FormulaRewriteRequest::DefinedName(NameRename {
            old_name: "SUM",
            new_name: "Total",
            scope_sheet: None,
            shadowing_sheets: &[],
            owner_sheet: Some("S1"),
        });
        assert_eq!(rewrite("SUM(1)", builtin), None);
    }

    #[test]
    fn a_reference_that_names_a_removed_table_column_becomes_a_ref_error() {
        // Excel 16.114 turns `T[c]` and `T[[a]:[c]]`, whose end is the removed `c`, into `#REF!`.
        let request = |owner_is_target_table| FormulaRewriteRequest::TableColumnsRemoved {
            table_name: "T",
            removed: &["c"],
            owner_is_target_table,
        };
        assert_eq!(
            rewrite(
                "SUM(T[c])+SUM(T[[a]:[c]])+SUM(T[[a]:[b]])+SUM(T)+Other[c]+T[[#Totals],[C]]",
                request(false)
            ),
            Some("SUM(#REF!)+SUM(#REF!)+SUM(T[[a]:[b]])+SUM(T)+Other[c]+#REF!".to_owned())
        );
        assert_eq!(
            rewrite("[@c]*2+[@a]", request(true)),
            Some("#REF!*2+[@a]".to_owned())
        );
        assert_eq!(rewrite("[@c]*2", request(false)), None);
    }

    #[test]
    fn converting_a_table_to_a_range_writes_its_references_as_excel_saves_them() {
        // Excel 16.114: `Things` at C1:E5 on `S2`, columns key, val, and dbl, a totals row.
        let area = TableArea {
            display_name: "Things",
            sheet_name: "S2",
            range: crate::CellRange::new(
                crate::CellAddress::from_a1("C1").expect("C1"),
                crate::CellAddress::from_a1("E5").expect("E5"),
            )
            .expect("range"),
            header_rows: 1,
            totals_rows: 1,
            columns: &["key", "val", "dbl"],
        };
        let request = |owner_is_target_table, this_row| FormulaRewriteRequest::TableToRange {
            area: &area,
            owner_is_target_table,
            this_row,
        };
        for (source, rewritten) in [
            ("SUM(Things[val])", "SUM('S2'!$D$2:$D$4)"),
            ("SUM(Things[[#Data],[val]:[dbl]])", "SUM('S2'!$D$2:$E$4)"),
            ("Things[[#Totals],[val]]", "'S2'!$D$5"),
            ("ROWS(Things[#All])", "ROWS('S2'!$C$1:$E$5)"),
            ("COUNTA(things[#Headers])", "COUNTA('S2'!$C$1:$E$1)"),
            ("SUM(Things)", "SUM('S2'!$C$2:$E$4)"),
            ("Other[val]+Things[missing]", "Other[val]+#REF!"),
            ("Things[@val]", "'S2'!$D3"),
        ] {
            assert_eq!(
                rewrite(source, request(false, Some(3))),
                Some(rewritten.to_owned()),
                "{source}"
            );
        }
        // A this-row reference keeps its row relative, and names the table inside it.
        assert_eq!(
            rewrite("[@val]*2+Things[@[val]:[dbl]]", request(true, Some(3))),
            Some("'S2'!$D3*2+'S2'!$D3:$E3".to_owned())
        );
        assert_eq!(rewrite("[@val]*2", request(false, Some(3))), None);
        assert_eq!(
            rewrite("Things[@val]", request(false, None)),
            Some("#REF!".to_owned())
        );
    }

    #[test]
    fn deleted_3d_ends_skip_tabs_that_are_not_worksheets() {
        // Excel 16.114 with tabs S1, S2, a chartsheet, S3: deleting S2 turns `'S2:S3'!A1` into
        // `'S3'!A1`, since a 3-D end moves to the nearest worksheet.
        for (deleted, source, rewritten) in [
            ("S2", "SUM('S2:S3'!A1)", "SUM('S3'!A1)"),
            ("S2", "SUM(S2:S4!A1)", "SUM('S3:S4'!A1)"),
            ("S3", "SUM(S1:S3!A1)", "SUM('S1:S2'!A1)"),
            ("S3", "SUM(Chart1:S3!A1)", "SUM(#REF!)"),
            ("S4", "SUM(Chart1:S4!A1)", "SUM('Chart1:S3'!A1)"),
        ] {
            assert_eq!(
                rewrite(
                    source,
                    Changes::new(
                        &["S1", "S2", "Chart1", "S3", "S4"],
                        &["Chart1"],
                        &[],
                        &[deleted]
                    )
                    .request(&[])
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
                    rewrite_formula(source, &deletion(&["S2"]).request(&["Things"]), &mut budget),
                    Err(FormulaRewriteError::Parse { .. })
                ),
                "{source}"
            );
        }
        for source in ["'S3'!A1+", "Other[val]+", "[Book.xlsx]S2!A1+"] {
            let mut budget = FormulaRewriteBudget::new(FormulaRewriteLimits::UNBOUNDED, &|| false);
            assert_eq!(
                rewrite_formula(source, &deletion(&["S2"]).request(&["Things"]), &mut budget)
                    .expect(source),
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
