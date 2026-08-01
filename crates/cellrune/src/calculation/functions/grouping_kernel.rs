use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use super::super::decimal::DecimalTrace;
use super::super::eval::{Engine, EvalContext};
use super::super::scope::ArrayEvaluation;
use super::super::value::{ErrorKind, Value};
use super::array_common::{cell_count, poll_cancellation};
use super::array_sort::{compare_sort_values, stable_sort_indexes};
use super::grouping_options::{FieldRelationship, SortCriterion, TotalDepth, TotalPlacement};

const TOTAL_LABEL: &str = "Total";
const GRAND_TOTAL_LABEL: &str = "Grand Total";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum KeyValue {
    Blank,
    Number(u64),
    Text(String),
    Logical(bool),
    Error(ErrorKind),
}

type SubtotalMembers = HashMap<(usize, Vec<KeyValue>), Vec<u32>>;

impl KeyValue {
    fn from_value(value: &Value) -> Self {
        match value {
            Value::Blank => Self::Blank,
            Value::Number(number) => {
                let normalized = if *number == 0.0 { 0.0 } else { *number };
                Self::Number(normalized.to_bits())
            }
            Value::Text(text) => Self::Text(text.chars().flat_map(char::to_lowercase).collect()),
            Value::Logical(logical) => Self::Logical(*logical),
            Value::Error(kind) => Self::Error(*kind),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AxisGroupKind {
    Detail,
    Subtotal { prefix_len: usize },
    GrandTotal,
}

#[derive(Debug, Clone)]
pub(super) struct AxisGroup {
    pub(super) labels: Vec<Value>,
    pub(super) label_traces: Vec<Option<DecimalTrace>>,
    pub(super) members: Vec<u32>,
    pub(super) kind: AxisGroupKind,
    pub(super) sort_values: Vec<Value>,
    hierarchy_sort_values: Vec<Arc<[Value]>>,
}

impl AxisGroup {
    pub(super) fn matching_prefix_len(&self) -> usize {
        match self.kind {
            AxisGroupKind::Detail => self.labels.len(),
            AxisGroupKind::Subtotal { prefix_len } => prefix_len,
            AxisGroupKind::GrandTotal => 0,
        }
    }
}

pub(super) fn active_rows(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    input_rows: u32,
    header_rows: u32,
    filter: Option<&[bool]>,
) -> Result<Vec<u32>, ErrorKind> {
    engine.charge_function_iterations(context, u64::from(input_rows))?;
    let mut active = Vec::new();
    for row in header_rows..input_rows {
        poll_cancellation(context)?;
        if filter.is_none_or(|filter| filter[row as usize]) {
            active.push(row);
        }
    }
    Ok(active)
}

pub(super) fn build_detail_groups(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    fields: &ArrayEvaluation,
    rows: &[u32],
) -> Result<Vec<AxisGroup>, ErrorKind> {
    let visits = u64::try_from(rows.len())
        .ok()
        .and_then(|rows| rows.checked_mul(u64::from(fields.array.cols)))
        .ok_or(ErrorKind::Num)?;
    engine.charge_function_iterations(context, visits)?;
    let mut indexes = HashMap::<Vec<KeyValue>, usize>::new();
    let mut groups = Vec::<AxisGroup>::new();
    for row in rows {
        poll_cancellation(context)?;
        let mut labels = Vec::with_capacity(fields.array.cols as usize);
        let mut label_traces = Vec::with_capacity(fields.array.cols as usize);
        let mut key = Vec::with_capacity(fields.array.cols as usize);
        for column in 0..fields.array.cols {
            poll_cancellation(context)?;
            let value = fields.array.at(*row, column).clone();
            key.push(KeyValue::from_value(&value));
            labels.push(value);
            label_traces.push(fields.decimal_at(*row, column));
        }
        if let Some(index) = indexes.get(&key).copied() {
            groups[index].members.push(*row);
        } else {
            indexes.insert(key, groups.len());
            groups.push(AxisGroup {
                labels,
                label_traces,
                members: vec![*row],
                kind: AxisGroupKind::Detail,
                sort_values: Vec::new(),
                hierarchy_sort_values: Vec::new(),
            });
        }
    }
    Ok(groups)
}

pub(super) fn sort_detail_groups(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    groups: &mut Vec<AxisGroup>,
    criteria: &[SortCriterion],
    relationship: FieldRelationship,
) -> Result<(), ErrorKind> {
    let item_count = u32::try_from(groups.len()).map_err(|_| ErrorKind::Num)?;
    let field_count = groups.first().map_or(0, |group| group.labels.len());
    let comparison_width = u64::try_from(field_count.max(1))
        .map_err(|_| ErrorKind::Num)?
        .checked_add(criteria.len() as u64)
        .ok_or(ErrorKind::Num)?;
    let comparisons = merge_sort_operation_bound(item_count)?
        .checked_mul(comparison_width)
        .ok_or(ErrorKind::Num)?;
    let setup = u64::from(item_count)
        .checked_mul(3)
        .and_then(|operations| operations.checked_add(u64::try_from(field_count).ok()?))
        .and_then(|operations| operations.checked_add(criteria.len() as u64))
        .ok_or(ErrorKind::Num)?;
    engine.charge_function_iterations(
        context,
        comparisons.checked_add(setup).ok_or(ErrorKind::Num)?,
    )?;
    let field_directions = if relationship == FieldRelationship::Hierarchy
        && criteria
            .iter()
            .all(|criterion| criterion.index < groups.first().map_or(0, |group| group.labels.len()))
    {
        let mut directions = Vec::with_capacity(field_count);
        for _ in 0..field_count {
            poll_cancellation(context)?;
            directions.push(false);
        }
        for criterion in criteria {
            poll_cancellation(context)?;
            directions[criterion.index] = criterion.descending;
        }
        Some(directions)
    } else {
        None
    };
    let mut indexes = Vec::with_capacity(item_count as usize);
    for index in 0..item_count {
        poll_cancellation(context)?;
        indexes.push(index);
    }
    stable_sort_indexes(&mut indexes, context, |left, right| {
        compare_groups(
            context,
            &groups[left as usize],
            &groups[right as usize],
            criteria,
            relationship,
            field_directions.as_deref(),
        )
    })?;
    let mut reordered = Vec::with_capacity(groups.len());
    let mut old = Vec::with_capacity(groups.len());
    for group in std::mem::take(groups) {
        poll_cancellation(context)?;
        old.push(Some(group));
    }
    for index in indexes {
        poll_cancellation(context)?;
        let index = usize::try_from(index).map_err(|_| ErrorKind::Num)?;
        reordered.push(
            old.get_mut(index)
                .and_then(Option::take)
                .ok_or(ErrorKind::Value)?,
        );
    }
    *groups = reordered;
    Ok(())
}

fn compare_groups(
    context: EvalContext<'_>,
    left: &AxisGroup,
    right: &AxisGroup,
    criteria: &[SortCriterion],
    relationship: FieldRelationship,
    field_directions: Option<&[bool]>,
) -> Result<Ordering, ErrorKind> {
    if criteria.is_empty() {
        return compare_value_slices(context, &left.labels, &right.labels);
    }
    if let Some(field_directions) = field_directions {
        for ((left_value, right_value), descending) in
            left.labels.iter().zip(&right.labels).zip(field_directions)
        {
            poll_cancellation(context)?;
            let ordering = compare_group_values(left_value, right_value, *descending);
            if ordering != Ordering::Equal {
                return Ok(ordering);
            }
        }
        return Ok(Ordering::Equal);
    }
    if relationship == FieldRelationship::Hierarchy && !left.hierarchy_sort_values.is_empty() {
        return compare_hierarchy_value_groups(context, left, right, criteria);
    }
    for criterion in criteria {
        poll_cancellation(context)?;
        let field_count = left.labels.len();
        let (left_value, right_value) = if criterion.index < field_count {
            (
                &left.labels[criterion.index],
                &right.labels[criterion.index],
            )
        } else {
            let index = criterion.index - field_count;
            let left_value = left.sort_values.get(index).ok_or(ErrorKind::Value)?;
            let right_value = right.sort_values.get(index).ok_or(ErrorKind::Value)?;
            (left_value, right_value)
        };
        let ordering = compare_group_values(left_value, right_value, criterion.descending);
        if ordering != Ordering::Equal {
            return Ok(ordering);
        }
    }
    compare_value_slices(context, &left.labels, &right.labels)
}

fn compare_hierarchy_value_groups(
    context: EvalContext<'_>,
    left: &AxisGroup,
    right: &AxisGroup,
    criteria: &[SortCriterion],
) -> Result<Ordering, ErrorKind> {
    let field_count = left.labels.len();
    let mut differing_level = None;
    for (index, (left_label, right_label)) in left.labels.iter().zip(&right.labels).enumerate() {
        poll_cancellation(context)?;
        if compare_group_values(left_label, right_label, false) != Ordering::Equal {
            differing_level = Some(index + 1);
            break;
        }
    }
    let Some(level) = differing_level else {
        return Ok(Ordering::Equal);
    };
    for criterion in criteria {
        poll_cancellation(context)?;
        let value_index = criterion
            .index
            .checked_sub(field_count)
            .ok_or(ErrorKind::Value)?;
        let (left_value, right_value) = if level == field_count {
            (
                left.sort_values.get(value_index),
                right.sort_values.get(value_index),
            )
        } else {
            (
                left.hierarchy_sort_values
                    .get(level - 1)
                    .and_then(|values| values.get(value_index)),
                right
                    .hierarchy_sort_values
                    .get(level - 1)
                    .and_then(|values| values.get(value_index)),
            )
        };
        let ordering = compare_group_values(
            left_value.ok_or(ErrorKind::Value)?,
            right_value.ok_or(ErrorKind::Value)?,
            criterion.descending,
        );
        if ordering != Ordering::Equal {
            return Ok(ordering);
        }
    }
    Ok(compare_group_values(
        &left.labels[level - 1],
        &right.labels[level - 1],
        false,
    ))
}

pub(super) fn populate_hierarchy_sort_values(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    groups: &mut [AxisGroup],
    value_count: u32,
    mut aggregate: impl FnMut(u32, &[u32]) -> Result<Value, ErrorKind>,
) -> Result<(), ErrorKind> {
    let field_count = groups.first().map_or(0, |group| group.labels.len());
    let prefix_levels = field_count.saturating_sub(1);
    if prefix_levels == 0 {
        return Ok(());
    }
    let membership_count = groups
        .iter()
        .try_fold(0_u64, |count, group| {
            count.checked_add(group.members.len() as u64)
        })
        .ok_or(ErrorKind::Num)?;
    let components = prefix_component_count(prefix_levels).ok_or(ErrorKind::Num)?;
    let operations = membership_count
        .checked_mul(prefix_levels as u64)
        .and_then(|members| {
            (groups.len() as u64)
                .checked_mul(components)
                .and_then(|keys| keys.checked_mul(2))
                .and_then(|keys| members.checked_add(keys))
        })
        .ok_or(ErrorKind::Num)?;
    engine.charge_function_iterations(context, operations)?;
    let members = collect_subtotal_members(engine, context, groups, prefix_levels)?;
    let mut values = HashMap::with_capacity(members.len());
    for (key, members) in members {
        poll_cancellation(context)?;
        let mut aggregate_values = Vec::with_capacity(value_count as usize);
        for column in 0..value_count {
            poll_cancellation(context)?;
            let value = aggregate(column, &members)?;
            if let Value::Error(kind) = &value
                && kind.is_engine_issue()
            {
                return Err(*kind);
            }
            aggregate_values.push(value);
        }
        values.insert(key, Arc::<[Value]>::from(aggregate_values));
    }
    for group in groups {
        group.hierarchy_sort_values.reserve(prefix_levels);
        for prefix_len in 1..=prefix_levels {
            poll_cancellation(context)?;
            let key = (
                prefix_len,
                key_values(context, &group.labels[..prefix_len])?,
            );
            group
                .hierarchy_sort_values
                .push(values.get(&key).cloned().ok_or(ErrorKind::Value)?);
        }
    }
    Ok(())
}

fn compare_value_slices(
    context: EvalContext<'_>,
    left: &[Value],
    right: &[Value],
) -> Result<Ordering, ErrorKind> {
    for (left, right) in left.iter().zip(right) {
        poll_cancellation(context)?;
        let ordering = compare_group_values(left, right, false);
        if ordering != Ordering::Equal {
            return Ok(ordering);
        }
    }
    Ok(left.len().cmp(&right.len()))
}

fn compare_group_values(left: &Value, right: &Value, descending: bool) -> Ordering {
    match (left, right) {
        (Value::Blank, Value::Blank) => Ordering::Equal,
        (Value::Blank, _) => Ordering::Greater,
        (_, Value::Blank) => Ordering::Less,
        _ => {
            let ordering = compare_sort_values(left, right);
            if descending {
                ordering.reverse()
            } else {
                ordering
            }
        }
    }
}

pub(super) fn add_totals(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    details: Vec<AxisGroup>,
    total_depth: TotalDepth,
    relationship: FieldRelationship,
) -> Result<Vec<AxisGroup>, ErrorKind> {
    let effective_levels = if total_depth.automatic {
        total_depth.levels.min(1)
    } else {
        total_depth.levels
    };
    if effective_levels == 0 {
        return Ok(details);
    }
    let field_count = details.first().map_or(0, |group| group.labels.len());
    if relationship == FieldRelationship::Table && effective_levels > 1 {
        return Err(ErrorKind::Value);
    }
    let subtotal_levels = usize::try_from(effective_levels.saturating_sub(1))
        .map_err(|_| ErrorKind::Num)?
        .min(field_count.saturating_sub(1));
    let membership_count = details
        .iter()
        .try_fold(0_u64, |count, group| {
            count.checked_add(group.members.len() as u64)
        })
        .ok_or(ErrorKind::Num)?;
    let operations = membership_count
        .checked_mul(subtotal_levels as u64 + 1)
        .and_then(|visits| {
            prefix_component_count(subtotal_levels).and_then(|components| {
                (details.len() as u64)
                    .checked_mul(components)
                    .and_then(|keys| keys.checked_mul(3))
                    .and_then(|keys| visits.checked_add(keys))
            })
        })
        .ok_or(ErrorKind::Num)?;
    let label_group_count = (details.len() as u64)
        .checked_mul(subtotal_levels as u64)
        .and_then(|subtotals| subtotals.checked_add(1))
        .ok_or(ErrorKind::Num)?;
    let label_cells = label_group_count
        .checked_mul(field_count as u64)
        .ok_or(ErrorKind::Num)?;
    engine.charge_function_iterations(
        context,
        operations.checked_add(label_cells).ok_or(ErrorKind::Num)?,
    )?;
    let grand_label = if effective_levels > 1 {
        GRAND_TOTAL_LABEL
    } else {
        TOTAL_LABEL
    };
    engine.ensure_text_bytes(grand_label.len())?;
    let grand_label = Value::Text(grand_label.to_owned());

    let mut grand = Some(grand_total(
        engine,
        context,
        &details,
        field_count,
        &grand_label,
    )?);
    let mut subtotals = collect_subtotal_members(engine, context, &details, subtotal_levels)?;
    let mut insertion_levels = Vec::with_capacity(details.len());
    let mut subtotal_count = 0_usize;
    for index in 0..details.len() {
        poll_cancellation(context)?;
        let mut levels = Vec::with_capacity(subtotal_levels);
        match total_depth.placement {
            TotalPlacement::Start => {
                for prefix_len in 1..=subtotal_levels {
                    poll_cancellation(context)?;
                    if prefix_starts(context, &details, index, prefix_len)? {
                        levels.push(prefix_len);
                    }
                }
            }
            TotalPlacement::End => {
                for prefix_len in (1..=subtotal_levels).rev() {
                    poll_cancellation(context)?;
                    if prefix_ends(context, &details, index, prefix_len)? {
                        levels.push(prefix_len);
                    }
                }
            }
        }
        subtotal_count = subtotal_count
            .checked_add(levels.len())
            .ok_or(ErrorKind::Num)?;
        insertion_levels.push(levels);
    }
    let capacity = details
        .len()
        .checked_add(subtotal_count)
        .and_then(|count| count.checked_add(1))
        .ok_or(ErrorKind::Num)?;
    let mut result = Vec::with_capacity(capacity);
    if total_depth.placement == TotalPlacement::Start {
        result.push(grand.take().ok_or(ErrorKind::Value)?);
    }
    for (detail, levels) in details.into_iter().zip(insertion_levels) {
        poll_cancellation(context)?;
        match total_depth.placement {
            TotalPlacement::Start => {
                for prefix_len in levels {
                    poll_cancellation(context)?;
                    result.push(subtotal(
                        context,
                        &detail,
                        prefix_len,
                        field_count,
                        &mut subtotals,
                    )?);
                }
                result.push(detail);
            }
            TotalPlacement::End => {
                let mut pending = Vec::with_capacity(levels.len());
                for prefix_len in levels {
                    poll_cancellation(context)?;
                    pending.push(subtotal(
                        context,
                        &detail,
                        prefix_len,
                        field_count,
                        &mut subtotals,
                    )?);
                }
                result.push(detail);
                for subtotal in pending {
                    poll_cancellation(context)?;
                    result.push(subtotal);
                }
            }
        }
    }
    if total_depth.placement == TotalPlacement::End {
        result.push(grand.take().ok_or(ErrorKind::Value)?);
    }
    debug_assert!(subtotals.is_empty());
    Ok(result)
}

fn grand_total(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    details: &[AxisGroup],
    field_count: usize,
    total_label: &Value,
) -> Result<AxisGroup, ErrorKind> {
    let mut labels = Vec::with_capacity(field_count);
    let mut label_traces = Vec::with_capacity(field_count);
    for _ in 0..field_count {
        poll_cancellation(context)?;
        labels.push(Value::Blank);
        label_traces.push(None);
    }
    if let Some(label) = labels.first_mut() {
        *label = total_label.clone();
    }
    Ok(AxisGroup {
        labels,
        label_traces,
        members: merge_members(engine, context, details.iter())?,
        kind: AxisGroupKind::GrandTotal,
        sort_values: Vec::new(),
        hierarchy_sort_values: Vec::new(),
    })
}

fn subtotal(
    context: EvalContext<'_>,
    detail: &AxisGroup,
    prefix_len: usize,
    field_count: usize,
    subtotals: &mut SubtotalMembers,
) -> Result<AxisGroup, ErrorKind> {
    let key = &detail.labels[..prefix_len];
    let mut labels = Vec::with_capacity(field_count);
    let mut label_traces = Vec::with_capacity(field_count);
    for _ in 0..field_count {
        poll_cancellation(context)?;
        labels.push(Value::Blank);
        label_traces.push(None);
    }
    labels[..prefix_len].clone_from_slice(key);
    label_traces[..prefix_len].copy_from_slice(&detail.label_traces[..prefix_len]);
    let map_key = (prefix_len, key_values(context, key)?);
    Ok(AxisGroup {
        labels,
        label_traces,
        members: subtotals.remove(&map_key).ok_or(ErrorKind::Value)?,
        kind: AxisGroupKind::Subtotal { prefix_len },
        sort_values: Vec::new(),
        hierarchy_sort_values: Vec::new(),
    })
}

fn collect_subtotal_members(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    details: &[AxisGroup],
    subtotal_levels: usize,
) -> Result<SubtotalMembers, ErrorKind> {
    let mut subtotals = SubtotalMembers::new();
    for detail in details {
        for prefix_len in 1..=subtotal_levels {
            poll_cancellation(context)?;
            let key = (
                prefix_len,
                key_values(context, &detail.labels[..prefix_len])?,
            );
            subtotals
                .entry(key)
                .or_default()
                .extend(detail.members.iter().copied());
        }
    }
    for members in subtotals.values_mut() {
        sort_member_rows(engine, context, members)?;
    }
    Ok(subtotals)
}

fn merge_members<'a>(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    groups: impl Iterator<Item = &'a AxisGroup>,
) -> Result<Vec<u32>, ErrorKind> {
    let mut members = Vec::new();
    for group in groups {
        for member in &group.members {
            poll_cancellation(context)?;
            members.push(*member);
        }
    }
    sort_member_rows(engine, context, &mut members)?;
    Ok(members)
}

fn sort_member_rows(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    members: &mut [u32],
) -> Result<(), ErrorKind> {
    let item_count = u32::try_from(members.len()).map_err(|_| ErrorKind::Num)?;
    engine.charge_function_iterations(context, merge_sort_operation_bound(item_count)?)?;
    stable_sort_indexes(members, context, |left, right| Ok(left.cmp(&right)))
}

fn prefix_starts(
    context: EvalContext<'_>,
    groups: &[AxisGroup],
    index: usize,
    prefix_len: usize,
) -> Result<bool, ErrorKind> {
    Ok(index == 0
        || compare_value_slices(
            context,
            &groups[index - 1].labels[..prefix_len],
            &groups[index].labels[..prefix_len],
        )? != Ordering::Equal)
}

fn prefix_ends(
    context: EvalContext<'_>,
    groups: &[AxisGroup],
    index: usize,
    prefix_len: usize,
) -> Result<bool, ErrorKind> {
    Ok(index + 1 == groups.len()
        || compare_value_slices(
            context,
            &groups[index].labels[..prefix_len],
            &groups[index + 1].labels[..prefix_len],
        )? != Ordering::Equal)
}

pub(super) fn intersect_members(
    context: EvalContext<'_>,
    left: &[u32],
    right: &[u32],
) -> Result<Vec<u32>, ErrorKind> {
    let (mut left_index, mut right_index) = (0, 0);
    let mut result = Vec::new();
    while left_index < left.len() && right_index < right.len() {
        poll_cancellation(context)?;
        match left[left_index].cmp(&right[right_index]) {
            Ordering::Less => left_index += 1,
            Ordering::Greater => right_index += 1,
            Ordering::Equal => {
                result.push(left[left_index]);
                left_index += 1;
                right_index += 1;
            }
        }
    }
    Ok(result)
}

pub(super) struct ParentMemberIndex<'a> {
    active_rows: &'a [u32],
    members: HashMap<(usize, Vec<KeyValue>), Vec<u32>>,
}

impl ParentMemberIndex<'_> {
    pub(super) fn parent_of(
        &self,
        context: EvalContext<'_>,
        group: &AxisGroup,
    ) -> Result<&[u32], ErrorKind> {
        let parent_prefix = group.matching_prefix_len().saturating_sub(1);
        if parent_prefix == 0 {
            return Ok(self.active_rows);
        }
        let key = (
            parent_prefix,
            key_values(context, &group.labels[..parent_prefix])?,
        );
        Ok(self.members.get(&key).map_or(&[], Vec::as_slice))
    }
}

