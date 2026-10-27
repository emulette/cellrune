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
use super::kernel::{ArrayEvaluator, LegacyArrayFunction};
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
    if let Some(kind) = direct_sheet_span_error(engine, context, descriptor, args)
        .or_else(|| range_argument_error(engine, context, descriptor, args))
    {
        return Some(Ok(ArrayEvaluation::scalar(ScalarEvaluation::new(
            Value::Error(kind),
        ))));
    }
    // The other arguments are read once for every element, as Excel reads them: a random value
    // is the same in each, and a computed argument is not recomputed.
    let layout = descriptor.call_contract().layout();
    let once = read_once(
        engine,
        context,
        args,
        &lifted,
        (0..args.len()).filter_map(|index| match layout.mode_at(index, args.len()) {
            Some(ArgumentMode::Scalar) => Some((index, Once::Value)),
            Some(ArgumentMode::Array) => Some((index, Once::Array)),
            Some(ArgumentMode::Reference) => Some((index, Once::Reference)),
            _ => None,
        }),
    );
    let lifting = Lifting {
        lifted: &lifted,
        once: &once,
        only_several: false,
    };
    Some(
        lift(
            engine,
            context,
            args,
            written,
            &lifting,
            |context, args, written| {
                evaluate_prepared(descriptor, engine, context, args, written).value
            },
        )
        .map(|array| {
            ArrayEvaluation::new(array.expect("lifting without `only_several` always answers"))
        }),
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

/// Which of `reads` an element-wise call reads once: the arguments that are not lifted and that
/// evaluating per element would compute again, a value that is not a literal or a cell, or an
/// array or reference that is not written as one.
pub(super) fn read_once(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    lifted: &[usize],
    reads: impl Iterator<Item = (usize, Once)>,
) -> Vec<(usize, Once)> {
    reads
        .filter(|(index, once)| {
            args.get(*index).is_some_and(|arg| {
                !lifted.contains(index)
                    && match once {
                        Once::Value => !is_plain_scalar(arg),
                        Once::Array | Once::Reference => {
                            !is_reference(arg) && may_hold_array(engine, context, arg)
                        }
                    }
            })
        })
        .collect()
}

/// How an argument read once reaches every element's call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Once {
    /// Its value, evaluated as the function reads a value.
    Value,
    /// Its array, evaluated as the function reads an array.
    Array,
    /// The reference it returns, or its array when it returns none, as a function reads an
    /// argument that takes a reference (`COUNTIF(Data,{"a","b"})` over a defined name).
    Reference,
}

/// Which arguments an element-wise call reads per element and which once.
pub(super) struct Lifting<'positions> {
    /// The arguments read element by element.
    pub(super) lifted: &'positions [usize],
    /// The arguments evaluated once and bound for every element.
    pub(super) once: &'positions [(usize, Once)],
    /// Whether a call whose lifted arguments hold one value each answers `None`, for a function
    /// that reads its arguments differently when none holds an array.
    pub(super) only_several: bool,
}

