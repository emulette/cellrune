use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::kernel::ArrayFunction;
use super::util::required_number;

pub(super) fn call_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: ArrayFunction,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    match function {
        ArrayFunction::ChooseCols => super::modern_array::choose_cols(engine, context, args),
        ArrayFunction::ChooseRows => super::modern_array::choose_rows(engine, context, args),
        ArrayFunction::Drop => super::modern_array::drop(engine, context, args),
        ArrayFunction::Expand => super::array_reshape::expand(engine, context, args),
        ArrayFunction::Filter => super::modern_array::filter_array(engine, context, args),
        ArrayFunction::HStack => super::modern_array::hstack(engine, context, args),
        ArrayFunction::MInverse => minverse(engine, context, args),
        ArrayFunction::Sort => super::modern_array::sort_array(engine, context, args),
        ArrayFunction::SortBy => super::array_sort::sort_by(engine, context, args),
        ArrayFunction::Take => super::modern_array::take(engine, context, args),
        ArrayFunction::ToCol => super::array_reshape::to_col(engine, context, args),
        ArrayFunction::ToRow => super::array_reshape::to_row(engine, context, args),
        ArrayFunction::Unique => super::modern_array::unique_array(engine, context, args),
        ArrayFunction::VStack => super::modern_array::vstack(engine, context, args),
        ArrayFunction::TrimRange => super::array_reshape::trim_range(engine, context, args),
        ArrayFunction::WrapCols => super::array_reshape::wrap_cols(engine, context, args),
        ArrayFunction::WrapRows => super::array_reshape::wrap_rows(engine, context, args),
        ArrayFunction::MMult => mmult(engine, context, args),
        ArrayFunction::MUnit => munit(engine, context, args),
        ArrayFunction::Sequence => sequence(engine, context, args),
        ArrayFunction::Transpose => transpose(engine, context, args),
    }
}

fn minverse(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    let [argument] = args else {
        return Err(ErrorKind::Value);
    };
    let source = engine.eval_array(context, argument)?;
    if source.rows != source.cols {
        return Err(ErrorKind::Value);
    }
    let dimension = u64::from(source.rows);
    let cells = dimension.checked_mul(dimension).ok_or(ErrorKind::Num)?;
    let workspace = cells
        .checked_mul(3)
        .and_then(|value| value.checked_add(dimension.checked_mul(2)?))
        .ok_or(ErrorKind::Num)?;
    engine.ensure_array_cells(workspace)?;
    let numbers = strict_numbers(source.data)?;
    let matrix = super::linear_algebra::DenseMatrix::new(
        source.rows as usize,
        source.cols as usize,
        numbers,
    )?;
    let inverse = super::linear_algebra::invert(matrix, |work| {
        engine.charge_function_iterations(context, work)
    })?;
    Ok(Array {
        rows: source.rows,
        cols: source.cols,
        data: inverse.into_data().into_iter().map(Value::Number).collect(),
    })
}

