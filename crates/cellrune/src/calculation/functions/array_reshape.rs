use super::super::ast::Expr;
use super::super::coerce::{to_logical, to_number};
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::array_common::{cell_count, poll_cancellation, validate_array_input};

pub(super) fn expand(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if !(2..=4).contains(&args.len()) {
        return Err(ErrorKind::Value);
    }
    let source = engine.eval_array(context, &args[0])?;
    validate_array_input(engine, context, &source)?;
    let rows = optional_dimension(engine, context, args.get(1), source.rows)?;
    let cols = optional_dimension(engine, context, args.get(2), source.cols)?;
    if rows < source.rows || cols < source.cols {
        return Err(ErrorKind::Value);
    }
    let cells = cell_count(rows, cols)?;
    engine.ensure_array_cells(cells)?;
    engine.charge_function_iterations(context, cells)?;
    let padding = padding_value(engine, context, args.get(3))?;
    let mut data = Vec::with_capacity(usize::try_from(cells).map_err(|_| ErrorKind::Num)?);
    for row in 0..rows {
        for column in 0..cols {
            poll_cancellation(context)?;
            data.push(if row < source.rows && column < source.cols {
                source.at(row, column).clone()
            } else {
                padding.clone()
            });
        }
    }
    Ok(Array { rows, cols, data })
}

pub(super) fn to_col(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    flatten(engine, context, args, FlattenShape::Column)
}

pub(super) fn to_row(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    flatten(engine, context, args, FlattenShape::Row)
}

pub(super) fn trim_range(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if args.is_empty() || args.len() > 3 {
        return Err(ErrorKind::Value);
    }
    let source = engine.eval_array(context, &args[0])?;
    validate_array_input(engine, context, &source)?;
    let row_mode = trim_mode(engine, context, args.get(1))?;
    let column_mode = trim_mode(engine, context, args.get(2))?;
    let input_cells = cell_count(source.rows, source.cols)?;
    let passes = 1_u64
        .checked_add(row_mode.scan_passes())
        .and_then(|passes| passes.checked_add(column_mode.scan_passes()))
        .ok_or(ErrorKind::Num)?;
    engine.charge_function_iterations(
        context,
        input_cells.checked_mul(passes).ok_or(ErrorKind::Num)?,
    )?;
    let (row_start, row_end) = trimmed_bounds(source.rows, row_mode, |row| {
        for column in 0..source.cols {
            poll_cancellation(context)?;
            if !matches!(source.at(row, column), Value::Blank) {
                return Ok(true);
            }
        }
        Ok(false)
    })?;
    let (column_start, column_end) = trimmed_bounds(source.cols, column_mode, |column| {
        for row in 0..source.rows {
            poll_cancellation(context)?;
            if !matches!(source.at(row, column), Value::Blank) {
                return Ok(true);
            }
        }
        Ok(false)
    })?;
    let rows = row_end - row_start;
    let cols = column_end - column_start;
    let cells = cell_count(rows, cols)?;
    engine.ensure_array_cells(cells)?;
    let mut data = Vec::with_capacity(usize::try_from(cells).map_err(|_| ErrorKind::Num)?);
    for row in row_start..row_end {
        for column in column_start..column_end {
            poll_cancellation(context)?;
            data.push(source.at(row, column).clone());
        }
    }
    Ok(Array { rows, cols, data })
}

pub(super) fn wrap_cols(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    wrap(engine, context, args, WrapShape::Columns)
}

pub(super) fn wrap_rows(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    wrap(engine, context, args, WrapShape::Rows)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinearOrder {
    RowMajor,
    ColumnMajor,
}

struct LinearArrayIter<'array> {
    array: &'array Array,
    order: LinearOrder,
    index: u64,
    cells: u64,
}

impl<'array> LinearArrayIter<'array> {
    fn new(array: &'array Array, order: LinearOrder) -> Result<Self, ErrorKind> {
        Ok(Self {
            array,
            order,
            index: 0,
            cells: cell_count(array.rows, array.cols)?,
        })
    }
}

impl<'array> Iterator for LinearArrayIter<'array> {
    type Item = &'array Value;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.cells {
            return None;
        }
        let (row, column) = match self.order {
            LinearOrder::RowMajor => (
                self.index / u64::from(self.array.cols),
                self.index % u64::from(self.array.cols),
            ),
            LinearOrder::ColumnMajor => (
                self.index % u64::from(self.array.rows),
                self.index / u64::from(self.array.rows),
            ),
        };
        self.index += 1;
        Some(self.array.at(row as u32, column as u32))
    }
}

