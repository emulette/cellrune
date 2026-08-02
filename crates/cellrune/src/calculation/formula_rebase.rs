use std::collections::BTreeSet;

use super::ast::{CellRef, Expr, RefBody, Reference, RowRef};
use super::eval::{Engine, EvalContext};
use super::functions::descriptor::DependencyKind;
use super::functions::{function_dependency_kind, function_result_kind};
use super::limits::CalculationLimitKind;
use super::runtime::{CellId, Rect};
use super::scope::DefinedLambdaId;
use super::value::{ErrorKind, Value};

/// A formula criterion validated once at its authored cell and rebased only along database rows.
#[derive(Debug, Clone)]
pub(super) struct FormulaCriteria {
    origin: FormulaOrigin,
    root: Expr,
}

/// The authored formula cell owns sheet and name lookup; row rebasing never changes that scope.
#[derive(Debug, Clone, Copy)]
struct FormulaOrigin {
    cell: CellId,
}

impl FormulaOrigin {
    fn context<'scope>(self, context: EvalContext<'scope>) -> EvalContext<'scope> {
        context
            .with_cell(self.cell)
            .without_bindings()
            .with_defined_name_scope(None)
    }
}

impl FormulaCriteria {
    pub(super) fn prepare(
        engine: &Engine<'_>,
        context: EvalContext<'_>,
        origin: CellId,
        root: &Expr,
        database: Rect,
    ) -> Result<Self, ErrorKind> {
        let origin = FormulaOrigin { cell: origin };
        let origin_context = origin.context(context);
        let mut visited_names = BTreeSet::new();
        validate_expr(
            engine,
            origin_context,
            root,
            database,
            RelativeReferencePolicy::DatabaseRecord,
            &mut visited_names,
        )?;
        Ok(Self {
            origin,
            root: root.clone(),
        })
    }

