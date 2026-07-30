use std::cmp::Ordering;

use super::super::ast::Expr;
use super::super::coerce::{to_logical, to_number};
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    match name {
        "CHOOSECOLS" => choose(engine, context, args, Axis::Columns),
        "CHOOSEROWS" => choose(engine, context, args, Axis::Rows),
        "DROP" => take_or_drop(engine, context, args, SliceOperation::Drop),
        "FILTER" => filter(engine, context, args),
        "HSTACK" => stack(engine, context, args, Axis::Columns),
        "SORT" => sort(engine, context, args),
        "TAKE" => take_or_drop(engine, context, args, SliceOperation::Take),
        "UNIQUE" => unique(engine, context, args),
        "VSTACK" => stack(engine, context, args, Axis::Rows),
        _ => Err(ErrorKind::Unsupported),
    }
}

#[derive(Debug, Clone, Copy)]
enum Axis {
    Rows,
    Columns,
}

#[derive(Debug, Clone, Copy)]
enum SliceOperation {
    Take,
    Drop,
}

fn choose(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    axis: Axis,
) -> Result<Array, ErrorKind> {
    if args.len() < 2 {
        return Err(ErrorKind::Value);
    }
    let source = engine.eval_array(context, &args[0])?;
    let dimension = match axis {
        Axis::Rows => source.rows,
        Axis::Columns => source.cols,
    };
    let mut indexes = Vec::new();
    for argument in &args[1..] {
        let values = engine.eval_array(context, argument)?;
        for value in values.data {
            let index = resolve_index(to_number(&value)?.trunc(), dimension)?;
            let index_count = u32::try_from(indexes.len() + 1).map_err(|_| ErrorKind::Num)?;
            let (rows, cols) = match axis {
                Axis::Rows => (index_count, source.cols),
                Axis::Columns => (source.rows, index_count),
            };
            let cells = cell_count(rows, cols)?;
            engine.ensure_array_cells(cells)?;
            engine.ensure_function_iterations(cells)?;
            indexes.push(index);
        }
    }
    if indexes.is_empty() {
        return Err(ErrorKind::Value);
    }
    let (rows, cols) = match axis {
        Axis::Rows => (indexes.len() as u32, source.cols),
        Axis::Columns => (source.rows, indexes.len() as u32),
    };
    let cells = cell_count(rows, cols)?;
    engine.ensure_array_cells(cells)?;
    engine.charge_function_iterations(context, cells)?;
    let mut data = Vec::with_capacity(cells as usize);
    match axis {
        Axis::Rows => {
            for row in indexes {
                for column in 0..source.cols {
                    data.push(source.at(row, column).clone());
                }
            }
        }
        Axis::Columns => {
            for row in 0..source.rows {
                for column in &indexes {
                    data.push(source.at(row, *column).clone());
                }
            }
        }
    }
    Ok(Array { rows, cols, data })
}

fn resolve_index(index: f64, dimension: u32) -> Result<u32, ErrorKind> {
    if index == 0.0 || index.abs() > f64::from(dimension) {
        return Err(ErrorKind::Value);
    }
    if index.is_sign_positive() {
        Ok(index as u32 - 1)
    } else {
        Ok((i64::from(dimension) + index as i64) as u32)
    }
}

fn take_or_drop(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: SliceOperation,
) -> Result<Array, ErrorKind> {
    if args.len() < 2 || args.len() > 3 {
        return Err(ErrorKind::Value);
    }
    let source = engine.eval_array(context, &args[0])?;
    let (row_start, rows) = slice_axis(engine, context, Some(&args[1]), source.rows, operation)?;
    let (column_start, cols) = slice_axis(engine, context, args.get(2), source.cols, operation)?;
    let cells = cell_count(rows, cols)?;
    engine.ensure_array_cells(cells)?;
    engine.charge_function_iterations(context, cells)?;
    let mut data = Vec::with_capacity(cells as usize);
    for row in row_start..row_start + rows {
        for column in column_start..column_start + cols {
            data.push(source.at(row, column).clone());
        }
    }
    Ok(Array { rows, cols, data })
}

