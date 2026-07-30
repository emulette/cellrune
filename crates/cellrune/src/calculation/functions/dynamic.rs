use std::collections::BTreeSet;
use std::sync::Arc;

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::lambda::{LocalNamePolicy, definition, validate_local_name};
use super::super::limits::CalculationLimitKind;
use super::super::operators::element_at;
use super::super::runtime::{Array, RectSpan};
use super::super::scope::{
    ArrayEvaluation, LambdaClosure, ScalarEvaluation, ScopeEntry, ScopeValue,
};
use super::super::value::{ErrorKind, Value};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    match name {
        "MAP" => map_scalar_with_trace(engine, context, args).value,
        "ISOMITTED" => is_omitted(context, args),
        "BYROW" | "BYCOL" | "REDUCE" | "SCAN" | "MAKEARRAY" => {
            helper_scalar_with_trace(engine, context, name, args).value
        }
        "LET" => {
            let scoped = let_scope_value(engine, context, args);
            engine.scalar_from_scope(context, &scoped).value
        }
        _ => Value::Error(ErrorKind::Unsupported),
    }
}

fn is_omitted(context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let Expr::Name(name) = &args[0] else {
        return Value::Error(ErrorKind::Value);
    };
    match context.binding(name) {
        Some(ScopeValue::Missing) => Value::Logical(true),
        Some(_) => Value::Logical(false),
        None => Value::Error(ErrorKind::Value),
    }
}

pub(in crate::calculation) fn lambda_scope_value(
    context: EvalContext<'_>,
    args: &[Expr],
    defined_name: Option<&str>,
) -> ScopeValue {
    let expression = Expr::Call {
        name: "LAMBDA".to_owned(),
        args: args.to_vec(),
    };
    let Some(definition) = definition(&expression) else {
        return ScopeValue::Scalar(ScalarEvaluation::untracked(Value::Error(ErrorKind::Value)));
    };
    ScopeValue::Callable(std::sync::Arc::new(LambdaClosure {
        parameters: definition.parameters().to_vec(),
        body: definition.body().clone(),
        captured: context.bindings().to_vec(),
        defined_name: defined_name.map(str::to_owned).map(Into::into),
    }))
}

pub(in crate::calculation) fn invoke_lambda(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    closure: &LambdaClosure,
    args: &[Expr],
) -> ScopeValue {
    if closure.parameters.len() != args.len() {
        return ScopeValue::Scalar(ScalarEvaluation::untracked(Value::Error(ErrorKind::Value)));
    }
    let mut values = Vec::with_capacity(args.len());
    for arg in args {
        if context.is_cancelled() {
            return ScopeValue::Scalar(ScalarEvaluation::untracked(Value::Error(
                ErrorKind::ResourceLimit(CalculationLimitKind::LambdaInvocations),
            )));
        }
        let value = engine.eval_scope_value(context, arg);
        if let Some(kind) = value.engine_issue() {
            return ScopeValue::Scalar(ScalarEvaluation::untracked(Value::Error(kind)));
        }
        if let ScopeValue::Scalar(evaluated) = &value
            && let Some(kind) = evaluated.engine_issue()
        {
            return ScopeValue::Scalar(ScalarEvaluation::untracked(Value::Error(kind)));
        }
        values.push(value);
    }
    invoke_lambda_values(engine, context, closure, values)
}

pub(in crate::calculation) fn invoke_lambda_values(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    closure: &LambdaClosure,
    values: Vec<ScopeValue>,
) -> ScopeValue {
    if closure.parameters.len() != values.len() {
        return ScopeValue::Scalar(ScalarEvaluation::untracked(Value::Error(ErrorKind::Value)));
    }
    let _active_lambda = match context.enter_lambda(engine.calculation_limits()) {
        Ok(active) => active,
        Err(kind) => return ScopeValue::Scalar(ScalarEvaluation::untracked(Value::Error(kind))),
    };
    let mut bindings = closure.captured.clone();
    bindings.extend(
        closure
            .parameters
            .iter()
            .cloned()
            .zip(values)
            .map(|(name, value)| ScopeEntry::new(name, value)),
    );
    if context.is_cancelled() {
        return ScopeValue::Scalar(ScalarEvaluation::untracked(Value::Error(
            ErrorKind::ResourceLimit(CalculationLimitKind::LambdaInvocations),
        )));
    }
    engine.eval_scope_value(context.with_bindings(&bindings), &closure.body)
}

