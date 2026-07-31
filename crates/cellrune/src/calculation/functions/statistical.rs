use std::collections::BTreeMap;

use super::super::ast::Expr;
use super::super::coerce::to_logical;
use super::super::criteria::{Criteria, WildcardStepBudget, parse_criteria};
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::runtime::Rect;
use super::super::sheet_span::SheetSpanPolicy;
use super::super::value::{ErrorKind, Value};
use super::util::{
    collect_argument_values, excel_numeric_arguments, excel_numeric_arguments_with_policy,
    required_number,
};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    match name {
        "LARGE" => order_statistic(engine, context, args, false),
        "SMALL" => order_statistic(engine, context, args, true),
        "MEDIAN" => median(engine, context, args),
        "MODE.SNGL" => mode(engine, context, args),
        "CORREL" => paired_statistic(engine, context, args, PairedStatistic::Correlation),
        "SLOPE" => paired_statistic(engine, context, args, PairedStatistic::Slope),
        "PERCENTRANK.INC" => percent_rank(engine, context, args),
        "PERCENTILE.INC" => percentile(engine, context, args, false),
        "QUARTILE.INC" => percentile(engine, context, args, true),
        "RANK.EQ" => rank(engine, context, args),
        "NORMSDIST" => standard_normal_distribution(engine, context, args, true),
        "NORM.S.DIST" => standard_normal_distribution(engine, context, args, false),
        "STDEV.S" => sample_variance(engine, context, args, true),
        "VAR.S" => sample_variance(engine, context, args, false),
        "MINIFS" => conditional_extreme(engine, context, args, true),
        "MAXIFS" => conditional_extreme(engine, context, args, false),
        "PEARSON" => paired_statistic(engine, context, args, PairedStatistic::Correlation),
        "RSQ" => rsq(engine, context, args),
        "INTERCEPT" => paired_statistic(engine, context, args, PairedStatistic::Intercept),
        "COVARIANCE.P" => paired_statistic(engine, context, args, PairedStatistic::Covariance),
        _ => Value::Error(ErrorKind::Unsupported),
    }
}

#[derive(Debug, Clone, Copy)]
enum PairedStatistic {
    Correlation,
    Slope,
    Intercept,
    Covariance,
}

fn paired_statistic(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    statistic: PairedStatistic,
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let pairs = match numeric_pairs(engine, context, args) {
        Ok(pairs) if !pairs.is_empty() => pairs,
        Ok(_) => return Value::Error(ErrorKind::Div0),
        Err(kind) => return Value::Error(kind),
    };
    let count = pairs.len() as f64;
    let left_mean = pairs.iter().map(|(left, _)| left).sum::<f64>() / count;
    let right_mean = pairs.iter().map(|(_, right)| right).sum::<f64>() / count;
    let mut cross_deviation = 0.0;
    let mut left_deviation = 0.0;
    let mut right_deviation = 0.0;
    for (left, right) in pairs {
        let left_delta = left - left_mean;
        let right_delta = right - right_mean;
        cross_deviation += left_delta * right_delta;
        left_deviation += left_delta * left_delta;
        right_deviation += right_delta * right_delta;
    }
    let denominator = match statistic {
        PairedStatistic::Correlation => (left_deviation * right_deviation).sqrt(),
        PairedStatistic::Slope | PairedStatistic::Intercept => right_deviation,
        PairedStatistic::Covariance => count,
    };
    if denominator == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    let result = match statistic {
        PairedStatistic::Intercept => left_mean - cross_deviation / denominator * right_mean,
        PairedStatistic::Correlation | PairedStatistic::Slope | PairedStatistic::Covariance => {
            cross_deviation / denominator
        }
    };
    if result.is_finite() {
        Value::Number(result)
    } else {
        Value::Error(ErrorKind::Num)
    }
}

fn percent_rank(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let mut numbers = match numeric_arguments(engine, context, &args[..1]) {
        Ok(numbers) if !numbers.is_empty() => numbers,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let target = match required_number(engine, context, &args[1]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let significance = match args.get(2) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) if number >= 1.0 => number.trunc() as i32,
            Ok(_) => return Value::Error(ErrorKind::Num),
            Err(kind) => return Value::Error(kind),
        },
        None => 3,
    };
    numbers.sort_by(f64::total_cmp);
    if numbers.len() == 1 {
        return if numbers[0] == target {
            Value::Number(0.0)
        } else {
            Value::Error(ErrorKind::NA)
        };
    }
    let first = numbers[0];
    let last = numbers[numbers.len() - 1];
    if target < first || target > last {
        return Value::Error(ErrorKind::NA);
    }
    let upper = numbers.partition_point(|number| *number < target);
    let raw_rank = if upper < numbers.len() && numbers[upper] == target {
        upper as f64 / (numbers.len() - 1) as f64
    } else {
        let lower = upper - 1;
        let fraction = (target - numbers[lower]) / (numbers[upper] - numbers[lower]);
        (lower as f64 + fraction) / (numbers.len() - 1) as f64
    };
    let factor = 10_f64.powi(significance);
    let result = (raw_rank * factor).trunc() / factor;
    if result.is_finite() {
        Value::Number(result)
    } else {
        Value::Error(ErrorKind::Num)
    }
}

