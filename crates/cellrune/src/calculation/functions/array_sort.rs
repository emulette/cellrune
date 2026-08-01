use std::cmp::Ordering;

use super::super::ast::Expr;
use super::super::coerce::{compare_text_case_insensitive, to_number};
use super::super::eval::{Engine, EvalContext};
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::array_common::{cell_count, poll_cancellation, validate_array_input};

pub(super) fn sort_by(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    if args.len() < 3 || args.len().is_multiple_of(2) {
        return Err(ErrorKind::Value);
    }
    let source = engine.eval_array(context, &args[0])?;
    validate_array_input(engine, context, &source)?;
    let mut keys = Vec::with_capacity((args.len() - 1) / 2);
    let mut axis = None;
    for pair in args[1..].chunks_exact(2) {
        poll_cancellation(context)?;
        let values = engine.eval_array(context, &pair[0])?;
        validate_array_input(engine, context, &values)?;
        let key_axis = sort_axis(&source, &values)?;
        if axis.is_some_and(|axis| axis != key_axis) {
            return Err(ErrorKind::Value);
        }
        axis = Some(key_axis);
        keys.push(SortKey {
            values,
            order: sort_order(engine, context, &pair[1])?,
        });
    }
    let axis = axis.ok_or(ErrorKind::Value)?;
    let item_count = match axis {
        SortAxis::Rows => source.rows,
        SortAxis::Columns => source.cols,
    };
    let merge_operations = merge_sort_operation_bound(item_count)?;
    let comparisons = merge_operations
        .checked_mul(u64::try_from(keys.len()).map_err(|_| ErrorKind::Num)?)
        .ok_or(ErrorKind::Num)?;
    let output_cells = cell_count(source.rows, source.cols)?;
    engine.ensure_array_cells(output_cells)?;
    engine.charge_function_iterations(
        context,
        comparisons
            .checked_add(merge_operations)
            .ok_or(ErrorKind::Num)?
            .checked_add(output_cells)
            .ok_or(ErrorKind::Num)?,
    )?;

    let mut indexes = (0..item_count).collect::<Vec<_>>();
    stable_sort_indexes(&mut indexes, context, |left, right| {
        compare_sort_keys(&keys, axis, left, right, context)
    })?;
    let mut data = Vec::with_capacity(source.data.len());
    match axis {
        SortAxis::Rows => {
            for row in indexes {
                for column in 0..source.cols {
                    poll_cancellation(context)?;
                    data.push(source.at(row, column).clone());
                }
            }
        }
        SortAxis::Columns => {
            for row in 0..source.rows {
                for column in &indexes {
                    poll_cancellation(context)?;
                    data.push(source.at(row, *column).clone());
                }
            }
        }
    }
    Ok(Array {
        rows: source.rows,
        cols: source.cols,
        data,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortAxis {
    Rows,
    Columns,
}

#[derive(Debug)]
struct SortKey {
    values: Array,
    order: SortOrder,
}

#[derive(Debug, Clone, Copy)]
enum SortOrder {
    Ascending,
    Descending,
}

fn sort_axis(source: &Array, key: &Array) -> Result<SortAxis, ErrorKind> {
    if key.rows == source.rows && key.cols == 1 {
        Ok(SortAxis::Rows)
    } else if key.rows == 1 && key.cols == source.cols {
        Ok(SortAxis::Columns)
    } else {
        Err(ErrorKind::Value)
    }
}

fn sort_order(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<SortOrder, ErrorKind> {
    match integer(engine, context, expr)? {
        1 => Ok(SortOrder::Ascending),
        -1 => Ok(SortOrder::Descending),
        _ => Err(ErrorKind::Value),
    }
}

fn integer(engine: &Engine<'_>, context: EvalContext<'_>, expr: &Expr) -> Result<i64, ErrorKind> {
    let value = to_number(&engine.eval_scalar(context, expr))?.trunc();
    if !value.is_finite() || value < i64::MIN as f64 || value > i64::MAX as f64 {
        return Err(ErrorKind::Num);
    }
    Ok(value as i64)
}

fn compare_sort_keys(
    keys: &[SortKey],
    axis: SortAxis,
    left: u32,
    right: u32,
    context: EvalContext<'_>,
) -> Result<Ordering, ErrorKind> {
    for key in keys {
        poll_cancellation(context)?;
        let (left_value, right_value) = match axis {
            SortAxis::Rows => (key.values.at(left, 0), key.values.at(right, 0)),
            SortAxis::Columns => (key.values.at(0, left), key.values.at(0, right)),
        };
        if let Some(kind) = [left_value, right_value]
            .into_iter()
            .find_map(|value| match value {
                Value::Error(kind) if kind.is_engine_issue() => Some(*kind),
                _ => None,
            })
        {
            return Err(kind);
        }
        let ordering = compare_sort_values(left_value, right_value);
        let ordering = match key.order {
            SortOrder::Ascending => ordering,
            SortOrder::Descending => ordering.reverse(),
        };
        if ordering != Ordering::Equal {
            return Ok(ordering);
        }
    }
    Ok(Ordering::Equal)
}

pub(super) fn compare_sort_values(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Blank, Value::Blank) => Ordering::Equal,
        (Value::Blank, _) => Ordering::Less,
        (_, Value::Blank) => Ordering::Greater,
        (Value::Number(left), Value::Number(right)) => left.total_cmp(right),
        (Value::Text(left), Value::Text(right)) => compare_text_case_insensitive(left, right),
        (Value::Logical(left), Value::Logical(right)) => left.cmp(right),
        (Value::Error(left), Value::Error(right)) => left.as_str().cmp(right.as_str()),
        (left, right) => sort_value_rank(left).cmp(&sort_value_rank(right)),
    }
}

fn sort_value_rank(value: &Value) -> u8 {
    match value {
        Value::Blank => 0,
        Value::Number(_) => 1,
        Value::Text(_) => 2,
        Value::Logical(_) => 3,
        Value::Error(_) => 4,
    }
}

pub(super) fn stable_sort_indexes(
    indexes: &mut [u32],
    context: EvalContext<'_>,
    mut compare: impl FnMut(u32, u32) -> Result<Ordering, ErrorKind>,
) -> Result<(), ErrorKind> {
    let mut buffer = indexes.to_vec();
    let mut width = 1_usize;
    while width < indexes.len() {
        let block = width.checked_mul(2).ok_or(ErrorKind::Num)?;
        for start in (0..indexes.len()).step_by(block) {
            let middle = start.saturating_add(width).min(indexes.len());
            let end = start.saturating_add(block).min(indexes.len());
            let (mut left, mut right) = (start, middle);
            for output in &mut buffer[start..end] {
                poll_cancellation(context)?;
                let take_left = right >= end
                    || (left < middle
                        && compare(indexes[left], indexes[right])? != Ordering::Greater);
                *output = if take_left {
                    let value = indexes[left];
                    left += 1;
                    value
                } else {
                    let value = indexes[right];
                    right += 1;
                    value
                };
            }
            indexes[start..end].copy_from_slice(&buffer[start..end]);
        }
        width = block;
    }
    Ok(())
}

fn merge_sort_operation_bound(item_count: u32) -> Result<u64, ErrorKind> {
    if item_count <= 1 {
        return Ok(0);
    }
    let levels = u64::from(u32::BITS - (item_count - 1).leading_zeros());
    u64::from(item_count)
        .checked_mul(levels)
        .ok_or(ErrorKind::Num)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::calculation::eval::EvaluationBudget;
    use crate::calculation::limits::CalculationLimitKind;

    #[test]
    fn sort_value_order_matches_existing_dynamic_array_order() {
        assert_eq!(
            compare_sort_values(&Value::Blank, &Value::Number(0.0)),
            Ordering::Less
        );
        assert_eq!(
            compare_sort_values(&Value::Text("a".to_owned()), &Value::Text("A".to_owned())),
            Ordering::Equal
        );
        assert_eq!(
            compare_sort_values(&Value::Logical(false), &Value::Error(ErrorKind::NA)),
            Ordering::Less
        );
    }

    #[test]
    fn fallible_merge_sort_is_stable_and_polls_cancellation() {
        let budget = EvaluationBudget::default();
        let never_cancelled = || false;
        let context = EvalContext::for_cancellable((0, 1, 1), &budget, &never_cancelled);
        let values = [2_u8, 1, 1, 3];
        let mut indexes = [0_u32, 1, 2, 3];
        stable_sort_indexes(&mut indexes, context, |left, right| {
            Ok(values[left as usize].cmp(&values[right as usize]))
        })
        .expect("stable comparison succeeds");
        assert_eq!(indexes, [1, 2, 0, 3]);

        let cancelled = Cell::new(false);
        let is_cancelled = || cancelled.get();
        let context = EvalContext::for_cancellable((0, 1, 1), &budget, &is_cancelled);
        let mut indexes = [0_u32, 1, 2, 3];
        let result = stable_sort_indexes(&mut indexes, context, |left, right| {
            cancelled.set(true);
            Ok(left.cmp(&right))
        });
        assert_eq!(
            result,
            Err(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations
            ))
        );
    }
}
