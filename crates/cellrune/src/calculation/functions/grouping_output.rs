use std::borrow::Cow;

use super::super::eval::{Engine, EvalContext};
use super::super::scope::ArrayEvaluation;
use super::super::value::{ErrorKind, Value, number_to_general_text};
use super::array_common::poll_cancellation;
use super::grouped::{AggregateCallable, invoke_aggregate};
use super::grouping_kernel::{
    AxisGroup, AxisGroupKind, ParentMemberIndex, build_parent_member_index, ensure_output_shape,
    intersect_members,
};
use super::grouping_options::{FieldHeaders, RelativeSet, output_headers_are_shown};

pub(super) struct GroupByOutput<'a> {
    pub(super) headers: FieldHeaders,
    pub(super) input_has_header: bool,
    pub(super) fields: &'a ArrayEvaluation,
    pub(super) values: &'a ArrayEvaluation,
    pub(super) active_members: &'a [u32],
    pub(super) aggregate: &'a AggregateCallable,
    pub(super) groups: &'a [AxisGroup],
}

pub(super) struct PivotOutput<'a> {
    pub(super) headers: FieldHeaders,
    pub(super) input_has_header: bool,
    pub(super) relative_to: RelativeSet,
    pub(super) row_fields: &'a ArrayEvaluation,
    pub(super) column_fields: &'a ArrayEvaluation,
    pub(super) values: &'a ArrayEvaluation,
    pub(super) active_members: &'a [u32],
    pub(super) aggregate: &'a AggregateCallable,
    pub(super) row_groups: &'a [AxisGroup],
    pub(super) column_groups: &'a [AxisGroup],
}

pub(super) fn build_groupby_output(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    output: GroupByOutput<'_>,
) -> Result<ArrayEvaluation, ErrorKind> {
    let show_headers = output_headers_are_shown(output.headers);
    let header_rows = u32::from(show_headers);
    let rows = header_rows
        .checked_add(u32::try_from(output.groups.len()).map_err(|_| ErrorKind::Num)?)
        .ok_or(ErrorKind::Num)?;
    let cols = output
        .fields
        .array
        .cols
        .checked_add(output.values.array.cols)
        .ok_or(ErrorKind::Num)?;
    let capacity = ensure_output_shape(engine, rows, cols)?;
    engine.charge_function_iterations(
        context,
        u64::try_from(capacity).map_err(|_| ErrorKind::Num)?,
    )?;
    let mut data = Vec::with_capacity(capacity);
    let mut traces = Vec::with_capacity(capacity);
    if show_headers {
        for column in 0..output.fields.array.cols {
            poll_cancellation(context)?;
            let (value, trace) = output_header(
                engine,
                output.headers,
                output.input_has_header,
                output.fields,
                column,
                "Row Field",
            )?;
            data.push(value);
            traces.push(trace);
        }
        for column in 0..output.values.array.cols {
            poll_cancellation(context)?;
            let (value, trace) = output_header(
                engine,
                output.headers,
                output.input_has_header,
                output.values,
                column,
                "Value",
            )?;
            data.push(value);
            traces.push(trace);
        }
    }
    for group in output.groups {
        poll_cancellation(context)?;
        for (column, (label, trace)) in group.labels.iter().zip(&group.label_traces).enumerate() {
            poll_cancellation(context)?;
            let (label, trace) = output_group_label(group, column, label, *trace);
            data.push(label);
            traces.push(trace);
        }
        for column in 0..output.values.array.cols {
            poll_cancellation(context)?;
            let evaluated = invoke_aggregate(
                engine,
                context,
                output.aggregate,
                output.values,
                column,
                &group.members,
                output.active_members,
            )?;
            data.push(evaluated.value);
            traces.push(evaluated.decimal_trace);
        }
    }
    Ok(ArrayEvaluation {
        array: super::super::runtime::Array { rows, cols, data },
        decimal_traces: traces,
    })
}

