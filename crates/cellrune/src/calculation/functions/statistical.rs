use super::kernel::StatisticalFunction;
use std::collections::BTreeMap;

use super::super::ast::Expr;
use super::super::coerce::to_logical;
use super::super::criteria::CompiledCriteria;
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::runtime::Rect;
use super::super::sheet_span::SheetSpanPolicy;
use super::super::value::{ErrorKind, Value};
use super::array_common::poll_cancellation;
use super::criteria_runtime::CriteriaRuntime;
use super::moments::{NumericMoments, PairedMoments, VarianceKind};
use super::special_functions::{
    standard_normal_density, standard_normal_lower, standard_normal_upper,
};
use super::util::{
    ExcelSum, collect_argument_values, excel_numeric_arguments,
    excel_numeric_arguments_with_policy, required_number,
};

/// PROB accepts probabilities whose total differs from one by at most this much, the tolerance
/// LibreOffice applies, so decimal probabilities that sum to one in exact arithmetic are accepted
/// despite binary rounding.
const PROBABILITY_TOTAL_TOLERANCE: f64 = 1e-7;

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: StatisticalFunction,
    args: &[Expr],
) -> Value {
    match function {
        StatisticalFunction::Large => order_statistic(engine, context, args, false),
        StatisticalFunction::Small => order_statistic(engine, context, args, true),
        StatisticalFunction::Median => median(engine, context, args),
        StatisticalFunction::ModeSingle => mode(engine, context, args),
        StatisticalFunction::Correl => {
            paired_statistic(engine, context, args, PairedStatistic::Correlation)
        }
        StatisticalFunction::Slope => {
            paired_statistic(engine, context, args, PairedStatistic::Slope)
        }
        StatisticalFunction::Steyx => {
            paired_statistic(engine, context, args, PairedStatistic::StandardError)
        }
        StatisticalFunction::PercentRankInc => {
            percent_rank(engine, context, args, RankInterval::Inclusive)
        }
        StatisticalFunction::PercentRankExc => {
            percent_rank(engine, context, args, RankInterval::Exclusive)
        }
        StatisticalFunction::PercentileInc => {
            percentile(engine, context, args, false, RankInterval::Inclusive)
        }
        StatisticalFunction::PercentileExc => {
            percentile(engine, context, args, false, RankInterval::Exclusive)
        }
        StatisticalFunction::QuartileInc => {
            percentile(engine, context, args, true, RankInterval::Inclusive)
        }
        StatisticalFunction::QuartileExc => {
            percentile(engine, context, args, true, RankInterval::Exclusive)
        }
        StatisticalFunction::TrimMean => trim_mean(engine, context, args),
        StatisticalFunction::Prob => probability(engine, context, args),
        StatisticalFunction::RankEq => rank(engine, context, args, false),
        StatisticalFunction::RankAvg => rank(engine, context, args, true),
        StatisticalFunction::ForecastLinear => forecast(engine, context, args),
        StatisticalFunction::NormSDistLegacy => {
            standard_normal_distribution(engine, context, args, true)
        }
        StatisticalFunction::NormSDist => {
            standard_normal_distribution(engine, context, args, false)
        }
        StatisticalFunction::StDevS => sample_variance(engine, context, args, true),
        StatisticalFunction::VarS => sample_variance(engine, context, args, false),
        StatisticalFunction::MinIfs => conditional_extreme(engine, context, args, true),
        StatisticalFunction::MaxIfs => conditional_extreme(engine, context, args, false),
        StatisticalFunction::Pearson => {
            paired_statistic(engine, context, args, PairedStatistic::Correlation)
        }
        StatisticalFunction::Rsq => rsq(engine, context, args),
        StatisticalFunction::Intercept => {
            paired_statistic(engine, context, args, PairedStatistic::Intercept)
        }
        StatisticalFunction::CovarianceP => paired_statistic(
            engine,
            context,
            args,
            PairedStatistic::Covariance(VarianceKind::Population),
        ),
        StatisticalFunction::CovarianceS => paired_statistic(
            engine,
            context,
            args,
            PairedStatistic::Covariance(VarianceKind::Sample),
        ),
        StatisticalFunction::FTest => super::distribution::f::f_test(engine, context, args),
        StatisticalFunction::TTest => super::distribution::t::t_test(engine, context, args),
        StatisticalFunction::ZTest => z_test(engine, context, args),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PairedStatistic {
    Correlation,
    Slope,
    Intercept,
    /// STEYX: the standard error of the predicted y for each x of the linear fit.
    StandardError,
    Covariance(VarianceKind),
}

/// Which order-statistic positions a percentile or percent rank spans: the .INC functions place
/// the n sorted values at ranks 0 through n − 1 of an interval with n − 1 gaps, while the .EXC
/// functions place them at ranks 1 through n of an interval with n + 1 gaps, excluding its ends.
#[derive(Debug, Clone, Copy)]
enum RankInterval {
    Inclusive,
    Exclusive,
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
    let moments = match paired_moments(engine, context, args) {
        Ok(moments) => moments,
        Err(kind) => return Value::Error(kind),
    };
    if let PairedStatistic::Covariance(kind) = statistic {
        return match moments.covariance(kind) {
            Ok(covariance) => Value::Number(covariance),
            Err(kind) => Value::Error(kind),
        };
    }
    if statistic == PairedStatistic::StandardError && moments.count() < 3 {
        return Value::Error(ErrorKind::Div0);
    }
    let right_deviation = moments.right_second_moment();
    if right_deviation == 0.0
        || statistic == PairedStatistic::Correlation && moments.left_second_moment() == 0.0
    {
        return Value::Error(ErrorKind::Div0);
    }
    let result = match statistic {
        PairedStatistic::Intercept => {
            let left_mean = match moments.left_mean() {
                Ok(mean) => mean,
                Err(kind) => return Value::Error(kind),
            };
            let right_mean = match moments.right_mean() {
                Ok(mean) => mean,
                Err(kind) => return Value::Error(kind),
            };
            left_mean - moments.co_moment() / right_deviation * right_mean
        }
        PairedStatistic::Correlation => {
            moments.co_moment() / moments.left_second_moment().sqrt() / right_deviation.sqrt()
        }
        PairedStatistic::Slope => moments.co_moment() / right_deviation,
        PairedStatistic::StandardError => {
            // The residual sum of squares of an exact fit can round below zero; clamp it like the
            // variance kernels clamp their second moment.
            let residual = (moments.left_second_moment()
                - moments.co_moment() * (moments.co_moment() / right_deviation))
                .max(0.0);
            (residual / (moments.count() - 2) as f64).sqrt()
        }
        PairedStatistic::Covariance(_) => unreachable!("covariance returned before finalization"),
    };
    if result.is_finite() {
        Value::Number(result)
    } else {
        Value::Error(ErrorKind::Num)
    }
}

fn paired_moments(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<PairedMoments, ErrorKind> {
    let pairs = numeric_pairs(engine, context, args)?;
    if pairs.is_empty() {
        return Err(ErrorKind::Div0);
    }
    PairedMoments::collect_with_work(pairs, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    })
}

fn forecast(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [x, _, _] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let result = (|| {
        let x = required_number(engine, context, x)?;
        let moments = paired_moments(engine, context, &args[1..])?;
        let deviation = moments.right_second_moment();
        if deviation == 0.0 {
            return Err(ErrorKind::Div0);
        }
        // Center the prediction instead of subtracting a large intercept.
        let predicted =
            moments.left_mean()? + moments.co_moment() / deviation * (x - moments.right_mean()?);
        if predicted.is_finite() {
            Ok(predicted)
        } else {
            Err(ErrorKind::Num)
        }
    })();
    match result {
        Ok(value) => Value::Number(value),
        Err(kind) => Value::Error(kind),
    }
}

/// PROB(x_range, prob_range, lower_limit, [upper_limit]): the total probability of the paired
/// values within the inclusive limits, or of the values equal to `lower_limit` when the upper
/// limit is omitted. Every probability must lie in [0, 1] and together they must sum to one.
fn probability(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if !(3..=4).contains(&args.len()) {
        return Value::Error(ErrorKind::Value);
    }
    let result = (|| {
        let pairs = numeric_pairs(engine, context, &args[..2])?;
        let lower = required_number(engine, context, &args[2])?;
        let upper = match args.get(3) {
            Some(expr) => required_number(engine, context, expr)?,
            None => lower,
        };
        let mut total: ExcelSum = ExcelSum::new(engine);
        let mut within: ExcelSum = ExcelSum::new(engine);
        for (value, probability) in pairs {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)?;
            // Microsoft documents #NUM! at or below 0, but Excel accepts a zero probability
            // (PROB({1,2},{0,1},1,2) is 1 in Excel 16.0), so only values outside [0, 1] fail.
            if !(0.0..=1.0).contains(&probability) {
                return Err(ErrorKind::Num);
            }
            total.add_with_trace(probability, None);
            if (lower..=upper).contains(&value) {
                within.add_with_trace(probability, None);
            }
        }
        if (total.total() - 1.0).abs() > PROBABILITY_TOTAL_TOLERANCE {
            return Err(ErrorKind::Num);
        }
        Ok(within.total())
    })();
    match result {
        Ok(value) => Value::Number(value),
        Err(kind) => Value::Error(kind),
    }
}

/// Pure Z.TEST core (plan §6.5): p = Q((mean − x)/se) from the sample
/// moments. An explicit sigma uses se = sigma/√n and needs sigma > 0; an
/// omitted sigma falls back to the sample standard error √(M2/(n·(n−1))),
/// which requires n ≥ 2 and M2 > 0. The p-value is the direct
/// standard-normal upper tail — small p-values are never formed as 1 − lower.
/// Divide first only when the difference overflows; the direct quotient
/// preserves the cancellation structure of the Welford mean.
fn z_test_p_value(
    mean: f64,
    m2: f64,
    n: u64,
    x: f64,
    sigma: Option<f64>,
) -> Result<f64, ErrorKind> {
    let se = match sigma {
        Some(sigma) if sigma.is_finite() && sigma > 0.0 => sigma / (n as f64).sqrt(),
        Some(_) => return Err(ErrorKind::Num),
        None => {
            if n < 2 || m2 == 0.0 {
                return Err(ErrorKind::Div0);
            }
            let nf = n as f64;
            (m2 / (nf * (nf - 1.0))).sqrt()
        }
    };
    let difference = mean - x;
    if difference.is_nan() {
        return Err(ErrorKind::Num);
    }
    let z = if se == 0.0 {
        if difference == 0.0 {
            0.0
        } else {
            f64::INFINITY.copysign(difference)
        }
    } else if difference.is_finite() {
        difference / se
    } else {
        mean / se - x / se
    };
    if z.is_nan() {
        Err(ErrorKind::Num)
    } else {
        Ok(standard_normal_upper(z))
    }
}

/// Z.TEST(array, x, [sigma]): sample moments from the array (the empty sample
/// is #N/A), then the §6.5 p-value.
fn z_test(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let moments = match super::distribution::f::sample_moments(engine, context, &args[0]) {
        Ok(moments) if moments.count() > 0 => moments,
        Ok(_) => return Value::Error(ErrorKind::NA),
        Err(kind) => return Value::Error(kind),
    };
    let x = match required_number(engine, context, &args[1]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let sigma = match args.get(2) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => Some(number),
            Err(kind) => return Value::Error(kind),
        },
        None => None,
    };
    let mean = match moments.mean() {
        Ok(mean) => mean,
        Err(kind) => return Value::Error(kind),
    };
    match z_test_p_value(mean, moments.second_moment(), moments.count(), x, sigma) {
        Ok(p_value) => Value::Number(p_value),
        Err(kind) => Value::Error(kind),
    }
}

