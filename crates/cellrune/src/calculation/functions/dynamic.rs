use std::collections::BTreeSet;

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::lambda::{LocalNamePolicy, definition, validate_local_name};
use super::super::limits::CalculationLimitKind;
use super::super::operators::element_at;
use super::super::runtime::{Array, RectSpan};
use super::super::scope::{ArrayEvaluation, ScalarEvaluation, ScopeEntry, ScopeValue};
use super::super::value::{ErrorKind, Value};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    match name {
        "MAP" => map_scalar_with_trace(engine, context, args).value,
        "LET" => {
            let scoped = let_scope_value(engine, context, args);
            engine.scalar_from_scope(context, &scoped).value
        }
        _ => Value::Error(ErrorKind::Unsupported),
    }
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

    let outer_binding_count = context.bindings().len();
    let mut bindings = context.bindings().to_vec();
    bindings.extend(
        lambda
            .parameters()
            .iter()
            .cloned()
            .map(ScopeEntry::placeholder),
    );
    let capacity = usize::try_from(cells)
        .map_err(|_| ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
    let mut data = Vec::with_capacity(capacity);
    let mut decimal_traces = Vec::with_capacity(capacity);
    for row in 0..rows {
        for col in 0..cols {
            for (binding, evaluated) in bindings[outer_binding_count..].iter_mut().zip(&arrays) {
                binding.set_value(ScopeValue::Scalar(ScalarEvaluation {
                    value: element_at(&evaluated.array, row, col).clone(),
                    decimal_trace: evaluated.decimal_at(row, col),
                }));
            }
            let _active_lambda = context.enter_lambda(engine.calculation_limits())?;
            let evaluated =
                engine.eval_scalar_with_trace(context.with_bindings(&bindings), lambda.body());
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
        ScopeValue::Missing | ScopeValue::Array(_) => Err(ErrorKind::Value),
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
