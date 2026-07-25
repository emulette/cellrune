use super::ast::{BinaryOp, UnaryOp};
use super::coerce::{compare, to_number, to_text};
use super::limits::CalculationLimitKind;
use super::runtime::Array;
use super::value::{ErrorKind, Value};

const NOT_AVAILABLE: Value = Value::Error(ErrorKind::NA);

pub(super) fn apply_unary(op: UnaryOp, value: &Value) -> Value {
    let result = match op {
        UnaryOp::Negate => to_number(value).map(|number| Value::Number(-number)),
        UnaryOp::Plus => to_number(value).map(Value::Number),
        UnaryOp::Percent => to_number(value).map(|number| Value::Number(number / 100.0)),
    };
    result.unwrap_or_else(Value::Error)
}

pub(super) fn apply_binary(
    op: BinaryOp,
    left: &Value,
    right: &Value,
    max_text_bytes: u64,
) -> Value {
    match op {
        BinaryOp::Concat => match to_text(left)
            .and_then(|left_text| to_text(right).map(|right_text| (left_text, right_text)))
        {
            Ok((mut left_text, right_text)) => {
                let output_bytes = (left_text.len() as u64).checked_add(right_text.len() as u64);
                if output_bytes.is_none_or(|bytes| bytes > max_text_bytes) {
                    Value::Error(ErrorKind::ResourceLimit(CalculationLimitKind::TextBytes))
                } else {
                    left_text.push_str(&right_text);
                    Value::Text(left_text)
                }
            }
            Err(kind) => Value::Error(kind),
        },
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            match compare(left, right) {
                Ok(ordering) => Value::Logical(match op {
                    BinaryOp::Eq => ordering == std::cmp::Ordering::Equal,
                    BinaryOp::Ne => ordering != std::cmp::Ordering::Equal,
                    BinaryOp::Lt => ordering == std::cmp::Ordering::Less,
                    BinaryOp::Le => ordering != std::cmp::Ordering::Greater,
                    BinaryOp::Gt => ordering == std::cmp::Ordering::Greater,
                    BinaryOp::Ge => ordering != std::cmp::Ordering::Less,
                    _ => false,
                }),
                Err(kind) => Value::Error(kind),
            }
        }
        BinaryOp::Add
        | BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Power => {
            match to_number(left).and_then(|left| to_number(right).map(|right| (left, right))) {
                Ok((left, right)) => arithmetic(op, left, right),
                Err(kind) => Value::Error(kind),
            }
        }
    }
}

fn arithmetic(op: BinaryOp, left: f64, right: f64) -> Value {
    let result = match op {
        BinaryOp::Add => left + right,
        BinaryOp::Subtract => left - right,
        BinaryOp::Multiply => left * right,
        BinaryOp::Divide if right == 0.0 => return Value::Error(ErrorKind::Div0),
        BinaryOp::Divide => left / right,
        BinaryOp::Power if left == 0.0 && right == 0.0 => return Value::Error(ErrorKind::Num),
        BinaryOp::Power => left.powf(right),
        _ => return Value::Error(ErrorKind::Value),
    };
    if result.is_finite() {
        Value::Number(result)
    } else {
        Value::Error(ErrorKind::Num)
    }
}

pub(super) fn lift_binary(
    op: BinaryOp,
    left: &Array,
    right: &Array,
    max_text_bytes: u64,
    max_array_cells: u64,
) -> Result<Array, ErrorKind> {
    let (rows, cols) = broadcast_shape(left, right)?;
    let cells = u64::from(rows)
        .checked_mul(u64::from(cols))
        .ok_or(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells))?;
    if cells > max_array_cells {
        return Err(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells));
    }
    let mut data = Vec::with_capacity(cells as usize);
    for row in 0..rows {
        for col in 0..cols {
            data.push(apply_binary(
                op,
                element_at(left, row, col),
                element_at(right, row, col),
                max_text_bytes,
            ));
        }
    }
    Ok(Array { rows, cols, data })
}

pub(super) fn broadcast_shape(left: &Array, right: &Array) -> Result<(u32, u32), ErrorKind> {
    Ok((left.rows.max(right.rows), left.cols.max(right.cols)))
}

pub(super) fn element_at(array: &Array, row: u32, col: u32) -> &Value {
    let source_row = if array.rows == 1 { 0 } else { row };
    let source_col = if array.cols == 1 { 0 } else { col };
    if source_row >= array.rows || source_col >= array.cols {
        &NOT_AVAILABLE
    } else {
        array.at(source_row, source_col)
    }
}

#[cfg(test)]
mod tests {
    use super::{BinaryOp, element_at, lift_binary};
    use crate::calculation::runtime::Array;
    use crate::calculation::value::{ErrorKind, Value};

    #[test]
    fn binary_arrays_broadcast_vectors_and_pad_missing_dimensions() {
        let row = Array {
            rows: 1,
            cols: 3,
            data: vec![Value::Number(1.0), Value::Number(2.0), Value::Number(3.0)],
        };
        let column = Array {
            rows: 3,
            cols: 1,
            data: vec![
                Value::Number(10.0),
                Value::Number(20.0),
                Value::Number(30.0),
            ],
        };
        let broadcast =
            lift_binary(BinaryOp::Add, &row, &column, 32_767, 1_000_000).expect("vector broadcast");
        assert_eq!((broadcast.rows, broadcast.cols), (3, 3));
        assert_eq!(broadcast.data[0], Value::Number(11.0));
        assert_eq!(broadcast.data[8], Value::Number(33.0));

        let two_by_two = Array {
            rows: 2,
            cols: 2,
            data: vec![
                Value::Number(1.0),
                Value::Number(2.0),
                Value::Number(3.0),
                Value::Number(4.0),
            ],
        };
        let padded = lift_binary(BinaryOp::Add, &two_by_two, &row, 32_767, 1_000_000)
            .expect("mismatched arrays produce a padded result");
        assert_eq!((padded.rows, padded.cols), (2, 3));
        assert_eq!(padded.data[0], Value::Number(2.0));
        assert_eq!(padded.data[4], Value::Number(6.0));
        assert_eq!(padded.data[2], Value::Error(ErrorKind::NA));
        assert_eq!(padded.data[5], Value::Error(ErrorKind::NA));
        assert_eq!(element_at(&two_by_two, 3, 3), &Value::Error(ErrorKind::NA));
    }
}