pub(super) fn build_pivot_output(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    pivot: PivotOutput<'_>,
) -> Result<ArrayEvaluation, ErrorKind> {
    let show_field_headers = output_headers_are_shown(pivot.headers);
    let field_header_rows = if show_field_headers { 2 } else { 0 };
    let header_rows = pivot
        .column_fields
        .array
        .cols
        .checked_add(field_header_rows)
        .ok_or(ErrorKind::Num)?;
    let rows = header_rows
        .checked_add(u32::try_from(pivot.row_groups.len()).map_err(|_| ErrorKind::Num)?)
        .ok_or(ErrorKind::Num)?;
    let value_columns = u32::try_from(pivot.column_groups.len())
        .ok()
        .and_then(|groups| groups.checked_mul(pivot.values.array.cols))
        .ok_or(ErrorKind::Num)?;
    let cols = pivot
        .row_fields
        .array
        .cols
        .checked_add(value_columns)
        .ok_or(ErrorKind::Num)?;
    let capacity = ensure_output_shape(engine, rows, cols)?;
    engine.charge_function_iterations(
        context,
        u64::try_from(capacity).map_err(|_| ErrorKind::Num)?,
    )?;
    let mut output = ArrayEvaluation::untracked(super::super::runtime::Array {
        rows,
        cols,
        data: vec![Value::Text(String::new()); capacity],
    });
    let uses_relative_set = pivot.aggregate.argument_count == 2;
    let row_parents = if uses_relative_set && pivot.relative_to == RelativeSet::ParentRow {
        Some(build_parent_member_index(
            engine,
            context,
            pivot.row_fields,
            pivot.active_members,
        )?)
    } else {
        None
    };
    let column_parents = if uses_relative_set && pivot.relative_to == RelativeSet::ParentColumn {
        Some(build_parent_member_index(
            engine,
            context,
            pivot.column_fields,
            pivot.active_members,
        )?)
    } else {
        None
    };
    let row_denominators = if uses_relative_set {
        match pivot.relative_to {
            RelativeSet::Row => Some(axis_denominators(context, pivot.row_groups)?),
            RelativeSet::ParentRow => Some(parent_denominators(
                engine,
                context,
                row_parents.as_ref().ok_or(ErrorKind::Value)?,
                pivot.row_groups,
            )?),
            RelativeSet::Column | RelativeSet::Grand | RelativeSet::ParentColumn => None,
        }
    } else {
        None
    };
    let column_denominators = if uses_relative_set {
        match pivot.relative_to {
            RelativeSet::Column => Some(axis_denominators(context, pivot.column_groups)?),
            RelativeSet::ParentColumn => Some(parent_denominators(
                engine,
                context,
                column_parents.as_ref().ok_or(ErrorKind::Value)?,
                pivot.column_groups,
            )?),
            RelativeSet::Row | RelativeSet::Grand | RelativeSet::ParentRow => None,
        }
    } else {
        None
    };
    if let Some(anchor) = output.array.data.first_mut() {
        *anchor = Value::Text(String::new());
    }
    write_pivot_headers(engine, context, &mut output, &pivot, header_rows)?;
    for (row_index, row_group) in pivot.row_groups.iter().enumerate() {
        poll_cancellation(context)?;
        let output_row = header_rows
            .checked_add(u32::try_from(row_index).map_err(|_| ErrorKind::Num)?)
            .ok_or(ErrorKind::Num)?;
        for column in 0..pivot.row_fields.array.cols {
            poll_cancellation(context)?;
            let (label, trace) = output_group_label(
                row_group,
                column as usize,
                &row_group.labels[column as usize],
                row_group.label_traces[column as usize],
            );
            set_cell(&mut output, output_row, column, label, trace)?;
        }
        for (column_index, column_group) in pivot.column_groups.iter().enumerate() {
            let intersection_visits = row_group
                .members
                .len()
                .checked_add(column_group.members.len())
                .and_then(|visits| u64::try_from(visits).ok())
                .ok_or(ErrorKind::Num)?;
            engine.charge_function_iterations(context, intersection_visits)?;
            let subset = intersect_members(context, &row_group.members, &column_group.members)?;
            let relative_members = relative_members(
                uses_relative_set,
                pivot.relative_to,
                row_index,
                column_index,
                pivot.active_members,
                row_denominators.as_deref(),
                column_denominators.as_deref(),
            )?;
            for value_column in 0..pivot.values.array.cols {
                poll_cancellation(context)?;
                let group_offset = u32::try_from(column_index)
                    .map_err(|_| ErrorKind::Num)?
                    .checked_mul(pivot.values.array.cols)
                    .ok_or(ErrorKind::Num)?;
                let output_column = pivot
                    .row_fields
                    .array
                    .cols
                    .checked_add(group_offset)
                    .and_then(|column| column.checked_add(value_column))
                    .ok_or(ErrorKind::Num)?;
                if subset.is_empty() {
                    set_cell(
                        &mut output,
                        output_row,
                        output_column,
                        Value::Text(String::new()),
                        None,
                    )?;
                } else {
                    let evaluated = invoke_aggregate(
                        engine,
                        context,
                        pivot.aggregate,
                        pivot.values,
                        value_column,
                        &subset,
                        relative_members,
                    )?;
                    set_cell(
                        &mut output,
                        output_row,
                        output_column,
                        evaluated.value,
                        evaluated.decimal_trace,
                    )?;
                }
            }
        }
    }
    Ok(output)
}

