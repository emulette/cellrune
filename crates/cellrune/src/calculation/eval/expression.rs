use std::collections::BTreeSet;
use std::sync::Arc;

use super::{Engine, EvalContext};
use crate::calculation::ArithmeticSemantics;
use crate::calculation::ast::{BinaryOp, Expr, UnaryOp};
use crate::calculation::decimal::{DecimalTrace, is_excel_near_zero_cancellation};
use crate::calculation::functions::{
    DynamicFunction, Evaluator, FunctionResultKind, call_function_array, call_function_with_trace,
    callable_call_scope, function_call_shape_is_valid, function_evaluator, function_result_kind,
    intrinsic_scope_value, invoke_lambda, is_reference_returning_function, lambda_scope_value,
    let_scope_value, reduce_scope_value,
};
use crate::calculation::limits::CalculationLimitKind;
use crate::calculation::operators::{apply_binary, apply_unary, broadcast_shape, element_at};
use crate::calculation::runtime::{Array, ArrayExtent, Rect, ReferenceValue};
use crate::calculation::scope::{ArrayEvaluation, DefinedLambdaId, ScalarEvaluation, ScopeValue};
use crate::calculation::value::{ErrorKind, Value};

struct ArrayEvaluationContext {
    extent: Option<ArrayExtent>,
    visited_cells: u64,
}

impl ArrayEvaluationContext {
    const fn new(extent: Option<ArrayExtent>) -> Self {
        Self {
            extent,
            visited_cells: 0,
        }
    }

