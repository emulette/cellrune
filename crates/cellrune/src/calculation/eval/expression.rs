use super::reference::is_reference_returning_function;
use super::{Engine, EvalContext};
use crate::calculation::ArithmeticSemantics;
use crate::calculation::ast::{BinaryOp, Expr, UnaryOp};
use crate::calculation::decimal::DecimalTrace;
use crate::calculation::functions::{call_function, call_function_array};
use crate::calculation::limits::CalculationLimitKind;
use crate::calculation::operators::{apply_binary, apply_unary, broadcast_shape, element_at};
use crate::calculation::runtime::{Array, Rect};
use crate::calculation::value::{ErrorKind, Value};

pub(super) struct ScalarEvaluation {
    pub(super) value: Value,
    pub(super) decimal_trace: Option<DecimalTrace>,
}

impl ScalarEvaluation {
    const fn untracked(value: Value) -> Self {
        Self {
            value,
            decimal_trace: None,
        }
    }
}

pub(in crate::calculation) struct ArrayEvaluation {
    pub(in crate::calculation) array: Array,
    pub(in crate::calculation) decimal_traces: Vec<Option<DecimalTrace>>,
}

impl ArrayEvaluation {
    fn untracked(array: Array) -> Self {
        let decimal_traces = vec![None; array.data.len()];
        Self {
            array,
            decimal_traces,
        }
    }

    fn scalar(evaluated: ScalarEvaluation) -> Self {
        Self {
            array: Array::scalar(evaluated.value),
            decimal_traces: vec![evaluated.decimal_trace],
        }
    }
}

fn evaluate_binary(
    op: BinaryOp,
    left: ScalarEvaluation,
    right: ScalarEvaluation,
    max_text_bytes: u64,
    arithmetic: ArithmeticSemantics,
) -> ScalarEvaluation {
    let mut value = apply_binary(op, &left.value, &right.value, max_text_bytes);
    let decimal_trace = left.decimal_trace.and_then(|left| match op {
        BinaryOp::Add => left.add(right.decimal_trace?),
        BinaryOp::Subtract => left.subtract(right.decimal_trace?),
        _ => None,
    });
    if matches!(arithmetic, ArithmeticSemantics::ExcelNearZero)
        && decimal_trace.is_some_and(DecimalTrace::is_zero)
        && matches!(value, Value::Number(_))
    {
        value = Value::Number(0.0);
    }
    ScalarEvaluation {
        value,
        decimal_trace,
    }
}

fn evaluate_unary(op: UnaryOp, operand: ScalarEvaluation) -> ScalarEvaluation {
    let value = apply_unary(op, &operand.value);
    let decimal_trace = match value {
        Value::Number(_) => operand.decimal_trace.and_then(|trace| match op {
            UnaryOp::Negate => trace.negate(),
            UnaryOp::Plus => Some(trace),
            UnaryOp::Percent => trace.percent(),
        }),
        _ => None,
    };
    ScalarEvaluation {
        value,
        decimal_trace,
    }
}

