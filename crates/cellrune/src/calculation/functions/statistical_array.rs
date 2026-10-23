//! Statistics that return a vertical array: FREQUENCY and MODE.MULT.

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::array_common::poll_cancellation;
use super::statistical::modes;

/// FREQUENCY(data_array, bins_array) counts the numbers of `data_array` at or below the first
/// bin, above each bin through the next, and above the last bin, as a vertical array one longer
/// than the bins. Text and blanks are ignored in both arguments, and bins without any number
/// count every number once.
///
/// Microsoft documents neither logical values nor errors in either argument, nor bins that are
/// out of ascending order or mix numbers with text or blanks, so those inputs are unsupported
/// rather than guessed.
pub(super) fn frequency(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    let [data, bins] = args else {
        return Err(ErrorKind::Value);
    };
    let data = engine.eval_array(context, data)?;
    let bins = engine.eval_array(context, bins)?;
    let mut numbers = frequency_numbers(&data.data)?;
    let edges = frequency_numbers(&bins.data)?;
    if !edges.is_empty() && edges.len() != bins.data.len() {
        return Err(ErrorKind::Unsupported);
    }
    if edges.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(ErrorKind::Unsupported);
    }
    let rows = u32::try_from(edges.len() + 1).map_err(|_| ErrorKind::Num)?;
    engine.ensure_array_cells(u64::from(rows))?;
    engine.charge_function_iterations(
        context,
        u64::try_from(numbers.len() + edges.len()).map_err(|_| ErrorKind::Num)?,
    )?;
    numbers.sort_by(f64::total_cmp);
    let mut data = Vec::with_capacity(edges.len() + 1);
    let mut counted = 0;
    for edge in &edges {
        poll_cancellation(context)?;
        let through = numbers.partition_point(|number| number <= edge);
        data.push(Value::Number((through - counted) as f64));
        counted = through;
    }
    data.push(Value::Number((numbers.len() - counted) as f64));
    Ok(Array {
        rows,
        cols: 1,
        data,
    })
}

fn frequency_numbers(values: &[Value]) -> Result<Vec<f64>, ErrorKind> {
    let mut numbers = Vec::new();
    for value in values {
        match value {
            Value::Number(number) => numbers.push(*number),
            Value::Text(_) | Value::Blank => {}
            Value::Error(kind) if kind.is_engine_issue() => return Err(*kind),
            Value::Logical(_) | Value::Error(_) => return Err(ErrorKind::Unsupported),
        }
    }
    Ok(numbers)
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