/// Evaluates the lifted arguments as arrays and calls `call` once per element of their broadcast
/// shape, with those arguments bound to the element's values, the arguments read once bound to
/// their values, and the others as written. `written` is substituted alike where it has the
/// position. A reference the call reads is charged to the formula's work once, not per element.
pub(super) fn lift(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    written: &[Expr],
    lifting: &Lifting<'_>,
    mut call: impl FnMut(EvalContext<'_>, &[Expr], &[Expr]) -> Value,
) -> Result<Option<Array>, ErrorKind> {
    let mut arrays = Vec::with_capacity(lifting.lifted.len());
    for index in lifting.lifted {
        arrays.push(match engine.eval_array(context, &args[*index]) {
            Ok(array) => array,
            Err(kind) if kind.is_engine_issue() => return Err(kind),
            // The argument's own error is the value the kernel reads, as in a scalar call.
            Err(kind) => Array::scalar(Value::Error(kind)),
        });
    }
    if lifting.only_several && arrays.iter().all(Array::is_scalar) {
        return Ok(None);
    }
    let (rows, cols) = broadcast_all(arrays.iter())?;
    let cells = u64::from(rows) * u64::from(cols);
    let several = cells > 1;
    if several {
        engine.ensure_array_cells(cells)?;
        engine.charge_function_iterations(context, cells)?;
    }
    let mut fixed = Vec::with_capacity(lifting.once.len());
    for (index, once) in lifting.once {
        let value = match once {
            Once::Value => {
                ScopeValue::Scalar(engine.eval_scalar_evaluation(context, &args[*index]))
            }
            Once::Array => match engine.eval_array_evaluation(context, &args[*index]) {
                Ok(evaluated) => ScopeValue::Array(std::sync::Arc::new(evaluated)),
                Err(kind) if kind.is_engine_issue() => return Err(kind),
                Err(kind) => ScopeValue::Scalar(ScalarEvaluation::new(Value::Error(kind))),
            },
            Once::Reference => {
                let value = engine.eval_scope_value(context, &args[*index]);
                if let Some(kind) = value.engine_issue() {
                    return Err(kind);
                }
                value
            }
        };
        fixed.push((*index, value));
    }
    let positions = lifting
        .lifted
        .iter()
        .chain(fixed.iter().map(|(index, _)| index))
        .copied()
        .collect::<Vec<_>>();
    let names = (0..positions.len()).map(binding_name).collect::<Vec<_>>();
    let substitute = |expressions: &[Expr]| {
        let mut substituted = expressions.to_vec();
        for (name, index) in names.iter().zip(&positions) {
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
        names[..lifting.lifted.len()]
            .iter()
            .map(|name| ScopeEntry::new(name.clone(), ScopeValue::Missing)),
    );
    bindings.extend(
        names[lifting.lifted.len()..]
            .iter()
            .zip(fixed)
            .map(|(name, (_, value))| ScopeEntry::new(name.clone(), value)),
    );
    let lifted_end = outer + lifting.lifted.len();
    let mut data = Vec::with_capacity(cells as usize);
    for row in 0..rows {
        for col in 0..cols {
            for (entry, array) in bindings[outer..lifted_end].iter_mut().zip(&arrays) {
                entry.set_value(ScopeValue::Scalar(ScalarEvaluation::new(
                    element_at(array, row, col).clone(),
                )));
            }
            let index = u64::from(row) * u64::from(cols) + u64::from(col);
            let mut element_context = context.with_bindings(&bindings);
            if several {
                element_context = element_context.at_iteration(index);
            }
            if index > 0 {
                element_context = element_context.without_reference_work_charge();
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
    Ok(Some(Array { rows, cols, data }))
}

/// The shape the arrays broadcast to together.
fn broadcast_all<'array>(
    mut arrays: impl Iterator<Item = &'array Array>,
) -> Result<(u32, u32), ErrorKind> {
    arrays.try_fold((1, 1), |(rows, cols), array| {
        broadcast_shape(
            &Array {
                rows,
                cols,
                data: Vec::new(),
            },
            array,
        )
    })
}

/// What an element of a selection takes: the argument at a position, or a value of its own.
pub(super) enum Pick {
    Argument(usize),
    Value(Value),
}

/// Evaluates a selection element by element, as `CHOOSE`, `IFS`, and `SWITCH` do over arrays:
/// `pick` reads one element of each selector array and names the argument that element takes. The
/// arguments some element takes are evaluated once as arrays and broadcast with the selectors, so
/// `CHOOSE({1,2},B1:B3,A1:A3)` is three rows by two columns (Mac Excel 16.114).
pub(super) fn select(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    selectors: &[Array],
    mut pick: impl FnMut(&[&Value]) -> Pick,
) -> Result<Array, ErrorKind> {
    let (rows, cols) = broadcast_all(selectors.iter())?;
    // The selectors' shape bounds the work before any element is read, as in `lift`.
    let cells = u64::from(rows) * u64::from(cols);
    if cells > 1 {
        engine.ensure_array_cells(cells)?;
        engine.charge_function_iterations(context, cells)?;
    }
    let picks_at = |row: u32, col: u32, pick: &mut dyn FnMut(&[&Value]) -> Pick| {
        let elements = selectors
            .iter()
            .map(|selector| element_at(selector, row, col))
            .collect::<Vec<_>>();
        pick(&elements)
    };
    let mut taken = std::collections::BTreeMap::new();
    for row in 0..rows {
        for col in 0..cols {
            if let Pick::Argument(position) = picks_at(row, col, &mut pick)
                && !taken.contains_key(&position)
            {
                let array = match engine.eval_array(context, &args[position]) {
                    Ok(array) => array,
                    Err(kind) if kind.is_engine_issue() => return Err(kind),
                    Err(kind) => Array::scalar(Value::Error(kind)),
                };
                taken.insert(position, array);
            }
        }
    }
    let (rows, cols) = broadcast_all(selectors.iter().chain(taken.values()))?;
    let cells = u64::from(rows) * u64::from(cols);
    engine.ensure_array_cells(cells)?;
    let mut data = Vec::with_capacity(cells as usize);
    for row in 0..rows {
        for col in 0..cols {
            data.push(match picks_at(row, col, &mut pick) {
                Pick::Argument(position) => taken
                    .get(&position)
                    .map_or(Value::Error(ErrorKind::NA), |array| {
                        element_at(array, row, col).clone()
                    }),
                Pick::Value(value) => value,
            });
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
pub(super) fn value_argument_holds_array(
    context: EvalContext<'_>,
    descriptor: FunctionDescriptor,
    args: &[Expr],
) -> bool {
    value_arguments(descriptor, args).any(|index| holds_array_value(context, &args[index]))
}

/// `#VALUE!` where a function that rejects ranges receives a range of several cells in a value
/// argument; see `FunctionDescriptor::rejects_ranges`.
pub(super) fn range_argument_error(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    descriptor: FunctionDescriptor,
    args: &[Expr],
) -> Option<ErrorKind> {
    if !descriptor.rejects_ranges() {
        return None;
    }
    value_arguments(descriptor, args)
        .filter(|&index| may_hold_array(engine, context, &args[index]))
        .any(|index| {
            engine
                .resolve_reference_value_expr(context, &args[index])
                .is_ok_and(|reference| reference.cell_count() > 1)
        })
        .then_some(ErrorKind::Value)
}

/// Whether `expr` certainly evaluates to an array value: an array constant, a function returning
/// an array, a name bound to an array, or an operator or element-wise call over one.
pub(super) fn holds_array_value(context: EvalContext<'_>, expr: &Expr) -> bool {
    crate::calculation::stack::grow(|| match expr {
        Expr::Array(_) => true,
        Expr::Paren(inner) | Expr::Unary { operand: inner, .. } => {
            holds_array_value(context, inner)
        }
        Expr::Binary { left, right, .. } => {
            holds_array_value(context, left) || holds_array_value(context, right)
        }
        Expr::Name(name) => matches!(context.binding(name), Some(ScopeValue::Array(_))),
        Expr::Call { name, args } => {
            descriptor::resolve(name).is_some_and(|descriptor| match descriptor.array_evaluator() {
                Some(evaluator) if evaluator.reads_value_arrays() => {
                    returns_array_value(context, evaluator, args)
                }
                _ => match descriptor.result_kind() {
                    FunctionResultKind::Array => true,
                    FunctionResultKind::Scalar => {
                        value_argument_holds_array(context, descriptor, args)
                    }
                    _ => false,
                },
            })
        }
        _ => false,
    })
}

/// Whether a call to `evaluator`, which reads arrays where the call contract takes values,
/// certainly returns an array value. `TYPE` answers one number, and an order statistic one value
/// for each `k` (`LARGE({3,1,2},1)` is one number).
fn returns_array_value(context: EvalContext<'_>, evaluator: ArrayEvaluator, args: &[Expr]) -> bool {
    match evaluator {
        ArrayEvaluator::Type => false,
        ArrayEvaluator::OrderStatistic(_) => {
            args.get(1).is_some_and(|k| holds_array_value(context, k))
        }
        ArrayEvaluator::Legacy(LegacyArrayFunction::Index) => {
            index_returns_array_value(context, args)
        }
        _ => args.iter().any(|arg| holds_array_value(context, arg)),
    }
}

/// Whether `INDEX` certainly returns an array value: an index holds an array, or it takes a whole
/// row or column of an array source, through an index of 0 or an omitted one, or through a single
/// index into a source that is not one row or one column (`INDEX({10,20,30},2)` is one number).
fn index_returns_array_value(context: EvalContext<'_>, args: &[Expr]) -> bool {
    let Some((source, indexes)) = args.split_first() else {
        return false;
    };
    if indexes
        .iter()
        .any(|index| holds_array_value(context, index))
    {
        return true;
    }
    if !holds_array_value(context, source) {
        return false;
    }
    let whole = |index: &Expr| match index {
        Expr::Missing => true,
        Expr::Number(number) => number.value() == 0.0,
        _ => false,
    };
    match indexes {
        [index] => whole(index) || !is_one_dimensional_constant(source),
        _ => indexes.iter().any(whole),
    }
}

/// Whether `expr` is an array constant of one row or one column.
fn is_one_dimensional_constant(expr: &Expr) -> bool {
    match expr {
        Expr::Paren(inner) => is_one_dimensional_constant(inner),
        Expr::Array(rows) => rows.len() == 1 || rows.iter().all(|row| row.len() == 1),
        _ => false,
    }
}

/// A name no formula can bind, through which an element reaches the kernel.
fn binding_name(index: usize) -> String {
    format!("\u{1}{index}")
}

/// Whether `expr` may evaluate to more than one value: literals, a single cell, and calls that
/// return one value from values that cannot hold arrays never do.
pub(super) fn may_hold_array(engine: &Engine<'_>, context: EvalContext<'_>, expr: &Expr) -> bool {
    crate::calculation::stack::grow(|| match expr {
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
    })
}

/// Whether `expr` is written as a reference.
fn is_reference(expr: &Expr) -> bool {
    match expr {
        Expr::Paren(inner) => is_reference(inner),
        Expr::Ref(_)
        | Expr::Range { .. }
        | Expr::StructuredRef(_)
        | Expr::SpillRef(_)
        | Expr::ReferenceUnion { .. }
        | Expr::ReferenceIntersection { .. } => true,
        _ => false,
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