fn slice_axis(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
    dimension: u32,
    operation: SliceOperation,
) -> Result<(u32, u32), ErrorKind> {
    match expr {
        Some(Expr::Missing) | None => Ok((0, dimension)),
        Some(expr) => slice_bounds(
            dimension,
            array_count(engine, context, expr, dimension)?,
            operation,
        ),
    }
}

fn array_count(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
    maximum: u32,
) -> Result<i64, ErrorKind> {
    let value = to_number(&engine.eval_scalar(context, expr))?.trunc();
    if !value.is_finite() {
        return Err(ErrorKind::Num);
    }
    if value.abs() > f64::from(maximum) {
        return Err(ErrorKind::Num);
    }
    Ok(value as i64)
}

fn slice_bounds(
    dimension: u32,
    count: i64,
    operation: SliceOperation,
) -> Result<(u32, u32), ErrorKind> {
    if count == 0 {
        return Err(ErrorKind::Calc);
    }
    let amount = count.unsigned_abs() as u32;
    match operation {
        SliceOperation::Take if count.is_positive() => Ok((0, amount)),
        SliceOperation::Take => Ok((dimension - amount, amount)),
        SliceOperation::Drop if amount >= dimension => Err(ErrorKind::Calc),
        SliceOperation::Drop if count.is_positive() => Ok((amount, dimension - amount)),
        SliceOperation::Drop => Ok((0, dimension - amount)),
    }
}

fn filter(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if args.len() < 2 || args.len() > 3 {
        return Err(ErrorKind::Value);
    }
    let source = engine.eval_array(context, &args[0])?;
    let include = engine.eval_array(context, &args[1])?;
    let axis = if include.rows == source.rows && include.cols == 1 {
        Axis::Rows
    } else if include.rows == 1 && include.cols == source.cols {
        Axis::Columns
    } else {
        return Err(ErrorKind::Value);
    };
    let inspected_cells = cell_count(source.rows, source.cols)?
        .checked_add(cell_count(include.rows, include.cols)?)
        .ok_or(ErrorKind::Num)?;
    engine.charge_function_iterations(context, inspected_cells)?;
    let selected = include
        .data
        .iter()
        .map(to_logical)
        .collect::<Result<Vec<_>, _>>()?;
    let selected_count = selected.iter().filter(|value| **value).count() as u32;
    if selected_count == 0 {
        return match args.get(2) {
            Some(Expr::Missing) | None => Err(ErrorKind::Calc),
            Some(if_empty) => Ok(Array::scalar(engine.eval_scalar(context, if_empty))),
        };
    }
    let (rows, cols) = match axis {
        Axis::Rows => (selected_count, source.cols),
        Axis::Columns => (source.rows, selected_count),
    };
    let cells = cell_count(rows, cols)?;
    engine.ensure_array_cells(cells)?;
    engine.charge_function_iterations(context, cells)?;
    let mut data = Vec::with_capacity(cells as usize);
    match axis {
        Axis::Rows => {
            for row in 0..source.rows {
                if selected[row as usize] {
                    for column in 0..source.cols {
                        data.push(source.at(row, column).clone());
                    }
                }
            }
        }
        Axis::Columns => {
            for row in 0..source.rows {
                for column in 0..source.cols {
                    if selected[column as usize] {
                        data.push(source.at(row, column).clone());
                    }
                }
            }
        }
    }
    Ok(Array { rows, cols, data })
}

