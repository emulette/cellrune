//! Element-wise calls: a function that takes a value where an array arrives is called once per
//! element, and its results keep the array's shape, as Excel does (`=SQRT(A1:A3)` spills three
//! roots and `SUMPRODUCT(LEN(A1:A3))` adds three lengths).
//!
//! The arguments the call contract takes as values are evaluated once, as arrays, and the
//! kernel reads each element through a binding: a blank element stays a blank cell rather than
//! becoming an omitted argument (`LEFT("abc",B1:B2)` with `B1` blank is `""` for that element, not
//! `"a"`). Only a call in an array context lifts; the result of a formula that is not an array
//! formula takes its value by implicit intersection before it gets here.

use super::super::ast::{Expr, RefBody};
use super::super::eval::{Engine, EvalContext};
use super::super::operators::{broadcast_shape, element_at};
use super::super::runtime::Array;
use super::super::scope::{ArrayEvaluation, ScalarEvaluation, ScopeEntry, ScopeValue};
use super::super::value::{ErrorKind, Value};
use super::contract::ArgumentMode;
use super::descriptor::{self, FunctionDescriptor, FunctionResultKind};
use super::{direct_sheet_span_error, evaluate_prepared};

/// Calls `descriptor` element by element over the arrays its value arguments hold. Returns `None`
/// when no value argument can hold an array, so the call stays scalar without evaluating them.
pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    descriptor: FunctionDescriptor,
    args: &[Expr],
    written: &[Expr],
) -> Option<Result<ArrayEvaluation, ErrorKind>> {
    if descriptor.result_kind() != FunctionResultKind::Scalar {
        return None;
    }
    let lifted = lifted_positions(engine, context, args, value_arguments(descriptor, args));
    if lifted.is_empty() {
        return None;
    }
    if let Some(kind) = direct_sheet_span_error(engine, context, descriptor, args) {
        return Some(Ok(ArrayEvaluation::scalar(ScalarEvaluation::new(
            Value::Error(kind),
        ))));
    }
    Some(
        lift(
            engine,
            context,
            args,
            written,
            &lifted,
            |context, args, written| {
                evaluate_prepared(descriptor, engine, context, args, written).value
            },
        )
        .map(ArrayEvaluation::new),
    )
}

/// The positions among `positions` whose argument may hold an array.
pub(super) fn lifted_positions(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    positions: impl Iterator<Item = usize>,
) -> Vec<usize> {
    positions
        .filter(|index| {
            args.get(*index)
                .is_some_and(|arg| may_hold_array(engine, context, arg))
        })
        .collect()
}

/// Evaluates the arguments at `lifted` as arrays and calls `call` once per element of their
/// broadcast shape, with those arguments bound to the element's values and the others as written.
/// `written` is substituted alike where it has the position.
pub(super) fn lift(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    written: &[Expr],
    lifted: &[usize],
    mut call: impl FnMut(EvalContext<'_>, &[Expr], &[Expr]) -> Value,
) -> Result<Array, ErrorKind> {
    let mut arrays = Vec::with_capacity(lifted.len());
    for index in lifted {
        arrays.push(match engine.eval_array(context, &args[*index]) {
            Ok(array) => array,
            Err(kind) if kind.is_engine_issue() => return Err(kind),
            // The argument's own error is the value the kernel reads, as in a scalar call.
            Err(kind) => Array::scalar(Value::Error(kind)),
        });
    }
    let (rows, cols) = arrays.iter().try_fold((1, 1), |(rows, cols), array| {
        broadcast_shape(
            &Array {
                rows,
                cols,
                data: Vec::new(),
            },
            array,
        )
    })?;
    let cells = u64::from(rows) * u64::from(cols);
    let several = cells > 1;
    if several {
        engine.ensure_array_cells(cells)?;
        engine.charge_function_iterations(context, cells)?;
    }
    let names = (0..lifted.len()).map(binding_name).collect::<Vec<_>>();
    let substitute = |expressions: &[Expr]| {
        let mut substituted = expressions.to_vec();
        for (name, index) in names.iter().zip(lifted) {
            if let Some(expression) = substituted.get_mut(*index) {
                *expression = Expr::Name(name.clone());
            }
        }
        substituted
    };
    let element_args = substitute(args);
    let element_written = substitute(written);
    let outer = context.bindings().len();
    let mut bindings = context.bindings().to_vec();
    bindings.extend(
        names
            .iter()
            .map(|name| ScopeEntry::new(name.clone(), ScopeValue::Missing)),
    );
    let mut data = Vec::with_capacity(cells as usize);
    for row in 0..rows {
        for col in 0..cols {
            for (entry, array) in bindings[outer..].iter_mut().zip(&arrays) {
                entry.set_value(ScopeValue::Scalar(ScalarEvaluation::new(
                    element_at(array, row, col).clone(),
                )));
            }
            let mut element_context = context.with_bindings(&bindings);
            if several {
                element_context =
                    element_context.at_iteration(u64::from(row) * u64::from(cols) + u64::from(col));
            }
            let element_context = element_context.with_arguments(&element_args);
            let value = call(element_context, &element_args, &element_written);
            if let Value::Error(kind) = value
                && kind.is_engine_issue()
            {
                return Err(kind);
            }
            data.push(value);
        }
    }
    Ok(Array { rows, cols, data })
}

/// The positions of the arguments the call contract takes as values.
fn value_arguments(
    descriptor: FunctionDescriptor,
    args: &[Expr],
) -> impl Iterator<Item = usize> + '_ {
    let layout = descriptor.call_contract().layout();
    (0..args.len()).filter(move |index| {
        layout.mode_at(*index, args.len()) == Some(ArgumentMode::Scalar)
            && !matches!(args[*index], Expr::Missing)
    })
}

