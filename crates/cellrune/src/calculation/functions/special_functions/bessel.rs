//! Integer-order cylindrical Bessel kernels and their worksheet boundary.
//!
//! The oscillatory J/Y family deliberately has a different recurrence policy
//! from the exponential I/K family.  In particular, I/K keep their internal
//! primitives in exponentially scaled coordinates so intermediate overflow
//! cannot decide a worksheet result.

use super::double_double::DoubleDouble;
use super::ln_gamma;
use crate::calculation::ast::Expr;
use crate::calculation::eval::{Engine, EvalContext};
use crate::calculation::functions::array_common::poll_cancellation;
use crate::calculation::functions::util::required_number;
use crate::calculation::value::{ErrorKind, Value};

const MAX_ORDER: u32 = 100_000;
const MAX_SERIES_TERMS: usize = 100_000;
const SERIES_EPSILON: f64 = 8.0 * f64::EPSILON;
const LN_MAX_FINITE: f64 = 709.782_712_893_384;
const LN_MIN_SUBNORMAL: f64 = -744.440_071_921_381_2;
const EULER_GAMMA: f64 = 0.577_215_664_901_532_9;
const SMALL_K_SEED_CUTOFF: f64 = 0.1;
const OSCILLATORY_SERIES_CUTOFF: f64 = 40.0;
const HANKEL_ASYMPTOTIC_CUTOFF: f64 = 8.148_143_905_337_944e90;

/// The worksheet-visible Bessel families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::calculation::functions) enum BesselFamily {
    I,
    J,
    K,
    Y,
}

/// Implements the common Excel-facing policy.  The numerical kernels below
/// receive a validated non-negative order and their natural positive-domain
/// argument; coercion, parity and non-finite result policy stay here.
pub(in crate::calculation::functions) fn worksheet_bessel(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    family: BesselFamily,
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match required_number(engine, context, &args[0]) {
        Ok(value) if value.is_finite() => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let order = match required_number(engine, context, &args[1]).and_then(normalized_order) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };

    if matches!(family, BesselFamily::K | BesselFamily::Y) && x <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }

    let mut on_iteration = || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    };
    let positive_x = x.abs();
    let result = match family {
        BesselFamily::I => bessel_i(order, positive_x, &mut on_iteration),
        BesselFamily::J => bessel_j(order, positive_x, &mut on_iteration),
        BesselFamily::K => bessel_k(order, x, &mut on_iteration),
        BesselFamily::Y => bessel_y(order, x, &mut on_iteration),
    };
    match result {
        Ok(mut value) => {
            if matches!(family, BesselFamily::I | BesselFamily::J)
                && x.is_sign_negative()
                && order % 2 == 1
            {
                value = -value;
            }
            if value.is_finite() {
                Value::Number(value)
            } else {
                Value::Error(ErrorKind::Num)
            }
        }
        Err(kind) => Value::Error(kind),
    }
}

fn normalized_order(value: f64) -> Result<u32, ErrorKind> {
    if !value.is_finite() || value < 0.0 {
        return Err(ErrorKind::Num);
    }
    let truncated = value.trunc();
    if truncated > f64::from(MAX_ORDER) {
        return Err(ErrorKind::Num);
    }
    Ok(truncated as u32)
}

/// J_n(x), x >= 0.  Branches follow DLMF 10.2.E2 and 10.6.E1:
/// <https://dlmf.nist.gov/10.2.E2>, <https://dlmf.nist.gov/10.6.E1>.
pub(in crate::calculation::functions) fn bessel_j(
    order: u32,
    x: f64,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    validate_nonnegative_argument(x)?;
    match select_j_branch(order, x) {
        JBranch::PowerSeries => j_series(order, x, &mut on_iteration),
        JBranch::ForwardRecurrence => j_forward(order, x, &mut on_iteration),
        JBranch::MillerBackward => j_miller(order, x, &mut on_iteration),
        // libm's fdlibm-derived large-argument path is a Hankel asymptotic
        // seed. It has no iterative work to charge at this boundary.
        JBranch::HankelAsymptotic => Ok(libm::jn(order as i32, x)),
    }
}

