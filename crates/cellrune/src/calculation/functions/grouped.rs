use std::sync::Arc;

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::operators::apply_binary;
use super::super::runtime::Array;
use super::super::scope::{ArrayEvaluation, CallableValue, ScalarEvaluation, ScopeValue};
use super::super::value::{ErrorKind, Value};
use super::array_common::{poll_cancellation, validate_array_input};
use super::grouping_kernel::{
    AxisGroup, active_rows, add_totals, build_detail_groups, populate_hierarchy_sort_values,
    sort_detail_groups,
};
use super::grouping_options::{
    parse_field_headers, parse_field_relationship, parse_filter, parse_relative_to,
    parse_sort_order, parse_total_depth, resolve_input_header, validate_relationship_total_depth,
};
use super::grouping_output::{
    GroupByOutput, PivotOutput, build_groupby_output, build_pivot_output,
};
use super::kernel::{AggregateFunction, GroupedArrayFunction, GroupedFunction};
use super::{builtin_aggregate_capability, dynamic};

#[derive(Debug, Clone)]
pub(super) struct AggregateCallable {
    pub(super) callable: CallableValue,
    pub(super) argument_count: usize,
}

pub(super) fn call_scalar(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: GroupedFunction,
    args: &[Expr],
) -> Value {
    match function {
        GroupedFunction::PercentOf => percent_of(engine, context, args),
        GroupedFunction::GroupBy => first_value(group_by(engine, context, args)),
        GroupedFunction::PivotBy => first_value(pivot_by(engine, context, args)),
    }
}

pub(super) fn call_array(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: GroupedArrayFunction,
    args: &[Expr],
) -> Result<ArrayEvaluation, ErrorKind> {
    match function {
        GroupedArrayFunction::GroupBy => group_by(engine, context, args),
        GroupedArrayFunction::PivotBy => pivot_by(engine, context, args),
    }
}

fn first_value(result: Result<ArrayEvaluation, ErrorKind>) -> Value {
    match result {
        Ok(evaluated) => evaluated
            .array
            .data
            .into_iter()
            .next()
            .unwrap_or(Value::Error(ErrorKind::Calc)),
        Err(kind) => Value::Error(kind),
    }
}

fn percent_of(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [subset, all] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let subset = super::aggregate::call(
        engine,
        context,
        AggregateFunction::Sum,
        std::slice::from_ref(subset),
    );
    let all = super::aggregate::call(
        engine,
        context,
        AggregateFunction::Sum,
        std::slice::from_ref(all),
    );
    percent_values(engine, subset, all)
}

pub(super) fn percent_of_scope_values(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[ScopeValue],
) -> Value {
    let [subset, all] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let subset = super::aggregate::call_scope_values(
        engine,
        context,
        AggregateFunction::Sum,
        std::slice::from_ref(subset),
    );
    let all = super::aggregate::call_scope_values(
        engine,
        context,
        AggregateFunction::Sum,
        std::slice::from_ref(all),
    );
    percent_values(engine, subset, all)
}

fn percent_values(engine: &Engine<'_>, subset: Value, all: Value) -> Value {
    apply_binary(
        super::super::ast::BinaryOp::Divide,
        &subset,
        &all,
        engine.calculation_limits().max_text_bytes(),
    )
}

