use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::value::{ErrorKind, Value};

#[derive(Debug, Clone)]
pub(super) struct ArgumentValue {
    pub(super) value: Value,
    pub(super) from_collection: bool,
    pub(super) from_single_cell_reference: bool,
}

pub(super) fn collect_argument_values(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Vec<ArgumentValue>, ErrorKind> {
    let mut visited_cells = 0_u64;
    collect_argument_values_with_counter(engine, context, args, &mut visited_cells)
}

pub(super) fn collect_argument_values_with_counter(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    visited_cells: &mut u64,
) -> Result<Vec<ArgumentValue>, ErrorKind> {
    let mut values = Vec::new();
    for arg in args {
        if let Ok(rect) = engine.resolve_rect_expr(context, arg) {
            let row_end = engine.clamped_row_end(&rect);
            if row_end < rect.row_start {
                continue;
            }
            let rows = u64::from(row_end - rect.row_start) + 1;
            let cells = rows
                .checked_mul(rect.width())
                .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
            *visited_cells = visited_cells
                .checked_add(cells)
                .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
            engine.ensure_array_cells(*visited_cells)?;
            for row in rect.row_start..=row_end {
                for column in rect.col_start..=rect.col_end {
                    values.push(ArgumentValue {
                        value: engine.cell_value((rect.sheet, row, column)),
                        from_collection: true,
                        from_single_cell_reference: rect.is_single_cell(),
                    });
                }
            }
        } else {
            let array = engine.eval_array(context, arg)?;
            let from_collection = !array.is_scalar() || matches!(arg, Expr::Array(_));
            *visited_cells = visited_cells
                .checked_add(array.data.len() as u64)
                .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
            engine.ensure_array_cells(*visited_cells)?;
            values.extend(array.data.into_iter().map(|value| ArgumentValue {
                value,
                from_collection,
                from_single_cell_reference: false,
            }));
        }
    }
    Ok(values)
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

/// Adds Excel numeric values using `+0.0` as the additive identity.
///
/// `Iterator::sum` for `f64` folds from `-0.0`, which is the true additive identity for floats
/// because `-0.0 + x == x` holds even when `x` is `-0.0`. That makes an empty sum negative zero,
/// while Excel reports `0` for a sum over no numbers. Spreadsheet kernels must therefore not use
/// `Iterator::sum` directly.
pub(super) fn excel_sum(values: impl IntoIterator<Item = f64>) -> f64 {
    values.into_iter().fold(0.0, |total, value| total + value)
}

pub(super) fn excel_numeric_arguments(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Vec<f64>, ErrorKind> {
    let mut numbers = Vec::new();
    for ArgumentValue {
        value,
        from_collection,
        ..
    } in collect_argument_values(engine, context, args)?
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