/// Y_n(x), x > 0.  The y0/y1 seeds are dedicated approximations rather than
/// an integer-order J combination, avoiding the singular cancellation in
/// DLMF 10.2.E3.  Recurrence is DLMF 10.6.E1.
pub(in crate::calculation::functions) fn bessel_y(
    order: u32,
    x: f64,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    validate_positive_argument(x)?;
    let (mut previous, mut current) = y_seeds(x, &mut on_iteration)?;
    if order == 0 {
        return Ok(previous);
    }
    if order == 1 {
        return Ok(current);
    }
    for index in 1..order {
        on_iteration()?;
        let next = (2.0 * f64::from(index) / x) * current - previous;
        previous = current;
        current = next;
        if current.is_infinite() {
            break;
        }
    }
    Ok(current)
}

/// I_n(x), x >= 0.  A log-sum-exp power series is evaluated around its modal
/// term, which is the exponentially-scaled primitive's stable coordinate.
/// The final conversion alone decides overflow or underflow.
pub(in crate::calculation::functions) fn bessel_i(
    order: u32,
    x: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    validate_nonnegative_argument(x)?;
    if x == 0.0 {
        return Ok(if order == 0 { 1.0 } else { 0.0 });
    }
    Ok(exp_from_log(log_i(order, x, on_iteration)?))
}

/// exp(-x) I_n(x), retained as an internal primitive for callers that need
/// the exponential family without an intermediate overflowing I_n(x).
#[cfg(test)]
pub(in crate::calculation::functions) fn scaled_bessel_i(
    order: u32,
    x: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    validate_nonnegative_argument(x)?;
    if x == 0.0 {
        return Ok(if order == 0 { 1.0 } else { 0.0 });
    }
    Ok(exp_from_log(log_i(order, x, on_iteration)? - x))
}

/// K_n(x), x > 0.  K0/K1 are computed in scaled coordinates and higher order
/// values use their stable forward recurrence (DLMF 10.29.E1).
pub(in crate::calculation::functions) fn bessel_k(
    order: u32,
    x: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    Ok(exp_from_log(
        log_scaled_bessel_k(order, x, on_iteration)? - x,
    ))
}

/// exp(x) K_n(x), the stable coordinate used before the final K conversion.
#[cfg(test)]
pub(in crate::calculation::functions) fn scaled_bessel_k(
    order: u32,
    x: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    Ok(exp_from_log(log_scaled_bessel_k(order, x, on_iteration)?))
}

fn log_scaled_bessel_k(
    order: u32,
    x: f64,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    validate_positive_argument(x)?;
    let (previous, current) = scaled_k_seeds(x, &mut on_iteration)?;
    if previous <= 0.0 || current <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let mut log_previous = previous.ln();
    let mut log_current = current.ln();
    if order == 0 {
        return Ok(log_previous);
    }
    if order == 1 {
        return Ok(log_current);
    }
    for index in 1..order {
        on_iteration()?;
        let coefficient = 2.0 * f64::from(index) / x;
        let log_recurrence_term = coefficient.ln() + log_current;
        let log_next = log_add_exp(log_previous, log_recurrence_term);
        log_previous = log_current;
        log_current = log_next;
    }
    Ok(log_current)
}

fn log_add_exp(left: f64, right: f64) -> f64 {
    let high = left.max(right);
    let low = left.min(right);
    if high.is_infinite() {
        high
    } else {
        high + (low - high).exp().ln_1p()
    }
}

#[derive(Debug, Clone, Copy)]
enum JBranch {
    PowerSeries,
    ForwardRecurrence,
    MillerBackward,
    HankelAsymptotic,
}

fn select_j_branch(order: u32, x: f64) -> JBranch {
    if x >= HANKEL_ASYMPTOTIC_CUTOFF {
        JBranch::HankelAsymptotic
    } else if x <= OSCILLATORY_SERIES_CUTOFF {
        JBranch::PowerSeries
    } else if f64::from(order) <= x {
        JBranch::ForwardRecurrence
    } else {
        JBranch::MillerBackward
    }
}