    pub(super) fn evaluate(
        &self,
        engine: &Engine<'_>,
        context: EvalContext<'_>,
        row_delta: u32,
    ) -> Result<bool, ErrorKind> {
        let mut rebased = self.root.clone();
        let origin_context = self.origin.context(context);
        shift_expr_rows(engine, origin_context, &mut rebased, row_delta)?;
        match engine.eval_scalar(origin_context, &rebased) {
            Value::Logical(value) => Ok(value),
            Value::Error(kind) => Err(kind),
            Value::Blank | Value::Number(_) | Value::Text(_) => Err(ErrorKind::Value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelativeReferencePolicy {
    DatabaseRecord,
    AbsoluteOnly,
}

fn validate_expr(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
    database: Rect,
    relative_policy: RelativeReferencePolicy,
    visited_names: &mut BTreeSet<DefinedLambdaId>,
) -> Result<(), ErrorKind> {
    charge_node(engine, context)?;
    match expr {
        Expr::Ref(reference) => {
            validate_reference(engine, context, reference, database, relative_policy)
        }
        Expr::Name(name) => {
            let (id, named) = engine
                .resolve_name_expr_with_id_in_context(context, name)
                .ok_or(ErrorKind::Value)?;
            if !visited_names.insert(id.clone()) {
                return Err(ErrorKind::Value);
            }
            let result = validate_expr(
                engine,
                context
                    .without_bindings()
                    .with_defined_name_scope(Some(id.scope())),
                named,
                database,
                RelativeReferencePolicy::AbsoluteOnly,
                visited_names,
            );
            visited_names.remove(&id);
            result
        }
        Expr::Call { name, args } => {
            if engine
                .resolve_name_expr_with_id_in_context(context, name)
                .is_some()
                || matches!(
                    function_dependency_kind(name),
                    Some(DependencyKind::DynamicReference(_))
                )
                || function_result_kind(name).is_some_and(|kind| kind.returns_reference())
                || function_result_kind(name).is_none()
            {
                return Err(ErrorKind::Value);
            }
            for arg in args {
                validate_expr(
                    engine,
                    context,
                    arg,
                    database,
                    relative_policy,
                    visited_names,
                )?;
            }
            Ok(())
        }
        Expr::Unary { operand, .. }
        | Expr::Paren(operand)
        | Expr::ImplicitIntersection(operand) => validate_expr(
            engine,
            context,
            operand,
            database,
            relative_policy,
            visited_names,
        ),
        Expr::Binary { left, right, .. }
        | Expr::Range {
            start: left,
            end: right,
        } => {
            validate_expr(
                engine,
                context,
                left,
                database,
                relative_policy,
                visited_names,
            )?;
            validate_expr(
                engine,
                context,
                right,
                database,
                relative_policy,
                visited_names,
            )
        }
        Expr::Array(rows) => {
            for row in rows {
                for value in row {
                    validate_expr(
                        engine,
                        context,
                        value,
                        database,
                        relative_policy,
                        visited_names,
                    )?;
                }
            }
            Ok(())
        }
        Expr::Number(_) | Expr::Text(_) | Expr::Logical(_) | Expr::ErrorLit(_) | Expr::Missing => {
            Ok(())
        }
        Expr::StructuredRef(_)
        | Expr::ReferenceUnion { .. }
        | Expr::ReferenceIntersection { .. }
        | Expr::SpillRef(_)
        | Expr::ExternalReference(_)
        | Expr::QualifiedName { .. }
        | Expr::BuiltinCallable(_)
        | Expr::Invoke { .. } => Err(ErrorKind::Value),
    }
}

fn validate_reference(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    reference: &Reference,
    database: Rect,
    policy: RelativeReferencePolicy,
) -> Result<(), ErrorKind> {
    let target = engine
        .resolve_reference_span(context.sheet(), reference)?
        .into_rect()
        .map_err(|_| ErrorKind::Value)?;
    match &reference.body {
        RefBody::Cell(cell) => validate_cell_ref(*cell, target.sheet, database, policy),
        RefBody::Area(start, end) => {
            validate_cell_ref(*start, target.sheet, database, policy)?;
            validate_cell_ref(*end, target.sheet, database, policy)
        }
        RefBody::Columns(start, end) => {
            if !start.absolute || !end.absolute {
                Err(ErrorKind::Value)
            } else {
                Ok(())
            }
        }
        RefBody::Rows(start, end) => {
            validate_row_ref(*start, policy).and_then(|()| validate_row_ref(*end, policy))
        }
    }
}

fn validate_cell_ref(
    cell: CellRef,
    sheet: usize,
    database: Rect,
    policy: RelativeReferencePolicy,
) -> Result<(), ErrorKind> {
    if policy == RelativeReferencePolicy::AbsoluteOnly
        && (!cell.row_absolute || !cell.column_absolute)
    {
        return Err(ErrorKind::Value);
    }
    if cell.row_absolute && !cell.column_absolute {
        return Err(ErrorKind::Value);
    }
    if !cell.row_absolute
        && (sheet != database.sheet
            || cell.row != database.row_start.saturating_add(1)
            || cell.column < database.col_start
            || cell.column > database.col_end)
    {
        return Err(ErrorKind::Value);
    }
    Ok(())
}

fn validate_row_ref(row: RowRef, _policy: RelativeReferencePolicy) -> Result<(), ErrorKind> {
    if !row.absolute {
        Err(ErrorKind::Value)
    } else {
        Ok(())
    }
}

fn shift_expr_rows(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &mut Expr,
    row_delta: u32,
) -> Result<(), ErrorKind> {
    charge_node(engine, context)?;
    match expr {
        Expr::Ref(reference) => shift_reference_rows(reference, row_delta),
        Expr::Unary { operand, .. }
        | Expr::Paren(operand)
        | Expr::ImplicitIntersection(operand) => {
            shift_expr_rows(engine, context, operand, row_delta)
        }
        Expr::Binary { left, right, .. }
        | Expr::Range {
            start: left,
            end: right,
        } => {
            shift_expr_rows(engine, context, left, row_delta)?;
            shift_expr_rows(engine, context, right, row_delta)
        }
        Expr::Call { args, .. } => {
            for arg in args {
                shift_expr_rows(engine, context, arg, row_delta)?;
            }
            Ok(())
        }
        Expr::Array(rows) => {
            for row in rows {
                for value in row {
                    shift_expr_rows(engine, context, value, row_delta)?;
                }
            }
            Ok(())
        }
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Logical(_)
        | Expr::ErrorLit(_)
        | Expr::Name(_)
        | Expr::Missing => Ok(()),
        Expr::StructuredRef(_)
        | Expr::ReferenceUnion { .. }
        | Expr::ReferenceIntersection { .. }
        | Expr::SpillRef(_)
        | Expr::ExternalReference(_)
        | Expr::QualifiedName { .. }
        | Expr::BuiltinCallable(_)
        | Expr::Invoke { .. } => Err(ErrorKind::Value),
    }
}

fn shift_reference_rows(reference: &mut Reference, row_delta: u32) -> Result<(), ErrorKind> {
    match &mut reference.body {
        RefBody::Cell(cell) => shift_cell_row(cell, row_delta),
        RefBody::Area(start, end) => {
            shift_cell_row(start, row_delta)?;
            shift_cell_row(end, row_delta)
        }
        RefBody::Columns(_, _) => Ok(()),
        RefBody::Rows(start, end) => {
            shift_row(start, row_delta)?;
            shift_row(end, row_delta)
        }
    }
}

fn shift_cell_row(cell: &mut CellRef, row_delta: u32) -> Result<(), ErrorKind> {
    if !cell.row_absolute {
        cell.row = cell.row.checked_add(row_delta).ok_or(ErrorKind::Value)?;
        if cell.row > super::EXCEL_MAX_ROWS {
            return Err(ErrorKind::Value);
        }
    }
    Ok(())
}

fn shift_row(row: &mut RowRef, row_delta: u32) -> Result<(), ErrorKind> {
    if !row.absolute {
        row.row = row.row.checked_add(row_delta).ok_or(ErrorKind::Value)?;
        if row.row > super::EXCEL_MAX_ROWS {
            return Err(ErrorKind::Value);
        }
    }
    Ok(())
}

fn charge_node(engine: &Engine<'_>, context: EvalContext<'_>) -> Result<(), ErrorKind> {
    if context.is_cancelled() {
        return Err(ErrorKind::ResourceLimit(
            CalculationLimitKind::FunctionIterations,
        ));
    }
    engine.charge_function_iterations(context, 1)
}
