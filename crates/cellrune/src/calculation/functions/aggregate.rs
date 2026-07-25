use super::super::ast::Expr;
use super::super::criteria::{Criteria, WildcardStepBudget, parse_criteria};
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::runtime::Rect;
use super::super::value::{ErrorKind, Value};
use super::util::{ArgumentValue, collect_argument_values, excel_sum, required_number};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    match name {
        "SUM" => aggregate_numbers(engine, context, args, Aggregate::Sum),
        "AVERAGE" => aggregate_numbers(engine, context, args, Aggregate::Average),
        "MIN" => aggregate_numbers(engine, context, args, Aggregate::Min),
        "MAX" => aggregate_numbers(engine, context, args, Aggregate::Max),
        "PRODUCT" => aggregate_numbers(engine, context, args, Aggregate::Product),
        "COUNT" => count_numbers(engine, context, args),
        "COUNTA" => count_nonblank(engine, context, args),
        "COUNTBLANK" => count_blank(engine, context, args),
        "SUBTOTAL" => subtotal(engine, context, args),
        "SUMIF" => conditional_aggregate(engine, context, args, ConditionalAggregate::SumIf),
        "SUMIFS" => conditional_aggregate(engine, context, args, ConditionalAggregate::SumIfs),
        "AVERAGEIF" => {
            conditional_aggregate(engine, context, args, ConditionalAggregate::AverageIf)
        }
        "AVERAGEIFS" => {
            conditional_aggregate(engine, context, args, ConditionalAggregate::AverageIfs)
        }
        _ => Value::Error(ErrorKind::Unsupported),
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
    let values = match collect_argument_values(engine, context, args) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let mut numbers = Vec::new();
    for ArgumentValue {
        value,
        from_collection,
        ..
    } in values
    {
        match value {
            Value::Number(number) => numbers.push(number),
            Value::Logical(logical) if !from_collection => {
                numbers.push(if logical { 1.0 } else { 0.0 });
            }
            Value::Text(text) if !from_collection => match text.parse::<f64>() {
                Ok(number) => numbers.push(number),
                Err(_) => return Value::Error(ErrorKind::Value),
            },
            Value::Error(kind) => return Value::Error(kind),
            Value::Blank | Value::Text(_) | Value::Logical(_) => {}
        }
    }
    let result = match aggregate {
        Aggregate::Sum => excel_sum(numbers.iter().copied()),
        Aggregate::Average if numbers.is_empty() => return Value::Error(ErrorKind::Div0),
        Aggregate::Average => excel_sum(numbers.iter().copied()) / numbers.len() as f64,
        Aggregate::Min => numbers.into_iter().reduce(f64::min).unwrap_or(0.0),
        Aggregate::Max => numbers.into_iter().reduce(f64::max).unwrap_or(0.0),
        Aggregate::Product if numbers.is_empty() => 0.0,
        Aggregate::Product => numbers.into_iter().product(),
    };
    finite_number(result)
}

fn count_numbers(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    let values = match collect_argument_values(engine, context, args) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
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
    match collect_argument_values(engine, context, args) {
        Ok(values) => Value::Number(
            values
                .iter()
                .filter(|item| !matches!(item.value, Value::Blank))
                .count() as f64,
        ),
        Err(kind) => Value::Error(kind),
    }
}

fn count_blank(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let rect = match engine.resolve_rect_expr(context, &args[0]) {
        Ok(rect) => rect,
        Err(kind) => return Value::Error(kind),
    };
    if let Err(kind) = engine.ensure_array_cells(rect.height() * rect.width()) {
        return Value::Error(kind);
    }
    let mut count = 0_u64;
    for row in rect.row_start..=rect.row_end {
        for column in rect.col_start..=rect.col_end {
            if engine.cell_value((rect.sheet, row, column)).is_blank_like() {
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
        Ok(number) => number.trunc() as i32 % 100,
        Err(kind) => return Value::Error(kind),
    };
    match function {
        1 => aggregate_numbers(engine, context, &args[1..], Aggregate::Average),
        2 => count_numbers(engine, context, &args[1..]),
        3 => count_nonblank(engine, context, &args[1..]),
        4 => aggregate_numbers(engine, context, &args[1..], Aggregate::Max),
        5 => aggregate_numbers(engine, context, &args[1..], Aggregate::Min),
        9 => aggregate_numbers(engine, context, &args[1..], Aggregate::Sum),
        _ => Value::Error(ErrorKind::Unsupported),
    }
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
    let parsed = match parse_conditional_arguments(engine, context, args, operation) {
        Ok(parsed) => parsed,
        Err(kind) => return Value::Error(kind),
    };
    let iter_rows = parsed
        .criteria
        .iter()
        .map(|(range, _)| available_rows(engine, range))
        .chain(std::iter::once(available_rows(engine, &parsed.value_range)))
        .max()
        .unwrap_or(0)
        .min(parsed.value_range.height());
    let visits = iter_rows
        .checked_mul(parsed.value_range.width())
        .and_then(|cells| cells.checked_mul(parsed.criteria.len() as u64 + 1));
    if visits.is_none_or(|cells| engine.ensure_array_cells(cells).is_err()) {
        return Value::Error(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells));
    }
    let mut total = 0.0_f64;
    let mut count = 0_u64;
    let mut wildcard_budget = WildcardStepBudget::new(engine.max_function_iterations());
    for row_offset in 0..iter_rows as u32 {
        for col_offset in 0..parsed.value_range.width() as u32 {
            let mut matched = true;
            for (range, criterion) in &parsed.criteria {
                match criterion.matches(
                    &engine.cell_value((
                        range.sheet,
                        range.row_start + row_offset,
                        range.col_start + col_offset,
                    )),
                    &mut wildcard_budget,
                ) {
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
            match engine.cell_value((
                parsed.value_range.sheet,
                parsed.value_range.row_start + row_offset,
                parsed.value_range.col_start + col_offset,
            )) {
                Value::Number(number) => {
                    total += number;
                    count += 1;
                }
                Value::Error(kind) => return Value::Error(kind),
                Value::Blank | Value::Text(_) | Value::Logical(_) => {}
            }
        }
    }
    match operation {
        ConditionalAggregate::SumIf | ConditionalAggregate::SumIfs => finite_number(total),
        ConditionalAggregate::AverageIf | ConditionalAggregate::AverageIfs if count == 0 => {
            Value::Error(ErrorKind::Div0)
        }
        ConditionalAggregate::AverageIf | ConditionalAggregate::AverageIfs => {
            finite_number(total / count as f64)
        }
    }
}

struct ConditionalArguments {
    value_range: Rect,
    criteria: Vec<(Rect, Criteria)>,
}

fn parse_conditional_arguments(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    operation: ConditionalAggregate,
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
        let criterion = parse_criteria(&engine.eval_scalar(context, &args[1]))?;
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
        let criterion = parse_criteria(&engine.eval_scalar(context, &pair[1]))?;
        criteria.push((range, criterion));
    }
    Ok(ConditionalArguments {
        value_range,
        criteria,
    })
}

fn available_rows(engine: &Engine<'_>, rect: &Rect) -> u64 {
    let row_end = engine.clamped_row_end(rect);
    if row_end < rect.row_start {
        0
    } else {
        u64::from(row_end - rect.row_start) + 1
    }
}

fn finite_number(number: f64) -> Value {
    if number.is_finite() {
        Value::Number(number)
    } else {
        Value::Error(ErrorKind::Num)
    }
}
