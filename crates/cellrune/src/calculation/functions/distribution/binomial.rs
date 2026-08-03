use super::super::super::ast::Expr;
use super::super::super::coerce::to_logical;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::array_common::poll_cancellation;
use super::super::special_functions::{
    binomial_pmf, binomial_pmf_sum, negative_binomial_cdf, negative_binomial_pmf,
    smallest_binomial_quantile,
};
use super::super::util::required_number;
use super::finite;

pub(super) fn binomial_distribution(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 4 {
        return Value::Error(ErrorKind::Value);
    }
    // Excel truncates both counts before applying the domain rules.
    let number_s = match truncated_number(engine, context, &args[0]) {
        Ok(value) if value >= 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let trials = match truncated_number(engine, context, &args[1]) {
        Ok(value) if value >= number_s => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let probability = match probability_number(engine, context, &args[2]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let cumulative = match to_logical(&engine.eval_scalar(context, &args[3])) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let result = if cumulative {
        binomial_pmf_sum(trials, probability, 0.0, number_s, || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        })
    } else {
        binomial_pmf(trials, number_s, probability)
    };
    match result {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn binomial_distribution_range(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if !(3..=4).contains(&args.len()) {
        return Value::Error(ErrorKind::Value);
    }
    let trials = match truncated_number(engine, context, &args[0]) {
        Ok(value) if value >= 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let probability = match probability_number(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let first = match truncated_number(engine, context, &args[2]) {
        Ok(value) if (0.0..=trials).contains(&value) => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    // An absent number_s2 collapses the range to the single mass at number_s.
    let last = if let Some(argument) = args.get(3) {
        match truncated_number(engine, context, argument) {
            Ok(value) if (first..=trials).contains(&value) => value,
            Ok(_) => return Value::Error(ErrorKind::Num),
            Err(kind) => return Value::Error(kind),
        }
    } else {
        first
    };
    let summed = binomial_pmf_sum(trials, probability, first, last, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    });
    match summed {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn binomial_inverse(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let trials = match truncated_number(engine, context, &args[0]) {
        Ok(value) if value >= 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let probability = match probability_number(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    // Boundary policy: probability_s and alpha keep the inclusive [0, 1]
    // domain. Microsoft's worksheet page for BINOM.INV alone claims #NUM! at
    // the boundaries, contradicting its own VBA documentation (CritBinom,
    // Binom_Inv: errors only below 0 or above 1), ODF OpenFormula 1.3
    // §6.18.19 (0 ≤ Alpha ≤ 1), and interoperating engines, which pin
    // alpha = 0 → 0, alpha = 1 → trials, p = 0 → 0, p = 1 → trials·(alpha>0).
    let alpha = match probability_number(engine, context, &args[2]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let solved = smallest_binomial_quantile(trials, probability, alpha, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    });
    match solved {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn negative_binomial_distribution(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 4 {
        return Value::Error(ErrorKind::Value);
    }
    let (failures, successes, probability) =
        match negative_binomial_arguments(engine, context, args) {
            Ok(values) => values,
            Err(kind) => return Value::Error(kind),
        };
    let cumulative = match to_logical(&engine.eval_scalar(context, &args[3])) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let result = if cumulative {
        negative_binomial_cdf(failures, successes, probability, || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        })
    } else {
        negative_binomial_pmf(failures, successes, probability)
    };
    match result {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

/// NEGBINOMDIST is an argument-mapping adapter over the NEGBINOM.DIST
/// kernel: Microsoft documents byte-identical domain rules for both names,
/// so the legacy spelling differs only in arity and is always the
/// non-cumulative mass.
pub(super) fn negative_binomial_distribution_legacy(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let (failures, successes, probability) =
        match negative_binomial_arguments(engine, context, args) {
            Ok(values) => values,
            Err(kind) => return Value::Error(kind),
        };
    match negative_binomial_pmf(failures, successes, probability) {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

fn negative_binomial_arguments(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<(f64, f64, f64), ErrorKind> {
    let failures = match truncated_number(engine, context, &args[0]) {
        Ok(value) if value >= 0.0 => value,
        Ok(_) => return Err(ErrorKind::Num),
        Err(kind) => return Err(kind),
    };
    let successes = match truncated_number(engine, context, &args[1]) {
        Ok(value) if value >= 1.0 => value,
        Ok(_) => return Err(ErrorKind::Num),
        Err(kind) => return Err(kind),
    };
    let probability = probability_number(engine, context, &args[2])?;
    Ok((failures, successes, probability))
}

fn truncated_number(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<f64, ErrorKind> {
    required_number(engine, context, expr).map(f64::trunc)
}

fn probability_number(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<f64, ErrorKind> {
    match required_number(engine, context, expr) {
        Ok(value) if (0.0..=1.0).contains(&value) => Ok(value),
        Ok(_) => Err(ErrorKind::Num),
        Err(kind) => Err(kind),
    }
}
