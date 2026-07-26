use super::super::ArithmeticSemantics;
use super::super::ast::Expr;
use super::super::decimal::DecimalTrace;
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::value::{ErrorKind, Value};

#[derive(Debug, Clone)]
pub(super) struct ArgumentValue {
    pub(super) value: Value,
    pub(super) decimal_trace: Option<DecimalTrace>,
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
                    let cell = (rect.sheet, row, column);
                    let value = engine.cell_value(cell);
                    let decimal_trace = match &value {
                        Value::Number(_) => engine.numeric_decimal_trace(cell),
                        _ => None,
                    };
                    values.push(ArgumentValue {
                        value,
                        decimal_trace,
                        from_collection: true,
                        from_single_cell_reference: rect.is_single_cell(),
                    });
                }
            }
        } else {
            let evaluated = engine.eval_array_with_trace(context, arg)?;
            let from_collection = !evaluated.array.is_scalar() || matches!(arg, Expr::Array(_));
            *visited_cells = visited_cells
                .checked_add(evaluated.array.data.len() as u64)
                .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
            engine.ensure_array_cells(*visited_cells)?;
            values.extend(
                evaluated
                    .array
                    .data
                    .into_iter()
                    .zip(evaluated.decimal_traces)
                    .map(|(value, decimal_trace)| ArgumentValue {
                        value,
                        decimal_trace,
                        from_collection,
                        from_single_cell_reference: false,
                    }),
            );
        }
    }
    Ok(values)
}

pub(super) fn required_number(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<f64, ErrorKind> {
    required_number_with_trace(engine, context, expr).map(|(number, _)| number)
}

pub(super) fn required_number_with_trace(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<(f64, Option<DecimalTrace>), ErrorKind> {
    engine.eval_number_with_trace(context, expr)
}

pub(super) fn required_text(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<String, ErrorKind> {
    super::super::coerce::to_text(&engine.eval_scalar(context, expr))
}

/// Adds Excel numeric values using `+0.0` as the additive identity, under the engine's configured
/// arithmetic policy.
///
/// `Iterator::sum` for `f64` folds from `-0.0`, which is the true additive identity for floats
/// because `-0.0 + x == x` holds even when `x` is `-0.0`. That makes an empty sum negative zero,
/// while Excel reports `0` for a sum over no numbers. Spreadsheet kernels must therefore not use
/// `Iterator::sum` directly.
///
/// A running total is also a chain of additions, so it accumulates exactly the residue the
/// operator path corrects. Without the same correction here, one release would answer
/// `=A1+A2+A3` and `=SUM(A1:A3)` differently, which is not a compatibility mode but a
/// contradiction. The correction is applied at each step, against the term that produced it,
/// matching what `a + b + c` does in the operator path.
/// Policy-aware streaming accumulator shared by ordinary and conditional aggregates.
///
/// Besides the running `f64`, this carries the exact parsed-decimal sum when it fits in the bounded
/// trace representation. A later term is snapped only when that exact sum is zero.
pub(super) struct ExcelSum {
    excel_near_zero: bool,
    total: f64,
    decimal_trace: Option<DecimalTrace>,
}

impl ExcelSum {
    pub(super) fn new(engine: &Engine<'_>) -> Self {
        Self {
            excel_near_zero: matches!(
                engine.arithmetic_semantics(),
                ArithmeticSemantics::ExcelNearZero
            ),
            total: 0.0,
            decimal_trace: Some(DecimalTrace::ZERO),
        }
    }

    pub(super) fn add_with_trace(&mut self, value: f64, decimal_trace: Option<DecimalTrace>) {
        let next = self.total + value;
        if !self.excel_near_zero {
            self.total = next;
            return;
        }

        self.decimal_trace = self
            .decimal_trace
            .and_then(|total| total.add(decimal_trace?));
        if self.decimal_trace.is_some_and(DecimalTrace::is_zero) {
            self.total = 0.0;
        } else {
            self.total = next;
        }
    }

    pub(super) const fn total(&self) -> f64 {
        self.total
    }
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
