use super::super::ast::Expr;
use super::super::coerce::{to_logical, to_number, to_text};
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::array_common::{poll_cancellation, validate_array_input};
use super::util::required_text;

struct Delimiter {
    text: String,
    folded: String,
    characters: usize,
}

struct DelimiterSearch<'a> {
    text: &'a str,
    boundaries: &'a [usize],
    delimiters: &'a [Delimiter],
    comparison_work: u64,
    case_insensitive: bool,
}

pub(super) fn call_scalar(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    match call_array(engine, context, args) {
        Ok(array) => array
            .data
            .into_iter()
            .next()
            .unwrap_or(Value::Error(ErrorKind::Calc)),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn call_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if args.len() < 2 || args.len() > 6 {
        return Err(ErrorKind::Value);
    }
    let text = required_text(engine, context, &args[0])?;
    engine.ensure_text_bytes(text.len())?;
    engine.charge_function_iterations(context, text.len() as u64)?;

    let column_delimiters = delimiters(engine, context, args.get(1))?;
    let row_delimiters = delimiters(engine, context, args.get(2))?;
    if column_delimiters.is_empty() && row_delimiters.is_empty() {
        return Err(ErrorKind::Value);
    }
    let ignore_empty = args.get(3).map_or(Ok(false), |expression| {
        if matches!(expression, Expr::Missing) {
            Ok(false)
        } else {
            to_logical(&engine.eval_scalar(context, expression))
        }
    })?;
    let case_insensitive = args.get(4).map_or(Ok(false), |expression| {
        if matches!(expression, Expr::Missing) {
            Ok(false)
        } else {
            match to_number(&engine.eval_scalar(context, expression))?.trunc() {
                0.0 => Ok(false),
                1.0 => Ok(true),
                _ => Err(ErrorKind::Value),
            }
        }
    })?;
    let padding = match args.get(5) {
        None | Some(Expr::Missing) => Value::Error(ErrorKind::NA),
        Some(expression) => engine.eval_scalar(context, expression),
    };
    if let Value::Error(kind) = padding
        && kind.is_engine_issue()
    {
        return Err(kind);
    }
    if let Value::Text(text) = &padding {
        engine.ensure_text_bytes(text.len())?;
    }

    let rows = split(
        engine,
        context,
        &text,
        &row_delimiters,
        ignore_empty,
        case_insensitive,
    )?;
    let mut split_rows = Vec::with_capacity(rows.len());
    let mut columns = 0_usize;
    for row in rows {
        poll_cancellation(context)?;
        let cells = split(
            engine,
            context,
            &row,
            &column_delimiters,
            ignore_empty,
            case_insensitive,
        )?;
        columns = columns.max(cells.len());
        split_rows.push(cells);
    }
    if split_rows.is_empty() || columns == 0 {
        return Err(ErrorKind::Calc);
    }
    let row_count = u32::try_from(split_rows.len()).map_err(|_| ErrorKind::Num)?;
    let column_count = u32::try_from(columns).map_err(|_| ErrorKind::Num)?;
    let cells = u64::from(row_count)
        .checked_mul(u64::from(column_count))
        .ok_or(ErrorKind::Num)?;
    engine.ensure_array_cells(cells)?;
    engine.charge_function_iterations(context, cells)?;
    let populated_cells = split_rows.iter().try_fold(0_u64, |count, row| {
        count
            .checked_add(u64::try_from(row.len()).map_err(|_| ErrorKind::Num)?)
            .ok_or(ErrorKind::Num)
    })?;
    let padding_cells = cells.checked_sub(populated_cells).ok_or(ErrorKind::Num)?;
    if let Value::Text(text) = &padding {
        let padding_bytes = padding_cells
            .checked_mul(u64::try_from(text.len()).map_err(|_| ErrorKind::Num)?)
            .ok_or(ErrorKind::Num)?;
        engine.charge_function_iterations(context, padding_bytes)?;
    }

    let mut data = Vec::with_capacity(usize::try_from(cells).map_err(|_| ErrorKind::Num)?);
    for row in split_rows {
        let row_length = row.len();
        data.extend(row.into_iter().map(Value::Text));
        data.resize(data.len() + columns - row_length, padding.clone());
    }
    Ok(Array {
        rows: row_count,
        cols: column_count,
        data,
    })
}

fn delimiters(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expression: Option<&Expr>,
) -> Result<Vec<Delimiter>, ErrorKind> {
    let Some(expression) = expression else {
        return Ok(Vec::new());
    };
    if matches!(expression, Expr::Missing) {
        return Ok(Vec::new());
    }
    let array = engine.eval_array(context, expression)?;
    validate_array_input(engine, context, &array)?;
    array
        .data
        .into_iter()
        .map(|value| {
            let text = to_text(&value)?;
            if text.is_empty() {
                return Err(ErrorKind::Value);
            }
            engine.ensure_text_bytes(text.len())?;
            let preprocessing_work = u64::try_from(text.len())
                .map_err(|_| ErrorKind::Num)?
                .checked_mul(2)
                .ok_or(ErrorKind::Num)?;
            engine.charge_function_iterations(context, preprocessing_work)?;
            poll_cancellation(context)?;
            Ok(Delimiter {
                folded: text.to_lowercase(),
                characters: text.chars().count(),
                text,
            })
        })
        .collect()
}

fn split(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    text: &str,
    delimiters: &[Delimiter],
    ignore_empty: bool,
    case_insensitive: bool,
) -> Result<Vec<String>, ErrorKind> {
    if delimiters.is_empty() {
        return Ok(vec![text.to_owned()]);
    }
    let boundaries: Vec<usize> = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect();
    let comparison_work = delimiters.iter().try_fold(0_u64, |work, delimiter| {
        work.checked_add(delimiter.characters.max(1) as u64)
            .ok_or(ErrorKind::Num)
    })?;
    let mut output = Vec::new();
    let mut cursor = 0;
    let mut boundary_index = 0;
    let search = DelimiterSearch {
        text,
        boundaries: &boundaries,
        delimiters,
        comparison_work,
        case_insensitive,
    };
    while let Some((start, end, next_boundary)) =
        next_match(engine, context, &search, boundary_index)?
    {
        poll_cancellation(context)?;
        let piece = text.get(cursor..start).ok_or(ErrorKind::Value)?;
        if !ignore_empty || !piece.is_empty() {
            output.push(piece.to_owned());
        }
        cursor = end;
        boundary_index = next_boundary;
    }
    let tail = text.get(cursor..).ok_or(ErrorKind::Value)?;
    if !ignore_empty || !tail.is_empty() {
        output.push(tail.to_owned());
    }
    Ok(output)
}

fn next_match(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    search: &DelimiterSearch<'_>,
    boundary_index: usize,
) -> Result<Option<(usize, usize, usize)>, ErrorKind> {
    for index in boundary_index..search.boundaries.len().saturating_sub(1) {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, search.comparison_work)?;
        let start = search.boundaries[index];
        let mut matched_end = None;
        for delimiter in search.delimiters {
            let Some(end_index) = index.checked_add(delimiter.characters) else {
                continue;
            };
            let Some(&end) = search.boundaries.get(end_index) else {
                continue;
            };
            let candidate = search.text.get(start..end).ok_or(ErrorKind::Value)?;
            let matched = if search.case_insensitive {
                candidate.to_lowercase() == delimiter.folded
            } else {
                candidate == delimiter.text
            };
            if matched && matched_end.is_none_or(|current| end > current) {
                matched_end = Some(end);
            }
        }
        if let Some(end) = matched_end {
            let next_boundary = search
                .boundaries
                .binary_search(&end)
                .map_err(|_| ErrorKind::Value)?;
            return Ok(Some((start, end, next_boundary)));
        }
    }
    Ok(None)
}