fn stack(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    axis: Axis,
) -> Result<Array, ErrorKind> {
    if args.is_empty() {
        return Err(ErrorKind::Value);
    }
    let mut arrays = Vec::with_capacity(args.len());
    let mut rows = 0_u32;
    let mut cols = 0_u32;
    let mut input_cells = 0_u64;
    for argument in args {
        let array = engine.eval_array(context, argument)?;
        input_cells = input_cells
            .checked_add(cell_count(array.rows, array.cols)?)
            .ok_or(ErrorKind::Num)?;
        engine.ensure_function_iterations(input_cells)?;
        match axis {
            Axis::Rows => {
                rows = rows.checked_add(array.rows).ok_or(ErrorKind::Num)?;
                cols = cols.max(array.cols);
            }
            Axis::Columns => {
                rows = rows.max(array.rows);
                cols = cols.checked_add(array.cols).ok_or(ErrorKind::Num)?;
            }
        }
        engine.ensure_array_cells(cell_count(rows, cols)?)?;
        arrays.push(array);
    }
    let cells = cell_count(rows, cols)?;
    engine.ensure_array_cells(cells)?;
    engine.charge_function_iterations(
        context,
        input_cells.checked_add(cells).ok_or(ErrorKind::Num)?,
    )?;
    let mut data = Vec::with_capacity(cells as usize);
    match axis {
        Axis::Rows => {
            for array in arrays {
                for row in 0..array.rows {
                    for column in 0..cols {
                        data.push(if column < array.cols {
                            array.at(row, column).clone()
                        } else {
                            Value::Error(ErrorKind::NA)
                        });
                    }
                }
            }
        }
        Axis::Columns => {
            for row in 0..rows {
                for array in &arrays {
                    for column in 0..array.cols {
                        data.push(if row < array.rows {
                            array.at(row, column).clone()
                        } else {
                            Value::Error(ErrorKind::NA)
                        });
                    }
                }
            }
        }
    }
    Ok(Array { rows, cols, data })
}

fn sort(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Result<Array, ErrorKind> {
    if args.is_empty() || args.len() > 4 {
        return Err(ErrorKind::Value);
    }
    let source = engine.eval_array(context, &args[0])?;
    let by_column = optional_logical(engine, context, args.get(3), false)?;
    let key_dimension = if by_column { source.rows } else { source.cols };
    let sort_index = optional_integer(engine, context, args.get(1), 1)?;
    if sort_index < 1 || sort_index > i64::from(key_dimension) {
        return Err(ErrorKind::Value);
    }
    let sort_order = optional_integer(engine, context, args.get(2), 1)?;
    if !matches!(sort_order, -1 | 1) {
        return Err(ErrorKind::Value);
    }
    let item_count = if by_column { source.cols } else { source.rows };
    engine.charge_function_iterations(context, u64::from(item_count).saturating_mul(64))?;
    let mut indexes = (0..item_count).collect::<Vec<_>>();
    indexes.sort_by(|left, right| {
        let (left_value, right_value) = if by_column {
            (
                source.at(sort_index as u32 - 1, *left),
                source.at(sort_index as u32 - 1, *right),
            )
        } else {
            (
                source.at(*left, sort_index as u32 - 1),
                source.at(*right, sort_index as u32 - 1),
            )
        };
        let ordering = compare_values(left_value, right_value);
        if sort_order == 1 {
            ordering
        } else {
            ordering.reverse()
        }
    });
    let mut data = Vec::with_capacity(source.data.len());
    if by_column {
        for row in 0..source.rows {
            for column in &indexes {
                data.push(source.at(row, *column).clone());
            }
        }
    } else {
        for row in indexes {
            for column in 0..source.cols {
                data.push(source.at(row, column).clone());
            }
        }
    }
    Ok(Array {
        rows: source.rows,
        cols: source.cols,
        data,
    })
}

fn compare_values(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Blank, Value::Blank) => Ordering::Equal,
        (Value::Blank, _) => Ordering::Less,
        (_, Value::Blank) => Ordering::Greater,
        (Value::Number(left), Value::Number(right)) => left.total_cmp(right),
        (Value::Text(left), Value::Text(right)) => left.to_lowercase().cmp(&right.to_lowercase()),
        (Value::Logical(left), Value::Logical(right)) => left.cmp(right),
        (Value::Error(left), Value::Error(right)) => left.as_str().cmp(right.as_str()),
        (left, right) => value_rank(left).cmp(&value_rank(right)),
    }
}

fn value_rank(value: &Value) -> u8 {
    match value {
        Value::Blank => 0,
        Value::Number(_) => 1,
        Value::Text(_) => 2,
        Value::Logical(_) => 3,
        Value::Error(_) => 4,
    }
}

