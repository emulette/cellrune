use super::super::ArithmeticSemantics;
use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::excel_digits::cancel_near_zero;
use super::super::limits::CalculationLimitKind;
use super::super::runtime::{CellId, Rect};
use super::super::scope::ScopeValue;
use super::super::sheet_span::SheetSpanPolicy;
use super::super::value::{ErrorKind, Value};
use super::{DynamicFunction, Evaluator, function_evaluator, let_scope_value};

#[derive(Debug, Clone)]
pub(super) struct ArgumentValue {
    pub(super) value: Value,
    pub(super) from_collection: bool,
    pub(super) from_single_cell_reference: bool,
}

/// Which referenced cells become argument values.
struct CellSelection<'filter, F> {
    include: &'filter F,
}

fn include_every_cell(_: CellId) -> bool {
    true
}

pub(super) fn collect_argument_values(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Vec<ArgumentValue>, ErrorKind> {
    collect_argument_values_with_policy(engine, context, args, SheetSpanPolicy::Unsupported)
}

pub(super) fn collect_argument_values_with_policy(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    sheet_span_policy: SheetSpanPolicy,
) -> Result<Vec<ArgumentValue>, ErrorKind> {
    let mut visited_cells = 0_u64;
    collect_argument_values_with_counter_and_policy(
        engine,
        context,
        args,
        &mut visited_cells,
        sheet_span_policy,
    )
}

pub(super) fn collect_argument_values_with_counter(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    visited_cells: &mut u64,
) -> Result<Vec<ArgumentValue>, ErrorKind> {
    collect_argument_values_with_counter_and_policy(
        engine,
        context,
        args,
        visited_cells,
        SheetSpanPolicy::Unsupported,
    )
}

pub(super) fn collect_argument_values_with_counter_and_policy(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    visited_cells: &mut u64,
    sheet_span_policy: SheetSpanPolicy,
) -> Result<Vec<ArgumentValue>, ErrorKind> {
    collect_filtered_argument_values(
        engine,
        context,
        args,
        visited_cells,
        sheet_span_policy,
        &CellSelection {
            include: &include_every_cell,
        },
    )
}

/// Collects argument values like [`collect_argument_values_with_policy`], leaving out referenced
/// cells for which `include` returns false.
pub(super) fn collect_argument_values_including(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    sheet_span_policy: SheetSpanPolicy,
    include: &impl Fn(CellId) -> bool,
) -> Result<Vec<ArgumentValue>, ErrorKind> {
    let mut visited_cells = 0_u64;
    collect_filtered_argument_values(
        engine,
        context,
        args,
        &mut visited_cells,
        sheet_span_policy,
        &CellSelection { include },
    )
}

fn collect_filtered_argument_values(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    visited_cells: &mut u64,
    sheet_span_policy: SheetSpanPolicy,
    cells: &CellSelection<'_, impl Fn(CellId) -> bool>,
) -> Result<Vec<ArgumentValue>, ErrorKind> {
    let mut values = Vec::new();
    for arg in args {
        if let Some(scoped) = collection_preserving_scope_value(engine, context, arg) {
            collect_scope_values(
                engine,
                context,
                scoped,
                visited_cells,
                sheet_span_policy,
                cells,
                &mut values,
            )?;
            continue;
        }
        if let Ok(reference) = engine.resolve_reference_value_expr(context, arg) {
            if matches!(&reference, super::super::runtime::ReferenceValue::Empty) {
                return Err(ErrorKind::Ref);
            }
            if reference.has_sheet_span() {
                match sheet_span_policy {
                    SheetSpanPolicy::CollectAcrossSheets => {}
                    SheetSpanPolicy::ReturnExcelError(kind) => return Err(kind),
                    SheetSpanPolicy::Unsupported => return Err(ErrorKind::Unsupported),
                }
            }
            for rect in reference.rects() {
                collect_rect_values(engine, context, rect, visited_cells, cells, &mut values)?;
            }
        } else {
            let evaluated = engine.eval_array_evaluation(context, arg)?;
            let from_collection = !evaluated.array.is_scalar() || matches!(arg, Expr::Array(_));
            *visited_cells = visited_cells
                .checked_add(evaluated.array.data.len() as u64)
                .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
            engine.ensure_array_cells(*visited_cells)?;
            values.extend(evaluated.array.data.into_iter().map(|value| ArgumentValue {
                value,
                from_collection,
                from_single_cell_reference: false,
            }));
        }
    }
    Ok(values)
}

fn collection_preserving_scope_value(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Option<ScopeValue> {
    let context = context.enter(expr);
    crate::calculation::stack::grow(|| match expr {
        Expr::Paren(inner) => collection_preserving_scope_value(engine, context, inner),
        Expr::Array(_) | Expr::Name(_) | Expr::BuiltinCallable(_) => {
            Some(engine.eval_scope_value(context, expr))
        }
        Expr::Call { name, args }
            if context.binding(name).is_none()
                && engine
                    .resolve_name_expr_with_id_in_context(context, name)
                    .is_none()
                && function_evaluator(name) == Some(Evaluator::Dynamic(DynamicFunction::Let)) =>
        {
            Some(let_scope_value(engine, context, args))
        }
        _ => None,
    })
}

pub(super) fn collect_callable_argument_values(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[ScopeValue],
) -> Result<Vec<ArgumentValue>, ErrorKind> {
    let mut visited_cells = 0_u64;
    let mut values = Vec::new();
    for value in args {
        collect_scope_values(
            engine,
            context,
            value.clone(),
            &mut visited_cells,
            SheetSpanPolicy::CollectAcrossSheets,
            &CellSelection {
                include: &include_every_cell,
            },
            &mut values,
        )?;
    }
    Ok(values)
}

fn collect_scope_values(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    scoped: ScopeValue,
    visited_cells: &mut u64,
    sheet_span_policy: SheetSpanPolicy,
    cells: &CellSelection<'_, impl Fn(CellId) -> bool>,
    values: &mut Vec<ArgumentValue>,
) -> Result<(), ErrorKind> {
    match scoped {
        ScopeValue::Missing => {
            charge_array_cells(engine, visited_cells, 1)?;
            values.push(ArgumentValue {
                value: Value::Blank,
                from_collection: false,
                from_single_cell_reference: false,
            });
        }
        ScopeValue::Scalar(evaluated) => {
            charge_array_cells(engine, visited_cells, 1)?;
            values.push(ArgumentValue {
                value: evaluated.value,
                from_collection: false,
                from_single_cell_reference: false,
            });
        }
        ScopeValue::Array(evaluated) => {
            charge_array_cells(engine, visited_cells, evaluated.array.data.len() as u64)?;
            values.extend(
                evaluated
                    .array
                    .data
                    .iter()
                    .cloned()
                    .map(|value| ArgumentValue {
                        value,
                        from_collection: true,
                        from_single_cell_reference: false,
                    }),
            );
        }
        ScopeValue::Reference(reference) => {
            if matches!(&reference, super::super::runtime::ReferenceValue::Empty) {
                return Err(ErrorKind::Ref);
            }
            if reference.has_sheet_span() {
                match sheet_span_policy {
                    SheetSpanPolicy::CollectAcrossSheets => {}
                    SheetSpanPolicy::ReturnExcelError(kind) => return Err(kind),
                    SheetSpanPolicy::Unsupported => return Err(ErrorKind::Unsupported),
                }
            }
            for rect in reference.rects() {
                collect_rect_values(engine, context, rect, visited_cells, cells, values)?;
            }
        }
        ScopeValue::Callable(_) => return Err(ErrorKind::Value),
    }
    Ok(())
}

fn charge_array_cells(
    engine: &Engine<'_>,
    visited_cells: &mut u64,
    cells: u64,
) -> Result<(), ErrorKind> {
    *visited_cells = visited_cells
        .checked_add(cells)
        .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
    engine.ensure_array_cells(*visited_cells)
}

fn collect_rect_values(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    rect: Rect,
    visited_cells: &mut u64,
    selection: &CellSelection<'_, impl Fn(CellId) -> bool>,
    values: &mut Vec<ArgumentValue>,
) -> Result<(), ErrorKind> {
    let rows = engine.operation_row_count([&rect]);
    if rows == 0 {
        return Ok(());
    }
    let cells = rows
        .checked_mul(rect.width())
        .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
    *visited_cells = visited_cells
        .checked_add(cells)
        .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
    engine.ensure_array_cells(*visited_cells)?;
    let populated_row_end = engine.populated_row_end(&rect);
    let mut stored = engine.rect_cells(
        &rect,
        populated_row_end.min(rect.row_start + (rows - 1) as u32),
    );
    for row_offset in 0..rows as u32 {
        let row = rect.row_start + row_offset;
        for column in rect.col_start..=rect.col_end {
            let cell = (rect.sheet, row, column);
            if !(selection.include)(cell) {
                continue;
            }
            let value = if row > populated_row_end {
                engine.read_blank_reference_cell(context)?
            } else {
                engine.read_stored_reference_cell(context, cell, stored.at(row, column))?
            };
            values.push(ArgumentValue {
                value,
                from_collection: true,
                from_single_cell_reference: rect.is_single_cell(),
            });
        }
    }
    Ok(())
}

pub(super) fn required_number(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<f64, ErrorKind> {
    super::super::coerce::to_number(&engine.eval_scalar(context, expr))
}

pub(super) fn required_text(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<String, ErrorKind> {
    super::super::coerce::to_text(&engine.eval_scalar(context, expr))
}

/// Whether Excel's fifteen-significant-digit rules apply: rounding reads a scaled quotient to
/// fifteen digits and comparison treats numbers that differ only past them as equal. Under
/// `ArithmeticSemantics::Ieee754` both use the binary values as they are.
pub(super) fn fifteen_digit_rules(engine: &Engine<'_>) -> bool {
    matches!(
        engine.arithmetic_semantics(),
        ArithmeticSemantics::ExcelNearZero
    )
}

/// Streaming total shared by every kernel that adds numbers up.
///
/// `Iterator::sum` for `f64` folds from `-0.0`, which makes an empty sum negative zero, while Excel
/// reports `0` for a sum over no numbers; the total here starts at `0.0`.
///
/// `SUM`, `AVERAGE`, `AVERAGEA`, `SUBTOTAL`, and `AGGREGATE` correct a total whose final addition
/// cancels to near zero, as the root of a formula does (see [`cancel_near_zero`]): Excel returns
/// `0` for `SUM(0.1,0.2,-0.3)` but keeps the residue of `SUM(0.1,0.2,-0.3,0)`, whose final addition
/// adds zero. The conditional aggregates, `SUMPRODUCT`, and `NPV` do not correct their totals.
pub(super) struct ExcelSum {
    correct_final_cancellation: bool,
    total: f64,
    final_addition: Option<(f64, f64)>,
}

impl ExcelSum {
    /// A total that is never corrected.
    pub(super) const fn plain() -> Self {
        Self {
            correct_final_cancellation: false,
            total: 0.0,
            final_addition: None,
        }
    }

    /// A total whose final addition is corrected under Excel's rules.
    pub(super) fn correcting_final_cancellation(engine: &Engine<'_>) -> Self {
        Self {
            correct_final_cancellation: fifteen_digit_rules(engine),
            ..Self::plain()
        }
    }

    pub(super) fn add(&mut self, value: f64) {
        self.final_addition = Some((self.total, value));
        self.total += value;
    }

    pub(super) fn total(&self) -> f64 {
        match self.final_addition {
            Some((before, value)) if self.correct_final_cancellation => {
                cancel_near_zero(before, value, self.total)
            }
            _ => self.total,
        }
    }
}

pub(super) fn excel_numeric_arguments(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Vec<f64>, ErrorKind> {
    excel_numeric_arguments_with_policy(engine, context, args, SheetSpanPolicy::Unsupported)
}

pub(super) fn excel_numeric_arguments_with_policy(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    sheet_span_policy: SheetSpanPolicy,
) -> Result<Vec<f64>, ErrorKind> {
    let mut visited_cells = 0_u64;
    excel_numbers(collect_filtered_argument_values(
        engine,
        context,
        args,
        &mut visited_cells,
        sheet_span_policy,
        &CellSelection {
            include: &include_every_cell,
        },
    )?)
}

/// Converts collected values to numbers with the direct-argument coercion of Excel's numeric
/// statistics: referenced text and logical values are skipped and errors propagate.
pub(super) fn excel_numbers(values: Vec<ArgumentValue>) -> Result<Vec<f64>, ErrorKind> {
    let mut numbers = Vec::new();
    for ArgumentValue {
        value,
        from_collection,
        ..
    } in values
    {
        match value {
            Value::Number(number) => numbers.push(number),
            Value::Logical(logical) if !from_collection => {
                numbers.push(if logical { 1.0 } else { 0.0 });
            }
            Value::Text(text) if !from_collection => {
                let number = text
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|number| number.is_finite())
                    .ok_or(ErrorKind::Value)?;
                numbers.push(number);
            }
            Value::Error(kind) => return Err(kind),
            Value::Blank | Value::Text(_) | Value::Logical(_) => {}
        }
    }
    Ok(numbers)
}