fn j_series(
    order: u32,
    x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    if x == 0.0 {
        return Ok(if order == 0 { 1.0 } else { 0.0 });
    }
    let half_x = DoubleDouble::new(x * 0.5);
    let mut term = DoubleDouble::new(1.0);
    for factor in 1..=order {
        on_iteration()?;
        term = term.mul(half_x.div(DoubleDouble::new(f64::from(factor))));
    }
    if term.as_f64() == 0.0 {
        return Ok(0.0);
    }
    let mut sum = term;
    let quarter_x_squared = half_x.mul(half_x);
    for index in 0..MAX_SERIES_TERMS {
        on_iteration()?;
        let denominator = f64::from((index + 1) as u32) * (f64::from(order) + index as f64 + 1.0);
        term = term
            .mul(quarter_x_squared)
            .div(DoubleDouble::new(-denominator));
        let next = sum.add(term);
        if term.magnitude() <= SERIES_EPSILON * next.magnitude().max(f64::MIN_POSITIVE) {
            return Ok(next.as_f64());
        }
        sum = next;
    }
    Err(ErrorKind::Num)
}

fn j_forward(
    order: u32,
    x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    if order == 0 {
        return j_seed(0, x, on_iteration);
    }
    if order == 1 {
        return j_seed(1, x, on_iteration);
    }
    let mut previous = j_seed(0, x, on_iteration)?;
    let mut current = j_seed(1, x, on_iteration)?;
    for index in 1..order {
        on_iteration()?;
        let next = (2.0 * f64::from(index) / x) * current - previous;
        previous = current;
        current = next;
    }
    Ok(current)
}

fn j_miller(
    order: u32,
    x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    if x < 2.0_f64.powi(-29) {
        return j_series(order, x, on_iteration);
    }

    let order_f64 = f64::from(order);
    let w = 2.0 * order_f64 / x;
    let h = 2.0 / x;
    let mut q0 = w;
    let mut z = w + h;
    let mut q1 = w * z - 1.0;
    let mut continuation_terms = 1_u32;
    while q1 < 1.0e9 {
        on_iteration()?;
        continuation_terms = continuation_terms.checked_add(1).ok_or(ErrorKind::Num)?;
        if usize::try_from(continuation_terms).map_err(|_| ErrorKind::Num)? > MAX_SERIES_TERMS {
            return Err(ErrorKind::Num);
        }
        z += h;
        let next = z * q1 - q0;
        q0 = q1;
        q1 = next;
        if !q1.is_finite() {
            break;
        }
    }

    let mut ratio = 0.0;
    let upper = 2_u32
        .checked_mul(
            order
                .checked_add(continuation_terms)
                .ok_or(ErrorKind::Num)?,
        )
        .ok_or(ErrorKind::Num)?;
    let lower = 2 * order;
    let mut numerator = upper;
    loop {
        on_iteration()?;
        ratio = 1.0 / (f64::from(numerator) / x - ratio);
        if numerator == lower {
            break;
        }
        numerator -= 2;
    }

    let mut a = ratio;
    let mut b = 1.0;
    let mut coefficient = 2.0 * f64::from(order - 1);
    for _ in (1..order).rev() {
        on_iteration()?;
        let previous_b = b;
        b = b * coefficient / x - a;
        a = previous_b;
        coefficient -= 2.0;
        if b.abs() > 1.0e100 {
            a /= b;
            ratio /= b;
            b = 1.0;
        }
    }
    let j0 = j_seed(0, x, on_iteration)?;
    let j1 = j_seed(1, x, on_iteration)?;
    if j0.abs() >= j1.abs() {
        Ok(ratio * j0 / b)
    } else {
        Ok(ratio * j1 / a)
    }
}

fn j_seed(
    order: u32,
    x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    if x <= OSCILLATORY_SERIES_CUTOFF {
        return j_series(order, x, on_iteration);
    }
    Ok(hankel_seed(order, x, on_iteration)?.0)
}