impl Engine<'_> {
    fn eval_implicit_intersection(&self, context: EvalContext<'_>, expr: &Expr) -> Value {
        match expr {
            Expr::Paren(inner) | Expr::ImplicitIntersection(inner) => {
                self.eval_implicit_intersection(context, inner)
            }
            Expr::Ref(reference) => self
                .resolve_reference(context.sheet(), reference)
                .and_then(|rect| self.implicit_intersection_rect(context, rect))
                .map_or_else(Value::Error, |rect| {
                    self.cell_value((rect.sheet, rect.row_start, rect.col_start))
                }),
            Expr::Range { .. } => self
                .resolve_rect_expr(context, expr)
                .and_then(|rect| self.implicit_intersection_rect(context, rect))
                .map_or_else(Value::Error, |rect| {
                    self.cell_value((rect.sheet, rect.row_start, rect.col_start))
                }),
            Expr::Name(name) => match self.resolve_name_expr(context.sheet(), name) {
                Some(named) => self.eval_implicit_intersection(context, named),
                None => Value::Error(ErrorKind::Name),
            },
            Expr::Call { name, .. } if is_reference_returning_function(name) => self
                .resolve_rect_expr(context, expr)
                .and_then(|rect| self.implicit_intersection_rect(context, rect))
                .map_or_else(Value::Error, |rect| {
                    self.cell_value((rect.sheet, rect.row_start, rect.col_start))
                }),
            _ => self
                .eval_array(context, expr)
                .map_or_else(Value::Error, |array| {
                    array
                        .data
                        .into_iter()
                        .next()
                        .unwrap_or(Value::Error(ErrorKind::Value))
                }),
        }
    }

    pub fn eval_scalar(&self, context: EvalContext<'_>, expr: &Expr) -> Value {
        self.eval_scalar_with_trace(context, expr).value
    }

    pub(in crate::calculation) fn eval_number_with_trace(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
    ) -> Result<(f64, Option<DecimalTrace>), ErrorKind> {
        let evaluated = self.eval_scalar_with_trace(context, expr);
        let decimal_trace = match evaluated.value {
            Value::Number(_) => evaluated.decimal_trace,
            _ => None,
        };
        let number = crate::calculation::coerce::to_number(&evaluated.value)?;
        Ok((number, decimal_trace))
    }

    pub(super) fn eval_scalar_with_trace(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
    ) -> ScalarEvaluation {
        match expr {
            Expr::Number(number) => ScalarEvaluation {
                value: Value::Number(number.value()),
                decimal_trace: number.decimal_trace(),
            },
            Expr::Text(text) => ScalarEvaluation::untracked(Value::Text(text.clone())),
            Expr::Logical(logical) => ScalarEvaluation::untracked(Value::Logical(*logical)),
            Expr::ErrorLit(kind) => ScalarEvaluation::untracked(Value::Error(*kind)),
            Expr::Missing => ScalarEvaluation::untracked(Value::Blank),
            Expr::Paren(inner) => self.eval_scalar_with_trace(context, inner),
            Expr::ImplicitIntersection(inner) => match inner.as_ref() {
                Expr::Name(name) if context.binding(name).is_some() => ScalarEvaluation::untracked(
                    context
                        .binding(name)
                        .cloned()
                        .expect("binding presence checked"),
                ),
                Expr::Ref(_) | Expr::Range { .. } | Expr::Name(_) => {
                    self.eval_reference_with_trace(context, inner)
                }
                Expr::Call { name, .. } if is_reference_returning_function(name) => {
                    self.eval_reference_with_trace(context, inner)
                }
                _ => self.first_array_value_with_trace(context, inner),
            },
            Expr::Array(_) => ScalarEvaluation::untracked(Value::Error(ErrorKind::Unsupported)),
            Expr::Name(name) => match context.binding(name) {
                Some(value) => ScalarEvaluation::untracked(value.clone()),
                None => match self.resolve_name_expr(context.sheet(), name) {
                    Some(named) => self.eval_scalar_with_trace(context, named),
                    None => ScalarEvaluation::untracked(Value::Error(ErrorKind::Name)),
                },
            },
            Expr::Ref(_) | Expr::Range { .. } => self.eval_reference_with_trace(context, expr),
            Expr::Call { name, .. } if is_reference_returning_function(name) => {
                self.eval_reference_with_trace(context, expr)
            }
            Expr::Call { name, args } => {
                ScalarEvaluation::untracked(call_function(self, context, name, args))
            }
            Expr::Unary { op, operand } => {
                evaluate_unary(*op, self.eval_scalar_with_trace(context, operand))
            }
            Expr::Binary { op, left, right } => evaluate_binary(
                *op,
                self.eval_scalar_with_trace(context, left),
                self.eval_scalar_with_trace(context, right),
                self.options.limits().max_text_bytes(),
                self.arithmetic_semantics(),
            ),
        }
    }

    fn first_array_value_with_trace(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
    ) -> ScalarEvaluation {
        let mut evaluated = match self.eval_array_with_trace(context, expr) {
            Ok(evaluated) => evaluated,
            Err(kind) => return ScalarEvaluation::untracked(Value::Error(kind)),
        };
        let value = evaluated
            .array
            .data
            .drain(..)
            .next()
            .unwrap_or(Value::Error(ErrorKind::Value));
        let decimal_trace = evaluated.decimal_traces.drain(..).next().flatten();
        ScalarEvaluation {
            value,
            decimal_trace,
        }
    }

    fn eval_reference_with_trace(&self, context: EvalContext<'_>, expr: &Expr) -> ScalarEvaluation {
        let Ok(rect) = self.resolve_rect_expr(context, expr) else {
            return ScalarEvaluation::untracked(self.eval_implicit_intersection(context, expr));
        };
        let Ok(rect) = self.implicit_intersection_rect(context, rect) else {
            return ScalarEvaluation::untracked(self.eval_implicit_intersection(context, expr));
        };
        let cell = (rect.sheet, rect.row_start, rect.col_start);
        let value = self.cell_value(cell);
        let decimal_trace = match value {
            Value::Number(_) => self.numeric_decimal_trace(cell),
            _ => None,
        };
        ScalarEvaluation {
            value,
            decimal_trace,
        }
    }

    pub fn eval_array(&self, context: EvalContext<'_>, expr: &Expr) -> Result<Array, ErrorKind> {
        self.eval_array_with_trace(context, expr)
            .map(|evaluated| evaluated.array)
    }

    pub(in crate::calculation) fn eval_array_with_trace(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
    ) -> Result<ArrayEvaluation, ErrorKind> {
        match expr {
            Expr::Paren(inner) => self.eval_array_with_trace(context, inner),
            Expr::ImplicitIntersection(inner) => {
                Ok(ArrayEvaluation::scalar(match inner.as_ref() {
                    Expr::Name(name) if context.binding(name).is_some() => {
                        ScalarEvaluation::untracked(
                            context
                                .binding(name)
                                .cloned()
                                .expect("binding presence checked"),
                        )
                    }
                    Expr::Ref(_) | Expr::Range { .. } | Expr::Name(_) => {
                        self.eval_reference_with_trace(context, inner)
                    }
                    Expr::Call { name, .. } if is_reference_returning_function(name) => {
                        self.eval_reference_with_trace(context, inner)
                    }
                    _ => self.first_array_value_with_trace(context, inner),
                }))
            }
            Expr::Name(name) if context.binding(name).is_some() => {
                Ok(ArrayEvaluation::scalar(ScalarEvaluation::untracked(
                    context
                        .binding(name)
                        .cloned()
                        .expect("binding presence checked"),
                )))
            }
            Expr::Ref(_) | Expr::Range { .. } | Expr::Name(_) => {
                let rect = self.resolve_rect_expr(context, expr)?;
                self.array_from_rect_with_trace(rect)
            }
            Expr::Array(rows) => {
                let cols = rows.first().map_or(0, Vec::len);
                if rows.is_empty() || cols == 0 || rows.iter().any(|row| row.len() != cols) {
                    return Err(ErrorKind::Value);
                }
                let cell_count = (rows.len() as u64) * (cols as u64);
                self.ensure_array_cells(cell_count)?;
                let evaluated: Vec<ScalarEvaluation> = rows
                    .iter()
                    .flat_map(|row| {
                        row.iter()
                            .map(|value| self.eval_scalar_with_trace(context, value))
                    })
                    .collect();
                Ok(ArrayEvaluation {
                    array: Array {
                        rows: rows.len() as u32,
                        cols: cols as u32,
                        data: evaluated.iter().map(|value| value.value.clone()).collect(),
                    },
                    decimal_traces: evaluated
                        .into_iter()
                        .map(|value| value.decimal_trace)
                        .collect(),
                })
            }
            Expr::Binary { op, left, right } => {
                let left = self.eval_array_with_trace(context, left)?;
                let right = self.eval_array_with_trace(context, right)?;
                let (rows, cols) = broadcast_shape(&left.array, &right.array)?;
                let cells = u64::from(rows)
                    .checked_mul(u64::from(cols))
                    .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
                self.ensure_array_cells(cells)?;
                let mut data = Vec::with_capacity(cells as usize);
                let mut decimal_traces = Vec::with_capacity(cells as usize);
                for row in 0..rows {
                    for column in 0..cols {
                        let evaluated = evaluate_binary(
                            *op,
                            ScalarEvaluation {
                                value: element_at(&left.array, row, column).clone(),
                                decimal_trace: array_decimal_at(&left, row, column),
                            },
                            ScalarEvaluation {
                                value: element_at(&right.array, row, column).clone(),
                                decimal_trace: array_decimal_at(&right, row, column),
                            },
                            self.options.limits().max_text_bytes(),
                            self.arithmetic_semantics(),
                        );
                        data.push(evaluated.value);
                        decimal_traces.push(evaluated.decimal_trace);
                    }
                }
                Ok(ArrayEvaluation {
                    array: Array { rows, cols, data },
                    decimal_traces,
                })
            }
            Expr::Unary { op, operand } => {
                let operand = self.eval_array_with_trace(context, operand)?;
                let evaluated: Vec<ScalarEvaluation> = operand
                    .array
                    .data
                    .into_iter()
                    .zip(operand.decimal_traces)
                    .map(|(value, decimal_trace)| {
                        evaluate_unary(
                            *op,
                            ScalarEvaluation {
                                value,
                                decimal_trace,
                            },
                        )
                    })
                    .collect();
                Ok(ArrayEvaluation {
                    array: Array {
                        rows: operand.array.rows,
                        cols: operand.array.cols,
                        data: evaluated.iter().map(|value| value.value.clone()).collect(),
                    },
                    decimal_traces: evaluated
                        .into_iter()
                        .map(|value| value.decimal_trace)
                        .collect(),
                })
            }
            Expr::Call { name, args } => {
                if let Some(result) = call_function_array(self, context, name, args) {
                    result.map(ArrayEvaluation::untracked)
                } else {
                    Ok(ArrayEvaluation::scalar(ScalarEvaluation::untracked(
                        call_function(self, context, name, args),
                    )))
                }
            }
            _ => Ok(ArrayEvaluation::scalar(
                self.eval_scalar_with_trace(context, expr),
            )),
        }
    }

    pub(in crate::calculation) fn array_from_rect(&self, rect: Rect) -> Result<Array, ErrorKind> {
        self.array_from_rect_with_trace(rect)
            .map(|evaluated| evaluated.array)
    }

    fn array_from_rect_with_trace(&self, rect: Rect) -> Result<ArrayEvaluation, ErrorKind> {
        if rect.is_single_cell() {
            let cell = (rect.sheet, rect.row_start, rect.col_start);
            return Ok(ArrayEvaluation::scalar(ScalarEvaluation {
                value: self.cell_value(cell),
                decimal_trace: self.numeric_decimal_trace(cell),
            }));
        }
        if rect.whole_rows {
            return Err(ErrorKind::Unsupported);
        }
        let cells = rect.height() * rect.width();
        self.ensure_array_cells(cells)?;
        let mut data = Vec::with_capacity(cells as usize);
        let mut decimal_traces = Vec::with_capacity(cells as usize);
        for row in rect.row_start..=rect.row_end {
            for column in rect.col_start..=rect.col_end {
                let cell = (rect.sheet, row, column);
                data.push(self.cell_value(cell));
                decimal_traces.push(self.numeric_decimal_trace(cell));
            }
        }
        Ok(ArrayEvaluation {
            array: Array {
                rows: rect.height() as u32,
                cols: rect.width() as u32,
                data,
            },
            decimal_traces,
        })
    }
}

fn array_decimal_at(array: &ArrayEvaluation, row: u32, column: u32) -> Option<DecimalTrace> {
    let source_row = if array.array.rows == 1 { 0 } else { row };
    let source_column = if array.array.cols == 1 { 0 } else { column };
    if source_row >= array.array.rows || source_column >= array.array.cols {
        return None;
    }
    let index = source_row as usize * array.array.cols as usize + source_column as usize;
    array.decimal_traces.get(index).copied().flatten()
}
