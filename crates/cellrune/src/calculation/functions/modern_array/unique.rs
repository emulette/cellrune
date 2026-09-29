use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};

use super::super::super::coerce::compare_text_case_insensitive;
use super::super::array_common::poll_cancellation;
use super::{Array, Engine, ErrorKind, EvalContext, Expr, Value, cell_count, optional_logical};

struct Group {
    first: u32,
    count: u32,
    next: Option<usize>,
}

pub(super) fn unique(
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
    let hash_state = RandomState::new();
    let mut buckets = HashMap::<u64, usize>::new();
    let mut groups = Vec::<Group>::new();
    for candidate in 0..item_count {
        let hash = item_hash(engine, context, &source, candidate, by_column, &hash_state)?;
        let head = buckets.get(&hash).copied();
        let mut current = head;
        let mut matched = false;
        while let Some(index) = current {
            let group = &mut groups[index];
            if items_equal(engine, context, &source, candidate, group.first, by_column)? {
                group.count += 1;
                matched = true;
                break;
            }
            current = group.next;
        }
        if !matched {
            buckets.insert(hash, groups.len());
            groups.push(Group {
                first: candidate,
                count: 1,
                next: head,
            });
        }
    }
    let mut selected = Vec::new();
    for group in groups {
        charge_work(engine, context, 1)?;
        if !exactly_once || group.count == 1 {
            selected.push(group.first);
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
    for row in 0..rows {
        for column in 0..cols {
            let value = if by_column {
                source.at(row, selected[column as usize])
            } else {
                source.at(selected[row as usize], column)
            };
            charge_work(engine, context, 1)?;
            if let Value::Text(text) = value {
                charge_work(engine, context, text.len() as u64)?;
            }
            data.push(value.clone());
        }
    }
    Ok(Array { rows, cols, data })
}

fn item_value(source: &Array, item: u32, offset: u32, by_column: bool) -> &Value {
    if by_column {
        source.at(offset, item)
    } else {
        source.at(item, offset)
    }
}

fn item_hash(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    source: &Array,
    item: u32,
    by_column: bool,
    state: &RandomState,
) -> Result<u64, ErrorKind> {
    let mut hasher = state.build_hasher();
    let width = if by_column { source.rows } else { source.cols };
    for offset in 0..width {
        charge_work(engine, context, 1)?;
        let value = item_value(source, item, offset, by_column);
        std::mem::discriminant(value).hash(&mut hasher);
        match value {
            Value::Blank => {}
            Value::Number(number) => {
                let normalized = if *number == 0.0 { 0.0 } else { *number };
                normalized.to_bits().hash(&mut hasher);
            }
            Value::Logical(value) => value.hash(&mut hasher),
            Value::Error(kind) => kind.hash(&mut hasher),
            Value::Text(text) => {
                // Hash the same Unicode lowercase form that `items_equal` compares, so letters
                // outside ASCII group case-insensitively as in COUNTIF and GROUPBY.
                charge_work(engine, context, text.len() as u64)?;
                for character in text.chars().flat_map(char::to_lowercase) {
                    character.hash(&mut hasher);
                }
            }
        }
    }
    Ok(hasher.finish())
}

fn items_equal(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    source: &Array,
    left: u32,
    right: u32,
    by_column: bool,
) -> Result<bool, ErrorKind> {
    let width = if by_column { source.rows } else { source.cols };
    for offset in 0..width {
        charge_work(engine, context, 1)?;
        let left = item_value(source, left, offset, by_column);
        let right = item_value(source, right, offset, by_column);
        if let (Value::Text(left), Value::Text(right)) = (left, right) {
            charge_work(engine, context, left.len().max(right.len()) as u64)?;
            if compare_text_case_insensitive(left, right) != Ordering::Equal {
                return Ok(false);
            }
        } else if left != right {
            return Ok(false);
        }
    }
    Ok(true)
}

fn charge_work(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    iterations: u64,
) -> Result<(), ErrorKind> {
    poll_cancellation(context)?;
    engine.charge_function_iterations(context, iterations)
}