    fn charge(&mut self, engine: &Engine<'_>, cells: u64) -> Result<(), ErrorKind> {
        if self.extent.is_none() {
            return engine.ensure_array_cells(cells);
        }
        self.visited_cells = self
            .visited_cells
            .checked_add(cells)
            .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
        engine.ensure_array_cells(self.visited_cells)
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
    let near_zero_cancellation = match (&left.value, &right.value, &value) {
        (Value::Number(left), Value::Number(right), Value::Number(result)) => {
            is_excel_near_zero_cancellation(*left, *right, *result)
        }
        _ => false,
    };
    if matches!(arithmetic, ArithmeticSemantics::ExcelNearZero)
        && decimal_trace.is_some_and(DecimalTrace::is_zero)
        && near_zero_cancellation
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

fn scope_error(kind: ErrorKind) -> ScopeValue {
    ScopeValue::Scalar(ScalarEvaluation::untracked(Value::Error(kind)))
}

fn scope_from_array(evaluated: ArrayEvaluation) -> ScopeValue {
    if evaluated.array.is_scalar() {
        ScopeValue::Scalar(ScalarEvaluation {
            value: evaluated.array.data[0].clone(),
            decimal_trace: evaluated.decimal_traces[0],
        })
    } else {
        ScopeValue::Array(Arc::new(evaluated))
    }
}

impl Engine<'_> {
    pub(in crate::calculation) fn eval_scope_value(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
    ) -> ScopeValue {
        match expr {
            Expr::Missing => ScopeValue::Missing,
            Expr::Paren(inner) => self.eval_scope_value(context, inner),
            Expr::Name(name) => {
                if let Some(value) = context.binding(name) {
                    return value.clone();
                }
                match self.resolve_name_expr_with_id_in_context(context, name) {
                    Some((id, named))
                        if crate::calculation::lambda::definition(named).is_some() =>
                    {
                        lambda_scope_value(context, &named_lambda_args(named), Some(id))
                    }
                    Some((id, named)) => self.eval_scope_value(
                        context
                            .without_bindings()
                            .with_defined_name_scope(Some(id.scope())),
                        named,
                    ),
                    None => scope_error(ErrorKind::Name),
                }
            }
            Expr::Ref(_)
            | Expr::StructuredRef(_)
            | Expr::SpillRef(_)
            | Expr::ReferenceUnion { .. }
            | Expr::ReferenceIntersection { .. }
            | Expr::Range { .. } => self
                .resolve_reference_value_expr(context, expr)
                .map_or_else(scope_error, ScopeValue::Reference),
            Expr::Call { name, args } => {
                if let Some(scoped) = callable_call_scope(self, context, name, args) {
                    return scoped;
                }
                if function_evaluator(name).is_some() && !function_call_shape_is_valid(name, args) {
                    return scope_error(ErrorKind::Value);
                }
                match function_evaluator(name) {
                    Some(Evaluator::Dynamic(DynamicFunction::Let)) => {
                        let_scope_value(self, context, args)
                    }
                    Some(Evaluator::Dynamic(DynamicFunction::Lambda)) => {
                        lambda_scope_value(context, args, None)
                    }
                    Some(Evaluator::Dynamic(DynamicFunction::Reduce)) => {
                        reduce_scope_value(self, context, args).unwrap_or_else(scope_error)
                    }
                    _ if is_reference_returning_function(name) => self
                        .resolve_reference_value_expr(context, expr)
                        .map_or_else(scope_error, ScopeValue::Reference),
                    _ => self
                        .eval_array_with_trace(context, expr)
                        .map_or_else(scope_error, scope_from_array),
                }
            }
            Expr::Invoke { callee, args } => invoke_scope_value(self, context, callee, args),
            _ => self
                .eval_array_with_trace(context, expr)
                .map_or_else(scope_error, scope_from_array),
        }
    }

    pub(in crate::calculation) fn scalar_from_scope(
        &self,
        context: EvalContext<'_>,
        scoped: &ScopeValue,
    ) -> ScalarEvaluation {
        match scoped {
            ScopeValue::Missing => ScalarEvaluation::untracked(Value::Blank),
            ScopeValue::Scalar(evaluated) => evaluated.clone(),
            ScopeValue::Array(evaluated) => ScalarEvaluation {
                value: evaluated
                    .array
                    .data
                    .first()
                    .cloned()
                    .unwrap_or(Value::Error(ErrorKind::Value)),
                decimal_trace: evaluated.decimal_traces.first().copied().flatten(),
            },
            ScopeValue::Reference(reference) => {
                self.eval_reference_value_with_trace(context, reference.clone())
            }
            ScopeValue::Callable(_) => ScalarEvaluation::untracked(Value::Error(ErrorKind::Value)),
        }
    }

    pub(in crate::calculation) fn eval_final_scalar_with_trace(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
    ) -> ScalarEvaluation {
        if let Expr::Paren(inner) = expr {
            return self.eval_final_scalar_with_trace(context, inner);
        }
        let may_return_callable = match expr {
            Expr::Name(_) | Expr::Invoke { .. } => true,
            Expr::Call { name, .. } => {
                function_result_kind(name).is_some_and(|kind| {
                    matches!(
                        kind,
                        FunctionResultKind::Callable | FunctionResultKind::Contextual
                    )
                }) || self
                    .resolve_defined_lambda_in_context(context, name)
                    .is_some()
            }
            _ => false,
        };
        if may_return_callable {
            return match self.eval_scope_value(context, expr) {
                ScopeValue::Callable(_) => {
                    ScalarEvaluation::untracked(Value::Error(ErrorKind::Calc))
                }
                scoped => self.scalar_from_scope(context, &scoped),
            };
        }
        self.eval_scalar_with_trace(context, expr)
    }

    fn eval_reference_value_with_trace(
        &self,
        context: EvalContext<'_>,
        reference: ReferenceValue,
    ) -> ScalarEvaluation {
        if reference.has_sheet_span() {
            return ScalarEvaluation::untracked(Value::Error(ErrorKind::Value));
        }
        let rect = match reference.single_rect() {
            Ok(rect) => rect,
            Err(kind) => return ScalarEvaluation::untracked(Value::Error(kind)),
        };
        let Ok(rect) = self.implicit_intersection_rect(context, rect) else {
            return ScalarEvaluation::untracked(Value::Error(ErrorKind::Value));
        };
        self.scalar_reference_cell(context, rect)
            .unwrap_or_else(|kind| ScalarEvaluation::untracked(Value::Error(kind)))
    }

    fn eval_implicit_intersection(&self, context: EvalContext<'_>, expr: &Expr) -> Value {
        match expr {
            Expr::Paren(inner) | Expr::ImplicitIntersection(inner) => {
                self.eval_implicit_intersection(context, inner)
            }
            Expr::Ref(_)
            | Expr::StructuredRef(_)
            | Expr::SpillRef(_)
            | Expr::ReferenceUnion { .. }
            | Expr::ReferenceIntersection { .. }
            | Expr::Range { .. } => self
                .resolve_reference_value_expr(context, expr)
                .and_then(ReferenceValue::into_single_rect)
                .and_then(|rect| self.implicit_intersection_rect(context, rect))
                .and_then(|rect| self.scalar_reference_cell(context, rect))
                .map_or_else(Value::Error, |evaluated| evaluated.value),
            Expr::Name(name) => match self.resolve_name_expr_with_id_in_context(context, name) {
                Some((id, named)) => self.eval_implicit_intersection(
                    context
                        .without_bindings()
                        .with_defined_name_scope(Some(id.scope())),
                    named,
                ),
                None => Value::Error(ErrorKind::Name),
            },
            Expr::Call { name, args } => {
                if let Some(scoped) = callable_call_scope(self, context, name, args) {
                    return self.scalar_from_scope(context, &scoped).value;
                }
                if is_reference_returning_function(name) {
                    return self
                        .resolve_rect_expr(context, expr)
                        .and_then(|rect| self.implicit_intersection_rect(context, rect))
                        .and_then(|rect| self.scalar_reference_cell(context, rect))
                        .map_or_else(Value::Error, |evaluated| evaluated.value);
                }
                self.eval_array(context, expr)
                    .map_or_else(Value::Error, |array| {
                        array
                            .data
                            .into_iter()
                            .next()
                            .unwrap_or(Value::Error(ErrorKind::Value))
                    })
            }
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

    pub(in crate::calculation) fn eval_scalar_with_trace(
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
            Expr::ExternalReference(_) | Expr::QualifiedName { .. } => {
                ScalarEvaluation::untracked(Value::Error(ErrorKind::Unsupported))
            }
            Expr::Missing => ScalarEvaluation::untracked(Value::Blank),
            Expr::Paren(inner) => self.eval_scalar_with_trace(context, inner),
            Expr::ImplicitIntersection(inner) => {
                self.eval_implicit_intersection_with_trace(context, inner)
            }
            Expr::Array(_) => ScalarEvaluation::untracked(Value::Error(ErrorKind::Unsupported)),
            Expr::Name(name) => match context.binding(name) {
                Some(value) => self.scalar_from_scope(context, value),
                None => match self.resolve_name_expr_with_id_in_context(context, name) {
                    Some((id, named)) => self.eval_scalar_with_trace(
                        context
                            .without_bindings()
                            .with_defined_name_scope(Some(id.scope())),
                        named,
                    ),
                    None => ScalarEvaluation::untracked(Value::Error(ErrorKind::Name)),
                },
            },
            Expr::Ref(_)
            | Expr::StructuredRef(_)
            | Expr::SpillRef(_)
            | Expr::ReferenceUnion { .. }
            | Expr::ReferenceIntersection { .. }
            | Expr::Range { .. } => self.eval_reference_with_trace(context, expr),
            Expr::Call { name, args } => {
                if let Some(scoped) = callable_call_scope(self, context, name, args) {
                    return self.scalar_from_scope(context, &scoped);
                }
                if is_reference_returning_function(name) {
                    self.eval_reference_with_trace(context, expr)
                } else {
                    call_function_with_trace(self, context, name, args)
                }
            }
            Expr::Invoke { callee, args } => {
                self.scalar_from_scope(context, &invoke_scope_value(self, context, callee, args))
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

    /// Resolves the operand of an `@` (implicit intersection) operator down to one scalar.
    ///
    /// The operand may be wrapped in parentheses or in another `@` — Excel round-trips
    /// `_xlfn.SINGLE((A1:A5))` into exactly that shape — so those unwrap before the operand kind
    /// decides the intersection. Dispatching without unwrapping first would silently treat
    /// `=@(A1:A5)` as "the array's first element" instead of intersecting it.
    fn eval_implicit_intersection_with_trace(
        &self,
        context: EvalContext<'_>,
        operand: &Expr,
    ) -> ScalarEvaluation {
        match operand {
            Expr::Paren(inner) | Expr::ImplicitIntersection(inner) => {
                self.eval_implicit_intersection_with_trace(context, inner)
            }
            Expr::Name(name) if context.binding(name).is_some() => self.scalar_from_scope(
                context,
                context.binding(name).expect("binding presence checked"),
            ),
            Expr::Ref(_)
            | Expr::StructuredRef(_)
            | Expr::SpillRef(_)
            | Expr::ReferenceUnion { .. }
            | Expr::ReferenceIntersection { .. }
            | Expr::Range { .. }
            | Expr::Name(_) => self.eval_reference_with_trace(context, operand),
            Expr::Call { name, args } => {
                if let Some(scoped) = callable_call_scope(self, context, name, args) {
                    return self.scalar_from_scope(context, &scoped);
                }
                if is_reference_returning_function(name) {
                    self.eval_reference_with_trace(context, operand)
                } else {
                    self.first_array_value_with_trace(context, operand)
                }
            }
            _ => self.first_array_value_with_trace(context, operand),
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
        let reference = match self.resolve_reference_value_expr(context, expr) {
            Ok(reference) => reference,
            Err(_) => {
                return ScalarEvaluation::untracked(self.eval_implicit_intersection(context, expr));
            }
        };
        if reference.has_sheet_span() {
            return ScalarEvaluation::untracked(Value::Error(ErrorKind::Value));
        }
        let rect = match reference.into_single_rect() {
            Ok(rect) => rect,
            Err(kind) => return ScalarEvaluation::untracked(Value::Error(kind)),
        };
        let rect = match self.implicit_intersection_rect(context, rect) {
            Ok(rect) => rect,
            Err(kind) => return ScalarEvaluation::untracked(Value::Error(kind)),
        };
        self.scalar_reference_cell(context, rect)
            .unwrap_or_else(|kind| ScalarEvaluation::untracked(Value::Error(kind)))
    }

    fn scalar_reference_cell(
        &self,
        context: EvalContext<'_>,
        rect: Rect,
    ) -> Result<ScalarEvaluation, ErrorKind> {
        let cell = (rect.sheet, rect.row_start, rect.col_start);
        let value = self.read_reference_cell(context, cell)?;
        let decimal_trace = match value {
            Value::Number(_) => self.numeric_decimal_trace(cell),
            _ => None,
        };
        Ok(ScalarEvaluation {
            value,
            decimal_trace,
        })
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
        let extent = self.array_extent(
            context.without_reference_work_charge(),
            expr,
            &mut BTreeSet::new(),
        );
        let mut evaluation = ArrayEvaluationContext::new(extent);
        self.eval_array_with_trace_at_extent(context, expr, &mut evaluation)
    }

    fn eval_array_with_trace_at_extent(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
        evaluation: &mut ArrayEvaluationContext,
    ) -> Result<ArrayEvaluation, ErrorKind> {
        match expr {
            Expr::Paren(inner) => self.eval_array_with_trace_at_extent(context, inner, evaluation),
            Expr::ImplicitIntersection(inner) => Ok(ArrayEvaluation::scalar(
                self.eval_implicit_intersection_with_trace(context, inner),
            )),
            Expr::Name(name) if context.binding(name).is_some() => self.array_from_scope(
                context,
                context.binding(name).expect("binding presence checked"),
                evaluation,
            ),
            Expr::Name(name) => match self.resolve_name_expr_with_id_in_context(context, name) {
                Some((id, named)) => self.eval_array_with_trace_at_extent(
                    context
                        .without_bindings()
                        .with_defined_name_scope(Some(id.scope())),
                    named,
                    evaluation,
                ),
                None => Err(ErrorKind::Name),
            },
            Expr::Ref(_)
            | Expr::StructuredRef(_)
            | Expr::SpillRef(_)
            | Expr::ReferenceUnion { .. }
            | Expr::ReferenceIntersection { .. }
            | Expr::Range { .. } => {
                let reference = self.resolve_reference_value_expr(context, expr)?;
                if reference.has_sheet_span() {
                    return Err(ErrorKind::Value);
                }
                let rect = reference.into_single_rect()?;
                self.array_from_rect_with_trace(context, rect, evaluation)
            }
            Expr::Array(rows) => {
                let cols = rows.first().map_or(0, Vec::len);
                if rows.is_empty() || cols == 0 || rows.iter().any(|row| row.len() != cols) {
                    return Err(ErrorKind::Value);
                }
                let cell_count = (rows.len() as u64) * (cols as u64);
                evaluation.charge(self, cell_count)?;
                let (data, decimal_traces): (Vec<Value>, Vec<Option<DecimalTrace>>) = rows
                    .iter()
                    .flat_map(|row| {
                        row.iter()
                            .map(|value| self.eval_scalar_with_trace(context, value))
                    })
                    .map(|evaluated| (evaluated.value, evaluated.decimal_trace))
                    .unzip();
                Ok(ArrayEvaluation {
                    array: Array {
                        rows: rows.len() as u32,
                        cols: cols as u32,
                        data,
                    },
                    decimal_traces,
                })
            }
            Expr::Binary { op, left, right } => {
                let left = self.eval_array_with_trace_at_extent(context, left, evaluation)?;
                let right = self.eval_array_with_trace_at_extent(context, right, evaluation)?;
                let (rows, cols) = broadcast_shape(&left.array, &right.array)?;
                let cells = u64::from(rows)
                    .checked_mul(u64::from(cols))
                    .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
                evaluation.charge(self, cells)?;
                let mut data = Vec::with_capacity(cells as usize);
                let mut decimal_traces = Vec::with_capacity(cells as usize);
                for row in 0..rows {
                    for column in 0..cols {
                        let evaluated = evaluate_binary(
                            *op,
                            ScalarEvaluation {
                                value: element_at(&left.array, row, column).clone(),
                                decimal_trace: left.decimal_at(row, column),
                            },
                            ScalarEvaluation {
                                value: element_at(&right.array, row, column).clone(),
                                decimal_trace: right.decimal_at(row, column),
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
                let operand = self.eval_array_with_trace_at_extent(context, operand, evaluation)?;
                let (rows, cols) = (operand.array.rows, operand.array.cols);
                let cells = u64::from(rows)
                    .checked_mul(u64::from(cols))
                    .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
                evaluation.charge(self, cells)?;
                let (data, decimal_traces): (Vec<Value>, Vec<Option<DecimalTrace>>) = operand
                    .array
                    .data
                    .into_iter()
                    .zip(operand.decimal_traces)
                    .map(|(value, decimal_trace)| {
                        let evaluated = evaluate_unary(
                            *op,
                            ScalarEvaluation {
                                value,
                                decimal_trace,
                            },
                        );
                        (evaluated.value, evaluated.decimal_trace)
                    })
                    .unzip();
                Ok(ArrayEvaluation {
                    array: Array { rows, cols, data },
                    decimal_traces,
                })
            }
            Expr::Invoke { callee, args } => {
                let scoped = invoke_scope_value(self, context, callee, args);
                let evaluated = self.array_from_scope(context, &scoped, evaluation)?;
                if evaluation.extent.is_some() {
                    let cells = u64::from(evaluated.array.rows)
                        .checked_mul(u64::from(evaluated.array.cols))
                        .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
                    evaluation.charge(self, cells)?;
                }
                Ok(evaluated)
            }
            Expr::Call { name, args } => {
                if let Some(scoped) = callable_call_scope(self, context, name, args) {
                    let evaluated = self.array_from_scope(context, &scoped, evaluation)?;
                    if evaluation.extent.is_some() {
                        let cells = u64::from(evaluated.array.rows)
                            .checked_mul(u64::from(evaluated.array.cols))
                            .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
                        evaluation.charge(self, cells)?;
                    }
                    return Ok(evaluated);
                }
                if let Some(scoped) = intrinsic_scope_value(self, context, name, args) {
                    return self.array_from_scope(context, &scoped, evaluation);
                }
                if let Some(result) = call_function_array(self, context, name, args) {
                    let evaluated = result?;
                    if evaluation.extent.is_some() {
                        let cells = u64::from(evaluated.array.rows)
                            .checked_mul(u64::from(evaluated.array.cols))
                            .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
                        evaluation.charge(self, cells)?;
                    }
                    Ok(evaluated)
                } else {
                    Ok(ArrayEvaluation::scalar(call_function_with_trace(
                        self, context, name, args,
                    )))
                }
            }
            _ => Ok(ArrayEvaluation::scalar(
                self.eval_scalar_with_trace(context, expr),
            )),
        }
    }

    fn array_from_scope(
        &self,
        context: EvalContext<'_>,
        scoped: &ScopeValue,
        evaluation: &mut ArrayEvaluationContext,
    ) -> Result<ArrayEvaluation, ErrorKind> {
        match scoped {
            ScopeValue::Missing => Ok(ArrayEvaluation::scalar(ScalarEvaluation::untracked(
                Value::Blank,
            ))),
            ScopeValue::Scalar(evaluated) => Ok(ArrayEvaluation::scalar(evaluated.clone())),
            ScopeValue::Array(evaluated) => Ok(evaluated.as_ref().clone()),
            ScopeValue::Reference(reference) => {
                if reference.has_sheet_span() {
                    return Err(ErrorKind::Value);
                }
                let rect = reference.clone().into_single_rect()?;
                self.array_from_rect_with_trace(context, rect, evaluation)
            }
            ScopeValue::Callable(_) => Ok(ArrayEvaluation::scalar(ScalarEvaluation::untracked(
                Value::Error(ErrorKind::Value),
            ))),
        }
    }

    pub(in crate::calculation) fn eval_final_array_with_trace(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
    ) -> Result<ArrayEvaluation, ErrorKind> {
        if let Expr::Paren(inner) = expr {
            return self.eval_final_array_with_trace(context, inner);
        }
        let may_return_callable = match expr {
            Expr::Invoke { .. } => true,
            Expr::Name(name) => self
                .resolve_defined_lambda_in_context(context, name)
                .is_some(),
            Expr::Call { name, .. } => {
                function_result_kind(name).is_some_and(|kind| {
                    matches!(
                        kind,
                        FunctionResultKind::Callable | FunctionResultKind::Contextual
                    )
                }) || self
                    .resolve_defined_lambda_in_context(context, name)
                    .is_some()
            }
            _ => false,
        };
        if may_return_callable {
            return match self.eval_scope_value(context, expr) {
                ScopeValue::Callable(_) => Ok(ArrayEvaluation::scalar(
                    ScalarEvaluation::untracked(Value::Error(ErrorKind::Calc)),
                )),
                scoped => self.array_from_scope_value(context, &scoped),
            };
        }
        self.eval_array_with_trace(context, expr)
    }

    pub(in crate::calculation) fn array_from_scope_value(
        &self,
        context: EvalContext<'_>,
        scoped: &ScopeValue,
    ) -> Result<ArrayEvaluation, ErrorKind> {
        self.array_from_scope(context, scoped, &mut ArrayEvaluationContext::new(None))
    }

    pub(in crate::calculation) fn array_from_rect(
        &self,
        context: EvalContext<'_>,
        source: &Expr,
        rect: Rect,
    ) -> Result<Array, ErrorKind> {
        let extent = self.array_extent(context, source, &mut BTreeSet::new());
        self.array_from_rect_with_trace(context, rect, &mut ArrayEvaluationContext::new(extent))
            .map(|evaluated| evaluated.array)
    }

    fn array_from_rect_with_trace(
        &self,
        context: EvalContext<'_>,
        rect: Rect,
        evaluation: &mut ArrayEvaluationContext,
    ) -> Result<ArrayEvaluation, ErrorKind> {
        if rect.is_single_cell() {
            let cell = (rect.sheet, rect.row_start, rect.col_start);
            return Ok(ArrayEvaluation::scalar(ScalarEvaluation {
                value: self.read_reference_cell(context, cell)?,
                decimal_trace: self.numeric_decimal_trace(cell),
            }));
        }
        let row_end = if rect.whole_rows {
            evaluation
                .extent
                .ok_or(ErrorKind::Unsupported)?
                .row_end()
                .min(rect.row_end)
        } else {
            rect.row_end
        };
        let rows = if row_end < rect.row_start {
            0
        } else {
            u64::from(row_end - rect.row_start) + 1
        };
        let cells = rows
            .checked_mul(rect.width())
            .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
        evaluation.charge(self, cells)?;
        let mut data = Vec::with_capacity(cells as usize);
        let mut decimal_traces = Vec::with_capacity(cells as usize);
        if row_end >= rect.row_start {
            for row in rect.row_start..=row_end {
                for column in rect.col_start..=rect.col_end {
                    let cell = (rect.sheet, row, column);
                    data.push(self.read_reference_cell(context, cell)?);
                    decimal_traces.push(self.numeric_decimal_trace(cell));
                }
            }
        }
        Ok(ArrayEvaluation {
            array: Array {
                rows: rows as u32,
                cols: rect.width() as u32,
                data,
            },
            decimal_traces,
        })
    }

    fn array_extent(
        &self,
        context: EvalContext<'_>,
        expr: &Expr,
        names: &mut BTreeSet<DefinedLambdaId>,
    ) -> Option<ArrayExtent> {
        match expr {
            Expr::Paren(inner) | Expr::Unary { operand: inner, .. } => {
                self.array_extent(context, inner, names)
            }
            Expr::Binary { left, right, .. } => match (
                self.array_extent(context, left, names),
                self.array_extent(context, right, names),
            ) {
                (Some(left), Some(right)) => Some(left.merged(right)),
                (Some(extent), None) | (None, Some(extent)) => Some(extent),
                (None, None) => None,
            },
            Expr::Ref(_)
            | Expr::StructuredRef(_)
            | Expr::SpillRef(_)
            | Expr::ReferenceUnion { .. }
            | Expr::ReferenceIntersection { .. }
            | Expr::Range { .. } => self
                .resolve_reference_value_expr(context, expr)
                .ok()
                .and_then(|reference| self.array_extent_from_reference(&reference)),
            Expr::Name(name) if context.binding(name).is_some() => {
                match context.binding(name).expect("binding presence checked") {
                    ScopeValue::Reference(reference) => self.array_extent_from_reference(reference),
                    ScopeValue::Missing
                    | ScopeValue::Scalar(_)
                    | ScopeValue::Array(_)
                    | ScopeValue::Callable(_) => None,
                }
            }
            Expr::Name(name) => {
                let (id, named) = self.resolve_name_expr_with_id_in_context(context, name)?;
                if !names.insert(id.clone()) {
                    return None;
                }
                self.array_extent(
                    context
                        .without_bindings()
                        .with_defined_name_scope(Some(id.scope())),
                    named,
                    names,
                )
            }
            Expr::Call { name, .. }
                if context.binding(name).is_none()
                    && self
                        .resolve_name_expr_with_id_in_context(context, name)
                        .is_none()
                    && is_reference_returning_function(name) =>
            {
                self.resolve_reference_value_expr(context, expr)
                    .ok()
                    .and_then(|reference| self.array_extent_from_reference(&reference))
            }
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::ExternalReference(_)
            | Expr::QualifiedName { .. }
            | Expr::Missing
            | Expr::ImplicitIntersection(_)
            | Expr::Array(_)
            | Expr::Call { .. }
            | Expr::Invoke { .. } => None,
        }
    }

    fn array_extent_from_reference(&self, reference: &ReferenceValue) -> Option<ArrayExtent> {
        reference
            .rects()
            .filter(|rect| rect.whole_rows)
            .map(|rect| ArrayExtent::new(self.whole_column_row_end(&rect)))
            .reduce(ArrayExtent::merged)
    }
}

fn named_lambda_args(expr: &Expr) -> Vec<Expr> {
    let Expr::Call { args, .. } = expr else {
        return Vec::new();
    };
    args.clone()
}

fn invoke_scope_value(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    callee: &Expr,
    args: &[Expr],
) -> ScopeValue {
    match engine.eval_scope_value(context, callee) {
        ScopeValue::Callable(closure) => invoke_lambda(engine, context, &closure, args),
        ScopeValue::Scalar(evaluated) if matches!(evaluated.value, Value::Error(_)) => {
            ScopeValue::Scalar(evaluated)
        }
        _ => ScopeValue::Scalar(ScalarEvaluation::untracked(Value::Error(ErrorKind::Value))),
    }
}