fn write_pivot_headers(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    output: &mut ArrayEvaluation,
    pivot: &PivotOutput<'_>,
    header_rows: u32,
) -> Result<(), ErrorKind> {
    let show_field_headers = output_headers_are_shown(pivot.headers);
    if show_field_headers {
        let value_start = pivot.row_fields.array.cols;
        let column_header = joined_column_field_header(
            engine,
            context,
            pivot.headers,
            pivot.input_has_header,
            pivot.column_fields,
        )?;
        set_cell(output, 0, value_start, column_header, None)?;
        let row = header_rows - 1;
        for column in 0..pivot.row_fields.array.cols {
            poll_cancellation(context)?;
            let (value, trace) = output_header(
                engine,
                pivot.headers,
                pivot.input_has_header,
                pivot.row_fields,
                column,
                "Row Field",
            )?;
            set_cell(output, row, column, value, trace)?;
        }
    }
    for (group_index, group) in pivot.column_groups.iter().enumerate() {
        for value_column in 0..pivot.values.array.cols {
            poll_cancellation(context)?;
            let group_offset = u32::try_from(group_index)
                .map_err(|_| ErrorKind::Num)?
                .checked_mul(pivot.values.array.cols)
                .and_then(|offset| offset.checked_add(value_column))
                .ok_or(ErrorKind::Num)?;
            let output_column = pivot
                .row_fields
                .array
                .cols
                .checked_add(group_offset)
                .ok_or(ErrorKind::Num)?;
            for level in 0..pivot.column_fields.array.cols {
                poll_cancellation(context)?;
                let (label, trace) = output_group_label(
                    group,
                    level as usize,
                    &group.labels[level as usize],
                    group.label_traces[level as usize],
                );
                set_cell(
                    output,
                    level + u32::from(show_field_headers),
                    output_column,
                    label,
                    trace,
                )?;
            }
            if show_field_headers {
                let (value, trace) = output_header(
                    engine,
                    pivot.headers,
                    pivot.input_has_header,
                    pivot.values,
                    value_column,
                    "Value",
                )?;
                set_cell(output, header_rows - 1, output_column, value, trace)?;
            }
        }
    }
    Ok(())
}

fn joined_column_field_header(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    headers: FieldHeaders,
    input_has_header: bool,
    fields: &ArrayEvaluation,
) -> Result<Value, ErrorKind> {
    let use_existing = input_has_header
        && matches!(
            headers,
            FieldHeaders::Automatic | FieldHeaders::ExistingShown
        );
    let header = if use_existing {
        let mut parts = Vec::with_capacity(fields.array.cols as usize);
        let mut byte_len = 0_usize;
        for column in 0..fields.array.cols {
            poll_cancellation(context)?;
            let part = rendered_header_part(fields.array.at(0, column))?;
            byte_len = byte_len
                .checked_add(part.len())
                .and_then(|len| {
                    if column == 0 {
                        Some(len)
                    } else {
                        len.checked_add(2)
                    }
                })
                .ok_or(ErrorKind::Num)?;
            parts.push(part);
        }
        engine.ensure_text_bytes(byte_len)?;
        let mut joined = String::with_capacity(byte_len);
        for (index, part) in parts.into_iter().enumerate() {
            poll_cancellation(context)?;
            if index > 0 {
                joined.push_str(", ");
            }
            joined.push_str(&part);
        }
        joined
    } else {
        "Column Field".to_owned()
    };
    engine.ensure_text_bytes(header.len())?;
    Ok(Value::Text(header))
}