#[derive(Debug, Clone, Copy)]
enum FlattenShape {
    Column,
    Row,
}

fn flatten(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    shape: FlattenShape,
) -> Result<Array, ErrorKind> {
    if args.is_empty() || args.len() > 3 {
        return Err(ErrorKind::Value);
    }
    let source = engine.eval_array(context, &args[0])?;
    validate_array_input(engine, context, &source)?;
    let ignore = ignore_mode(engine, context, args.get(1))?;
    let order = if optional_logical(engine, context, args.get(2), false)? {
        LinearOrder::ColumnMajor
    } else {
        LinearOrder::RowMajor
    };
    let input_cells = cell_count(source.rows, source.cols)?;
    engine.ensure_array_cells(input_cells)?;
    engine.charge_function_iterations(context, input_cells)?;
    let mut data = Vec::new();
    for value in LinearArrayIter::new(&source, order)? {
        poll_cancellation(context)?;
        if !ignore.ignores(value) {
            data.push(value.clone());
        }
    }
    if data.is_empty() {
        return Err(ErrorKind::Calc);
    }
    let length = u32::try_from(data.len()).map_err(|_| ErrorKind::Num)?;
    let (rows, cols) = match shape {
        FlattenShape::Column => (length, 1),
        FlattenShape::Row => (1, length),
    };
    engine.ensure_array_cells(u64::from(length))?;
    Ok(Array { rows, cols, data })
}

#[derive(Debug, Clone, Copy)]
struct IgnoreMode(u8);

impl IgnoreMode {
    fn ignores(self, value: &Value) -> bool {
        (self.0 & 1 != 0 && matches!(value, Value::Blank))
            || (self.0 & 2 != 0 && matches!(value, Value::Error(_)))
    }
}

fn ignore_mode(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
) -> Result<IgnoreMode, ErrorKind> {
    let mode = optional_integer(engine, context, expr, 0)?;
    u8::try_from(mode)
        .ok()
        .filter(|mode| *mode <= 3)
        .map(IgnoreMode)
        .ok_or(ErrorKind::Value)
}

#[derive(Debug, Clone, Copy)]
enum TrimMode {
    None,
    Leading,
    Trailing,
    Both,
}

impl TrimMode {
    fn trims_leading(self) -> bool {
        matches!(self, Self::Leading | Self::Both)
    }

    fn trims_trailing(self) -> bool {
        matches!(self, Self::Trailing | Self::Both)
    }

    fn scan_passes(self) -> u64 {
        u64::from(self.trims_leading()) + u64::from(self.trims_trailing())
    }
}

fn trim_mode(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
) -> Result<TrimMode, ErrorKind> {
    match optional_integer(engine, context, expr, 3)? {
        0 => Ok(TrimMode::None),
        1 => Ok(TrimMode::Leading),
        2 => Ok(TrimMode::Trailing),
        3 => Ok(TrimMode::Both),
        _ => Err(ErrorKind::Value),
    }
}

fn trimmed_bounds(
    length: u32,
    mode: TrimMode,
    mut has_value: impl FnMut(u32) -> Result<bool, ErrorKind>,
) -> Result<(u32, u32), ErrorKind> {
    let mut start = 0;
    let mut end = length;
    if mode.trims_leading() {
        start = length;
        for index in 0..length {
            if has_value(index)? {
                start = index;
                break;
            }
        }
        if start == length {
            return Err(ErrorKind::Ref);
        }
    }
    if mode.trims_trailing() {
        end = start;
        for index in (start..length).rev() {
            if has_value(index)? {
                end = index + 1;
                break;
            }
        }
        if end == start {
            return Err(ErrorKind::Ref);
        }
    }
    Ok((start, end))
}

#[derive(Debug, Clone, Copy)]
enum WrapShape {
    Columns,
    Rows,
}

