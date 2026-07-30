use super::super::ArithmeticSemantics;
use super::super::ast::Expr;
use super::super::decimal::{DecimalTrace, RationalTrace, is_excel_near_zero_cancellation};
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::runtime::Rect;
use super::super::scope::ScopeValue;
use super::super::sheet_span::SheetSpanPolicy;
use super::super::value::{ErrorKind, Value};
use super::{let_scope_value, normalize_name};

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
    let mut values = Vec::new();
    for arg in args {
        if let Some(let_args) = let_arguments(arg) {
            let scoped = let_scope_value(engine, context, let_args);
            collect_scope_values(
                engine,
                scoped,
                visited_cells,
                sheet_span_policy,
                &mut values,
            )?;
            continue;
        }
        if let Ok(span) = engine.resolve_rect_span_expr(context, arg) {
            if span.is_sheet_range() {
                match sheet_span_policy {
                    SheetSpanPolicy::CollectAcrossSheets => {}
                    SheetSpanPolicy::ReturnExcelError(kind) => return Err(kind),
                    SheetSpanPolicy::Unsupported => return Err(ErrorKind::Unsupported),
                }
            }
            for rect in span.rects() {
                collect_rect_values(engine, rect, visited_cells, &mut values)?;
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

fn let_arguments(expr: &Expr) -> Option<&[Expr]> {
    match expr {
        Expr::Paren(inner) => let_arguments(inner),
        Expr::Call { name, args } if normalize_name(name) == "LET" => Some(args),
        _ => None,
    }
}

fn collect_scope_values(
    engine: &Engine<'_>,
    scoped: ScopeValue,
    visited_cells: &mut u64,
    sheet_span_policy: SheetSpanPolicy,
    values: &mut Vec<ArgumentValue>,
) -> Result<(), ErrorKind> {
    match scoped {
        ScopeValue::Missing => {
            charge_array_cells(engine, visited_cells, 1)?;
            values.push(ArgumentValue {
                value: Value::Blank,
                decimal_trace: None,
                from_collection: false,
                from_single_cell_reference: false,
            });
        }
        ScopeValue::Scalar(evaluated) => {
            charge_array_cells(engine, visited_cells, 1)?;
            values.push(ArgumentValue {
                value: evaluated.value,
                decimal_trace: evaluated.decimal_trace,
                from_collection: false,
                from_single_cell_reference: false,
            });
        }
        ScopeValue::Array(evaluated) => {
            charge_array_cells(engine, visited_cells, evaluated.array.data.len() as u64)?;
            let from_collection = !evaluated.array.is_scalar();
            values.extend(
                evaluated
                    .array
                    .data
                    .iter()
                    .cloned()
                    .zip(evaluated.decimal_traces.iter().copied())
                    .map(|(value, decimal_trace)| ArgumentValue {
                        value,
                        decimal_trace,
                        from_collection,
                        from_single_cell_reference: false,
                    }),
            );
        }
        ScopeValue::Reference(span) => {
            if span.is_sheet_range() {
                match sheet_span_policy {
                    SheetSpanPolicy::CollectAcrossSheets => {}
                    SheetSpanPolicy::ReturnExcelError(kind) => return Err(kind),
                    SheetSpanPolicy::Unsupported => return Err(ErrorKind::Unsupported),
                }
            }
            for rect in span.rects() {
                collect_rect_values(engine, rect, visited_cells, values)?;
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
    rect: Rect,
    visited_cells: &mut u64,
    values: &mut Vec<ArgumentValue>,
) -> Result<(), ErrorKind> {
    let row_end = engine.clamped_row_end(&rect);
    if row_end < rect.row_start {
        return Ok(());
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
    Ok(())
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

/// Exact running value an [`ExcelSum`] can carry beside its `f64` total.
///
/// Two representations implement it: parsed decimals, for kernels that add their inputs as spelled;
/// and rationals, for kernels such as `NPV` that transform each input before adding it. Both fail
/// closed — an operation that does not fit the bounded representation returns `None`, and the
/// accumulator then stops claiming to know the exact total rather than guessing that it is zero.
pub(super) trait ExactTrace: Copy {
    const EXACT_ZERO: Self;

    fn combined_with(self, right: Self) -> Option<Self>;

    fn is_exact_zero(self) -> bool;
}

impl ExactTrace for DecimalTrace {
    const EXACT_ZERO: Self = Self::ZERO;

    fn combined_with(self, right: Self) -> Option<Self> {
        self.add(right)
    }

    fn is_exact_zero(self) -> bool {
        self.is_zero()
    }
}

impl ExactTrace for RationalTrace {
    const EXACT_ZERO: Self = Self::ZERO;

    fn combined_with(self, right: Self) -> Option<Self> {
        self.add(right)
    }

    fn is_exact_zero(self) -> bool {
        self.is_zero()
    }
}

/// Policy-aware streaming total shared by every kernel that adds numbers up.
///
/// Two things make this the single place the sum is formed. First, `Iterator::sum` for `f64` folds
/// from `-0.0`, which is the true additive identity for floats because `-0.0 + x == x` holds even
/// when `x` is `-0.0`; that makes an empty sum negative zero, while Excel reports `0` for a sum over
/// no numbers. Second, a running total is a chain of additions, so it accumulates exactly the
/// residue the operator path corrects — without the same correction here, one release would answer
/// `=A1+A2+A3` and `=SUM(A1:A3)` differently, which is not a compatibility mode but a
/// contradiction.
///
/// The correction is applied at each step against the term that produced it, matching what
/// `a + b + c` does in the operator path, and only when the exact total is zero. Ordinary and
/// conditional aggregates, `SUMPRODUCT`, and `NPV` all read the policy through this one type, so
/// none of them can drift from the others.
pub(super) struct ExcelSum<Trace: ExactTrace = DecimalTrace> {
    excel_near_zero: bool,
    total: f64,
    exact_total: Option<Trace>,
}

impl<Trace: ExactTrace> ExcelSum<Trace> {
    pub(super) fn new(engine: &Engine<'_>) -> Self {
        Self {
            excel_near_zero: matches!(
                engine.arithmetic_semantics(),
                ArithmeticSemantics::ExcelNearZero
            ),
            total: 0.0,
            exact_total: Some(Trace::EXACT_ZERO),
        }
    }

    pub(super) fn add_with_trace(&mut self, value: f64, trace: Option<Trace>) {
        let next = self.total + value;
        if !self.excel_near_zero {
            self.total = next;
            return;
        }

        self.exact_total = self
            .exact_total
            .and_then(|total| total.combined_with(trace?));
        if self.exact_total.is_some_and(ExactTrace::is_exact_zero)
            && is_excel_near_zero_cancellation(self.total, value, next)
        {
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
    excel_numeric_arguments_with_policy(engine, context, args, SheetSpanPolicy::Unsupported)
}

pub(super) fn excel_numeric_arguments_with_policy(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    sheet_span_policy: SheetSpanPolicy,
) -> Result<Vec<f64>, ErrorKind> {
    let mut numbers = Vec::new();
    for ArgumentValue {
        value,
        from_collection,
        ..
    } in collect_argument_values_with_policy(engine, context, args, sheet_span_policy)?
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
