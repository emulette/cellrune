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
                Ok((left, right)) => evaluate_arithmetic(op, left, right),
                Err(kind) => Value::Error(kind),
            }
        }
    }
}

fn evaluate_arithmetic(op: BinaryOp, left: f64, right: f64) -> Value {
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

pub(super) fn broadcast_shape(left: &Array, right: &Array) -> Result<(u32, u32), ErrorKind> {
    Ok((left.rows.max(right.rows), left.cols.max(right.cols)))
}

/// Position an operand of the given shape contributes to `(row, col)` of a broadcast result.
///
/// A singleton dimension repeats along that axis; a position the operand does not reach at all is
/// `None`, which the callers surface as `#N/A` (values) or as an untraced element (decimals).
/// Both the value and the decimal-trace side of the array path index through this one rule, so
/// broadcasting cannot change for one of them without changing for the other.
pub(super) const fn broadcast_index(rows: u32, cols: u32, row: u32, col: u32) -> Option<usize> {
    let source_row = if rows == 1 { 0 } else { row };
    let source_col = if cols == 1 { 0 } else { col };
    if source_row >= rows || source_col >= cols {
        return None;
    }
    Some(source_row as usize * cols as usize + source_col as usize)
}

pub(super) fn element_at(array: &Array, row: u32, col: u32) -> &Value {
    broadcast_index(array.rows, array.cols, row, col)
        .and_then(|index| array.data.get(index))
        .unwrap_or(&NOT_AVAILABLE)
}

#[cfg(test)]
mod tests {
    use super::{broadcast_shape, element_at};
    use crate::calculation::runtime::Array;
    use crate::calculation::value::{ErrorKind, Value};

    /// These are the primitives the array-binary path in `eval::expression` indexes through, so a
    /// change to broadcasting shows up here. The evaluation loop itself, and the array-cell budget
    /// it enforces, are covered end to end by the release integration suite rather than by a second
    /// copy of the loop kept alive for tests.
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
        assert_eq!(broadcast_shape(&row, &column), Ok((3, 3)));
        // A singleton dimension repeats: every row of the result reads the same row element.
        assert_eq!(element_at(&row, 0, 0), &Value::Number(1.0));
        assert_eq!(element_at(&row, 2, 0), &Value::Number(1.0));
        assert_eq!(element_at(&column, 2, 2), &Value::Number(30.0));

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
        // Mismatched extents pad rather than truncate, and the padding is `#N/A`.
        assert_eq!(broadcast_shape(&two_by_two, &row), Ok((2, 3)));
        assert_eq!(element_at(&two_by_two, 0, 2), &Value::Error(ErrorKind::NA));
        assert_eq!(element_at(&two_by_two, 1, 2), &Value::Error(ErrorKind::NA));
        assert_eq!(element_at(&two_by_two, 3, 3), &Value::Error(ErrorKind::NA));
    }
}
