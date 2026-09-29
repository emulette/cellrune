use super::super::ast::Expr;
use super::super::criteria::CompiledCriteria;
use super::super::decimal::DecimalTrace;
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::runtime::Rect;
use super::super::scope::ScopeValue;
use super::super::sheet_span::SheetSpanPolicy;
use super::super::value::{ErrorKind, Value};
use super::criteria_runtime::CriteriaRuntime;
use super::kernel::AggregateFunction;
use super::moments::VarianceKind;
use super::normalize_name;
use super::statistical::variance_value;
use super::util::{
    ArgumentValue, ExcelSum, collect_argument_values_including,
    collect_argument_values_with_policy, collect_callable_argument_values, excel_numbers,
    required_number,
};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: AggregateFunction,
    args: &[Expr],
) -> Value {
    match function {
        AggregateFunction::Sum => aggregate_numbers(engine, context, args, Aggregate::Sum),
        AggregateFunction::Average => aggregate_numbers(engine, context, args, Aggregate::Average),
        AggregateFunction::Min => aggregate_numbers(engine, context, args, Aggregate::Min),
        AggregateFunction::Max => aggregate_numbers(engine, context, args, Aggregate::Max),
        AggregateFunction::Product => aggregate_numbers(engine, context, args, Aggregate::Product),
        AggregateFunction::Count => count_numbers(engine, context, args),
        AggregateFunction::CountA => count_nonblank(engine, context, args),
        AggregateFunction::CountBlank => count_blank(engine, context, args),
        AggregateFunction::Subtotal => subtotal(engine, context, args),
        AggregateFunction::SumIf => {
            conditional_aggregate(engine, context, args, ConditionalAggregate::SumIf)
        }
        AggregateFunction::SumIfs => {
            conditional_aggregate(engine, context, args, ConditionalAggregate::SumIfs)
        }
        AggregateFunction::AverageIf => {
            conditional_aggregate(engine, context, args, ConditionalAggregate::AverageIf)
        }
        AggregateFunction::AverageIfs => {
            conditional_aggregate(engine, context, args, ConditionalAggregate::AverageIfs)
        }
    }
}