fn y_seeds(
    x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<(f64, f64), ErrorKind> {
    if x > OSCILLATORY_SERIES_CUTOFF {
        return Ok((
            hankel_seed(0, x, on_iteration)?.1,
            hankel_seed(1, x, on_iteration)?.1,
        ));
    }

    let j0 = j_series(0, x, on_iteration)?;
    let j1 = j_series(1, x, on_iteration)?;
    let half_x = DoubleDouble::new(x * 0.5);
    let quarter_x_squared = half_x.mul(half_x);
    let mut term = DoubleDouble::new(1.0);
    let mut harmonic = DoubleDouble::new(0.0);
    let mut harmonic_sum = DoubleDouble::new(0.0);
    let mut weighted_harmonic_sum = DoubleDouble::new(0.0);
    for index in 1..=MAX_SERIES_TERMS {
        on_iteration()?;
        let index_f64 = index as f64;
        term = term
            .mul(quarter_x_squared)
            .div(DoubleDouble::new(-(index_f64 * index_f64)));
        harmonic = harmonic.add(DoubleDouble::new(1.0).div(DoubleDouble::new(index_f64)));
        let harmonic_term = term.mul(harmonic.neg());
        harmonic_sum = harmonic_sum.add(harmonic_term);
        weighted_harmonic_sum =
            weighted_harmonic_sum.add(harmonic_term.mul(DoubleDouble::new(index_f64)));
        if harmonic_term.magnitude()
            <= SERIES_EPSILON * harmonic_sum.magnitude().max(f64::MIN_POSITIVE)
        {
            let logarithm = (x * 0.5).ln() + EULER_GAMMA;
            let scale = std::f64::consts::FRAC_2_PI;
            let y0 = scale * (logarithm * j0 + harmonic_sum.as_f64());
            let y1 = scale * (-j0 / x + logarithm * j1 - 2.0 * weighted_harmonic_sum.as_f64() / x);
            return Ok((y0, y1));
        }
    }
    Err(ErrorKind::Num)
}

// DLMF 10.17.E3/E4.  The first omitted term controls termination; once the
// asymptotic terms start growing, the previous partial sum is the least-term
// approximation and adding more would reduce accuracy.
fn hankel_seed(
    order: u32,
    x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<(f64, f64), ErrorKind> {
    let order_f64 = f64::from(order);
    let mu = 4.0 * order_f64 * order_f64;
    let reciprocal_x = 1.0 / x;
    let mut term = 1.0;
    let mut previous_magnitude = f64::INFINITY;
    let mut even_sum = 1.0;
    let mut odd_sum = 0.0;
    for index in 1..=MAX_SERIES_TERMS {
        on_iteration()?;
        let index_f64 = index as f64;
        let odd = 2.0 * index_f64 - 1.0;
        let next_term = term * (mu - odd * odd) * reciprocal_x / (8.0 * index_f64);
        let magnitude = next_term.abs();
        if index > 1 && magnitude > previous_magnitude {
            break;
        }
        term = next_term;
        if index % 2 == 0 {
            let sign = if (index / 2) % 2 == 0 { 1.0 } else { -1.0 };
            even_sum += sign * term;
        } else {
            let sign = if ((index - 1) / 2) % 2 == 0 {
                1.0
            } else {
                -1.0
            };
            odd_sum += sign * term;
        }
        previous_magnitude = magnitude;
        if magnitude <= SERIES_EPSILON * even_sum.abs().max(odd_sum.abs()).max(1.0) {
            break;
        }
    }
    // Calling sin_cos on `x - order * pi / 2 - pi / 4` loses the order-dependent
    // shift once x is large enough that its ulp exceeds pi.  Reduce x first, then
    // apply the exact integer-order quadrant rotation.
    let (sine_x, cosine_x) = x.sin_cos();
    let inverse_sqrt_two = std::f64::consts::FRAC_1_SQRT_2;
    let (sine_shift, cosine_shift) = match order % 4 {
        0 => (inverse_sqrt_two, inverse_sqrt_two),
        1 => (inverse_sqrt_two, -inverse_sqrt_two),
        2 => (-inverse_sqrt_two, -inverse_sqrt_two),
        3 => (-inverse_sqrt_two, inverse_sqrt_two),
        _ => unreachable!("order modulo four is always in range"),
    };
    let cosine = cosine_x * cosine_shift + sine_x * sine_shift;
    let sine = sine_x * cosine_shift - cosine_x * sine_shift;
    let scale = std::f64::consts::FRAC_2_PI.sqrt() / x.sqrt();
    Ok((
        scale * (cosine * even_sum - sine * odd_sum),
        scale * (sine * even_sum + cosine * odd_sum),
    ))
}

fn log_i(
    order: u32,
    x: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let mut on_iteration = on_iteration;
    let order_f64 = f64::from(order);
    let mode = ((libm::hypot(order_f64, x) - order_f64) * 0.5).floor();
    if !mode.is_finite() || mode < 0.0 || mode > MAX_SERIES_TERMS as f64 {
        return Err(ErrorKind::Num);
    }
    let mode = mode as usize;
    let log_half_x = (x * 0.5).ln();
    let log_peak = log_i_term(order, mode, log_half_x)?;
    let mut sum = 0.0;
    let mut compensation = 0.0;
    for index in 0..=MAX_SERIES_TERMS {
        on_iteration()?;
        let normalized = exp_from_log(log_i_term(order, index, log_half_x)? - log_peak);
        let corrected = normalized - compensation;
        let next = sum + corrected;
        compensation = (next - sum) - corrected;
        sum = next;
        if index > mode && normalized <= SERIES_EPSILON * sum {
            return Ok(log_peak + sum.ln());
        }
    }
    Err(ErrorKind::Num)
}

fn log_i_term(order: u32, index: usize, log_half_x: f64) -> Result<f64, ErrorKind> {
    let index_f64 = index as f64;
    let order_f64 = f64::from(order);
    Ok((order_f64 + 2.0 * index_f64) * log_half_x
        - ln_gamma(index_f64 + 1.0)?
        - ln_gamma(order_f64 + index_f64 + 1.0)?)
}

fn scaled_k_seeds(
    x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<(f64, f64), ErrorKind> {
    if x < SMALL_K_SEED_CUTOFF {
        let (k0, k1) = small_k_seeds(x, on_iteration)?;
        let scale = x.exp();
        return Ok((k0 * scale, k1 * scale));
    }
    quadrature_k_seeds(x, on_iteration)
}

// K0's integer-order series and its derivative (K1) follow DLMF 10.31.E2.
// This branch avoids quadrature's narrow near-zero peak while preserving the
// logarithmic singularity exactly enough for subnormal x.
fn small_k_seeds(
    x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<(f64, f64), ErrorKind> {
    let y = x * x * 0.25;
    let mut term = 1.0;
    let mut harmonic = 0.0;
    let mut harmonic_sum = 0.0;
    let mut weighted_harmonic_sum = 0.0;
    for index in 1..=MAX_SERIES_TERMS {
        on_iteration()?;
        let index_f64 = index as f64;
        term *= y / (index_f64 * index_f64);
        harmonic += 1.0 / index_f64;
        let harmonic_term = harmonic * term;
        harmonic_sum += harmonic_term;
        weighted_harmonic_sum += index_f64 * harmonic_term;
        if harmonic_term.abs() <= SERIES_EPSILON * harmonic_sum.abs().max(f64::MIN_POSITIVE) {
            let i0 = exp_from_log(log_i(0, x, &mut *on_iteration)?);
            let i1 = exp_from_log(log_i(1, x, &mut *on_iteration)?);
            let logarithm = (x * 0.5).ln() + EULER_GAMMA;
            let k0 = -logarithm * i0 + harmonic_sum;
            let k1 = i0 / x + logarithm * i1 - 2.0 * weighted_harmonic_sum / x;
            return Ok((k0, k1));
        }
    }
    Err(ErrorKind::Num)
}

// From K_n(x) = integral_0^infinity exp(-x cosh(t)) cosh(n t) dt,
// substitute v = sqrt(2x) sinh(t/2). The 64-node Gauss-Legendre rule below
// integrates the resulting exp(-v^2) tail on [0, 12] in scaled coordinates.
fn quadrature_k_seeds(
    x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<(f64, f64), ErrorKind> {
    let mut k0_sum = 0.0;
    let mut k1_sum = 0.0;
    for &(node, weight) in &GAUSS_LEGENDRE_64_POSITIVE {
        for v in [6.0 * (1.0 - node), 6.0 * (1.0 + node)] {
            on_iteration()?;
            let v_squared = v * v;
            let factor = 6.0 * weight * (-v_squared).exp() / (1.0 + v_squared / (2.0 * x)).sqrt();
            k0_sum += factor;
            k1_sum += factor * (1.0 + v_squared / x);
        }
    }
    let scale = (2.0 / x).sqrt();
    Ok((scale * k0_sum, scale * k1_sum))
}

fn exp_from_log(logarithm: f64) -> f64 {
    if logarithm > LN_MAX_FINITE {
        f64::INFINITY
    } else if logarithm < LN_MIN_SUBNORMAL {
        0.0
    } else {
        logarithm.exp()
    }
}

fn validate_nonnegative_argument(x: f64) -> Result<(), ErrorKind> {
    if x.is_finite() && x >= 0.0 {
        Ok(())
    } else {
        Err(ErrorKind::Num)
    }
}

fn validate_positive_argument(x: f64) -> Result<(), ErrorKind> {
    if x.is_finite() && x > 0.0 {
        Ok(())
    } else {
        Err(ErrorKind::Num)
    }
}

const GAUSS_LEGENDRE_64_POSITIVE: [(f64, f64); 32] = [
    (2.435_029_266_342_443_3e-2, 4.869_095_700_913_972_4e-2),
    (7.299_312_178_779_904e-2, 4.857_546_744_150_343e-2),
    (1.214_628_192_961_205_6e-1, 4.834_476_223_480_295_4e-2),
    (1.696_444_204_239_928_3e-1, 4.799_938_859_645_831e-2),
    (2.174_236_437_400_070_8e-1, 4.754_016_571_483_031e-2),
    (2.646_871_622_087_674e-1, 4.696_818_281_621_002e-2),
    (3.113_228_719_902_109_7e-1, 4.628_479_658_131_441_6e-2),
    (3.572_201_583_376_681e-1, 4.549_162_792_741_814e-2),
    (4.022_701_579_639_916e-1, 4.459_055_816_375_656_6e-2),
    (4.463_660_172_534_641e-1, 4.358_372_452_932_345e-2),
    (4.894_031_457_070_529_6e-1, 4.247_351_512_365_359e-2),
    (5.312_794_640_198_946e-1, 4.126_256_324_262_353e-2),
    (5.718_956_462_026_34e-1, 3.995_374_113_272_034e-2),
    (6.111_553_551_723_933e-1, 3.855_015_317_861_562_6e-2),
    (6.489_654_712_546_573e-1, 3.705_512_854_024_005e-2),
    (6.852_363_130_542_333e-1, 3.547_221_325_688_238_6e-2),
    (7.198_818_501_716_109e-1, 3.380_516_183_714_160_6e-2),
    (7.528_199_072_605_319e-1, 3.205_792_835_485_155e-2),
    (7.839_723_589_433_414e-1, 3.023_465_707_240_247_8e-2),
    (8.132_653_151_227_975e-1, 2.833_967_261_425_948_3e-2),
    (8.406_292_962_525_803e-1, 2.637_746_971_505_466e-2),
    (8.659_993_981_540_928e-1, 2.435_270_256_871_087_4e-2),
    (8.893_154_459_951_141e-1, 2.227_017_380_838_325_3e-2),
    (9.105_221_370_785_028e-1, 2.013_482_315_353_021e-2),
    (9.295_691_721_319_396e-1, 1.795_171_577_569_734_3e-2),
    (9.464_113_748_584_028e-1, 1.572_603_047_602_471_8e-2),
    (9.610_087_996_520_538e-1, 1.346_304_789_671_864_3e-2),
    (9.733_268_277_899_11e-1, 1.116_813_946_013_112_8e-2),
    (9.833_362_538_846_26e-1, 8.846_759_826_363_947e-3),
    (9.910_133_714_767_443e-1, 6.504_457_968_978_363e-3),
    (9.963_401_167_719_553e-1, 4.147_033_260_562_468e-3),
    (9.993_050_417_357_722e-1, 1.783_280_721_696_433e-3),
];

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{
        ErrorKind, bessel_i, bessel_j, bessel_k, bessel_y, normalized_order, scaled_bessel_i,
        scaled_bessel_k,
    };

    fn never() -> Result<(), ErrorKind> {
        Ok(())
    }

    fn assert_close(actual: f64, expected: f64, absolute: f64, relative: f64) {
        let tolerance = absolute + relative * expected.abs();
        assert!(
            (actual - expected).abs() <= tolerance,
            "expected {expected:.17e}, got {actual:.17e}, tolerance {tolerance:.17e}"
        );
    }

    #[test]
    fn pure_kernels_cover_near_origin_transition_roots_and_large_arguments() {
        assert_close(
            bessel_j(8, 1e-6, never).unwrap(),
            9.68812003968227e-56,
            5e-14,
            2e-11,
        );
        assert_close(
            bessel_y(8, 1e-6, never).unwrap(),
            -4.106961475497887e53,
            5e-14,
            2e-11,
        );
        assert_close(
            bessel_i(25, 5.0, never).unwrap(),
            7.274325905901249e-16,
            5e-15,
            5e-13,
        );
        assert_close(
            bessel_k(25, 5.0, never).unwrap(),
            2.6959282453367324e13,
            5e-15,
            5e-13,
        );
        assert_close(
            bessel_j(50, 50.0, never).unwrap(),
            0.12140902189761506,
            5e-14,
            2e-11,
        );
        assert_close(
            bessel_y(50, 50.0, never).unwrap(),
            -0.21031655464397742,
            5e-14,
            2e-11,
        );
        assert_close(
            bessel_i(50, 500.0, never).unwrap(),
            2.0552180163054087e214,
            5e-14,
            2e-11,
        );
        assert_close(
            bessel_k(50, 500.0, never).unwrap(),
            4.841518738593636e-218,
            5e-14,
            2e-11,
        );
        // mpmath 1.3.0 at 120 dps. exp(x) K_n(x) overflows here even though
        // the worksheet-visible K_n(x) remains finite.
        assert_close(
            bessel_k(500, 100.0, never).unwrap(),
            2.731_383_171_990_178_5e279,
            0.0,
            2e-11,
        );
        assert_close(
            bessel_j(0, 2.404_825_557_695_773, never).unwrap(),
            0.0,
            2e-13,
            0.0,
        );
        assert_close(
            bessel_y(5, 20.602899017175335, never).unwrap(),
            0.0,
            2e-13,
            0.0,
        );
    }

    #[test]
    fn exponential_scaling_survives_final_overflow_and_underflow() {
        assert!(bessel_i(0, 1000.0, never).unwrap().is_infinite());
        assert_eq!(bessel_k(0, 1000.0, never).unwrap(), 0.0);
        assert_close(
            scaled_bessel_i(50, 500.0, never).unwrap(),
            1.464255778967914e-3,
            5e-14,
            2e-11,
        );
        assert_close(
            scaled_bessel_k(50, 500.0, never).unwrap(),
            6.795518024078713e-1,
            5e-14,
            2e-11,
        );
    }

    #[test]
    fn large_argument_phase_reduction_preserves_the_integer_order_shift() {
        // mpmath 1.4.1 at 100 dps.  The old direct phase subtraction rounds the
        // n=1 shift away at this magnitude, producing errors around 1e-8.
        assert_close(
            bessel_j(1, 1e16, never).unwrap(),
            7.931694266803264e-9,
            5e-14,
            2e-11,
        );
        assert_close(
            bessel_y(1, 1e16, never).unwrap(),
            -8.661427680921673e-10,
            5e-14,
            2e-11,
        );
        // Computing PI*x before the division used to overflow the finite
        // Hankel amplitude to zero.
        assert_close(
            bessel_y(0, 9e307, never).unwrap(),
            4.066_895_414_404_214e-155,
            0.0,
            2e-11,
        );
    }

    #[test]
    fn order_rejects_negative_inputs_before_truncating_toward_zero() {
        assert_eq!(normalized_order(2.9), Ok(2));
        assert_eq!(normalized_order(-0.9), Err(ErrorKind::Num));
        assert_eq!(normalized_order(-1.0), Err(ErrorKind::Num));
    }

    #[test]
    fn every_iterative_path_observes_the_resource_callback() {
        fn fail_on_third(calls: &Cell<u32>) -> impl FnMut() -> Result<(), ErrorKind> + '_ {
            move || {
                let next = calls.get() + 1;
                calls.set(next);
                if next == 3 {
                    Err(ErrorKind::Num)
                } else {
                    Ok(())
                }
            }
        }
        fn assert_stops(call: impl FnOnce(&Cell<u32>) -> Result<f64, ErrorKind>) {
            let calls = Cell::new(0);
            assert_eq!(call(&calls), Err(ErrorKind::Num));
            assert_eq!(calls.get(), 3);
        }
        assert_stops(|calls| bessel_i(2, 2.0, fail_on_third(calls)));
        assert_stops(|calls| bessel_j(100, 10.0, fail_on_third(calls)));
        assert_stops(|calls| bessel_k(2, 2.0, fail_on_third(calls)));
        assert_stops(|calls| bessel_y(2, 2.0, fail_on_third(calls)));
    }
}