fn wrap(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    shape: WrapShape,
) -> Result<Array, ErrorKind> {
    if !(2..=3).contains(&args.len()) {
        return Err(ErrorKind::Value);
    }
    let source = engine.eval_array(context, &args[0])?;
    validate_array_input(engine, context, &source)?;
    if source.rows != 1 && source.cols != 1 {
        return Err(ErrorKind::Value);
    }
    let wrap_count = positive_wrap_count(engine, context, &args[1])?;
    let length = source.rows.max(source.cols);
    let extent = wrap_count.min(length);
    let groups = length.div_ceil(wrap_count);
    let (rows, cols) = match shape {
        WrapShape::Columns => (extent, groups),
        WrapShape::Rows => (groups, extent),
    };
    let cells = cell_count(rows, cols)?;
    engine.ensure_array_cells(cells)?;
    engine.charge_function_iterations(
        context,
        cell_count(source.rows, source.cols)?
            .checked_add(cells)
            .ok_or(ErrorKind::Num)?,
    )?;
    let padding = padding_value(engine, context, args.get(2))?;
    let mut values = Vec::with_capacity(source.data.len());
    for value in LinearArrayIter::new(&source, LinearOrder::RowMajor)? {
        poll_cancellation(context)?;
        values.push(value.clone());
    }
    let mut data = Vec::with_capacity(usize::try_from(cells).map_err(|_| ErrorKind::Num)?);
    for row in 0..rows {
        for column in 0..cols {
            poll_cancellation(context)?;
            let index = match shape {
                WrapShape::Columns => column * wrap_count + row,
                WrapShape::Rows => row * wrap_count + column,
            };
            data.push(
                values
                    .get(index as usize)
                    .cloned()
                    .unwrap_or_else(|| padding.clone()),
            );
        }
    }
    Ok(Array { rows, cols, data })
}

fn positive_wrap_count(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<u32, ErrorKind> {
    let value = to_number(&engine.eval_scalar(context, expr))?.trunc();
    if !value.is_finite() || value > f64::from(u32::MAX) {
        return Err(ErrorKind::Num);
    }
    if value < 1.0 {
        return Err(ErrorKind::Num);
    }
    Ok(value as u32)
}

fn optional_dimension(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
    default: u32,
) -> Result<u32, ErrorKind> {
    let Some(expr) = expr.filter(|expr| !matches!(expr, Expr::Missing)) else {
        return Ok(default);
    };
    let value = to_number(&engine.eval_scalar(context, expr))?.trunc();
    if !value.is_finite() || value > f64::from(u32::MAX) {
        return Err(ErrorKind::Num);
    }
    if value < 1.0 {
        return Err(ErrorKind::Value);
    }
    Ok(value as u32)
}

fn optional_integer(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
    default: i64,
) -> Result<i64, ErrorKind> {
    let Some(expr) = expr.filter(|expr| !matches!(expr, Expr::Missing)) else {
        return Ok(default);
    };
    let value = to_number(&engine.eval_scalar(context, expr))?.trunc();
    if !value.is_finite() || value < i64::MIN as f64 || value > i64::MAX as f64 {
        return Err(ErrorKind::Num);
    }
    Ok(value as i64)
}

fn optional_logical(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
    default: bool,
) -> Result<bool, ErrorKind> {
    match expr {
        Some(expr) if !matches!(expr, Expr::Missing) => {
            to_logical(&engine.eval_scalar(context, expr))
        }
        _ => Ok(default),
    }
}

fn padding_value(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
) -> Result<Value, ErrorKind> {
    match expr {
        Some(expr) if !matches!(expr, Expr::Missing) => match engine.eval_scalar(context, expr) {
            Value::Error(kind) if kind.is_engine_issue() => Err(kind),
            value => Ok(value),
        },
        _ => Ok(Value::Error(ErrorKind::NA)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn array(rows: u32, cols: u32, values: &[f64]) -> Array {
        Array {
            rows,
            cols,
            data: values.iter().copied().map(Value::Number).collect(),
        }
    }

    #[test]
    fn linear_iterator_preserves_both_excel_scan_orders() {
        let source = array(2, 3, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let values = |order| {
            LinearArrayIter::new(&source, order)
                .expect("valid shape")
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(
            values(LinearOrder::RowMajor),
            array(1, 6, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).data
        );
        assert_eq!(
            values(LinearOrder::ColumnMajor),
            array(1, 6, &[1.0, 4.0, 2.0, 5.0, 3.0, 6.0]).data
        );
    }

    #[test]
    fn trim_modes_select_exact_requested_edges() {
        let occupied = |index| Ok(matches!(index, 2 | 4));
        assert_eq!(trimmed_bounds(7, TrimMode::None, occupied), Ok((0, 7)));
        assert_eq!(trimmed_bounds(7, TrimMode::Leading, occupied), Ok((2, 7)));
        assert_eq!(trimmed_bounds(7, TrimMode::Trailing, occupied), Ok((0, 5)));
        assert_eq!(trimmed_bounds(7, TrimMode::Both, occupied), Ok((2, 5)));
        assert_eq!(
            trimmed_bounds(3, TrimMode::Both, |_| Ok(false)),
            Err(ErrorKind::Ref)
        );
    }
}