pub(super) fn call_scope_values(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: AggregateFunction,
    args: &[ScopeValue],
) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    let values = match collect_callable_argument_values(engine, context, args) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    match function {
        AggregateFunction::Sum => aggregate_collected(engine, values, Aggregate::Sum),
        AggregateFunction::Average => aggregate_collected(engine, values, Aggregate::Average),
        AggregateFunction::Min => aggregate_collected(engine, values, Aggregate::Min),
        AggregateFunction::Max => aggregate_collected(engine, values, Aggregate::Max),
        AggregateFunction::Product => aggregate_collected(engine, values, Aggregate::Product),
        AggregateFunction::Count => count_collected(values),
        AggregateFunction::CountA => count_nonblank_collected(&values),
        AggregateFunction::CountBlank
        | AggregateFunction::Subtotal
        | AggregateFunction::SumIf
        | AggregateFunction::SumIfs
        | AggregateFunction::AverageIf
        | AggregateFunction::AverageIfs => {
            unreachable!("non-callable aggregate was stored as BuiltinCallable")
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Aggregate {
    Sum,
    Average,
    Min,
    Max,
    Product,
}

fn aggregate_numbers(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    aggregate: Aggregate,
) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    let values = match collect_argument_values_with_policy(
        engine,
        context,
        args,
        SheetSpanPolicy::CollectAcrossSheets,
    ) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    aggregate_collected(engine, values, aggregate)
}

fn aggregate_collected(
    engine: &Engine<'_>,
    values: Vec<ArgumentValue>,
    aggregate: Aggregate,
) -> Value {
    let mut sum = ExcelSum::new(engine);
    let mut result = 0.0_f64;
    let mut count = 0_u64;
    for ArgumentValue {
        value,
        decimal_trace,
        from_collection,
        ..
    } in values
    {
        let (number, decimal_trace) = match value {
            Value::Number(number) => (number, decimal_trace),
            Value::Logical(logical) if !from_collection => {
                let number = if logical { 1.0 } else { 0.0 };
                (number, DecimalTrace::from_number(number))
            }
            Value::Text(text) if !from_collection => match text.parse::<f64>() {
                Ok(number) => (number, DecimalTrace::from_number(number)),
                Err(_) => return Value::Error(ErrorKind::Value),
            },
            Value::Error(kind) => return Value::Error(kind),
            Value::Blank | Value::Text(_) | Value::Logical(_) => continue,
        };
        match aggregate {
            Aggregate::Sum | Aggregate::Average => sum.add_with_trace(number, decimal_trace),
            Aggregate::Min => {
                result = if count == 0 {
                    number
                } else {
                    result.min(number)
                }
            }
            Aggregate::Max => {
                result = if count == 0 {
                    number
                } else {
                    result.max(number)
                }
            }
            Aggregate::Product => result = if count == 0 { 1.0 } else { result } * number,
        }
        count += 1;
    }
    let result = match aggregate {
        Aggregate::Sum => sum.total(),
        Aggregate::Average if count == 0 => return Value::Error(ErrorKind::Div0),
        Aggregate::Average => sum.total() / count as f64,
        Aggregate::Min | Aggregate::Max | Aggregate::Product => result,
    };
    finite_number(result)
}

fn count_numbers(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    let values = match collect_argument_values_with_policy(
        engine,
        context,
        args,
        SheetSpanPolicy::CollectAcrossSheets,
    ) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    count_collected(values)
}

fn count_collected(values: Vec<ArgumentValue>) -> Value {
    let mut count = 0_u64;
    for ArgumentValue {
        value,
        from_collection,
        ..
    } in values
    {
        match value {
            Value::Number(_) => count += 1,
            Value::Logical(_) if !from_collection => count += 1,
            Value::Text(text) if !from_collection && text.parse::<f64>().is_ok() => count += 1,
            Value::Error(kind) if kind.is_engine_issue() => return Value::Error(kind),
            Value::Error(_) => {}
            Value::Blank | Value::Text(_) | Value::Logical(_) => {}
        }
    }
    Value::Number(count as f64)
}

fn count_nonblank(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    match collect_argument_values_with_policy(
        engine,
        context,
        args,
        SheetSpanPolicy::CollectAcrossSheets,
    ) {
        Ok(values) => count_nonblank_collected(&values),
        Err(kind) => Value::Error(kind),
    }
}

fn count_nonblank_collected(values: &[ArgumentValue]) -> Value {
    let mut count = 0_u64;
    for item in values {
        match item.value {
            Value::Error(kind) if kind.is_engine_issue() => return Value::Error(kind),
            Value::Blank => {}
            Value::Number(_) | Value::Text(_) | Value::Logical(_) | Value::Error(_) => count += 1,
        }
    }
    Value::Number(count as f64)
}

fn count_blank(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let rect = match engine.resolve_rect_expr(context, &args[0]) {
        Ok(rect) => rect,
        Err(kind) => return Value::Error(kind),
    };
    let cells = rect.height() * rect.width();
    if let Err(kind) = engine.ensure_array_cells(cells) {
        return Value::Error(kind);
    }
    let mut count = 0_u64;
    for row in rect.row_start..=rect.row_end {
        for column in rect.col_start..=rect.col_end {
            let value = match engine.read_reference_cell(context, (rect.sheet, row, column)) {
                Ok(value) => value,
                Err(kind) => return Value::Error(kind),
            };
            if value.is_blank_like() {
                count += 1;
            }
        }
    }
    Value::Number(count as f64)
}

fn subtotal(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 {
        return Value::Error(ErrorKind::Value);
    }
    let function = match required_number(engine, context, &args[0]) {
        Ok(number) => number.trunc(),
        Err(kind) => return Value::Error(kind),
    };
    // 101 to 111 name the same aggregates while also skipping manually hidden rows, which the
    // workbook model does not represent.
    let operation = if (1.0..=11.0).contains(&function) {
        function
    } else if (101.0..=111.0).contains(&function) {
        function - 100.0
    } else {
        return Value::Error(ErrorKind::Value);
    };
    // Excel ignores cells that hold another SUBTOTAL or AGGREGATE so nested totals are not
    // counted twice.
    let values = match collect_argument_values_including(
        engine,
        context,
        &args[1..],
        SheetSpanPolicy::CollectAcrossSheets,
        &|cell| !engine.parsed_expr(cell).is_some_and(contains_subtotal_call),
    ) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let variance = |values, kind, square_root| match excel_numbers(values) {
        Ok(numbers) => variance_value(engine, context, numbers, kind, square_root),
        Err(kind) => Value::Error(kind),
    };
    match operation as u8 {
        1 => aggregate_collected(engine, values, Aggregate::Average),
        2 => count_collected(values),
        3 => count_nonblank_collected(&values),
        4 => aggregate_collected(engine, values, Aggregate::Max),
        5 => aggregate_collected(engine, values, Aggregate::Min),
        6 => aggregate_collected(engine, values, Aggregate::Product),
        7 => variance(values, VarianceKind::Sample, true),
        8 => variance(values, VarianceKind::Population, true),
        9 => aggregate_collected(engine, values, Aggregate::Sum),
        10 => variance(values, VarianceKind::Sample, false),
        11 => variance(values, VarianceKind::Population, false),
        _ => unreachable!("SUBTOTAL operation was validated above"),
    }
}

fn contains_subtotal_call(root: &Expr) -> bool {
    let mut pending = vec![root];
    while let Some(expr) = pending.pop() {
        match expr {
            Expr::Call { name, args } => {
                if matches!(normalize_name(name).as_str(), "SUBTOTAL" | "AGGREGATE") {
                    return true;
                }
                pending.extend(args);
            }
            Expr::Invoke { callee, args } => {
                pending.push(callee);
                pending.extend(args);
            }
            Expr::ReferenceUnion { left, right }
            | Expr::ReferenceIntersection { left, right }
            | Expr::Binary { left, right, .. } => pending.extend([&**left, &**right]),
            Expr::Range { start, end } => pending.extend([&**start, &**end]),
            Expr::SpillRef(inner)
            | Expr::ImplicitIntersection(inner)
            | Expr::Unary { operand: inner, .. }
            | Expr::Paren(inner) => pending.push(inner),
            Expr::Array(rows) => pending.extend(rows.iter().flatten()),
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Logical(_)
            | Expr::ErrorLit(_)
            | Expr::Ref(_)
            | Expr::StructuredRef(_)
            | Expr::ExternalReference(_)
            | Expr::QualifiedName { .. }
            | Expr::Name(_)
            | Expr::BuiltinCallable(_)
            | Expr::Missing => {}
        }
    }
    false
}

#[derive(Debug, Clone, Copy)]
enum ConditionalAggregate {
    SumIf,
    SumIfs,
    AverageIf,
    AverageIfs,
}

fn conditional_aggregate(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: ConditionalAggregate,
) -> Value {
    let mut runtime = CriteriaRuntime::new(engine, context);
    let parsed = match parse_conditional_arguments(engine, context, args, operation, &mut runtime) {
        Ok(parsed) => parsed,
        Err(kind) => return Value::Error(kind),
    };
    let iter_rows = engine.operation_row_count(
        parsed
            .criteria
            .iter()
            .map(|(range, _)| range)
            .chain(std::iter::once(&parsed.value_range)),
    );
    let visits = iter_rows
        .checked_mul(parsed.value_range.width())
        .and_then(|cells| cells.checked_mul(parsed.criteria.len() as u64 + 1));
    if visits.is_none_or(|cells| engine.ensure_array_cells(cells).is_err()) {
        return Value::Error(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells));
    }
    let mut total = ExcelSum::new(engine);
    let mut count = 0_u64;
    for row_offset in 0..iter_rows as u32 {
        for col_offset in 0..parsed.value_range.width() as u32 {
            let mut matched = true;
            for (range, criterion) in &parsed.criteria {
                let value = match engine.read_reference_cell(
                    context,
                    (
                        range.sheet,
                        range.row_start + row_offset,
                        range.col_start + col_offset,
                    ),
                ) {
                    Ok(value) => value,
                    Err(kind) => return Value::Error(kind),
                };
                match runtime.matches(criterion, &value) {
                    Ok(true) => {}
                    Ok(false) => {
                        matched = false;
                        break;
                    }
                    Err(kind) => return Value::Error(kind),
                }
            }
            if !matched {
                continue;
            }
            let cell = (
                parsed.value_range.sheet,
                parsed.value_range.row_start + row_offset,
                parsed.value_range.col_start + col_offset,
            );
            match engine.read_reference_cell(context, cell) {
                Err(kind) => return Value::Error(kind),
                Ok(Value::Number(number)) => {
                    total.add_with_trace(number, engine.numeric_decimal_trace(cell));
                    count += 1;
                }
                Ok(Value::Error(kind)) => return Value::Error(kind),
                Ok(Value::Blank | Value::Text(_) | Value::Logical(_)) => {}
            }
        }
    }
    match operation {
        ConditionalAggregate::SumIf | ConditionalAggregate::SumIfs => finite_number(total.total()),
        ConditionalAggregate::AverageIf | ConditionalAggregate::AverageIfs if count == 0 => {
            Value::Error(ErrorKind::Div0)
        }
        ConditionalAggregate::AverageIf | ConditionalAggregate::AverageIfs => {
            finite_number(total.total() / count as f64)
        }
    }
}

