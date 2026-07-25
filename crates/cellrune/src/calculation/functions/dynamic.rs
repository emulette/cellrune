use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::lambda::{LambdaBinding, definition};
use super::super::operators::element_at;
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};

pub(super) fn map_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
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
        .map(|expr| engine.eval_array(context, expr))
        .collect::<Result<Vec<_>, _>>()?;
    let (rows, cols) = common_shape(&arrays)?;
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
            .map(|name| LambdaBinding::new(name, Value::Blank)),
    );
    let mut data = Vec::with_capacity(cells as usize);
    for row in 0..rows {
        for col in 0..cols {
            for (binding, array) in bindings[outer_binding_count..].iter_mut().zip(&arrays) {
                binding.set_value(element_at(array, row, col).clone());
            }
            data.push(engine.eval_scalar(context.with_bindings(&bindings), lambda.body()));
        }
    }
    Ok(Array { rows, cols, data })
}

pub(super) fn map_scalar(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    match map_array(engine, context, args) {
        Ok(array) => array
            .data
            .into_iter()
            .next()
            .unwrap_or(Value::Error(ErrorKind::Value)),
        Err(kind) => Value::Error(kind),
    }
}

fn common_shape(arrays: &[Array]) -> Result<(u32, u32), ErrorKind> {
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