fn standard_normal_distribution(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    legacy_cumulative: bool,
) -> Value {
    let expected_len = if legacy_cumulative { 1 } else { 2 };
    if args.len() != expected_len {
        return Value::Error(ErrorKind::Value);
    }
    let z = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let cumulative = if legacy_cumulative {
        true
    } else {
        match to_logical(&engine.eval_scalar(context, &args[1])) {
            Ok(cumulative) => cumulative,
            Err(kind) => return Value::Error(kind),
        }
    };
    if cumulative {
        Value::Number(0.5 * libm::erfc(-z / std::f64::consts::SQRT_2))
    } else {
        Value::Number((-0.5 * z * z).exp() / (2.0 * std::f64::consts::PI).sqrt())
    }
}

pub(super) fn numeric_arguments(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Vec<f64>, ErrorKind> {
    excel_numeric_arguments(engine, context, args)
}

pub(super) fn numeric_arguments_with_policy(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    sheet_span_policy: SheetSpanPolicy,
) -> Result<Vec<f64>, ErrorKind> {
    excel_numeric_arguments_with_policy(engine, context, args, sheet_span_policy)
}

fn numeric_pairs(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Vec<(f64, f64)>, ErrorKind> {
    let left = engine.eval_array(context, &args[0])?;
    let right = engine.eval_array(context, &args[1])?;
    if left.data.len() != right.data.len() {
        return Err(ErrorKind::NA);
    }
    let mut pairs = Vec::new();
    for (left, right) in left.data.into_iter().zip(right.data) {
        match (left, right) {
            (Value::Error(kind), _) | (_, Value::Error(kind)) => return Err(kind),
            (Value::Number(left), Value::Number(right)) => pairs.push((left, right)),
            _ => {}
        }
    }
    Ok(pairs)
}

fn rsq(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    match paired_statistic(engine, context, args, PairedStatistic::Correlation) {
        Value::Number(correlation) => Value::Number(correlation * correlation),
        other => other,
    }
}

fn order_statistic(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    ascending: bool,
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let mut numbers = match numeric_arguments(engine, context, &args[..1]) {
        Ok(numbers) => numbers,
        Err(kind) => return Value::Error(kind),
    };
    let rank = match required_number(engine, context, &args[1]) {
        Ok(number) if number >= 1.0 => number.trunc() as usize,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    numbers.sort_by(f64::total_cmp);
    if !ascending {
        numbers.reverse();
    }
    numbers
        .get(rank - 1)
        .copied()
        .map(Value::Number)
        .unwrap_or(Value::Error(ErrorKind::Num))
}

fn median(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let mut numbers = match numeric_arguments(engine, context, args) {
        Ok(numbers) if !numbers.is_empty() => numbers,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    numbers.sort_by(f64::total_cmp);
    let middle = numbers.len() / 2;
    if numbers.len().is_multiple_of(2) {
        Value::Number((numbers[middle - 1] + numbers[middle]) / 2.0)
    } else {
        Value::Number(numbers[middle])
    }
}

fn mode(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let values = match collect_argument_values(engine, context, args) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let mut counts = BTreeMap::<u64, (f64, usize, usize)>::new();
    for (position, item) in values.into_iter().enumerate() {
        let number = match item.value {
            Value::Number(number) => number,
            Value::Error(kind) => return Value::Error(kind),
            Value::Blank if item.from_single_cell_reference => {
                return Value::Error(ErrorKind::Value);
            }
            Value::Blank | Value::Text(_) | Value::Logical(_) => continue,
        };
        let entry = counts
            .entry(number.to_bits())
            .or_insert((number, 0, position));
        entry.1 += 1;
    }
    counts
        .values()
        .filter(|(_, count, _)| *count > 1)
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.2.cmp(&left.2)))
        .map(|(number, _, _)| Value::Number(*number))
        .unwrap_or(Value::Error(ErrorKind::NA))
}

fn percentile(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    quartile: bool,
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let mut numbers = match numeric_arguments(engine, context, &args[..1]) {
        Ok(numbers) if !numbers.is_empty() => numbers,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let mut probability = match required_number(engine, context, &args[1]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    if quartile {
        probability /= 4.0;
    }
    if !(0.0..=1.0).contains(&probability) {
        return Value::Error(ErrorKind::Num);
    }
    numbers.sort_by(f64::total_cmp);
    let position = (numbers.len() - 1) as f64 * probability;
    let lower = position.floor() as usize;
    let fraction = position - lower as f64;
    let upper = (lower + 1).min(numbers.len() - 1);
    Value::Number(numbers[lower] + (numbers[upper] - numbers[lower]) * fraction)
}

fn rank(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let mut numbers = match numeric_arguments(engine, context, &args[1..2]) {
        Ok(numbers) => numbers,
        Err(kind) => return Value::Error(kind),
    };
    let ascending = match args.get(2) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(order) => order != 0.0,
            Err(kind) => return Value::Error(kind),
        },
        None => false,
    };
    numbers.sort_by(f64::total_cmp);
    if !ascending {
        numbers.reverse();
    }
    numbers
        .iter()
        .position(|candidate| *candidate == number)
        .map(|index| Value::Number((index + 1) as f64))
        .unwrap_or(Value::Error(ErrorKind::NA))
}

fn sample_variance(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    square_root: bool,
) -> Value {
    let numbers = match numeric_arguments_with_policy(
        engine,
        context,
        args,
        SheetSpanPolicy::CollectAcrossSheets,
    ) {
        Ok(numbers) if numbers.len() >= 2 => numbers,
        Ok(_) => return Value::Error(ErrorKind::Div0),
        Err(kind) => return Value::Error(kind),
    };
    let mean = numbers.iter().sum::<f64>() / numbers.len() as f64;
    let variance = numbers
        .iter()
        .map(|number| (number - mean).powi(2))
        .sum::<f64>()
        / (numbers.len() - 1) as f64;
    Value::Number(if square_root {
        variance.sqrt()
    } else {
        variance
    })
}

fn conditional_extreme(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    minimum: bool,
) -> Value {
    if args.len() < 3 || args.len().is_multiple_of(2) {
        return Value::Error(ErrorKind::Value);
    }
    let value_range = match engine.resolve_rect_expr(context, &args[0]) {
        Ok(rect) => rect,
        Err(kind) => return Value::Error(kind),
    };
    let criteria = match parse_pairs(engine, context, &args[1..], value_range) {
        Ok(criteria) => criteria,
        Err(kind) => return Value::Error(kind),
    };
    let visits = value_range
        .height()
        .checked_mul(value_range.width())
        .and_then(|cells| cells.checked_mul(criteria.len() as u64 + 1));
    if visits.is_none_or(|cells| engine.ensure_array_cells(cells).is_err()) {
        return Value::Error(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells));
    }
    let mut result: Option<f64> = None;
    let mut wildcard_budget = WildcardStepBudget::new(engine.max_function_iterations());
    for row in 0..value_range.height() as u32 {
        for column in 0..value_range.width() as u32 {
            let mut matched = true;
            for (rect, criterion) in &criteria {
                let value = match engine.read_reference_cell(
                    context,
                    (rect.sheet, rect.row_start + row, rect.col_start + column),
                ) {
                    Ok(value) => value,
                    Err(kind) => return Value::Error(kind),
                };
                match criterion.matches(&value, &mut wildcard_budget) {
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
            let value = match engine.read_reference_cell(
                context,
                (
                    value_range.sheet,
                    value_range.row_start + row,
                    value_range.col_start + column,
                ),
            ) {
                Ok(value) => value,
                Err(kind) => return Value::Error(kind),
            };
            if let Value::Number(number) = value {
                result = Some(result.map_or(number, |current| {
                    if minimum {
                        current.min(number)
                    } else {
                        current.max(number)
                    }
                }));
            }
        }
    }
    Value::Number(result.unwrap_or(0.0))
}

fn parse_pairs(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    value_range: Rect,
) -> Result<Vec<(Rect, Criteria)>, ErrorKind> {
    let mut result = Vec::new();
    for pair in args.chunks_exact(2) {
        let rect = engine.resolve_rect_expr(context, &pair[0])?;
        if rect.height() != value_range.height() || rect.width() != value_range.width() {
            return Err(ErrorKind::Value);
        }
        result.push((
            rect,
            parse_criteria(&engine.eval_scalar(context, &pair[1]))?,
        ));
    }
    Ok(result)
}