pub(super) fn build_parent_member_index<'a>(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    fields: &ArrayEvaluation,
    active_rows: &'a [u32],
) -> Result<ParentMemberIndex<'a>, ErrorKind> {
    let prefix_levels = fields.array.cols.saturating_sub(1);
    let visits = u64::try_from(active_rows.len())
        .ok()
        .and_then(|rows| {
            prefix_component_count(prefix_levels as usize)
                .and_then(|components| rows.checked_mul(components))
        })
        .ok_or(ErrorKind::Num)?;
    engine.charge_function_iterations(context, visits)?;
    let mut members = HashMap::<(usize, Vec<KeyValue>), Vec<u32>>::new();
    for row in active_rows {
        poll_cancellation(context)?;
        for prefix_len in 1..=prefix_levels as usize {
            poll_cancellation(context)?;
            let mut values = Vec::with_capacity(prefix_len);
            for column in 0..prefix_len {
                poll_cancellation(context)?;
                values.push(KeyValue::from_value(fields.array.at(*row, column as u32)));
            }
            let key = (prefix_len, values);
            members.entry(key).or_default().push(*row);
        }
    }
    Ok(ParentMemberIndex {
        active_rows,
        members,
    })
}

fn key_values(context: EvalContext<'_>, values: &[Value]) -> Result<Vec<KeyValue>, ErrorKind> {
    let mut key = Vec::with_capacity(values.len());
    for value in values {
        poll_cancellation(context)?;
        key.push(KeyValue::from_value(value));
    }
    Ok(key)
}

fn prefix_component_count(levels: usize) -> Option<u64> {
    let levels = u64::try_from(levels).ok()?;
    levels.checked_mul(levels.checked_add(1)?)?.checked_div(2)
}

pub(super) fn ensure_output_shape(
    engine: &Engine<'_>,
    rows: u32,
    columns: u32,
) -> Result<usize, ErrorKind> {
    let cells = cell_count(rows, columns)?;
    engine.ensure_array_cells(cells)?;
    usize::try_from(cells).map_err(|_| ErrorKind::Num)
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