fn unique(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if args.is_empty() || args.len() > 3 {
        return Err(ErrorKind::Value);
    }
    let source = engine.eval_array(context, &args[0])?;
    let by_column = optional_logical(engine, context, args.get(1), false)?;
    let exactly_once = optional_logical(engine, context, args.get(2), false)?;
    let item_count = if by_column { source.cols } else { source.rows };
    let comparisons = u64::from(item_count)
        .checked_mul(u64::from(item_count))
        .and_then(|count| {
            count.checked_mul(u64::from(if by_column { source.rows } else { source.cols }))
        })
        .ok_or(ErrorKind::Num)?;
    engine.charge_function_iterations(context, comparisons)?;
    let mut selected = Vec::new();
    for candidate in 0..item_count {
        let occurrences = (0..item_count)
            .filter(|other| array_item_eq(&source, candidate, *other, by_column))
            .count();
        let already_selected = selected
            .iter()
            .any(|existing| array_item_eq(&source, candidate, *existing, by_column));
        if !already_selected && (!exactly_once || occurrences == 1) {
            selected.push(candidate);
        }
    }
    if selected.is_empty() {
        return Err(ErrorKind::Calc);
    }
    let (rows, cols) = if by_column {
        (source.rows, selected.len() as u32)
    } else {
        (selected.len() as u32, source.cols)
    };
    let cell_count = cell_count(rows, cols)?;
    engine.ensure_array_cells(cell_count)?;
    let mut data = Vec::with_capacity(cell_count as usize);
    if by_column {
        for row in 0..source.rows {
            for column in &selected {
                data.push(source.at(row, *column).clone());
            }
        }
    } else {
        for row in selected {
            for column in 0..source.cols {
                data.push(source.at(row, column).clone());
            }
        }
    }
    Ok(Array { rows, cols, data })
}

fn array_item_eq(source: &Array, left: u32, right: u32, by_column: bool) -> bool {
    if by_column {
        (0..source.rows).all(|row| values_equal(source.at(row, left), source.at(row, right)))
    } else {
        (0..source.cols)
            .all(|column| values_equal(source.at(left, column), source.at(right, column)))
    }
}

fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Text(left), Value::Text(right)) => left.eq_ignore_ascii_case(right),
        _ => left == right,
    }
}

fn optional_logical(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
    default: bool,
) -> Result<bool, ErrorKind> {
    match expr {
        Some(Expr::Missing) | None => Ok(default),
        Some(expr) => to_logical(&engine.eval_scalar(context, expr)),
    }
}

fn optional_integer(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
    default: i64,
) -> Result<i64, ErrorKind> {
    match expr {
        Some(Expr::Missing) | None => Ok(default),
        Some(expr) => Ok(to_number(&engine.eval_scalar(context, expr))?.trunc() as i64),
    }
}

fn cell_count(rows: u32, cols: u32) -> Result<u64, ErrorKind> {
    u64::from(rows)
        .checked_mul(u64::from(cols))
        .ok_or(ErrorKind::Num)
}

#[cfg(test)]
mod tests {
    use super::{ErrorKind, SliceOperation, cell_count, resolve_index, slice_bounds};

    #[test]
    fn shape_arithmetic_and_signed_indexes_are_exact() {
        assert_eq!(cell_count(2, 3), Ok(6));
        assert_eq!(resolve_index(1.0, 3), Ok(0));
        assert_eq!(resolve_index(3.0, 3), Ok(2));
        assert_eq!(resolve_index(-1.0, 3), Ok(2));
        assert_eq!(resolve_index(-3.0, 3), Ok(0));
        assert_eq!(resolve_index(0.0, 3), Err(ErrorKind::Value));
        assert_eq!(resolve_index(4.0, 3), Err(ErrorKind::Value));
    }

    #[test]
    fn take_and_drop_bounds_distinguish_edges_and_directions() {
        assert_eq!(slice_bounds(5, 2, SliceOperation::Take), Ok((0, 2)));
        assert_eq!(slice_bounds(5, -2, SliceOperation::Take), Ok((3, 2)));
        assert_eq!(slice_bounds(5, 2, SliceOperation::Drop), Ok((2, 3)));
        assert_eq!(slice_bounds(5, -2, SliceOperation::Drop), Ok((0, 3)));
        assert_eq!(
            slice_bounds(5, 5, SliceOperation::Drop),
            Err(ErrorKind::Calc)
        );
        assert_eq!(
            slice_bounds(5, -5, SliceOperation::Drop),
            Err(ErrorKind::Calc)
        );
    }
}