fn group_by(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<ArrayEvaluation, ErrorKind> {
    let [fields_expr, values_expr, callable_expr, optional @ ..] = args else {
        return Err(ErrorKind::Value);
    };
    let fields = engine.eval_array_with_trace(context, fields_expr)?;
    let values = engine.eval_array_with_trace(context, values_expr)?;
    validate_group_inputs(engine, context, &[&fields, &values])?;
    let aggregate = resolve_aggregate(engine, context, callable_expr)?;
    let headers = parse_field_headers(engine, context, optional.first())?;
    let has_header = resolve_input_header(headers, &values.array);
    let total_depth = parse_total_depth(engine, context, optional.get(1), fields.array.cols)?;
    let sort = parse_sort_order(
        engine,
        context,
        optional.get(2),
        fields.array.cols as usize,
        values.array.cols as usize,
    )?;
    let relationship = parse_field_relationship(engine, context, optional.get(4))?;
    validate_relationship_total_depth(relationship, total_depth)?;
    let filter = parse_filter(engine, context, optional.get(3), fields.array.rows)?;
    let active = active_rows(
        engine,
        context,
        fields.array.rows,
        u32::from(has_header),
        filter.as_deref(),
    )?;
    if active.is_empty() {
        return Err(ErrorKind::Calc);
    }
    let mut details = build_detail_groups(engine, context, &fields, &active)?;
    if sort
        .iter()
        .any(|criterion| criterion.index >= fields.array.cols as usize)
    {
        populate_sort_values(
            engine,
            context,
            &aggregate,
            &values,
            &active,
            &mut details,
            relationship == super::grouping_options::FieldRelationship::Hierarchy,
        )?;
    }
    sort_detail_groups(engine, context, &mut details, &sort, relationship)?;
    let groups = add_totals(engine, context, details, total_depth, relationship)?;
    build_groupby_output(
        engine,
        context,
        GroupByOutput {
            headers,
            input_has_header: has_header,
            fields: &fields,
            values: &values,
            active_members: &active,
            aggregate: &aggregate,
            groups: &groups,
        },
    )
}

fn pivot_by(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<ArrayEvaluation, ErrorKind> {
    let [
        row_expr,
        column_expr,
        values_expr,
        callable_expr,
        optional @ ..,
    ] = args
    else {
        return Err(ErrorKind::Value);
    };
    let row_fields = engine.eval_array_with_trace(context, row_expr)?;
    let column_fields = engine.eval_array_with_trace(context, column_expr)?;
    let values = engine.eval_array_with_trace(context, values_expr)?;
    validate_group_inputs(engine, context, &[&row_fields, &column_fields, &values])?;
    let aggregate = resolve_aggregate(engine, context, callable_expr)?;
    let headers = parse_field_headers(engine, context, optional.first())?;
    let has_header = resolve_input_header(headers, &values.array);
    let row_totals = parse_total_depth(engine, context, optional.get(1), row_fields.array.cols)?;
    let row_sort = parse_sort_order(
        engine,
        context,
        optional.get(2),
        row_fields.array.cols as usize,
        values.array.cols as usize,
    )?;
    let column_totals =
        parse_total_depth(engine, context, optional.get(3), column_fields.array.cols)?;
    let column_sort = parse_sort_order(
        engine,
        context,
        optional.get(4),
        column_fields.array.cols as usize,
        values.array.cols as usize,
    )?;
    let filter = parse_filter(engine, context, optional.get(5), row_fields.array.rows)?;
    let relative_to = parse_relative_to(engine, context, optional.get(6))?;
    let active = active_rows(
        engine,
        context,
        row_fields.array.rows,
        u32::from(has_header),
        filter.as_deref(),
    )?;
    if active.is_empty() {
        return Err(ErrorKind::Calc);
    }
    let mut row_details = build_detail_groups(engine, context, &row_fields, &active)?;
    let mut column_details = build_detail_groups(engine, context, &column_fields, &active)?;
    if row_sort
        .iter()
        .any(|criterion| criterion.index >= row_fields.array.cols as usize)
    {
        populate_sort_values(
            engine,
            context,
            &aggregate,
            &values,
            &active,
            &mut row_details,
            true,
        )?;
    }
    if column_sort
        .iter()
        .any(|criterion| criterion.index >= column_fields.array.cols as usize)
    {
        populate_sort_values(
            engine,
            context,
            &aggregate,
            &values,
            &active,
            &mut column_details,
            true,
        )?;
    }
    sort_detail_groups(
        engine,
        context,
        &mut row_details,
        &row_sort,
        super::grouping_options::FieldRelationship::Hierarchy,
    )?;
    sort_detail_groups(
        engine,
        context,
        &mut column_details,
        &column_sort,
        super::grouping_options::FieldRelationship::Hierarchy,
    )?;
    let rows = add_totals(
        engine,
        context,
        row_details,
        row_totals,
        super::grouping_options::FieldRelationship::Hierarchy,
    )?;
    let columns = add_totals(
        engine,
        context,
        column_details,
        column_totals,
        super::grouping_options::FieldRelationship::Hierarchy,
    )?;
    build_pivot_output(
        engine,
        context,
        PivotOutput {
            headers,
            input_has_header: has_header,
            relative_to,
            row_fields: &row_fields,
            column_fields: &column_fields,
            values: &values,
            active_members: &active,
            aggregate: &aggregate,
            row_groups: &rows,
            column_groups: &columns,
        },
    )
}

fn validate_group_inputs(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    inputs: &[&ArrayEvaluation],
) -> Result<(), ErrorKind> {
    let Some(first) = inputs.first() else {
        return Err(ErrorKind::Value);
    };
    let rows = first.array.rows;
    for input in inputs {
        validate_array_input(engine, context, &input.array)?;
        if input.array.rows != rows || input.array.cols == 0 {
            return Err(ErrorKind::Value);
        }
    }
    Ok(())
}

fn resolve_aggregate(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<AggregateCallable, ErrorKind> {
    let value = engine.eval_scope_value(context, expr);
    if let Some(kind) = value.engine_issue() {
        return Err(kind);
    }
    let ScopeValue::Callable(callable) = value else {
        return Err(ErrorKind::Value);
    };
    let argument_count = match &callable {
        CallableValue::Lambda(closure) if matches!(closure.parameters.len(), 1 | 2) => {
            closure.parameters.len()
        }
        CallableValue::Builtin(callable) => builtin_aggregate_capability(*callable)
            .map(|capability| capability.argument_count())
            .ok_or(ErrorKind::Value)?,
        CallableValue::Lambda(_) => return Err(ErrorKind::Value),
    };
    Ok(AggregateCallable {
        callable,
        argument_count,
    })
}

fn populate_sort_values(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    aggregate: &AggregateCallable,
    values: &ArrayEvaluation,
    all_members: &[u32],
    groups: &mut [AxisGroup],
    hierarchical: bool,
) -> Result<(), ErrorKind> {
    for group in &mut *groups {
        for column in 0..values.array.cols {
            let evaluated = invoke_aggregate(
                engine,
                context,
                aggregate,
                values,
                column,
                &group.members,
                all_members,
            )?;
            groups_engine_issue(&evaluated.value)?;
            group.sort_values.push(evaluated.value);
        }
    }
    if hierarchical {
        populate_hierarchy_sort_values(
            engine,
            context,
            groups,
            values.array.cols,
            |column, members| {
                let evaluated = invoke_aggregate(
                    engine,
                    context,
                    aggregate,
                    values,
                    column,
                    members,
                    all_members,
                )?;
                Ok(evaluated.value)
            },
        )?;
    }
    Ok(())
}

fn groups_engine_issue(value: &Value) -> Result<(), ErrorKind> {
    match value {
        Value::Error(kind) if kind.is_engine_issue() => Err(*kind),
        _ => Ok(()),
    }
}

pub(super) fn invoke_aggregate(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    aggregate: &AggregateCallable,
    values: &ArrayEvaluation,
    column: u32,
    subset_members: &[u32],
    all_members: &[u32],
) -> Result<ScalarEvaluation, ErrorKind> {
    let visits = u64::try_from(subset_members.len())
        .ok()
        .and_then(|subset| {
            if aggregate.argument_count == 2 {
                subset.checked_add(all_members.len() as u64)
            } else {
                Some(subset)
            }
        })
        .ok_or(ErrorKind::Num)?;
    engine.charge_function_iterations(context, visits)?;
    let mut arguments = Vec::with_capacity(aggregate.argument_count);
    arguments.push(column_scope_value(
        engine,
        context,
        values,
        column,
        subset_members,
    )?);
    if aggregate.argument_count == 2 {
        arguments.push(column_scope_value(
            engine,
            context,
            values,
            column,
            all_members,
        )?);
    }
    let result = dynamic::invoke_callable_values(engine, context, &aggregate.callable, arguments);
    if let Some(kind) = result.engine_issue() {
        return Err(kind);
    }
    Ok(match result {
        ScopeValue::Scalar(evaluated) => evaluated,
        ScopeValue::Array(evaluated) if evaluated.array.is_scalar() => ScalarEvaluation {
            value: evaluated.array.data[0].clone(),
            decimal_trace: evaluated.decimal_traces[0],
        },
        ScopeValue::Missing
        | ScopeValue::Array(_)
        | ScopeValue::Reference(_)
        | ScopeValue::Callable(_) => ScalarEvaluation::untracked(Value::Error(ErrorKind::Value)),
    })
}

fn column_scope_value(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    values: &ArrayEvaluation,
    column: u32,
    members: &[u32],
) -> Result<ScopeValue, ErrorKind> {
    engine.ensure_array_cells(members.len() as u64)?;
    let rows = u32::try_from(members.len()).map_err(|_| ErrorKind::Num)?;
    let mut data = Vec::with_capacity(members.len());
    let mut decimal_traces = Vec::with_capacity(members.len());
    for row in members {
        poll_cancellation(context)?;
        data.push(values.array.at(*row, column).clone());
        decimal_traces.push(values.decimal_at(*row, column));
    }
    Ok(ScopeValue::Array(Arc::new(ArrayEvaluation {
        array: Array {
            rows,
            cols: 1,
            data,
        },
        decimal_traces,
    })))
}
