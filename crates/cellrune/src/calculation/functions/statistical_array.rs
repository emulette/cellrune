//! Statistics that return a vertical array: FREQUENCY and MODE.MULT.

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::array_common::poll_cancellation;
use super::statistical::modes;

/// FREQUENCY(data_array, bins_array) counts the numbers of `data_array` at or below the lowest
/// bin, above each bin through the next higher one, and above the highest bin. The result is a
/// vertical array with a row for each numeric bin in the order the bins are given, then the count
/// above the highest bin.
///
/// Microsoft documents that text and blanks are ignored. Excel 16.114 for Mac also leaves text and
/// blank bins out of the result, keeps the given order for bins out of ascending order, ignores a
/// logical value in a data array constant, and returns an error in the data as that one error.
/// Logical values elsewhere, errors in the bins or of more than one kind in the data, and equal
/// bins out of ascending order, where the count's row is unknown, are unsupported rather than
/// guessed.
pub(super) fn frequency(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    let [data_argument, bins] = args else {
        return Err(ErrorKind::Value);
    };
    let data = engine.eval_array(context, data_argument)?;
    let bins = engine.eval_array(context, bins)?;
    let edges = frequency_bins(&bins.data)?;
    let mut numbers = frequency_data(&data.data, matches!(data_argument, Expr::Array(_)))?;
    let mut order = (0..edges.len()).collect::<Vec<_>>();
    if edges.windows(2).any(|pair| pair[0] > pair[1]) {
        order.sort_by(|&left, &right| edges[left].total_cmp(&edges[right]));
        if order.windows(2).any(|pair| edges[pair[0]] == edges[pair[1]]) {
            return Err(ErrorKind::Unsupported);
        }
    }
    let rows = u32::try_from(edges.len() + 1).map_err(|_| ErrorKind::Num)?;
    engine.ensure_array_cells(u64::from(rows))?;
    engine.charge_function_iterations(
        context,
        u64::try_from(numbers.len() + edges.len()).map_err(|_| ErrorKind::Num)?,
    )?;
    numbers.sort_by(f64::total_cmp);
    let mut data = vec![Value::Number(0.0); edges.len() + 1];
    let mut counted = 0;
    for index in order {
        poll_cancellation(context)?;
        let through = numbers.partition_point(|number| *number <= edges[index]);
        data[index] = Value::Number((through - counted) as f64);
        counted = through;
    }
    data[edges.len()] = Value::Number((numbers.len() - counted) as f64);
    Ok(Array {
        rows,
        cols: 1,
        data,
    })
}

/// The numeric bins in the given order; text and blanks are left out.
fn frequency_bins(values: &[Value]) -> Result<Vec<f64>, ErrorKind> {
    let mut edges = Vec::new();
    for value in values {
        match value {
            Value::Number(number) => edges.push(*number),
            Value::Text(_) | Value::Blank => {}
            Value::Error(kind) if kind.is_engine_issue() => return Err(*kind),
            Value::Logical(_) | Value::Error(_) => return Err(ErrorKind::Unsupported),
        }
    }
    Ok(edges)
}

/// The numbers of the data; text and blanks, and logical values of an array constant, are left
/// out, and an error is the result.
fn frequency_data(values: &[Value], array_constant: bool) -> Result<Vec<f64>, ErrorKind> {
    let mut numbers = Vec::new();
    let mut error = None;
    for value in values {
        match value {
            Value::Number(number) => numbers.push(*number),
            Value::Text(_) | Value::Blank => {}
            Value::Logical(_) if array_constant => {}
            Value::Logical(_) => return Err(ErrorKind::Unsupported),
            Value::Error(kind) if kind.is_engine_issue() => return Err(*kind),
            Value::Error(kind) => match error {
                None => error = Some(*kind),
                Some(first) if first == *kind => {}
                Some(_) => return Err(ErrorKind::Unsupported),
            },
        }
    }
    match error {
        Some(kind) => Err(kind),
        None => Ok(numbers),
    }
}

/// MODE.MULT returns every most frequent number as a vertical array, in the order each first
/// occurs, so its first element is the MODE.SNGL result.
pub(super) fn mode_multiple(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    let modes = modes(engine, context, args)?;
    let rows = u32::try_from(modes.len()).map_err(|_| ErrorKind::Num)?;
    engine.ensure_array_cells(u64::from(rows))?;
    Ok(Array {
        rows,
        cols: 1,
        data: modes.into_iter().map(Value::Number).collect(),
    })
}
