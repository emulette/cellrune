//! t-distribution functions: T.DIST, T.DIST.RT, T.DIST.2T, T.INV, T.INV.2T,
//! TDIST and T.TEST.
//!
//! All names share one overflow-safe transform to the unit interval:
//! z = df/(df + x²) with w = 1 − z. T² ~ F(1, df) (FORMULAS.md) gives the
//! two-sided tail P(|T| > x) = I_z(df/2, 1/2, z) — the beta lower at the z
//! coordinate — and the density ((df+1)/2)·ln z − ln(df)/2 − lnB(1/2, df/2),
//! finite at x = 0 unlike the F. The exact log coordinates ln z, ln w are
//! derived from ln x directly so a coordinate that rounds to an endpoint
//! from an interior x still yields the representable subnormal tail, and so
//! the reflected tail always evaluates at the fine complement exp(ln w)
//! instead of the rounded f64 complement of a near-one z — the constant
//! 2⁻⁵³ unit-interval ULP would otherwise quantize every small tail into
//! ~f_z·2⁻⁵³ ≈ 1e-13 jumps. The beta central band never applies to the t:
//! one shape is exactly 1/2, far below the 1e6 threshold, so every tail is
//! a direct or reflected evaluation.

use super::super::super::ast::Expr;
use super::super::super::coerce::to_logical;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::array_common::poll_cancellation;
use super::super::moments::{NumericMoments, VarianceKind};
use super::super::special_functions::{
    DomainPolicy, invert_monotone_cdf, ln_beta, regularized_incomplete_beta_lower,
};
use super::super::statistical::numeric_pairs;
use super::super::util::required_number;
use super::f::{degrees_of_freedom, nonnegative_x, sample_moments};
use super::{finite, quantile_solver_error};