fn munit(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Result<Array, ErrorKind> {
    let [argument] = args else {
        return Err(ErrorKind::Value);
    };
    let dimension = required_number(engine, context, argument)?.trunc();
    if !dimension.is_finite() || dimension < 1.0 || dimension > f64::from(u32::MAX) {
        return Err(ErrorKind::Value);
    }
    let dimension = dimension as u32;
    let cells = u64::from(dimension)
        .checked_mul(u64::from(dimension))
        .ok_or(ErrorKind::Num)?;
    engine.ensure_array_cells(cells)?;
    engine.charge_function_iterations(context, cells)?;
    let capacity = usize::try_from(cells).map_err(|_| ErrorKind::Num)?;
    let mut data = Vec::with_capacity(capacity);
    for row in 0..dimension {
        for col in 0..dimension {
            data.push(Value::Number(if row == col { 1.0 } else { 0.0 }));
        }
    }
    Ok(Array {
        rows: dimension,
        cols: dimension,
        data,
    })
}

pub(super) fn call_scalar(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: ArrayFunction,
    args: &[Expr],
) -> Value {
    match call_array(engine, context, function, args) {
        Ok(array) => array
            .data
            .into_iter()
            .next()
            .unwrap_or(Value::Error(ErrorKind::Value)),
        Err(kind) => Value::Error(kind),
    }
}

fn mmult(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Result<Array, ErrorKind> {
    if args.len() != 2 {
        return Err(ErrorKind::Value);
    }
    let left = engine.eval_array(context, &args[0])?;
    let right = engine.eval_array(context, &args[1])?;
    if left.cols != right.rows {
        return Err(ErrorKind::Value);
    }
    let output_cells = u64::from(left.rows) * u64::from(right.cols);
    let operations = output_cells
        .checked_mul(u64::from(left.cols))
        .ok_or(ErrorKind::Num)?;
    engine.ensure_array_cells(output_cells)?;
    engine.charge_function_iterations(context, operations)?;

    let left_numbers = strict_numbers(left.data)?;
    let right_numbers = strict_numbers(right.data)?;
    let mut data = Vec::with_capacity(output_cells as usize);
    for row in 0..left.rows {
        for column in 0..right.cols {
            let mut result = 0.0;
            for inner in 0..left.cols {
                let left_index = (row * left.cols + inner) as usize;
                let right_index = (inner * right.cols + column) as usize;
                result += left_numbers[left_index] * right_numbers[right_index];
            }
            if !result.is_finite() {
                return Err(ErrorKind::Num);
            }
            data.push(Value::Number(result));
        }
    }
    Ok(Array {
        rows: left.rows,
        cols: right.cols,
        data,
    })
}

fn strict_numbers(values: Vec<Value>) -> Result<Vec<f64>, ErrorKind> {
    values
        .into_iter()
        .map(|value| match value {
            Value::Number(number) => Ok(number),
            Value::Error(kind) => Err(kind),
            Value::Blank | Value::Text(_) | Value::Logical(_) => Err(ErrorKind::Value),
        })
        .collect()
}

fn transpose(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if args.len() != 1 {
        return Err(ErrorKind::Value);
    }
    let source = engine.eval_array(context, &args[0])?;
    let cells = u64::from(source.rows) * u64::from(source.cols);
    engine.ensure_array_cells(cells)?;
    let mut data = Vec::with_capacity(cells as usize);
    for row in 0..source.cols {
        for column in 0..source.rows {
            data.push(source.at(column, row).clone());
        }
    }
    Ok(Array {
        rows: source.cols,
        cols: source.rows,
        data,
    })
}

fn sequence(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if args.is_empty() || args.len() > 4 {
        return Err(ErrorKind::Value);
    }
    let rows = dimension(engine, context, args.first())?;
    let columns = dimension(engine, context, args.get(1))?;
    let start = optional_number(engine, context, args.get(2), 1.0)?;
    let step = optional_number(engine, context, args.get(3), 1.0)?;
    let cells = u64::from(rows) * u64::from(columns);
    engine.ensure_array_cells(cells)?;
    engine.charge_function_iterations(context, cells)?;

    let mut data = Vec::with_capacity(cells as usize);
    for index in 0..cells {
        let value = start + step * index as f64;
        if !value.is_finite() {
            return Err(ErrorKind::Num);
        }
        data.push(Value::Number(value));
    }
    Ok(Array {
        rows,
        cols: columns,
        data,
    })
}

fn dimension(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
) -> Result<u32, ErrorKind> {
    let value = match expr {
        Some(Expr::Missing) | None => 1.0,
        Some(expr) => required_number(engine, context, expr)?,
    };
    let value = value.trunc();
    if value < 1.0 || value > f64::from(u32::MAX) {
        return Err(ErrorKind::Num);
    }
    Ok(value as u32)
}

fn optional_number(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
    default: f64,
) -> Result<f64, ErrorKind> {
    match expr {
        Some(Expr::Missing) | None => Ok(default),
        Some(expr) => required_number(engine, context, expr),
    }
}