pub(super) fn map_array_with_trace(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<ArrayEvaluation, ErrorKind> {
    let Some((lambda_expr, array_exprs)) = args.split_last() else {
        return Err(ErrorKind::Value);
    };
    if array_exprs.is_empty() {
        return Err(ErrorKind::Value);
    }
    let lambda = definition(lambda_expr).ok_or(ErrorKind::Value)?;
    if lambda.parameters().len() != array_exprs.len() {
        return Err(ErrorKind::Value);
    }

    let arrays = array_exprs
        .iter()
        .map(|expr| engine.eval_array_with_trace(context, expr))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(kind) = arrays.iter().find_map(ArrayEvaluation::engine_issue) {
        return Err(kind);
    }
    let shapes = arrays
        .iter()
        .map(|evaluated| &evaluated.array)
        .collect::<Vec<_>>();
    let (rows, cols) = common_shape(&shapes)?;
    let cells = u64::from(rows) * u64::from(cols);
    engine.ensure_array_cells(cells)?;
    engine.ensure_function_iterations(cells)?;

    let capacity = usize::try_from(cells)
        .map_err(|_| ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
    let mut data = Vec::with_capacity(capacity);
    let mut decimal_traces = Vec::with_capacity(capacity);
    for row in 0..rows {
        for col in 0..cols {
            let values = arrays
                .iter()
                .map(|evaluated| {
                    ScopeValue::Scalar(ScalarEvaluation {
                        value: element_at(&evaluated.array, row, col).clone(),
                        decimal_trace: evaluated.decimal_at(row, col),
                    })
                })
                .collect();
            let scoped = invoke_lambda_values(
                engine,
                context,
                &LambdaClosure {
                    parameters: lambda.parameters().to_vec(),
                    body: lambda.body().clone(),
                    captured: context.bindings().to_vec(),
                    defined_name: None,
                },
                values,
            );
            let evaluated = lambda_result_scalar(engine, context, &scoped)?;
            if let Some(kind) = evaluated.engine_issue() {
                return Err(kind);
            }
            data.push(evaluated.value);
            decimal_traces.push(evaluated.decimal_trace);
        }
    }
    Ok(ArrayEvaluation {
        array: Array { rows, cols, data },
        decimal_traces,
    })
}

pub(in crate::calculation) fn helper_array_with_trace(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Option<Result<ArrayEvaluation, ErrorKind>> {
    match name {
        "BYROW" => Some(byrow(engine, context, args)),
        "BYCOL" => Some(bycol(engine, context, args)),
        "REDUCE" => Some(reduce(engine, context, args, false)),
        "SCAN" => Some(reduce(engine, context, args, true)),
        "MAKEARRAY" => Some(makearray(engine, context, args)),
        _ => None,
    }
}

pub(in crate::calculation) fn helper_scalar_with_trace(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> ScalarEvaluation {
    match helper_array_with_trace(engine, context, name, args) {
        Some(Ok(result)) => ScalarEvaluation {
            value: result
                .array
                .data
                .first()
                .cloned()
                .unwrap_or(Value::Error(ErrorKind::Value)),
            decimal_trace: result.decimal_traces.first().copied().flatten(),
        },
        Some(Err(kind)) => ScalarEvaluation::untracked(Value::Error(kind)),
        None => ScalarEvaluation::untracked(Value::Error(ErrorKind::Unsupported)),
    }
}

fn lambda_closure(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<LambdaClosure, ErrorKind> {
    let Some(_) = definition(expr) else {
        return Err(ErrorKind::Value);
    };
    match engine.eval_scope_value(context, expr) {
        ScopeValue::Callable(closure) => Ok((*closure).clone()),
        _ => Err(ErrorKind::Value),
    }
}

fn scalar_scope(value: Value) -> ScopeValue {
    ScopeValue::Scalar(ScalarEvaluation::untracked(value))
}

fn lambda_result_scalar(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    result: &ScopeValue,
) -> Result<ScalarEvaluation, ErrorKind> {
    if let ScopeValue::Array(evaluated) = result
        && evaluated.array.data.len() != 1
    {
        return Err(ErrorKind::Calc);
    }
    Ok(engine.scalar_from_scope(context, result))
}

fn byrow(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<ArrayEvaluation, ErrorKind> {
    if args.len() != 2 {
        return Err(ErrorKind::Value);
    }
    let input = engine.eval_array_with_trace(context, &args[0])?;
    let closure = lambda_closure(engine, context, &args[1])?;
    if closure.parameters.len() != 1 {
        return Err(ErrorKind::Value);
    }
    engine.ensure_array_cells(u64::from(input.array.rows))?;
    engine.ensure_function_iterations(u64::from(input.array.rows))?;
    let mut data = Vec::with_capacity(input.array.rows as usize);
    let mut decimal_traces = Vec::with_capacity(input.array.rows as usize);
    for row in 0..input.array.rows {
        let row_data = (0..input.array.cols)
            .map(|col| element_at(&input.array, row, col).clone())
            .collect();
        let row_value = ScopeValue::Array(Arc::new(ArrayEvaluation {
            array: Array {
                rows: 1,
                cols: input.array.cols,
                data: row_data,
            },
            decimal_traces: (0..input.array.cols)
                .map(|col| input.decimal_at(row, col))
                .collect(),
        }));
        let result = invoke_lambda_values(engine, context, &closure, vec![row_value]);
        let scalar = lambda_result_scalar(engine, context, &result)?;
        if let Some(kind) = scalar.engine_issue() {
            return Err(kind);
        }
        data.push(scalar.value);
        decimal_traces.push(scalar.decimal_trace);
    }
    Ok(ArrayEvaluation {
        array: Array {
            rows: input.array.rows,
            cols: 1,
            data,
        },
        decimal_traces,
    })
}

fn bycol(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<ArrayEvaluation, ErrorKind> {
    if args.len() != 2 {
        return Err(ErrorKind::Value);
    }
    let input = engine.eval_array_with_trace(context, &args[0])?;
    let closure = lambda_closure(engine, context, &args[1])?;
    if closure.parameters.len() != 1 {
        return Err(ErrorKind::Value);
    }
    engine.ensure_array_cells(u64::from(input.array.cols))?;
    engine.ensure_function_iterations(u64::from(input.array.cols))?;
    let mut data = Vec::with_capacity(input.array.cols as usize);
    let mut decimal_traces = Vec::with_capacity(input.array.cols as usize);
    for col in 0..input.array.cols {
        let col_data = (0..input.array.rows)
            .map(|row| element_at(&input.array, row, col).clone())
            .collect();
        let col_value = ScopeValue::Array(Arc::new(ArrayEvaluation {
            array: Array {
                rows: input.array.rows,
                cols: 1,
                data: col_data,
            },
            decimal_traces: (0..input.array.rows)
                .map(|row| input.decimal_at(row, col))
                .collect(),
        }));
        let result = invoke_lambda_values(engine, context, &closure, vec![col_value]);
        let scalar = lambda_result_scalar(engine, context, &result)?;
        if let Some(kind) = scalar.engine_issue() {
            return Err(kind);
        }
        data.push(scalar.value);
        decimal_traces.push(scalar.decimal_trace);
    }
    Ok(ArrayEvaluation {
        array: Array {
            rows: 1,
            cols: input.array.cols,
            data,
        },
        decimal_traces,
    })
}

fn reduce(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    scan: bool,
) -> Result<ArrayEvaluation, ErrorKind> {
    if args.len() != 3 {
        return Err(ErrorKind::Value);
    }
    let mut accumulator = engine.eval_scope_value(context, &args[0]);
    lambda_result_scalar(engine, context, &accumulator)?;
    let input = engine.eval_array_with_trace(context, &args[1])?;
    let closure = lambda_closure(engine, context, &args[2])?;
    if closure.parameters.len() != 2 {
        return Err(ErrorKind::Value);
    }
    let cells = u64::from(input.array.rows) * u64::from(input.array.cols);
    engine.ensure_array_cells(cells)?;
    engine.ensure_function_iterations(cells)?;
    let mut output = Vec::with_capacity(input.array.data.len());
    let mut decimal_traces = Vec::with_capacity(input.array.data.len());
    for (index, value) in input.array.data.iter().enumerate() {
        accumulator = invoke_lambda_values(
            engine,
            context,
            &closure,
            vec![
                accumulator,
                ScopeValue::Scalar(ScalarEvaluation {
                    value: value.clone(),
                    decimal_trace: input.decimal_traces[index],
                }),
            ],
        );
        let scalar = lambda_result_scalar(engine, context, &accumulator)?;
        if let Some(kind) = scalar.engine_issue() {
            return Err(kind);
        }
        if scan {
            output.push(scalar.value.clone());
            decimal_traces.push(scalar.decimal_trace);
        }
    }
    if scan {
        Ok(ArrayEvaluation {
            array: Array {
                rows: input.array.rows,
                cols: input.array.cols,
                data: output,
            },
            decimal_traces,
        })
    } else {
        Ok(ArrayEvaluation::scalar(lambda_result_scalar(
            engine,
            context,
            &accumulator,
        )?))
    }
}

fn makearray(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<ArrayEvaluation, ErrorKind> {
    if args.len() != 3 {
        return Err(ErrorKind::Value);
    }
    let rows = engine.eval_number_with_trace(context, &args[0])?.0;
    let cols = engine.eval_number_with_trace(context, &args[1])?.0;
    if rows < 1.0 || cols < 1.0 || rows.fract() != 0.0 || cols.fract() != 0.0 {
        return Err(ErrorKind::Value);
    }
    let rows = u32::try_from(rows as u64).map_err(|_| ErrorKind::Value)?;
    let cols = u32::try_from(cols as u64).map_err(|_| ErrorKind::Value)?;
    let cells = u64::from(rows) * u64::from(cols);
    engine.ensure_array_cells(cells)?;
    engine.ensure_function_iterations(cells)?;
    let closure = lambda_closure(engine, context, &args[2])?;
    if closure.parameters.len() != 2 {
        return Err(ErrorKind::Value);
    }
    let mut data = Vec::with_capacity(cells as usize);
    let mut decimal_traces = Vec::with_capacity(cells as usize);
    for row in 1..=rows {
        for col in 1..=cols {
            let result = invoke_lambda_values(
                engine,
                context,
                &closure,
                vec![
                    scalar_scope(Value::Number(f64::from(row))),
                    scalar_scope(Value::Number(f64::from(col))),
                ],
            );
            let scalar = lambda_result_scalar(engine, context, &result)?;
            if let Some(kind) = scalar.engine_issue() {
                return Err(kind);
            }
            data.push(scalar.value);
            decimal_traces.push(scalar.decimal_trace);
        }
    }
    Ok(ArrayEvaluation {
        array: Array { rows, cols, data },
        decimal_traces,
    })
}

pub(in crate::calculation) fn map_scalar_with_trace(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> ScalarEvaluation {
    match map_array_with_trace(engine, context, args) {
        Ok(mut evaluated) => ScalarEvaluation {
            value: evaluated
                .array
                .data
                .drain(..)
                .next()
                .unwrap_or(Value::Error(ErrorKind::Value)),
            decimal_trace: evaluated.decimal_traces.drain(..).next().flatten(),
        },
        Err(kind) => ScalarEvaluation::untracked(Value::Error(kind)),
    }
}

pub(in crate::calculation) fn let_scope_value(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> ScopeValue {
    match with_let_scope(engine, context, args, |engine, scoped, expr, is_final| {
        is_final.then(|| engine.eval_scope_value(scoped, expr))
    }) {
        Ok(value) => value,
        Err(kind) => ScopeValue::Scalar(ScalarEvaluation::untracked(Value::Error(kind))),
    }
}

pub(in crate::calculation) fn let_reference(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<RectSpan, ErrorKind> {
    match let_scope_value(engine, context, args) {
        ScopeValue::Reference(span) => Ok(span),
        ScopeValue::Scalar(evaluated) => match evaluated.value {
            Value::Error(kind) => Err(kind),
            _ => Err(ErrorKind::Value),
        },
        ScopeValue::Missing | ScopeValue::Array(_) | ScopeValue::Callable(_) => {
            Err(ErrorKind::Value)
        }
    }
}

pub(in crate::calculation) fn with_let_scope<ResultValue>(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    mut visit: impl FnMut(&Engine<'_>, EvalContext<'_>, &Expr, bool) -> Option<ResultValue>,
) -> Result<ResultValue, ErrorKind> {
    if args.len() < 3 || args.len().is_multiple_of(2) {
        return Err(ErrorKind::Value);
    }
    let binding_count = (args.len() - 1) / 2;
    if binding_count as u64 > engine.calculation_limits().max_let_bindings() {
        return Err(ErrorKind::ResourceLimit(CalculationLimitKind::LetBindings));
    }

    let (final_expr, pairs) = args
        .split_last()
        .expect("minimum LET arity was checked above");
    let mut seen = BTreeSet::new();
    let mut names = Vec::with_capacity(binding_count);
    for pair in pairs.chunks_exact(2) {
        let Expr::Name(raw_name) = &pair[0] else {
            return Err(ErrorKind::Value);
        };
        let name = validate_local_name(raw_name, LocalNamePolicy::Let)
            .ok_or(ErrorKind::Value)?
            .into_string();
        if !seen.insert(name.clone()) {
            return Err(ErrorKind::Value);
        }
        names.push(name);
    }

    let mut bindings = context.bindings().to_vec();
    for (pair, name) in pairs.chunks_exact(2).zip(names) {
        let scoped = context.with_bindings(&bindings);
        let _ = visit(engine, scoped, &pair[1], false);
        let value = engine.eval_scope_value(scoped, &pair[1]);
        if let Some(kind) = value.engine_issue() {
            return Err(kind);
        }
        bindings.push(ScopeEntry::new(name, value));
    }
    visit(engine, context.with_bindings(&bindings), final_expr, true).ok_or(ErrorKind::Value)
}

fn common_shape(arrays: &[&Array]) -> Result<(u32, u32), ErrorKind> {
    let mut shape = None;
    for array in arrays {
        if array.is_scalar() {
            continue;
        }
        match shape {
            None => shape = Some((array.rows, array.cols)),
            Some((rows, cols)) if rows == array.rows && cols == array.cols => {}
            Some(_) => return Err(ErrorKind::Value),
        }
    }
    Ok(shape.unwrap_or((1, 1)))
}
