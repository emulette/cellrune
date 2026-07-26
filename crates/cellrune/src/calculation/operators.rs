use super::ArithmeticSemantics;
use super::ast::{BinaryOp, UnaryOp};
use super::coerce::{compare, to_number, to_text};
use super::limits::CalculationLimitKind;
use super::runtime::Array;
use super::value::{ErrorKind, Value};

const NOT_AVAILABLE: Value = Value::Error(ErrorKind::NA);

/// Relative width of the cancellation window, as a fraction of the larger operand.
///
/// `2^-48` is sixteen units in the last place of a 53-bit significand: the operands agree in
/// everything but the last four bits. A decimal literal that cannot be represented exactly is off
/// by well under one such unit, so the residue left when two of them cancel always lands inside
/// this window, while a difference the workbook author actually meant does not — `=1.1-1` leaves
/// `0.1`, nine percent of the larger operand and fourteen orders of magnitude outside it.
///
/// The window necessarily also swallows genuine differences of a few units in the last place,
/// such as `=1.0000000000000004-1`. That agrees with Excel rather than diverging from it: Excel
/// keeps fifteen significant decimal digits of any entered number, so both operands are the same
/// number there and the difference is zero for that reason instead of this one. Sixteenth-digit
/// distinctions do not survive a round trip through Excel either way.
///
/// # What this deliberately does not catch
///
/// The window is relative to the operands of the operation being performed, so it only sees
/// residue produced at that operation's magnitude. `=100.1-100-0.1` cancels to `-5.69e-15`, but
/// the residue was created by the first subtraction, where it is a fraction of `100.1`; by the
/// time the second subtraction runs, the operands are around `0.1` and the same residue is
/// fourteen times too large for this window.
///
/// Widening the window to reach it is not an option. `=1.0000000000001-1` is a difference the
/// author meant, is well inside the fifteen significant digits Excel keeps, and sits at almost
/// exactly the same relative magnitude. No fixed relative threshold separates the two, because
/// the distinction is not about the final operands at all — it is about where in the chain the
/// precision was lost. Correcting those cases would require carrying an error term through every
/// intermediate, which is a different engine, not a wider constant.
///
/// So the correction is bounded and honest: residue created by the operation it is applied to is
/// removed, residue inherited from a larger intermediate is not. `docs/NUMERICS.md` records this
/// boundary with the same example.
pub(super) const CANCELLATION_WINDOW: f64 = 1.0 / (1_u64 << 48) as f64;

/// Applies Excel's correction for a sum or difference that cancelled to near zero.
///
/// Only addition and subtraction are corrected. Multiplication and division do not accumulate
/// representation error into a residue this way: a small product is small because its operands
/// were, not because two nearly equal quantities cancelled.
fn correct_cancellation(op: BinaryOp, left: f64, right: f64, result: f64) -> f64 {
    if !matches!(op, BinaryOp::Add | BinaryOp::Subtract) || result == 0.0 {
        return result;
    }
    let larger = left.abs().max(right.abs());
    if result.abs() < larger * CANCELLATION_WINDOW {
        // Positive zero, matching the engine's existing normalization of calculated `-0.0`.
        0.0
    } else {
        result
    }
}

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
    arithmetic: ArithmeticSemantics,
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
                Ok((left, right)) => evaluate_arithmetic(op, left, right, arithmetic),
                Err(kind) => Value::Error(kind),
            }
        }
    }
}

fn evaluate_arithmetic(
    op: BinaryOp,
    left: f64,
    right: f64,
    arithmetic: ArithmeticSemantics,
) -> Value {
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
    let result = match arithmetic {
        ArithmeticSemantics::ExcelNearZero => correct_cancellation(op, left, right, result),
        ArithmeticSemantics::Ieee754 => result,
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
    arithmetic: ArithmeticSemantics,
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
                arithmetic,
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
    use super::{ArithmeticSemantics, BinaryOp, element_at, lift_binary};
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
        let broadcast = lift_binary(
            BinaryOp::Add,
            &row,
            &column,
            32_767,
            1_000_000,
            ArithmeticSemantics::default(),
        )
        .expect("vector broadcast");
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
        let padded = lift_binary(
            BinaryOp::Add,
            &two_by_two,
            &row,
            32_767,
            1_000_000,
            ArithmeticSemantics::default(),
        )
        .expect("mismatched arrays produce a padded result");
        assert_eq!((padded.rows, padded.cols), (2, 3));
        assert_eq!(padded.data[0], Value::Number(2.0));
        assert_eq!(padded.data[4], Value::Number(6.0));
        assert_eq!(padded.data[2], Value::Error(ErrorKind::NA));
        assert_eq!(padded.data[5], Value::Error(ErrorKind::NA));
        assert_eq!(element_at(&two_by_two, 3, 3), &Value::Error(ErrorKind::NA));
    }
}