fn percent_rank(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    interval: RankInterval,
) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let numbers = match numeric_arguments(engine, context, &args[..1]) {
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
    // Excel ranks a lone matching value at 1 in PERCENTRANK, .INC, and .EXC alike.
    if numbers.len() == 1 {
        return if numbers[0] == target {
            Value::Number(1.0)
        } else {
            Value::Error(ErrorKind::NA)
        };
    }
    // One pass finds how many values rank below the target and its nearest neighbours, which is
    // all the sorted position needs.
    let mut below = 0_usize;
    let mut equal = false;
    let mut nearest_below = f64::NEG_INFINITY;
    let mut nearest_above = f64::INFINITY;
    for &number in &numbers {
        if number < target {
            below += 1;
            nearest_below = nearest_below.max(number);
        } else if number > target {
            nearest_above = nearest_above.min(number);
        } else {
            equal = true;
        }
    }
    if !equal && (below == 0 || below == numbers.len()) {
        return Value::Error(ErrorKind::NA);
    }
    let (first_rank, gaps) = match interval {
        RankInterval::Inclusive => (0, numbers.len() - 1),
        RankInterval::Exclusive => (1, numbers.len() + 1),
    };
    let rank = if equal {
        (first_rank + below) as f64
    } else {
        let lower = first_rank + below - 1;
        let fraction = (target - nearest_below) / (nearest_above - nearest_below);
        lower as f64 + fraction
    };
    let raw_rank = rank / gaps as f64;
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
        Value::Number(standard_normal_lower(z))
    } else {
        Value::Number(standard_normal_density(z))
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

pub(super) fn numeric_pairs(
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
        poll_cancellation(context)?;
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
    if rank > numbers.len() {
        return Value::Error(ErrorKind::Num);
    }
    let index = if ascending {
        rank - 1
    } else {
        numbers.len() - rank
    };
    Value::Number(select_ascending(&mut numbers, index))
}

/// Returns the value at `index` of the ascending order and leaves smaller values before it and
/// larger values after it, without sorting the rest.
fn select_ascending(numbers: &mut [f64], index: usize) -> f64 {
    *numbers.select_nth_unstable_by(index, f64::total_cmp).1
}

/// The smallest value, in ascending order, of a non-empty slice.
fn minimum(numbers: &[f64]) -> f64 {
    numbers
        .iter()
        .copied()
        .min_by(f64::total_cmp)
        .expect("selection partitions are non-empty")
}

fn median(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let mut numbers = match numeric_arguments(engine, context, args) {
        Ok(numbers) if !numbers.is_empty() => numbers,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let middle = numbers.len() / 2;
    let upper = select_ascending(&mut numbers, middle);
    if numbers.len().is_multiple_of(2) {
        let lower = numbers[..middle]
            .iter()
            .copied()
            .max_by(f64::total_cmp)
            .expect("an even count leaves values below the middle");
        Value::Number(midpoint(lower, upper))
    } else {
        Value::Number(upper)
    }
}

/// TRIMMEAN(array, percent): the mean after excluding the same number of values from each end,
/// the trimmed count rounded down to an even number of values.
fn trim_mean(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [values, percent] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let mut numbers = match numeric_arguments(engine, context, std::slice::from_ref(values)) {
        Ok(numbers) if !numbers.is_empty() => numbers,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let percent = match required_number(engine, context, percent) {
        Ok(percent) if (0.0..1.0).contains(&percent) => percent,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    // A percent below one trims fewer than half the values from each end, so some remain.
    let trimmed = (numbers.len() as f64 * percent / 2.0).floor() as usize;
    let kept = numbers.len() - 2 * trimmed;
    select_ascending(&mut numbers, trimmed);
    let upper = &mut numbers[trimmed..];
    select_ascending(upper, kept - 1);
    let moments = match NumericMoments::collect_with_work(upper[..kept].iter().copied(), || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    }) {
        Ok(moments) => moments,
        Err(kind) => return Value::Error(kind),
    };
    match moments.sum() {
        Ok(sum) => Value::Number(sum / kept as f64),
        Err(kind) => Value::Error(kind),
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
    interval: RankInterval,
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
        probability = probability.trunc() / 4.0;
    }
    let count = numbers.len() as f64;
    // Zero-based position of the percentile among the sorted values.
    let position = match interval {
        RankInterval::Inclusive if (0.0..=1.0).contains(&probability) => {
            (count - 1.0) * probability
        }
        RankInterval::Exclusive if (1.0..=count).contains(&(probability * (count + 1.0))) => {
            probability * (count + 1.0) - 1.0
        }
        _ => return Value::Error(ErrorKind::Num),
    };
    let lower = position.floor() as usize;
    let fraction = position - lower as f64;
    let lower_value = select_ascending(&mut numbers, lower);
    let upper_value = if lower + 1 < numbers.len() {
        minimum(&numbers[lower + 1..])
    } else {
        lower_value
    };
    Value::Number(interpolate(lower_value, upper_value, fraction))
}

/// The mean of two finite numbers, halving first only when their sum overflows.
fn midpoint(lower: f64, upper: f64) -> f64 {
    let sum = lower + upper;
    if sum.is_finite() {
        sum / 2.0
    } else {
        lower / 2.0 + upper / 2.0
    }
}

/// Linear interpolation that stays finite when the gap between finite endpoints overflows.
fn interpolate(lower: f64, upper: f64, fraction: f64) -> f64 {
    if fraction == 0.0 {
        return lower;
    }
    let gap = upper - lower;
    if gap.is_finite() {
        lower + gap * fraction
    } else {
        lower * (1.0 - fraction) + upper * fraction
    }
}

fn rank(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr], average: bool) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let number = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let numbers = match numeric_arguments(engine, context, &args[1..2]) {
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
    let mut preceding = 0_u64;
    let mut equal = 0_u64;
    for candidate in numbers {
        if let Err(kind) =
            poll_cancellation(context).and_then(|()| engine.charge_function_iterations(context, 1))
        {
            return Value::Error(kind);
        }
        if candidate == number {
            equal += 1;
        } else if if ascending {
            candidate < number
        } else {
            candidate > number
        } {
            preceding += 1;
        }
    }
    if equal == 0 {
        return Value::Error(ErrorKind::NA);
    }
    let tied_offset = if average {
        (equal - 1) as f64 / 2.0
    } else {
        0.0
    };
    Value::Number(preceding as f64 + 1.0 + tied_offset)
}

fn sample_variance(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    square_root: bool,
) -> Value {
    match numeric_arguments_with_policy(engine, context, args, SheetSpanPolicy::CollectAcrossSheets)
    {
        Ok(numbers) => variance_value(engine, context, numbers, VarianceKind::Sample, square_root),
        Err(kind) => Value::Error(kind),
    }
}

/// Returns the variance, or with `square_root` the standard deviation, of collected numbers.
pub(super) fn variance_value(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    numbers: Vec<f64>,
    kind: VarianceKind,
    square_root: bool,
) -> Value {
    let minimum = match kind {
        VarianceKind::Sample => 2,
        VarianceKind::Population => 1,
    };
    if numbers.len() < minimum {
        return Value::Error(ErrorKind::Div0);
    }
    let moments = match NumericMoments::collect_with_work(numbers, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    }) {
        Ok(moments) => moments,
        Err(kind) => return Value::Error(kind),
    };
    match moments.variance(kind) {
        Ok(variance) if square_root => Value::Number(variance.sqrt()),
        Ok(variance) => Value::Number(variance),
        Err(kind) => Value::Error(kind),
    }
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
    let mut runtime = CriteriaRuntime::new(engine, context);
    let criteria = match parse_pairs(engine, context, &args[1..], value_range, &mut runtime) {
        Ok(criteria) => criteria,
        Err(kind) => return Value::Error(kind),
    };
    let iter_rows = engine.operation_row_count(
        criteria
            .iter()
            .map(|(range, _)| range)
            .chain(std::iter::once(&value_range)),
    );
    let visits = iter_rows
        .checked_mul(value_range.width())
        .and_then(|cells| cells.checked_mul(criteria.len() as u64 + 1));
    if visits.is_none_or(|cells| engine.ensure_array_cells(cells).is_err()) {
        return Value::Error(ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells));
    }
    let mut result: Option<f64> = None;
    for row in 0..iter_rows as u32 {
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
    runtime: &mut CriteriaRuntime<'_, '_, '_>,
) -> Result<Vec<(Rect, CompiledCriteria)>, ErrorKind> {
    let mut result = Vec::new();
    for pair in args.chunks_exact(2) {
        let rect = engine.resolve_rect_expr(context, &pair[0])?;
        if rect.height() != value_range.height() || rect.width() != value_range.width() {
            return Err(ErrorKind::Value);
        }
        result.push((
            rect,
            runtime.compile_criteria(&engine.eval_scalar(context, &pair[1]))?,
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::z_test_p_value;
    use crate::calculation::functions::moments::{NumericMoments, PairedMoments, VarianceKind};
    use crate::calculation::value::ErrorKind;

    fn assert_within(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, what: &str) {
        let diff = (actual - expected).abs();
        let limit = abs_tol + rel_tol * expected.abs();
        assert!(
            actual.is_finite() && expected.is_finite() && diff <= limit,
            "{what}: {actual} vs {expected} (diff {diff:e} > {limit:e})",
        );
    }

    /// Plan §6.5 tolerance policy: p-values use the direct-tail table
    /// (abs = 2e-14, rel = 2e-12 in [1e-12, 1]; abs = 2 ULP, rel = 5e-9
    /// below), mirroring T.TEST/F.TEST.
    fn assert_tail(actual: f64, expected: f64, what: &str) {
        if expected >= 1e-12 {
            assert_within(actual, expected, 2e-14, 2e-12, what);
        } else {
            assert_within(actual, expected, 2.0 * f64::from_bits(1), 5e-9, what);
        }
    }

    #[test]
    fn z_test_p_values_match_the_decimal_reference() {
        // Z.TEST grid. Reference: plan §6.5, mpmath 1.3.0 erfc at mp.dps=100,
        // z from Decimal-110 Welford moments of the exact f64 literals.
        // Fields: (sample, x, sigma, p). The offset rows verify the Welford
        // mean stays exact at 1e9 scale (kernel result is bit-identical to
        // the reference on the paired grid below).
        const Z_TEST_GRID: &[(&[f64], f64, Option<f64>, f64)] = &[
            (&[1.0, 2.0, 3.0, 4.0, 5.0], 3.0, None, 0.5),
            (
                &[1.0, 2.0, 3.0, 4.0, 5.0],
                3.0,
                Some(1.5811388300841898),
                0.5,
            ),
            (&[1.0, 2.0, 3.0, 4.0, 5.0], 10.0, None, 1.0),
            (&[1.0, 2.0, 3.0, 4.0, 5.0], 5.25, None, 0.9992686417066594),
            (&[1.0, 2.0, 3.0, 4.0, 5.0], -1000000.0, None, 0.0),
            (
                &[1.0, 2.0, 3.0, 4.0, 5.0],
                -2.3033008588991066,
                None,
                3.1908916729108894e-14,
            ),
            (&[1.0, 2.0, 3.0, 4.0, 5.0], 2.5, None, 0.23975006109347674),
            (
                &[1.0, 2.0, 3.0, 4.0, 5.0],
                2.5,
                Some(1.5811388300841898),
                0.23975006109347674,
            ),
            (
                &[1.0, 2.0, 3.0, 4.0, 5.0, 10.0, 100.0],
                3.0,
                None,
                0.1396861721364197,
            ),
            (
                &[1.0, 2.0, 3.0, 4.0, 5.0, 10.0, 100.0],
                3.0,
                Some(1.5811388300841898),
                9.892192793507626e-137,
            ),
            (
                &[1.0, 2.0, 3.0, 4.0, 5.0, 10.0, 100.0],
                10.0,
                None,
                0.2836376293013515,
            ),
            (
                &[1.0, 2.0, 3.0, 4.0, 5.0, 10.0, 100.0],
                100.25,
                None,
                0.9999999990068481,
            ),
            (
                &[1.0, 2.0, 3.0, 4.0, 5.0, 10.0, 100.0],
                -1000000.0,
                None,
                0.0,
            ),
            (
                &[1.0, 2.0, 3.0, 4.0, 5.0, 10.0, 100.0],
                -2.3033008588991066,
                None,
                0.07107150279647395,
            ),
            (
                &[1000000000.1, 1000000000.2, 1000000000.3, 1000000000.4],
                3.0,
                None,
                0.0,
            ),
            (
                &[1000000000.1, 1000000000.2, 1000000000.3, 1000000000.4],
                1000000000.65,
                None,
                0.9999999997118401,
            ),
        ];
        for &(sample, x, sigma, expected_p) in Z_TEST_GRID {
            let moments = NumericMoments::collect_with_work(sample.iter().copied(), || Ok(()))
                .expect("finite sample");
            let mean = moments.mean().expect("finite mean");
            let actual_p = z_test_p_value(mean, moments.second_moment(), moments.count(), x, sigma)
                .expect("valid z test input");
            let label = format!("Z.TEST({sample:?}, {x}, {sigma:?})");
            assert_tail(actual_p, expected_p, &label);
        }
    }

    #[test]
    fn z_test_omitted_sigma_matches_explicit_sigma() {
        // Plan fixture: on [1..5] the omitted path √(M2/(n·(n−1))) and the
        // explicit path with sigma = STDEV.S([1..5]) both round se to the
        // same f64 (0.7071067811865476), so the p-values agree to well below
        // the ~3.5e-15 the plan documents for this pair.
        let moments = NumericMoments::collect_with_work([1.0, 2.0, 3.0, 4.0, 5.0], || Ok(()))
            .expect("finite sample");
        let mean = moments.mean().expect("finite mean");
        let omitted = z_test_p_value(mean, moments.second_moment(), moments.count(), 2.5, None)
            .expect("omitted sigma valid");
        let explicit = z_test_p_value(
            mean,
            moments.second_moment(),
            moments.count(),
            2.5,
            Some(1.5811388300841898),
        )
        .expect("explicit sigma valid");
        let label = "Z.TEST([1,2,3,4,5], 2.5): omitted vs explicit sigma";
        assert_within(omitted, explicit, 3.5e-15, 0.0, label);
        assert_tail(omitted, 0.23975006109347674, label);
    }

    #[test]
    fn z_test_rejects_bad_sigma_and_undersized_samples() {
        // Explicit sigma <= 0 (or NaN) is #NUM!; the empty sample is #N/A at
        // the engine level, and the omitted path needs n >= 2 with M2 > 0.
        let moments = NumericMoments::collect_with_work([1.0, 2.0, 3.0, 4.0, 5.0], || Ok(()))
            .expect("finite sample");
        let (mean, m2) = (moments.mean().expect("mean"), moments.second_moment());
        for sigma in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                z_test_p_value(mean, m2, 5, 2.5, Some(sigma)),
                Err(ErrorKind::Num),
                "sigma = {sigma}"
            );
        }
        assert_eq!(
            z_test_p_value(mean, m2, 1, 2.5, None),
            Err(ErrorKind::Div0),
            "single-element sample needs explicit sigma"
        );
        assert_eq!(
            z_test_p_value(mean, 0.0, 5, 2.5, None),
            Err(ErrorKind::Div0),
            "zero sample variance with omitted sigma"
        );
        // A constant sample also has M2 == 0 through the collector.
        let constant =
            NumericMoments::collect_with_work([2.0, 2.0, 2.0], || Ok(())).expect("finite");
        assert_eq!(
            z_test_p_value(
                constant.mean().expect("mean"),
                constant.second_moment(),
                constant.count(),
                2.0,
                None,
            ),
            Err(ErrorKind::Div0),
            "constant sample with omitted sigma"
        );
        // Explicit sigma works on a single-element sample (n >= 1).
        let single = NumericMoments::collect_with_work([2.0], || Ok(())).expect("finite");
        assert_tail(
            z_test_p_value(
                single.mean().expect("mean"),
                single.second_moment(),
                single.count(),
                2.5,
                Some(1.0),
            )
            .expect("single-element sample with explicit sigma"),
            0.6914624612740131,
            "Z.TEST([2], 2.5, 1)",
        );
        // Non-finite x makes z non-finite.
        assert_eq!(
            z_test_p_value(mean, m2, 5, f64::NAN, None),
            Err(ErrorKind::Num),
            "NaN x"
        );
    }

    #[test]
    fn z_test_maps_overflowed_internal_scores_to_exact_tails() {
        let moments =
            NumericMoments::collect_with_work([1.0, 2.0], || Ok(())).expect("finite sample");
        let mean = moments.mean().expect("finite mean");
        assert_eq!(
            z_test_p_value(
                mean,
                moments.second_moment(),
                moments.count(),
                -1e308,
                Some(1e-308),
            ),
            Ok(0.0),
        );
        assert_eq!(
            z_test_p_value(
                mean,
                moments.second_moment(),
                moments.count(),
                1e308,
                Some(1e-308),
            ),
            Ok(1.0),
        );

        let minimum_subnormal = f64::from_bits(1);
        assert_eq!(
            z_test_p_value(1.5, 1.0, 4, 1.5, Some(minimum_subnormal)),
            Ok(0.5),
        );
        assert_eq!(
            z_test_p_value(1.5, 1.0, 4, 1.0, Some(minimum_subnormal)),
            Ok(0.0),
        );
        assert_eq!(
            z_test_p_value(1.5, 1.0, 4, 2.0, Some(minimum_subnormal)),
            Ok(1.0),
        );
    }

    #[test]
    fn covariance_s_matches_the_decimal_reference() {
        // COVARIANCE.S grid. Reference: plan §6.2, Decimal-110 paired
        // Welford, C/(n-1) at the exact f64 literals. The offset rows verify
        // the compensated paired moments stay exact beside 1e9-scale values
        // (the kernel result is bit-identical to the reference here).
        // Fields: (left, right, covariance).
        const COVARIANCE_S_GRID: &[(&[f64], &[f64], f64)] = &[
            (&[1.0, 2.0, 3.0, 4.0, 5.0], &[2.0, 3.0, 4.0, 5.0, 7.0], 3.0),
            (
                &[1.0, 2.0, 3.0, 4.0, 5.0],
                &[10.0, 11.0, 12.0, 13.0, 15.0],
                3.0,
            ),
            (
                &[1000000000.1, 1000000000.2, 1000000000.3, 1000000000.4],
                &[-999999999.5, -999999999.4, -999999999.3, -999999999.2],
                0.01666666070620219,
            ),
        ];
        for &(left, right, expected) in COVARIANCE_S_GRID {
            let moments = PairedMoments::collect_with_work(
                left.iter().copied().zip(right.iter().copied()),
                || Ok(()),
            )
            .expect("finite paired input");
            let actual = moments
                .covariance(VarianceKind::Sample)
                .expect("at least two accepted pairs");
            let label = format!("COVARIANCE.S({left:?}, {right:?})");
            assert_within(actual, expected, 2e-12, 2e-12, &label);
        }
    }

    #[test]
    fn covariance_s_requires_two_pairs() {
        // One accepted pair (or none) is #DIV/0!; the length mismatch is
        // #N/A at the engine level (numeric_pairs).
        let one =
            PairedMoments::collect_with_work([(1.0, 2.0)], || Ok(())).expect("finite paired input");
        assert_eq!(
            one.covariance(VarianceKind::Sample),
            Err(ErrorKind::Div0),
            "single pair"
        );
        let empty = PairedMoments::collect_with_work(std::iter::empty(), || Ok(()))
            .expect("finite paired input");
        assert_eq!(
            empty.covariance(VarianceKind::Sample),
            Err(ErrorKind::Div0),
            "no pairs"
        );
        // The population denominator still works on the single pair
        // (COVARIANCE.P semantics unchanged).
        assert_eq!(
            one.covariance(VarianceKind::Population),
            Ok(0.0),
            "COVARIANCE.P on one pair"
        );
    }
}