/// T.DIST(x, df, cumulative); x may carry any sign, the cumulative argument
/// is typed logical.
pub(super) fn t_distribution(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match finite_x(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let df = match degrees_of_freedom(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let cumulative = match to_logical(&engine.eval_scalar(context, &args[2])) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let on_iteration = || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    };
    if cumulative {
        match lower_tail(x, df, on_iteration) {
            Ok(value) => finite(value),
            Err(kind) => Value::Error(kind),
        }
    } else {
        match density(x, df) {
            Ok(value) => finite(value),
            Err(kind) => Value::Error(kind),
        }
    }
}

/// T.DIST.RT(x, df): the one-sided right tail P(T > x) for x ≥ 0.
pub(super) fn t_distribution_rt(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match nonnegative_x(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let df = match degrees_of_freedom(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let on_iteration = || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    };
    match right_tail(x, df, on_iteration) {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

/// T.DIST.2T(x, df): the two-sided tail P(|T| > x) for x ≥ 0.
pub(super) fn t_distribution_two_tail(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match nonnegative_x(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let df = match degrees_of_freedom(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let on_iteration = || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    };
    match two_tail(x, df, on_iteration) {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

/// T.INV(p, df): 0 < p < 1, with p = 0.5 mapping exactly to +0.0 by
/// symmetry. The magnitude solves I_z(df/2, 1/2, z) = 2·min(p, 1 − p).
pub(super) fn t_inverse(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let p = match required_number(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let df = match degrees_of_freedom(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    if p == 0.5 {
        return Value::Number(0.0);
    }
    if !(p > 0.0 && p < 1.0) {
        return Value::Error(ErrorKind::Num);
    }
    let two_tail_probability = 2.0 * p.min(1.0 - p);
    // The magnitude is solved in z-space, I_z(df/2, 1/2) = p, and restored
    // through x = √(df·w/z), mirroring the reference. An x-space solve of
    // the tail is numerically flat: near 1 the complement value sits on the
    // constant 2⁻⁵³ unit-interval grid, so the residual is exactly zero
    // across a wide plateau and bisection terminates at an arbitrary point
    // inside it. In z-space the target is evaluated with the kernel's fine
    // coordinates (see [`magnitude_pair`]), which pins the crossing to 4ε
    // of the coordinate with a smooth residual.
    match magnitude_pair(two_tail_probability, df, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    }) {
        Ok((z, w)) => {
            let magnitude = restore_t_coordinate(z, w, df);
            finite(if p < 0.5 { -magnitude } else { magnitude })
        }
        Err(kind) => Value::Error(quantile_solver_error(kind)),
    }
}

/// T.INV.2T(p, df): 0 < p ≤ 1, with p = 1 mapping exactly to +0.0. The
/// quantile solves I_z(df/2, 1/2, z) = p directly.
pub(super) fn t_inverse_two_tail(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let p = match required_number(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let df = match degrees_of_freedom(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    if p == 1.0 {
        return Value::Number(0.0);
    }
    if !(p > 0.0 && p < 1.0) {
        return Value::Error(ErrorKind::Num);
    }
    // See t_inverse: the quantile is solved in z-space at the kernel's fine
    // coordinates and restored through x = √(df·w/z).
    match magnitude_pair(p, df, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    }) {
        Ok((z, w)) => finite(restore_t_coordinate(z, w, df)),
        Err(kind) => Value::Error(quantile_solver_error(kind)),
    }
}

/// TDIST(x, df, tails): the legacy three-argument name. A typed descriptor
/// validating the tails argument and dispatching to the T.DIST.RT (1) or
/// T.DIST.2T (2) numeric paths — no independent implementation.
pub(super) fn tdist(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match nonnegative_x(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let df = match degrees_of_freedom(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let tails = match required_number(engine, context, &args[2]) {
        Ok(value) if value.trunc() == 1.0 => 1,
        Ok(value) if value.trunc() == 2.0 => 2,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let on_iteration = || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    };
    let result = if tails == 1 {
        right_tail(x, df, on_iteration)
    } else {
        two_tail(x, df, on_iteration)
    };
    match result {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

/// T.TEST(left, right, tails, type): the p-value min(1, tails·P(T > |t|))
/// with the t statistic and degrees of freedom of the selected test — paired
/// differences (1), pooled two-sample (2), or Welch with fractional
/// (untruncated) degrees of freedom (3).
pub(in crate::calculation::functions) fn t_test(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 4 {
        return Value::Error(ErrorKind::Value);
    }
    let tails = match tails_option(engine, context, &args[2]) {
        Ok(tails) => tails,
        Err(kind) => return Value::Error(kind),
    };
    let kind = match test_kind(engine, context, &args[3]) {
        Ok(kind) => kind,
        Err(kind) => return Value::Error(kind),
    };
    let on_iteration = || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    };
    let (statistic, df) = match kind {
        TestKind::Paired => match paired_difference_t(engine, context, args) {
            Ok(result) => result,
            Err(kind) => return Value::Error(kind),
        },
        TestKind::EqualVariance => match two_sample_t(engine, context, args, true) {
            Ok(result) => result,
            Err(kind) => return Value::Error(kind),
        },
        TestKind::Welch => match two_sample_t(engine, context, args, false) {
            Ok(result) => result,
            Err(kind) => return Value::Error(kind),
        },
    };
    match right_tail(statistic.abs(), df, on_iteration) {
        Ok(tail) => finite((tails as f64 * tail).min(1.0)),
        Err(kind) => Value::Error(kind),
    }
}

/// Overflow-safe t transform to (z, w) = (df/(df + x²), x²/(df + x²)). The
/// direct form is evaluated whenever x² is finite, exactly as the F kernel
/// prefers its direct transform; the ratio form (z = 1/(1 + r) with
/// r = (x/√df)²) is the fallback only when x² would overflow, and the
/// endpoint then falls to the exact log coordinates.
fn t_coordinates(x: f64, df: f64) -> (f64, f64) {
    if x == 0.0 {
        return (1.0, 0.0);
    }
    let squared = x * x;
    if squared.is_finite() {
        let z = df / (df + squared);
        (z, 1.0 - z)
    } else {
        let scaled = x / df.sqrt();
        let ratio = scaled * scaled;
        if ratio.is_finite() {
            let z = 1.0 / (1.0 + ratio);
            (z, 1.0 - z)
        } else {
            (0.0, 1.0)
        }
    }
}

/// Exact log coordinates of the t transform, derived from ln x so they
/// survive a coordinate rounding to an endpoint (FORMULAS.md).
fn t_log_coordinates(x: f64, df: f64) -> (f64, f64) {
    if x == 0.0 {
        return (0.0, f64::NEG_INFINITY);
    }
    let log_ratio = 2.0 * x.ln() - df.ln();
    if log_ratio <= 0.0 {
        let log_z = -log_ratio.exp().ln_1p();
        (log_z, log_ratio + log_z)
    } else {
        let log_w = -(-log_ratio).exp().ln_1p();
        (-log_ratio + log_w, log_w)
    }
}

/// P(|T| > x) for x ≥ 0: the beta lower I_z(df/2, 1/2, z) at the z
/// coordinate, so the tail is always a direct evaluation rather than a
/// complement of the CDF.
fn two_tail(
    x: f64,
    df: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let (z, _) = t_coordinates(x, df);
    let (log_z, log_w) = t_log_coordinates(x, df);
    // The exact transform logs go to the beta kernel unconditionally: the
    // reflected branch evaluates at the fine complement exp(log_w), whose
    // grid spacing (2⁻⁵³·w) is negligible compared with the f64 complement
    // of a rounded near-one z (constant 2⁻⁵³ steps that quantize every
    // small tail into ~f_z·2⁻⁵³ ≈ 1e-13 jumps). The direct branch uses the
    // logs only at a rounded endpoint, where the coordinate's own log is
    // meaningless; anywhere else it evaluates at the rounded z itself.
    regularized_incomplete_beta_lower(df / 2.0, 0.5, z, Some(log_z), Some(log_w), on_iteration)
}

/// P(T > x) for x ≥ 0: half the two-sided tail by symmetry.
fn right_tail(
    x: f64,
    df: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    Ok(two_tail(x, df, on_iteration)? / 2.0)
}

/// T.DIST(x, df, TRUE): the lower CDF, folded onto |x| so both signs share
/// one two-tail evaluation.
fn lower_tail(
    x: f64,
    df: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let tail = two_tail(x.abs(), df, on_iteration)? / 2.0;
    Ok(if x < 0.0 { tail } else { 1.0 - tail })
}

/// t density in log space: ((df+1)/2)·ln z − ln(df)/2 − lnB(1/2, df/2). The
/// kernel consumes the exact transform log ln z; at x = 0 the coordinate log
/// vanishes and the density is the finite value 1/(√df·B(1/2, df/2)) — no
/// pole, unlike the F.
fn density(x: f64, df: f64) -> Result<f64, ErrorKind> {
    let (log_z, _) = t_log_coordinates(x, df);
    let exponent = (df + 1.0) / 2.0 * log_z - df.ln() / 2.0 - ln_beta(0.5, df / 2.0)?;
    Ok(exponent.exp())
}

/// x = √(df·w/z). The square roots are taken before dividing so a subnormal
/// coordinate survives: 1/5e-324 = 2^1074 overflows f64, while √w/√z at the
/// same point is 2^537 and finite.
fn restore_t_coordinate(z: f64, w: f64, df: f64) -> f64 {
    df.sqrt() * (w.sqrt() / z.sqrt())
}

/// Solves I_z(df/2, 1/2, z) = p on the unit interval and returns (z, w) with
/// w the fine complement exp(ln(1 − z)). Every candidate is evaluated with
/// its exact log coordinates, mirroring [`two_tail`]: a candidate near one
/// has a coarse complement 1 − z whose unit-ULP rounding the large shape
/// df/2 amplifies into a ~(df/2)·2⁻⁵³ relative value error (5e-8 at
/// df = 1e9), far above the solver's probability residual tolerance, while
/// the exact log keeps the reflected evaluation on the fine grid. The pair
/// handed to the restore uses the fine complement for the same reason.
fn magnitude_pair(
    probability: f64,
    df: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind> + Clone,
) -> Result<(f64, f64), ErrorKind> {
    let a = df / 2.0;
    let mut iterations = on_iteration.clone();
    if probability > 0.5 {
        // p near 1: the crossing z* = df/(df + x²) can round to the z-grid
        // endpoint 1.0 (for x below ~√(df·2⁻⁵³) the coordinate is not
        // representable at all), where the CDF value quantizes at f_z·2⁻⁵³ —
        // far above the solver budget — so the z-solve would return the
        // endpoint with x̂ = 0. Solve the reflected side I_w(1/2, df/2, w)
        // = 1 − p on the fine w-grid instead: its crossing w* = x²/(df + x²)
        // is tiny and representable, and the solver budget scales with the
        // target 1 − p, which near one is exactly the required tail accuracy.
        // The restore consumes the coarse complement z = 1 − w, whose
        // rounding enters x at the negligible relative scale w/2.
        let w = invert_monotone_cdf(
            |position| {
                regularized_incomplete_beta_lower(
                    0.5,
                    a,
                    position,
                    Some(position.ln()),
                    Some((-position).ln_1p()),
                    &mut iterations,
                )
            },
            1.0 - probability,
            DomainPolicy::FiniteInterval {
                low: 0.0,
                high: 1.0,
            },
            on_iteration,
        )?;
        return Ok((1.0 - w, w));
    }
    let z = invert_monotone_cdf(
        |position| {
            regularized_incomplete_beta_lower(
                a,
                0.5,
                position,
                Some(position.ln()),
                Some((-position).ln_1p()),
                &mut iterations,
            )
        },
        probability,
        DomainPolicy::FiniteInterval {
            low: 0.0,
            high: 1.0,
        },
        on_iteration,
    )?;
    Ok((z, (-z).ln_1p().exp()))
}

/// x is a finite number of any sign for T.DIST.
fn finite_x(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    argument: &Expr,
) -> Result<f64, ErrorKind> {
    match required_number(engine, context, argument)? {
        value if value.is_finite() => Ok(value),
        _ => Err(ErrorKind::Num),
    }
}

/// The shared T.TEST tails contract: scalar numeric coercion, truncation
/// toward zero, and the two-value set {1, 2}.
fn tails_option(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    argument: &Expr,
) -> Result<u8, ErrorKind> {
    match required_number(engine, context, argument)?.trunc() {
        1.0 => Ok(1),
        2.0 => Ok(2),
        _ => Err(ErrorKind::Num),
    }
}

/// The shared T.TEST type contract: {1, 2, 3} after the same coercion.
fn test_kind(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    argument: &Expr,
) -> Result<TestKind, ErrorKind> {
    match required_number(engine, context, argument)?.trunc() {
        1.0 => Ok(TestKind::Paired),
        2.0 => Ok(TestKind::EqualVariance),
        3.0 => Ok(TestKind::Welch),
        _ => Err(ErrorKind::Num),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestKind {
    Paired,
    EqualVariance,
    Welch,
}

/// T.TEST type 1: paired differences d = x − y with
/// t = mean(d)/(s_d/√n) and df = n − 1. numeric_pairs preserves pair
/// positions and rejects unequal lengths (#N/A).
fn paired_difference_t(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<(f64, f64), ErrorKind> {
    let pairs = numeric_pairs(engine, context, args)?;
    let differences = pairs.into_iter().map(|(left, right)| left - right);
    let moments = NumericMoments::collect_with_work(differences, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    })?;
    paired_statistic(&moments)
}

/// The paired-difference statistic from already-collected moments of the
/// differences: t = mean(d)·√n/s_d with df = n − 1. Pure so the tests can
/// drive it without an Engine.
fn paired_statistic(moments: &NumericMoments) -> Result<(f64, f64), ErrorKind> {
    if moments.count() < 2 {
        return Err(ErrorKind::Div0);
    }
    let standard_deviation = moments.variance(VarianceKind::Sample)?.sqrt();
    if standard_deviation == 0.0 {
        return Err(ErrorKind::Div0);
    }
    let statistic = moments.mean()? * (moments.count() as f64).sqrt() / standard_deviation;
    Ok((statistic, (moments.count() - 1) as f64))
}

/// T.TEST types 2 and 3: two-sample t with pooled variance and integer
/// degrees of freedom (type 2) or Welch's unequal variances with the
/// fractional Welch–Satterthwaite degrees of freedom — the only df in the
/// family that is not truncated.
fn two_sample_t(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    pooled: bool,
) -> Result<(f64, f64), ErrorKind> {
    let left = sample_moments(engine, context, &args[0])?;
    let right = sample_moments(engine, context, &args[1])?;
    two_sample_statistic(&left, &right, pooled)
}

/// The two-sample statistic from already-collected sample moments: the
/// pooled-variance t with integer degrees of freedom (type 2) or Welch's
/// unequal-variances t with the fractional Welch–Satterthwaite degrees of
/// freedom (type 3). Pure so the tests can drive it without an Engine.
fn two_sample_statistic(
    left: &NumericMoments,
    right: &NumericMoments,
    pooled: bool,
) -> Result<(f64, f64), ErrorKind> {
    if left.count() < 2 || right.count() < 2 {
        return Err(ErrorKind::Div0);
    }
    let left_variance = left.variance(VarianceKind::Sample)?;
    let right_variance = right.variance(VarianceKind::Sample)?;
    let left_size = left.count() as f64;
    let right_size = right.count() as f64;
    let (standard_error, df) = if pooled {
        let left_m2 = left.second_moment();
        let right_m2 = right.second_moment();
        let scale = left_m2.max(right_m2);
        if scale == 0.0 {
            return Err(ErrorKind::Div0);
        }
        let df = left_size + right_size - 2.0;
        let pooled_variance = scale * ((left_m2 / scale + right_m2 / scale) / df);
        let standard_error = pooled_variance.sqrt() * (1.0 / left_size + 1.0 / right_size).sqrt();
        (standard_error, df)
    } else {
        let left_error = left_variance / left_size;
        let right_error = right_variance / right_size;
        let scale = left_error.max(right_error);
        if scale == 0.0 {
            return Err(ErrorKind::Div0);
        }
        let left_ratio = left_error / scale;
        let right_ratio = right_error / scale;
        let ratio_sum = left_ratio + right_ratio;
        let standard_error = scale.sqrt() * ratio_sum.sqrt();
        let df = ratio_sum * ratio_sum
            / (left_ratio * left_ratio / (left_size - 1.0)
                + right_ratio * right_ratio / (right_size - 1.0));
        (standard_error, df)
    };
    if standard_error == 0.0 {
        return Err(ErrorKind::Div0);
    }
    let left_mean = left.mean()?;
    let right_mean = right.mean()?;
    let difference = left_mean - right_mean;
    let statistic = if difference.is_finite() {
        difference / standard_error
    } else {
        left_mean / standard_error - right_mean / standard_error
    };
    if statistic.is_nan() || !df.is_finite() || df <= 0.0 {
        return Err(ErrorKind::Num);
    }
    Ok((statistic, df))
}

#[cfg(test)]
mod tests {
    use super::super::f::coordinate_logs;

    use super::{
        density, lower_tail, magnitude_pair, paired_statistic, restore_t_coordinate, right_tail,
        t_coordinates, t_log_coordinates, two_sample_statistic, two_tail,
    };
    use crate::calculation::functions::moments::NumericMoments;

    /// Plan §6.2 tolerance policy: CDF/direct tails in [1e-12, 1] use
    /// abs = 2e-14, rel = 2e-12; smaller tails use abs = 2 ULP, rel = 5e-9.
    fn assert_tail(actual: f64, expected: f64, what: &str) {
        if expected >= 1e-12 {
            assert_within(actual, expected, 2e-14, 2e-12, what);
        } else {
            assert_within(actual, expected, 2.0 * f64::from_bits(1), 5e-9, what);
        }
    }

    fn assert_density(actual: f64, expected: f64, what: &str) {
        assert_within(actual, expected, 2e-14, 2e-11, what);
    }

    /// Plan §6.4 inverse policy: x-space abs = 2e-12, rel = 2e-9, plus the
    /// f64 z-grid floor of the solve. The best ẑ lies within half a ULP of
    /// the Decimal-110 crossing — the CDF value quantizes at f_z·2⁻⁵³ per
    /// ULP, above the solver budget at the large-shape rows — and the restore
    /// x = √(df·w/z) amplifies that by x/(2w), up to ~1e9 at df = 1e10. The
    /// floor term (|x̂|/(2·ŵ))·½·ulp(ẑ) documents the irreducible grid
    /// distance; e.g. T.INV.2T(1e-6, 1e10−1) leaves a gap of 3.1e-8, above
    /// the 2e-9·x ≈ 9.8e-9 x-space tolerance alone. Rows whose coordinate
    /// rounds to an endpoint (the w-complement solves) need no term.
    fn assert_quantile(actual: f64, expected: f64, df: f64, what: &str) {
        let x = actual.abs();
        let (z, w) = t_coordinates(x, df);
        let grid_floor = if z > 0.0 && z < 1.0 && w > 0.0 {
            let ulp = f64::from_bits(z.to_bits() + 1) - z;
            (x / (2.0 * w)) * 0.5 * ulp
        } else {
            0.0
        };
        assert_within(actual, expected, 2e-12 + grid_floor, 2e-9, what);
    }

    fn assert_within(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, what: &str) {
        let diff = (actual - expected).abs();
        let limit = abs_tol + rel_tol * expected.abs();
        assert!(
            diff <= limit,
            "{what}: {actual} vs {expected} (diff {diff:e} > {limit:e})",
        );
    }

    fn t_inverse(p: f64, df: f64) -> f64 {
        if p == 0.5 {
            return 0.0;
        }
        let two_tail_probability = 2.0 * p.min(1.0 - p);
        let magnitude = two_tail_quantile(two_tail_probability, df);
        if p < 0.5 { -magnitude } else { magnitude }
    }

    /// The magnitude solve shared with the evaluator: inversion of the tail
    /// in the coordinate whose grid resolves it (I_z = p on the z-grid for
    /// p ≤ 1/2, I_w = 1 − p on the fine w-grid for p > 1/2), restored through
    /// x = √(df·w/z).
    fn two_tail_quantile(p: f64, df: f64) -> f64 {
        let (z, w) = magnitude_pair(p, df, || Ok(())).expect("solver converges");
        restore_t_coordinate(z, w, df)
    }

    // Cumulative grid. Reference: beta_lib.py (Decimal-110) evaluated at the
    // f64 t-coordinate D.from_float(z_f64) of each x literal, with the
    // reflected rows evaluated at the Decimal exp of the exact transform log
    // (the kernel's fine complement). The x = 1e300 / 1e307 rows sit below
    // the f64 subnormal floor (z = e^-1381.5) and evaluate at the Decimal
    // exp of the exact log coordinates: the tail they describe is still
    // representable (1/(πx) at df = 1), and the kernel produces it from the
    // logs. Negative rows carry the survival 1 - F(x).
    // Fields: (x, df, lower, upper).
    const CUMULATIVE_GRID: &[(f64, f64, f64, f64)] = &[
        (0.0, 1.0, 0.5, 0.5),
        (1e-300, 1.0, 0.5, 0.5),
        (1e-10, 1.0, 0.500000000031831, 0.499999999968169),
        (0.5, 1.0, 0.6475836176504333, 0.35241638234956674),
        (1.0, 1.0, 0.75, 0.25),
        (2.0, 1.0, 0.8524163823495667, 0.1475836176504333),
        (10.0, 1.0, 0.9682744825694465, 0.03172551743055357),
        (1000000.0, 1.0, 0.9999996816901138, 3.1830988618368455e-07),
        (1e+300, 1.0, 1.0, 3.1830988618379823e-301),
        (1e+307, 1.0, 1.0, 3.183098861837833e-308),
        (-1000000.0, 1.0, 3.1830988618368455e-07, 0.9999996816901138),
        (-10.0, 1.0, 0.03172551743055357, 0.9682744825694465),
        (-2.0, 1.0, 0.1475836176504333, 0.8524163823495667),
        (-1.0, 1.0, 0.25, 0.75),
        (-0.5, 1.0, 0.35241638234956674, 0.6475836176504333),
        (0.0, 2.0, 0.5, 0.5),
        (1e-300, 2.0, 0.5, 0.5),
        (1e-10, 2.0, 0.5000000000353554, 0.49999999996464467),
        (0.5, 2.0, 0.6666666666666667, 0.3333333333333333),
        (1.0, 2.0, 0.7886751345948129, 0.2113248654051871),
        (2.0, 2.0, 0.908248290463863, 0.09175170953613698),
        (10.0, 2.0, 0.9950737714883372, 0.004926228511662845),
        (1000000.0, 2.0, 0.9999999999995, 4.999999999992501e-13),
        (1e+300, 2.0, 1.0, 0.0),
        (1e+307, 2.0, 1.0, 0.0),
        (-1000000.0, 2.0, 4.999999999992501e-13, 0.9999999999995),
        (-10.0, 2.0, 0.004926228511662845, 0.9950737714883372),
        (-2.0, 2.0, 0.09175170953613698, 0.908248290463863),
        (-1.0, 2.0, 0.2113248654051871, 0.7886751345948129),
        (-0.5, 2.0, 0.3333333333333333, 0.6666666666666667),
        (0.0, 5.0, 0.5, 0.5),
        (1e-300, 5.0, 0.5, 0.5),
        (1e-10, 5.0, 0.5000000000379606, 0.4999999999620393),
        (0.5, 5.0, 0.6808505641795355, 0.3191494358204645),
        (1.0, 5.0, 0.8183912661754387, 0.1816087338245613),
        (2.0, 5.0, 0.9490302605850708, 0.05096973941492918),
        (10.0, 5.0, 0.9999145262121285, 8.547378787148179e-05),
        (1000000.0, 5.0, 1.0, 9.490167245460681e-30),
        (1e+300, 5.0, 1.0, 0.0),
        (1e+307, 5.0, 1.0, 0.0),
        (-1000000.0, 5.0, 9.490167245460681e-30, 1.0),
        (-10.0, 5.0, 8.547378787148179e-05, 0.9999145262121285),
        (-2.0, 5.0, 0.05096973941492918, 0.9490302605850708),
        (-1.0, 5.0, 0.1816087338245613, 0.8183912661754387),
        (-0.5, 5.0, 0.3191494358204645, 0.6808505641795355),
        (0.0, 30.0, 0.5, 0.5),
        (1e-300, 30.0, 0.5, 0.5),
        (1e-10, 30.0, 0.5000000000395632, 0.49999999996043676),
        (0.5, 30.0, 0.6896384975574363, 0.31036150244256366),
        (1.0, 30.0, 0.8373456922869851, 0.16265430771301495),
        (2.0, 30.0, 0.9726874775185085, 0.027312522481491536),
        (10.0, 30.0, 0.9999999999771237, 2.2876257041148084e-11),
        (1000000.0, 30.0, 1.0, 1.0364534648043785e-159),
        (1e+300, 30.0, 1.0, 0.0),
        (1e+307, 30.0, 1.0, 0.0),
        (-1000000.0, 30.0, 1.0364534648043785e-159, 1.0),
        (-10.0, 30.0, 2.2876257041148084e-11, 0.9999999999771237),
        (-2.0, 30.0, 0.027312522481491536, 0.9726874775185085),
        (-1.0, 30.0, 0.16265430771301495, 0.8373456922869851),
        (-0.5, 30.0, 0.31036150244256366, 0.6896384975574363),
        (0.0, 1000000.0, 0.5, 0.5),
        (1e-300, 1000000.0, 0.5, 0.5),
        (1e-10, 1000000.0, 0.5000000000398942, 0.4999999999601058),
        (0.5, 1000000.0, 0.6914624062638143, 0.3085375937361857),
        (1.0, 1000000.0, 0.841344625083211, 0.15865537491678902),
        (2.0, 1000000.0, 0.9772497330738124, 0.022750266926187625),
        (10.0, 1000000.0, 1.0, 7.639305384025062e-24),
        (1000000.0, 1000000.0, 1.0, 0.0),
        (1e+300, 1000000.0, 1.0, 0.0),
        (1e+307, 1000000.0, 1.0, 0.0),
        (-1000000.0, 1000000.0, 0.0, 1.0),
        (-10.0, 1000000.0, 7.639305384025062e-24, 1.0),
        (-2.0, 1000000.0, 0.022750266926187625, 0.9772497330738124),
        (-1.0, 1000000.0, 0.15865537491678902, 0.841344625083211),
        (-0.5, 1000000.0, 0.3085375937361857, 0.6914624062638143),
        (0.0, 10000000.0, 0.5, 0.5),
        (1e-300, 10000000.0, 0.5, 0.5),
        (1e-10, 10000000.0, 0.5000000000398942, 0.49999999996010575),
        (0.5, 10000000.0, 0.6914624557729925, 0.3085375442270075),
        (1.0, 10000000.0, 0.8413447339700071, 0.15865526602999294),
        (2.0, 10000000.0, 0.9772498545578985, 0.022750145442101556),
        (10.0, 10000000.0, 1.0, 7.621796147237043e-24),
        (1000000.0, 10000000.0, 1.0, 0.0),
        (1e+300, 10000000.0, 1.0, 0.0),
        (1e+307, 10000000.0, 1.0, 0.0),
        (-1000000.0, 10000000.0, 0.0, 1.0),
        (-10.0, 10000000.0, 7.621796147237043e-24, 1.0),
        (-2.0, 10000000.0, 0.022750145442101556, 0.9772498545578985),
        (-1.0, 10000000.0, 0.15865526602999294, 0.8413447339700071),
        (-0.5, 10000000.0, 0.3085375442270075, 0.6914624557729925),
        (0.0, 100000000.0, 0.5, 0.5),
        (1e-300, 100000000.0, 0.5, 0.5),
        (1e-10, 100000000.0, 0.5000000000398942, 0.49999999996010575),
        (0.5, 100000000.0, 0.691462460723911, 0.308537539276089),
        (1.0, 100000000.0, 0.841344744858689, 0.15865525514131099),
        (2.0, 100000000.0, 0.9772498667352962, 0.022750133264703754),
        (10.0, 100000000.0, 1.0, 7.620047295934089e-24),
        (1000000.0, 100000000.0, 1.0, 0.0),
        (1e+300, 100000000.0, 1.0, 0.0),
        (1e+307, 100000000.0, 1.0, 0.0),
        (-1000000.0, 100000000.0, 0.0, 1.0),
        (-10.0, 100000000.0, 7.620047295934089e-24, 1.0),
        (-2.0, 100000000.0, 0.022750133264703754, 0.9772498667352962),
        (-1.0, 100000000.0, 0.15865525514131099, 0.841344744858689),
        (-0.5, 100000000.0, 0.308537539276089, 0.691462460723911),
        (0.0, 1000000000.0, 0.5, 0.5),
        (1e-300, 1000000000.0, 0.5, 0.5),
        (1e-10, 1000000000.0, 0.5000000000398942, 0.49999999996010575),
        (0.5, 1000000000.0, 0.6914624612190029, 0.30853753878099716),
        (1.0, 1000000000.0, 0.8413447459475577, 0.15865525405244235),
        (2.0, 1000000000.0, 0.9772498681043887, 0.022750131895611217),
        (10.0, 1000000000.0, 1.0, 7.619872624804082e-24),
        (1000000.0, 1000000000.0, 1.0, 0.0),
        (1e+300, 1000000000.0, 1.0, 0.0),
        (1e+307, 1000000000.0, 1.0, 0.0),
        (-1000000.0, 1000000000.0, 0.0, 1.0),
        (-10.0, 1000000000.0, 7.619872624804082e-24, 1.0),
        (-2.0, 1000000000.0, 0.022750131895611217, 0.9772498681043887),
        (-1.0, 1000000000.0, 0.15865525405244235, 0.8413447459475577),
        (-0.5, 1000000000.0, 0.30853753878099716, 0.6914624612190029),
        (0.0, 9999999999.0, 0.5, 0.5),
        (1e-300, 9999999999.0, 0.5, 0.5),
        (1e-10, 9999999999.0, 0.5000000000398942, 0.49999999996010575),
        (0.5, 9999999999.0, 0.6914624612685122, 0.30853753873148787),
        (1.0, 9999999999.0, 0.8413447460564448, 0.1586552539435552),
        (2.0, 9999999999.0, 0.9772498725217524, 0.02275012747824752),
        (10.0, 9999999999.0, 1.0, 7.619853496405373e-24),
        (1000000.0, 9999999999.0, 1.0, 0.0),
        (1e+300, 9999999999.0, 1.0, 0.0),
        (1e+307, 9999999999.0, 1.0, 0.0),
        (-1000000.0, 9999999999.0, 0.0, 1.0),
        (-10.0, 9999999999.0, 7.619853496405373e-24, 1.0),
        (-2.0, 9999999999.0, 0.02275012747824752, 0.9772498725217524),
        (-1.0, 9999999999.0, 0.1586552539435552, 0.8413447460564448),
        (-0.5, 9999999999.0, 0.30853753873148787, 0.6914624612685122),
        (0.0, 3.7, 0.5, 0.5),
        (1e-300, 3.7, 0.5, 0.5),
        (1e-10, 3.7, 0.5000000000373162, 0.49999999996268385),
        (0.5, 3.7, 0.677332183340034, 0.322667816659966),
        (1.0, 3.7, 0.8109293001197738, 0.18907069988022623),
        (2.0, 3.7, 0.939091399047383, 0.060908600952617),
        (10.0, 3.7, 0.9995880165790877, 0.00041198342091232115),
        (1000000.0, 3.7, 1.0, 1.3771118493631556e-22),
        (1e+300, 3.7, 1.0, 0.0),
        (1e+307, 3.7, 1.0, 0.0),
        (-1000000.0, 3.7, 1.3771118493631556e-22, 1.0),
        (-10.0, 3.7, 0.00041198342091232115, 0.9995880165790877),
        (-2.0, 3.7, 0.060908600952617, 0.939091399047383),
        (-1.0, 3.7, 0.18907069988022623, 0.8109293001197738),
        (-0.5, 3.7, 0.322667816659966, 0.677332183340034),
        (1.0, 1000000.0, 0.841344625083211, 0.15865537491678902),
        (
            1.5537743900797805,
            1000000.0,
            0.9398807210797181,
            0.060119278920281904,
        ),
        (
            2.58009368457676,
            1000000.0,
            0.9950612536141583,
            0.004938746385841714,
        ),
        (
            3.5091068441049056,
            1000000.0,
            0.9997751830038698,
            0.0002248169961301986,
        ),
        (
            4.239203022684709,
            1000000.0,
            0.9999887832593027,
            1.1216740697274829e-05,
        ),
        (1.0, 10000000.0, 0.8413447339700071, 0.15865526602999294),
        (
            1.5537740156349737,
            10000000.0,
            0.9398808188123626,
            0.06011918118763742,
        ),
        (
            2.5800885966669718,
            10000000.0,
            0.9950612444170266,
            0.004938755582973363,
        ),
        (
            3.509091154887754,
            10000000.0,
            0.9997751786272823,
            0.0002248213727177749,
        ),
        (
            4.2391733510193275,
            10000000.0,
            0.999988782680892,
            1.1217319108047122e-05,
        ),
        (1.0, 100000000.0, 0.841344744858689, 0.15865525514131099),
        (
            1.5537739781905306,
            100000000.0,
            0.9398808285856323,
            0.06011917141436778,
        ),
        (
            2.5800880878776447,
            100000000.0,
            0.9950612435082968,
            0.004938756491703143,
        ),
        (
            3.509089585977585,
            100000000.0,
            0.9997751781900439,
            0.0002248218099560505,
        ),
        (
            4.2391703838870125,
            100000000.0,
            0.9999887826230607,
            1.1217376939328544e-05,
        ),
        (1.0, 1000000000.0, 0.8413447459475577, 0.15865525405244235),
        (
            1.5537739744460866,
            1000000000.0,
            0.9398808295629596,
            0.06011917043704044,
        ),
        (
            2.580088036998729,
            1000000000.0,
            0.9950612432540311,
            0.004938756745968886,
        ),
        (
            3.509089429086684,
            1000000000.0,
            0.9997751781493022,
            0.00022482185069786276,
        ),
        (
            4.239170087174124,
            1000000000.0,
            0.9999887826172631,
            1.1217382736861082e-05,
        ),
        (1.0, 9999999999.0, 0.8413447460564448, 0.1586552539435552),
        (
            1.5537739740716423,
            9999999999.0,
            0.9398808296606923,
            0.060119170339307734,
        ),
        (
            2.580088031910837,
            9999999999.0,
            0.9950612456668804,
            0.004938754333119617,
        ),
        (
            3.5090894133975947,
            9999999999.0,
            0.9997751781950922,
            0.00022482180490774212,
        ),
        (
            4.239170057502838,
            9999999999.0,
            0.9999887826105555,
            1.1217389444550972e-05,
        ),
    ];

    // z-coordinate rows: the t beta coordinate z = p - k*sigma_z around the
    // beta mean (p = df/(df+1), sigma_z = sqrt(a*b/(total^2*(total+1)))), k in
    // {0, 1, 4, 8, 12}, mapped to x = sqrt(df*w/z) and re-transformed at the
    // f64 x. The +k side leaves the unit interval, so only the lower side is
    // emitted. Fields: (x, df, lower, upper).
    const Z_COORDINATE_GRID: &[(f64, f64, f64, f64)] = &[
        (1.0, 1000000.0, 0.841344625083211, 0.15865537491678902),
        (
            1.5537743900797805,
            1000000.0,
            0.9398807210797181,
            0.060119278920281904,
        ),
        (
            2.58009368457676,
            1000000.0,
            0.9950612536141583,
            0.004938746385841714,
        ),
        (
            3.5091068441049056,
            1000000.0,
            0.9997751830038698,
            0.0002248169961301986,
        ),
        (
            4.239203022684709,
            1000000.0,
            0.9999887832593027,
            1.1216740697274829e-05,
        ),
        (1.0, 10000000.0, 0.8413447339700071, 0.15865526602999294),
        (
            1.5537740156349737,
            10000000.0,
            0.9398808188123626,
            0.06011918118763742,
        ),
        (
            2.5800885966669718,
            10000000.0,
            0.9950612444170266,
            0.004938755582973363,
        ),
        (
            3.509091154887754,
            10000000.0,
            0.9997751786272823,
            0.0002248213727177749,
        ),
        (
            4.2391733510193275,
            10000000.0,
            0.999988782680892,
            1.1217319108047122e-05,
        ),
        (1.0, 100000000.0, 0.841344744858689, 0.15865525514131099),
        (
            1.5537739781905306,
            100000000.0,
            0.9398808285856323,
            0.06011917141436778,
        ),
        (
            2.5800880878776447,
            100000000.0,
            0.9950612435082968,
            0.004938756491703143,
        ),
        (
            3.509089585977585,
            100000000.0,
            0.9997751781900439,
            0.0002248218099560505,
        ),
        (
            4.2391703838870125,
            100000000.0,
            0.9999887826230607,
            1.1217376939328544e-05,
        ),
        (1.0, 1000000000.0, 0.8413447459475577, 0.15865525405244235),
        (
            1.5537739744460866,
            1000000000.0,
            0.9398808295629596,
            0.06011917043704044,
        ),
        (
            2.580088036998729,
            1000000000.0,
            0.9950612432540311,
            0.004938756745968886,
        ),
        (
            3.509089429086684,
            1000000000.0,
            0.9997751781493022,
            0.00022482185069786276,
        ),
        (
            4.239170087174124,
            1000000000.0,
            0.9999887826172631,
            1.1217382736861082e-05,
        ),
        (1.0, 9999999999.0, 0.8413447460564448, 0.1586552539435552),
        (
            1.5537739740716423,
            9999999999.0,
            0.9398808296606923,
            0.060119170339307734,
        ),
        (
            2.580088031910837,
            9999999999.0,
            0.9950612456668804,
            0.004938754333119617,
        ),
        (
            3.5090894133975947,
            9999999999.0,
            0.9997751781950922,
            0.00022482180490774212,
        ),
        (
            4.239170057502838,
            9999999999.0,
            0.9999887826105555,
            1.1217389444550972e-05,
        ),
    ];

    #[test]
    fn cumulative_and_tails_match_the_decimal_reference() {
        for &(x, df, expected_lower, expected_upper) in
            CUMULATIVE_GRID.iter().chain(Z_COORDINATE_GRID)
        {
            let actual_lower = lower_tail(x, df, || Ok(())).expect("finite lower tail");
            assert_tail(
                actual_lower,
                expected_lower,
                &format!("T.DIST({x}, {df}, TRUE)"),
            );
            if x >= 0.0 {
                let actual_upper = right_tail(x, df, || Ok(())).expect("finite upper tail");
                assert_tail(
                    actual_upper,
                    expected_upper,
                    &format!("T.DIST.RT({x}, {df})"),
                );
            } else {
                // The negative-x fixture column is the survival 1 - F(x),
                // exactly 1 - lower_tail(x) at this precision.
                let actual_upper = 1.0 - actual_lower;
                assert_tail(
                    actual_upper,
                    expected_upper,
                    &format!("1 - T.DIST({x}, {df}, TRUE)"),
                );
            }
        }
    }

    // Density grid. Reference: the exact Decimal transform coordinates (the
    // kernel adds the exact transform log), Decimal-110. The t density is
    // finite at x = 0 for every df, unlike the F.
    // Fields: (x, df, density).
    // The literals are Decimal-110 reference values that must stay byte-exact
    // (e.g. the 1/π density at x = 0 for df = 1); they are not code
    // approximations to be replaced by std constants.
    #[allow(clippy::approx_constant)]
    const DENSITY_GRID: &[(f64, f64, f64)] = &[
        (0.0, 1.0, 0.3183098861837907),
        (1e-300, 1.0, 0.3183098861837907),
        (1e-10, 1.0, 0.3183098861837907),
        (0.5, 1.0, 0.25464790894703254),
        (1.0, 1.0, 0.15915494309189535),
        (2.0, 1.0, 0.06366197723675814),
        (10.0, 1.0, 0.0031515830315226798),
        (1000000.0, 1.0, 3.1830988618347235e-13),
        (0.0, 2.0, 0.3535533905932738),
        (1e-300, 2.0, 0.3535533905932738),
        (1e-10, 2.0, 0.3535533905932738),
        (0.5, 2.0, 0.2962962962962963),
        (1.0, 2.0, 0.19245008972987526),
        (2.0, 2.0, 0.06804138174397717),
        (10.0, 2.0, 0.0009707328852712493),
        (1000000.0, 2.0, 9.99999999997e-19),
        (0.0, 5.0, 0.37960668982249446),
        (1e-300, 5.0, 0.37960668982249446),
        (1e-10, 5.0, 0.37960668982249446),
        (0.5, 5.0, 0.3279185313227465),
        (1.0, 5.0, 0.21967979735098056),
        (2.0, 5.0, 0.06509031032621647),
        (10.0, 5.0, 4.098981641534331e-05),
        (1000000.0, 5.0, 4.745083622710004e-35),
        (0.0, 30.0, 0.39563218489409774),
        (1e-300, 30.0, 0.39563218489409774),
        (1e-10, 30.0, 0.39563218489409774),
        (0.5, 30.0, 0.34787857969720454),
        (1.0, 30.0, 0.23799334232287983),
        (2.0, 30.0, 0.05685227504719796),
        (10.0, 30.0, 5.327814578274168e-11),
        (1000000.0, 30.0, 3.1093603943227675e-164),
        (0.0, 1000000.0, 0.39894218066587506),
        (1e-300, 1000000.0, 0.39894218066587506),
        (1e-10, 1000000.0, 0.39894218066587506),
        (0.5, 1000000.0, 0.35206520024085),
        (1.0, 1000000.0, 0.2419706035338315),
        (2.0, 1000000.0, 0.053991060997102186),
        (10.0, 1000000.0, 7.713470311059334e-23),
        (1000000.0, 1000000.0, 0.0),
        (0.0, 10000000.0, 0.3989422704278758),
        (1e-300, 10000000.0, 0.3989422704278758),
        (1e-10, 10000000.0, 0.3989422704278758),
        (0.5, 10000000.0, 0.3520653141119521),
        (1.0, 10000000.0, 0.24197071242060764),
        (2.0, 10000000.0, 0.05399097596160442),
        (10.0, 10000000.0, 7.6964838292759e-23),
        (1000000.0, 10000000.0, 0.0),
        (0.0, 100000000.0, 0.398942279404077),
        (1e-300, 100000000.0, 0.398942279404077),
        (1e-10, 100000000.0, 0.398942279404077),
        (0.5, 100000000.0, 0.3520653254990647),
        (1.0, 100000000.0, 0.24197072330928973),
        (2.0, 100000000.0, 0.05399096745802994),
        (10.0, 100000000.0, 7.694787127318844e-23),
        (1000000.0, 100000000.0, 0.0),
        (0.0, 1000000000.0, 0.3989422803016971),
        (1e-300, 1000000000.0, 0.3989422803016971),
        (1e-10, 1000000000.0, 0.3989422803016971),
        (0.5, 1000000000.0, 0.352065326637776),
        (1.0, 1000000000.0, 0.24197072439815798),
        (2.0, 1000000000.0, 0.05399096660767224),
        (10.0, 1000000000.0, 7.694617476571231e-23),
        (1000000.0, 1000000000.0, 0.0),
        (0.0, 9999999999.0, 0.3989422803914591),
        (1e-300, 9999999999.0, 0.3989422803914591),
        (1e-10, 9999999999.0, 0.3989422803914591),
        (0.5, 9999999999.0, 0.35206532675164715),
        (1.0, 9999999999.0, 0.24197072450704482),
        (2.0, 9999999999.0, 0.05399096652263647),
        (10.0, 9999999999.0, 7.694600511690936e-23),
        (1000000.0, 9999999999.0, 0.0),
        (0.0, 3.7, 0.3731615494594903),
        (1e-300, 3.7, 0.3731615494594903),
        (1e-10, 3.7, 0.3731615494594903),
        (0.5, 3.7, 0.32001310564065993),
        (1.0, 3.7, 0.21268701353831673),
        (2.0, 3.7, 0.06666804733614608),
        (10.0, 3.7, 0.00014794178750277996),
        (1000000.0, 3.7, 5.0953138426281305e-28),
    ];

    #[test]
    fn density_matches_the_decimal_reference() {
        for &(x, df, expected) in DENSITY_GRID {
            let actual = density(x, df).expect("finite density");
            assert_density(actual, expected, &format!("T.DIST({x}, {df}, FALSE)"));
        }
        // No pole at the origin: the density at x = 0 equals the density at
        // x = 1e-300 (the origin grid column).
        for df in [1.0, 2.0, 30.0, 1000000.0] {
            let at_origin = density(0.0, df).expect("finite density");
            let near_origin = density(1e-300, df).expect("finite density");
            assert_density(
                at_origin,
                near_origin,
                &format!("origin density at df = {df}"),
            );
        }
    }

    // Quantile grid. Reference: bisection of I_z(df/2, 1/2, z) = p at
    // Decimal-110 with D.from_float(p) (the exact f64 probability); T.INV
    // solves the two-sided equation at 2*min(p, 1-p) and flips the sign below
    // p = 0.5, T.INV.2T solves it at p directly.
    // Fields: (p, df, signed_quantile, two_tail_quantile).
    const QUANTILE_GRID: &[(f64, f64, f64, f64)] = &[
        (1e-15, 1.0, -318309886183790.6, 636619772367581.2),
        (1e-15, 2.0, -22360679.774997864, 31622776.60168377),
        (1e-06, 5.0, -24.771029720515944, 28.47847346298421),
        (0.5, 1.0, 0.0, 1.0),
        (0.5, 2.0, 0.0, 0.816496580927726),
        (0.9, 5.0, 1.4758840488244813, 0.13217517523168723),
        (0.9, 30.0, 1.3104150253913958, 0.12672961313207357),
        (
            0.999999999999999,
            1.0,
            318564507734592.1,
            1.5695408241038843e-15,
        ),
        (
            0.999999999999999,
            2.0,
            22369621.3333333,
            1.4130832128153975e-15,
        ),
        (0.5, 1000000.0, 0.0, 0.6744899955310873),
        (0.9, 1000000.0, 1.2815524121299386, 0.12566137876648747),
        (
            0.999999999999999,
            100000000.0,
            7.94144575936834,
            1.2523123942330761e-15,
        ),
        (1e-06, 1000000000.0, -4.753424336862212, 4.891638506183437),
        (0.5, 9999999999.0, 0.0, 0.6744897502206152),
        (1e-06, 9999999999.0, -4.75342431162683, 4.891638478747075),
    ];

    #[test]
    fn quantiles_match_the_decimal_reference() {
        for &(p, df, expected_signed, expected_two_tail) in QUANTILE_GRID {
            let actual_signed = t_inverse(p, df);
            assert_quantile(
                actual_signed,
                expected_signed,
                df,
                &format!("T.INV({p}, {df})"),
            );
            let actual_two_tail = two_tail_quantile(p, df);
            assert_quantile(
                actual_two_tail,
                expected_two_tail,
                df,
                &format!("T.INV.2T({p}, {df})"),
            );
        }
    }

    /// Round-trip floor: the forward transform z_cdf(x̂) = df/(df + x̂²) rounds
    /// at the df-magnitude addition and the division (~1 ULP of z combined),
    /// and the quantile itself lands within ½ ULP of the crossing, so the CDF
    /// value at the returned x̂ differs from the target by up to f_z·2·ulp(ẑ),
    /// with f_z = f_t(x̂)·df/(x̂·ẑ²) the z-space density of the tail. The
    /// plan's 1e-15 + 1e-9·p limit is unreachable at the large-shape rows
    /// (e.g. ~2.9e-14 vs 3e-15 at p = 2e-6, df = 1e9), so the floor term
    /// documents the grid distance.
    fn round_trip_limit(p: f64, df: f64, quantile: f64) -> f64 {
        let mut limit = 1e-15 + 1e-9 * p;
        let x = quantile.abs();
        let (z, w) = t_coordinates(x, df);
        if z > 0.0 && z < 1.0 && w > 0.0 {
            let ulp = f64::from_bits(z.to_bits() + 1) - z;
            let density_z = density(x, df).expect("finite density") * df / (x * z * z);
            limit += density_z * 2.0 * ulp;
        }
        limit
    }

    #[test]
    fn quantiles_round_trip_through_the_cdf() {
        for &(p, df, _, _) in QUANTILE_GRID {
            let quantile = t_inverse(p, df);
            let cdf = lower_tail(quantile, df, || Ok(())).expect("finite CDF");
            let diff = (cdf - p).abs();
            let limit = round_trip_limit(p, df, quantile);
            assert!(
                diff <= limit,
                "T.DIST(T.INV({p}, {df}), TRUE) = {cdf} (diff {diff:e} > {limit:e})",
            );
            let two_tail_quantile = two_tail_quantile(p, df);
            let tail = two_tail(two_tail_quantile, df, || Ok(())).expect("finite tail");
            let diff = (tail - p).abs();
            let limit = round_trip_limit(p, df, two_tail_quantile);
            assert!(
                diff <= limit,
                "T.DIST.2T(T.INV.2T({p}, {df})) = {tail} (diff {diff:e} > {limit:e})",
            );
        }
    }

    // T.TEST grid. Reference: min(1, tails*I_z(df/2, 1/2, z)/2) at |t| from
    // Decimal sample moments (exact pair differences for type 1, unbiased
    // sample variance for types 2/3). Fields: (left, right, tails, kind, p).
    type TTestRow = (&'static [f64], &'static [f64], f64, f64, f64);
    const T_TEST_GRID: &[TTestRow] = &[
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[2.0, 3.0, 4.0, 5.0, 7.0],
            1.0,
            1.0,
            0.0019412685234802554,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[2.0, 3.0, 4.0, 5.0, 7.0],
            2.0,
            1.0,
            0.0038825370469605107,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[2.0, 3.0, 4.0, 5.0, 7.0],
            1.0,
            2.0,
            0.15630844825868645,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[2.0, 3.0, 4.0, 5.0, 7.0],
            2.0,
            2.0,
            0.3126168965173729,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[2.0, 3.0, 4.0, 5.0, 7.0],
            1.0,
            3.0,
            0.15687580473123758,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[2.0, 3.0, 4.0, 5.0, 7.0],
            2.0,
            3.0,
            0.31375160946247516,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[10.0, 11.0, 12.0, 13.0, 15.0],
            1.0,
            1.0,
            6.679175857660975e-07,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[10.0, 11.0, 12.0, 13.0, 15.0],
            2.0,
            1.0,
            1.335835171532195e-06,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[10.0, 11.0, 12.0, 13.0, 15.0],
            1.0,
            2.0,
            1.7303565426098327e-05,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[10.0, 11.0, 12.0, 13.0, 15.0],
            2.0,
            2.0,
            3.4607130852196655e-05,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[10.0, 11.0, 12.0, 13.0, 15.0],
            1.0,
            3.0,
            2.1430466214904908e-05,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[10.0, 11.0, 12.0, 13.0, 15.0],
            2.0,
            3.0,
            4.2860932429809816e-05,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[1.0, 2.0, 3.0, 4.0, 100.0],
            1.0,
            1.0,
            0.18695048315002943,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[1.0, 2.0, 3.0, 4.0, 100.0],
            2.0,
            1.0,
            0.37390096630005887,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[1.0, 2.0, 3.0, 4.0, 100.0],
            1.0,
            2.0,
            0.17943190754871124,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[1.0, 2.0, 3.0, 4.0, 100.0],
            2.0,
            2.0,
            0.3588638150974225,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[1.0, 2.0, 3.0, 4.0, 100.0],
            1.0,
            3.0,
            0.19266958132768283,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[1.0, 2.0, 3.0, 4.0, 100.0],
            2.0,
            3.0,
            0.38533916265536566,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0],
            &[2.5, 3.5, 4.5],
            1.0,
            2.0,
            0.15942883488918855,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0],
            &[2.5, 3.5, 4.5],
            2.0,
            2.0,
            0.3188576697783771,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0],
            &[2.5, 3.5, 4.5],
            1.0,
            3.0,
            0.15040135362758808,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0],
            &[2.5, 3.5, 4.5],
            2.0,
            3.0,
            0.30080270725517616,
        ),
        (
            &[1.0, 2.0],
            &[1.1, 2.1, 3.1, 4.1, 5.1, 6.1],
            1.0,
            2.0,
            0.09405146244624,
        ),
        (
            &[1.0, 2.0],
            &[1.1, 2.1, 3.1, 4.1, 5.1, 6.1],
            2.0,
            2.0,
            0.18810292489248,
        ),
        (
            &[1.0, 2.0],
            &[1.1, 2.1, 3.1, 4.1, 5.1, 6.1],
            1.0,
            3.0,
            0.03330004437421452,
        ),
        (
            &[1.0, 2.0],
            &[1.1, 2.1, 3.1, 4.1, 5.1, 6.1],
            2.0,
            3.0,
            0.06660008874842904,
        ),
        (
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0, 4.0],
            1.0,
            2.0,
            0.30194844884486643,
        ),
        (
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0, 4.0],
            2.0,
            2.0,
            0.6038968976897329,
        ),
        (
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0, 4.0],
            1.0,
            3.0,
            0.2944607746429128,
        ),
        (
            &[1.0, 2.0, 3.0],
            &[1.0, 2.0, 3.0, 4.0],
            2.0,
            3.0,
            0.5889215492858256,
        ),
    ];

    #[test]
    fn t_test_p_values_match_the_decimal_reference() {
        for &(left, right, tails, kind, expected_p) in T_TEST_GRID {
            let (statistic, df) = if kind == 1.0 {
                let differences = left.iter().zip(right).map(|(l, r)| l - r);
                let moments = NumericMoments::collect_with_work(differences, || Ok(()))
                    .expect("fixture samples are finite");
                paired_statistic(&moments).expect("paired statistic")
            } else {
                let left_moments =
                    NumericMoments::collect_with_work(left.iter().copied(), || Ok(()))
                        .expect("fixture samples are finite");
                let right_moments =
                    NumericMoments::collect_with_work(right.iter().copied(), || Ok(()))
                        .expect("fixture samples are finite");
                two_sample_statistic(&left_moments, &right_moments, kind == 2.0)
                    .expect("two-sample statistic")
            };
            let tail = right_tail(statistic.abs(), df, || Ok(())).expect("finite tail");
            let actual_p = (tails * tail).min(1.0);
            assert_tail(
                actual_p,
                expected_p,
                &format!("T.TEST({left:?}, {right:?}, {tails}, {kind})"),
            );
        }
    }

    #[test]
    fn two_sample_statistics_scale_before_summing_or_squaring() {
        let pooled_left =
            NumericMoments::collect_with_work([-7e153, 7e153], || Ok(())).expect("finite sample");
        let pooled_right =
            NumericMoments::collect_with_work([-7e153, 7e153], || Ok(())).expect("finite sample");
        let (pooled_statistic, pooled_df) =
            two_sample_statistic(&pooled_left, &pooled_right, true).expect("stable pooled test");
        assert_eq!(pooled_statistic, 0.0);
        assert_eq!(pooled_df, 2.0);

        let welch_left =
            NumericMoments::collect_with_work([-1e100, 1e100], || Ok(())).expect("finite sample");
        let welch_right =
            NumericMoments::collect_with_work([-2e100, 2e100], || Ok(())).expect("finite sample");
        let (welch_statistic, welch_df) =
            two_sample_statistic(&welch_left, &welch_right, false).expect("stable Welch test");
        assert_eq!(welch_statistic, 0.0);
        assert_within(welch_df, 25.0 / 17.0, 2e-15, 2e-15, "extreme Welch df");
    }

    #[test]
    fn coordinates_and_log_coordinates_survive_extreme_x() {
        // x² overflows at x = 1e300, so the direct transform falls to the
        // ratio form; the coordinate rounds to the endpoint (z = 0.0,
        // w = 1.0), and coordinate_logs hands the kernels the exact logs.
        // ln z = -1381.55 describes a coordinate below the f64 subnormal
        // floor — only the log survives — yet the tail 1/(πx) at df = 1 is
        // representable and the kernel produces it from the logs.
        let (z, w) = t_coordinates(1e300, 1.0);
        assert_eq!(z, 0.0);
        assert_eq!(w, 1.0);
        let (log_z, log_w) = t_log_coordinates(1e300, 1.0);
        assert_within(
            log_z,
            -1_381.551_055_796_427_4,
            1e-12,
            1e-15,
            "log_z at x = 1e300",
        );
        assert_eq!(log_w, -0.0);

        let (z, w) = t_coordinates(1e-300, 1.0);
        assert_eq!(z, 1.0);
        assert_eq!(w, 0.0);
        let (log_z, log_w) = t_log_coordinates(1e-300, 1.0);
        assert_eq!(log_z, -0.0);
        assert_within(
            log_w,
            -1_381.551_055_796_427_4,
            1e-12,
            1e-15,
            "log_w at x = 1e-300",
        );

        // Endpoint-rounded coordinates from interior x hand their exact logs
        // to the kernels; true endpoints do not.
        let (z, w) = t_coordinates(1e300, 1.0);
        let (log_z, log_w) = t_log_coordinates(1e300, 1.0);
        assert_eq!(
            coordinate_logs(1e300, z, w, log_z, log_w),
            (Some(log_z), Some(log_w)),
        );
        assert_eq!(
            coordinate_logs(0.0, 0.0, 1.0, f64::NEG_INFINITY, 0.0),
            (None, None)
        );
    }

    #[test]
    fn t_test_reports_division_by_zero_for_constant_differences() {
        // All paired differences equal (Excel's oracle pair 10..100 vs
        // 110..200): the standard deviation of the differences is exactly
        // zero, so the t statistic divides by zero and T.TEST is #DIV/0!,
        // not #VALUE!.
        let left = [10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0];
        let right = [
            110.0, 120.0, 130.0, 140.0, 150.0, 160.0, 170.0, 180.0, 190.0, 200.0,
        ];
        let differences = left.iter().zip(right).map(|(l, r)| l - r);
        let moments =
            NumericMoments::collect_with_work(differences, || Ok(())).expect("fixture samples");
        assert_eq!(
            moments
                .variance(crate::calculation::functions::moments::VarianceKind::Sample)
                .expect("zero variance"),
            0.0
        );
        assert_eq!(
            paired_statistic(&moments),
            Err(crate::calculation::value::ErrorKind::Div0)
        );
    }

    #[test]
    fn restore_scales_before_dividing_so_subnormal_coordinates_survive() {
        // x = sqrt(df) * sqrt(w/z) with z subnormal and w = 1.0: the product
        // (df*w)/z alone would overflow (1e10/5e-324 = inf), so restore takes
        // the square roots first: sqrt(1/5e-324) = sqrt(2^1074) = 2^537, and
        // 1e5 * 2^537 = 4.4989137945431964e166 (exact arithmetic; the f64
        // literal 5e-324 is 2^-1074 exactly).
        let restored = restore_t_coordinate(5e-324, 1.0, 1e10);
        assert!(restored.is_finite());
        assert_within(
            restored,
            4.4989137945431964e166,
            1e153,
            1e-13,
            "restore divides before scaling",
        );
    }
}