struct ConditionalArguments {
    value_range: Rect,
    criteria: Vec<(Rect, CompiledCriteria)>,
}

fn parse_conditional_arguments(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: ConditionalAggregate,
    runtime: &mut CriteriaRuntime<'_, '_, '_>,
) -> Result<ConditionalArguments, ErrorKind> {
    if matches!(
        operation,
        ConditionalAggregate::SumIf | ConditionalAggregate::AverageIf
    ) {
        if args.len() < 2 || args.len() > 3 {
            return Err(ErrorKind::Value);
        }
        let criteria_range = engine.resolve_rect_expr(context, &args[0])?;
        let value_anchor = engine.resolve_rect_expr(context, args.get(2).unwrap_or(&args[0]))?;
        let value_range = value_anchor
            .resized_from_anchor(criteria_range.height(), criteria_range.width())
            .ok_or(ErrorKind::Ref)?;
        let criterion = runtime.compile_criteria(&engine.eval_scalar(context, &args[1]))?;
        return Ok(ConditionalArguments {
            value_range,
            criteria: vec![(criteria_range, criterion)],
        });
    }

    let (value_expr, pairs): (&Expr, &[Expr]) = match operation {
        ConditionalAggregate::SumIfs | ConditionalAggregate::AverageIfs => {
            if args.len() < 3 || args.len().is_multiple_of(2) {
                return Err(ErrorKind::Value);
            }
            (&args[0], &args[1..])
        }
        ConditionalAggregate::SumIf | ConditionalAggregate::AverageIf => {
            unreachable!("single-criteria operations return above")
        }
    };
    let value_range = engine.resolve_rect_expr(context, value_expr)?;
    let mut criteria = Vec::new();
    for pair in pairs.chunks_exact(2) {
        let range = engine.resolve_rect_expr(context, &pair[0])?;
        if range.height() != value_range.height() || range.width() != value_range.width() {
            return Err(ErrorKind::Value);
        }
        let criterion = runtime.compile_criteria(&engine.eval_scalar(context, &pair[1]))?;
        criteria.push((range, criterion));
    }
    Ok(ConditionalArguments {
        value_range,
        criteria,
    })
}

fn finite_number(number: f64) -> Value {
    if number.is_finite() {
        Value::Number(number)
    } else {
        Value::Error(ErrorKind::Num)
    }
}