fn rendered_header_part(value: &Value) -> Result<Cow<'_, str>, ErrorKind> {
    match value {
        Value::Text(text) => Ok(Cow::Borrowed(text)),
        Value::Blank => Ok(Cow::Borrowed("")),
        Value::Number(number) => Ok(Cow::Owned(number_to_general_text(*number))),
        Value::Logical(true) => Ok(Cow::Borrowed("TRUE")),
        Value::Logical(false) => Ok(Cow::Borrowed("FALSE")),
        Value::Error(kind) => Err(*kind),
    }
}

fn relative_members<'a>(
    uses_relative_set: bool,
    relative_to: RelativeSet,
    row_index: usize,
    column_index: usize,
    active_members: &'a [u32],
    row_denominators: Option<&'a [&'a [u32]]>,
    column_denominators: Option<&'a [&'a [u32]]>,
) -> Result<&'a [u32], ErrorKind> {
    if !uses_relative_set {
        return Ok(active_members);
    }
    match relative_to {
        RelativeSet::Column | RelativeSet::ParentColumn => column_denominators
            .and_then(|denominators| denominators.get(column_index))
            .copied()
            .ok_or(ErrorKind::Value),
        RelativeSet::Row | RelativeSet::ParentRow => row_denominators
            .and_then(|denominators| denominators.get(row_index))
            .copied()
            .ok_or(ErrorKind::Value),
        RelativeSet::Grand => Ok(active_members),
    }
}

fn parent_denominators<'a>(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    parents: &'a ParentMemberIndex<'_>,
    groups: &[AxisGroup],
) -> Result<Vec<&'a [u32]>, ErrorKind> {
    let key_components = groups
        .iter()
        .try_fold(0_u64, |components, group| {
            components.checked_add(group.matching_prefix_len().saturating_sub(1) as u64)
        })
        .ok_or(ErrorKind::Num)?;
    engine.charge_function_iterations(context, key_components)?;
    let mut denominators = Vec::with_capacity(groups.len());
    for group in groups {
        poll_cancellation(context)?;
        denominators.push(parents.parent_of(context, group)?);
    }
    Ok(denominators)
}

fn axis_denominators<'a>(
    context: EvalContext<'_>,
    groups: &'a [AxisGroup],
) -> Result<Vec<&'a [u32]>, ErrorKind> {
    let mut denominators = Vec::with_capacity(groups.len());
    for group in groups {
        poll_cancellation(context)?;
        denominators.push(group.members.as_slice());
    }
    Ok(denominators)
}

fn output_header(
    engine: &Engine<'_>,
    headers: FieldHeaders,
    input_has_header: bool,
    source: &ArrayEvaluation,
    column: u32,
    generated_prefix: &str,
) -> Result<(Value, Option<super::super::decimal::DecimalTrace>), ErrorKind> {
    let use_existing = input_has_header
        && matches!(
            headers,
            FieldHeaders::Automatic | FieldHeaders::ExistingShown
        );
    if use_existing {
        return Ok((
            source.array.at(0, column).clone(),
            source.decimal_at(0, column),
        ));
    }
    let text = format!("{generated_prefix} {}", u64::from(column) + 1);
    engine.ensure_text_bytes(text.len())?;
    Ok((Value::Text(text), None))
}

fn set_cell(
    output: &mut ArrayEvaluation,
    row: u32,
    column: u32,
    value: Value,
    trace: Option<super::super::decimal::DecimalTrace>,
) -> Result<(), ErrorKind> {
    let index = u64::from(row)
        .checked_mul(u64::from(output.array.cols))
        .and_then(|offset| offset.checked_add(u64::from(column)))
        .and_then(|index| usize::try_from(index).ok())
        .ok_or(ErrorKind::Num)?;
    *output.array.data.get_mut(index).ok_or(ErrorKind::Num)? = value;
    *output.decimal_traces.get_mut(index).ok_or(ErrorKind::Num)? = trace;
    Ok(())
}

fn output_group_label(
    group: &AxisGroup,
    column: usize,
    value: &Value,
    trace: Option<super::super::decimal::DecimalTrace>,
) -> (Value, Option<super::super::decimal::DecimalTrace>) {
    let structural_blank = match group.kind {
        AxisGroupKind::Detail => false,
        AxisGroupKind::Subtotal { prefix_len } => column >= prefix_len,
        AxisGroupKind::GrandTotal => column > 0,
    };
    if structural_blank {
        (Value::Text(String::new()), None)
    } else {
        (value.clone(), trace)
    }
}