/// Whether an argument the call contract takes as a value certainly holds an array value, which a
/// function that is not called element by element would read as its first element. Such a call is
/// unsupported until its array behavior is established; references and names keep the implicit
/// intersection they always had.
pub(super) fn value_argument_holds_array(descriptor: FunctionDescriptor, args: &[Expr]) -> bool {
    value_arguments(descriptor, args).any(|index| holds_array_value(&args[index]))
}

/// Whether `expr` certainly evaluates to an array value: an array constant, a function returning
/// an array, or an operator or element-wise call over one.
fn holds_array_value(expr: &Expr) -> bool {
    match expr {
        Expr::Array(_) => true,
        Expr::Paren(inner) | Expr::Unary { operand: inner, .. } => holds_array_value(inner),
        Expr::Binary { left, right, .. } => holds_array_value(left) || holds_array_value(right),
        Expr::Call { name, args } => {
            descriptor::resolve(name).is_some_and(|descriptor| match descriptor.array_evaluator() {
                Some(evaluator) if evaluator.reads_value_arrays() => {
                    args.iter().any(holds_array_value)
                }
                _ => match descriptor.result_kind() {
                    FunctionResultKind::Array => true,
                    FunctionResultKind::Scalar => value_argument_holds_array(descriptor, args),
                    _ => false,
                },
            })
        }
        _ => false,
    }
}

/// A name no formula can bind, through which an element reaches the kernel.
fn binding_name(index: usize) -> String {
    format!("\u{1}{index}")
}

/// Whether `expr` may evaluate to more than one value: literals, a single cell, and calls that
/// return one value from values that cannot hold arrays never do.
fn may_hold_array(engine: &Engine<'_>, context: EvalContext<'_>, expr: &Expr) -> bool {
    match expr {
        Expr::Number(_)
        | Expr::Text(_)
        | Expr::Logical(_)
        | Expr::ErrorLit(_)
        | Expr::Missing
        | Expr::ImplicitIntersection(_) => false,
        Expr::Paren(inner) | Expr::Unary { operand: inner, .. } => {
            may_hold_array(engine, context, inner)
        }
        Expr::Binary { left, right, .. } => {
            may_hold_array(engine, context, left) || may_hold_array(engine, context, right)
        }
        Expr::Ref(reference) => {
            !matches!(reference.body, RefBody::Cell(_))
                || reference
                    .sheet
                    .as_ref()
                    .is_some_and(|sheet| sheet.end_name.is_some())
        }
        Expr::Name(name) => match context.binding(name) {
            Some(ScopeValue::Scalar(_) | ScopeValue::Missing | ScopeValue::Callable(_)) => false,
            Some(ScopeValue::Array(_) | ScopeValue::Reference(_)) | None => true,
        },
        Expr::Call { name, args } => {
            if context.binding(name).is_some()
                || engine
                    .resolve_name_expr_with_id_in_context(context, name)
                    .is_some()
            {
                return true;
            }
            let Some(descriptor) = descriptor::resolve(name) else {
                return false;
            };
            match descriptor.array_evaluator() {
                Some(evaluator) if evaluator.reads_value_arrays() => {
                    return args.iter().any(|arg| !is_plain_scalar(arg));
                }
                Some(_) => return true,
                None if descriptor.result_kind() != FunctionResultKind::Scalar => return true,
                None => {}
            }
            value_arguments(descriptor, args)
                .any(|index| may_hold_array(engine, context, &args[index]))
        }
        _ => true,
    }
}

/// Whether `expr` is a literal or a single cell.
fn is_plain_scalar(expr: &Expr) -> bool {
    match expr {
        Expr::Number(_) | Expr::Text(_) | Expr::Logical(_) | Expr::ErrorLit(_) | Expr::Missing => {
            true
        }
        Expr::Paren(inner) => is_plain_scalar(inner),
        Expr::Ref(reference) => {
            matches!(reference.body, RefBody::Cell(_))
                && reference
                    .sheet
                    .as_ref()
                    .is_none_or(|sheet| sheet.end_name.is_none())
        }
        _ => false,
    }
}
