use super::super::ast::Expr;
use super::super::coerce::to_logical;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::statistical::numeric_arguments;
use super::util::{collect_argument_values, required_number};

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    match name {
        "AVEDEV" => deviation_aggregate(engine, context, args, DeviationAggregate::Average),
        "DEVSQ" => deviation_aggregate(engine, context, args, DeviationAggregate::SumOfSquares),
        "AVERAGEA" => aggregate_a(engine, context, args, AggregateA::Average),
        "MAXA" => aggregate_a(engine, context, args, AggregateA::Maximum),
        "MINA" => aggregate_a(engine, context, args, AggregateA::Minimum),
        "GEOMEAN" => mean(engine, context, args, Mean::Geometric),
        "HARMEAN" => mean(engine, context, args, Mean::Harmonic),
        "VAR.P" => population_variance(engine, context, args, false),
        "STDEV.P" => population_variance(engine, context, args, true),
        "STANDARDIZE" => standardize(engine, context, args),
        "PHI" => normal_helper(engine, context, args, NormalHelper::Density),
        "GAUSS" => normal_helper(engine, context, args, NormalHelper::Gauss),
        "NORM.DIST" => normal_distribution(engine, context, args),
        "EXPON.DIST" => exponential_distribution(engine, context, args),
        "POISSON.DIST" => poisson_distribution(engine, context, args),
        _ => Value::Error(ErrorKind::Unsupported),
    }
}

#[derive(Debug, Clone, Copy)]
enum DeviationAggregate {
    Average,
    SumOfSquares,
}

fn deviation_aggregate(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    aggregate: DeviationAggregate,
) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    let numbers = match numeric_arguments(engine, context, args) {
        Ok(numbers) if !numbers.is_empty() => numbers,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let mean = numbers.iter().sum::<f64>() / numbers.len() as f64;
    let total = numbers
        .iter()
        .map(|number| {
            (number - mean).abs().powi(match aggregate {
                DeviationAggregate::Average => 1,
                DeviationAggregate::SumOfSquares => 2,
            })
        })
        .sum::<f64>();
    finite(match aggregate {
        DeviationAggregate::Average => total / numbers.len() as f64,
        DeviationAggregate::SumOfSquares => total,
    })
}

#[derive(Debug, Clone, Copy)]
enum AggregateA {
    Average,
    Maximum,
    Minimum,
}

fn aggregate_a(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    aggregate: AggregateA,
) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    let values = match collect_argument_values(engine, context, args) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    let mut numbers = Vec::new();
    for item in values {
        match item.value {
            Value::Number(number) => numbers.push(number),
            Value::Logical(logical) => numbers.push(if logical { 1.0 } else { 0.0 }),
            Value::Text(text) if !item.from_collection => {
                let number = match text
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|number| number.is_finite())
                {
                    Some(number) => number,
                    None => return Value::Error(ErrorKind::Value),
                };
                numbers.push(number);
            }
            Value::Text(_) => numbers.push(0.0),
            Value::Error(kind) => return Value::Error(kind),
            Value::Blank => {}
        }
    }
    match aggregate {
        AggregateA::Average if numbers.is_empty() => Value::Error(ErrorKind::Div0),
        AggregateA::Average => finite(numbers.iter().sum::<f64>() / numbers.len() as f64),
        AggregateA::Maximum => Value::Number(numbers.into_iter().reduce(f64::max).unwrap_or(0.0)),
        AggregateA::Minimum => Value::Number(numbers.into_iter().reduce(f64::min).unwrap_or(0.0)),
    }
}

#[derive(Debug, Clone, Copy)]
enum Mean {
    Geometric,
    Harmonic,
}

fn mean(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr], mean: Mean) -> Value {
    if args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    let numbers = match numeric_arguments(engine, context, args) {
        Ok(numbers) if !numbers.is_empty() => numbers,
        Ok(_) if matches!(mean, Mean::Harmonic) => return Value::Error(ErrorKind::NA),
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    if numbers.iter().any(|number| *number <= 0.0) {
        return Value::Error(ErrorKind::Num);
    }
    let result = match mean {
        Mean::Geometric => {
            (numbers.iter().map(|number| number.ln()).sum::<f64>() / numbers.len() as f64).exp()
        }
        Mean::Harmonic => {
            numbers.len() as f64 / numbers.iter().map(|number| number.recip()).sum::<f64>()
        }
    };
    finite(result)
}

fn population_variance(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    square_root: bool,
) -> Value {
    let numbers = match numeric_arguments(engine, context, args) {
        Ok(numbers) if !numbers.is_empty() => numbers,
        Ok(_) => return Value::Error(ErrorKind::Div0),
        Err(kind) => return Value::Error(kind),
    };
    let mean = numbers.iter().sum::<f64>() / numbers.len() as f64;
    let variance = numbers
        .iter()
        .map(|number| (number - mean).powi(2))
        .sum::<f64>()
        / numbers.len() as f64;
    finite(if square_root {
        variance.sqrt()
    } else {
        variance
    })
}

fn standardize(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let values = match args
        .iter()
        .map(|expr| required_number(engine, context, expr))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    if values[2] <= 0.0 {
        Value::Error(ErrorKind::Num)
    } else {
        finite((values[0] - values[1]) / values[2])
    }
}

#[derive(Debug, Clone, Copy)]
enum NormalHelper {
    Density,
    Gauss,
}

fn normal_helper(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    helper: NormalHelper,
) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let value = match required_number(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    Value::Number(match helper {
        NormalHelper::Density => standard_normal_density(value),
        NormalHelper::Gauss => standard_normal_cumulative(value) - 0.5,
    })
}

fn normal_distribution(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 4 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match required_number(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let mean = match required_number(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let deviation = match required_number(engine, context, &args[2]) {
        Ok(value) if value > 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let cumulative = match to_logical(&engine.eval_scalar(context, &args[3])) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let standardized = (x - mean) / deviation;
    Value::Number(if cumulative {
        standard_normal_cumulative(standardized)
    } else {
        standard_normal_density(standardized) / deviation
    })
}

fn exponential_distribution(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match required_number(engine, context, &args[0]) {
        Ok(value) if value >= 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[1]) {
        Ok(value) if value > 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let cumulative = match to_logical(&engine.eval_scalar(context, &args[2])) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    Value::Number(if cumulative {
        1.0 - (-rate * x).exp()
    } else {
        rate * (-rate * x).exp()
    })
}

fn poisson_distribution(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let events = match required_number(engine, context, &args[0]) {
        Ok(value) if value >= 0.0 => value.trunc() as u64,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let mean = match required_number(engine, context, &args[1]) {
        Ok(value) if value > 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let cumulative = match to_logical(&engine.eval_scalar(context, &args[2])) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    if let Err(kind) = engine.ensure_function_iterations(events.saturating_add(1)) {
        return Value::Error(kind);
    }
    let mut probability = (-mean).exp();
    let mut total = probability;
    for event in 1..=events {
        probability *= mean / event as f64;
        total += probability;
    }
    finite(if cumulative { total } else { probability })
}

fn standard_normal_density(value: f64) -> f64 {
    (-0.5 * value * value).exp() / (2.0 * std::f64::consts::PI).sqrt()
}

fn standard_normal_cumulative(value: f64) -> f64 {
    0.5 * libm::erfc(-value / std::f64::consts::SQRT_2)
}

fn finite(number: f64) -> Value {
    if number.is_finite() {
        Value::Number(number)
    } else {
        Value::Error(ErrorKind::Num)
    }
}
