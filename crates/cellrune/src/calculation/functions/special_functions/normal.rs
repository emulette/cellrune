//! Shared standard-normal kernel (plan §6.5).
//!
//! NORM.S.DIST, NORM.DIST, GAUSS, and Z.TEST all evaluate through these three
//! functions so the density/lower/upper pair cannot drift apart. The
//! expressions preserve the pre-0.1.13 evaluation order bit for bit
//! (statistical.rs `standard_normal_distribution` and
//! statistical_additional.rs private helpers).

/// Standard-normal density φ(z) = exp(−z²/2)/√(2π).
pub(in crate::calculation::functions) fn standard_normal_density(value: f64) -> f64 {
    (-0.5 * value * value).exp() / (2.0 * std::f64::consts::PI).sqrt()
}

/// Standard-normal lower tail Φ(z) = ½·erfc(−z/√2).
pub(in crate::calculation::functions) fn standard_normal_lower(value: f64) -> f64 {
    0.5 * libm::erfc(-value / std::f64::consts::SQRT_2)
}

/// Standard-normal upper tail 1 − Φ(z) = ½·erfc(z/√2).
pub(in crate::calculation::functions) fn standard_normal_upper(value: f64) -> f64 {
    0.5 * libm::erfc(value / std::f64::consts::SQRT_2)
}

use crate::calculation::value::ErrorKind;

/// Inverts Φ on the complete finite floating-point probability domain (0, 1).
///
/// Safeguarded Newton steps solve the centered erf near the median and the
/// logarithm of the smaller tail elsewhere. Thus neither subtraction from a
/// rounded CDF nor underflow of a subnormal tail controls the result. The
/// bracket [0, 40] encloses every magnitude attainable from an f64 probability
/// (the smallest positive probability has a quantile magnitude below 38.468).
/// Each solve step and continued-fraction term charges the caller's work
/// callback; errors pass through unchanged. At most 80 * 33 charges are made.
pub(in crate::calculation::functions) fn standard_normal_inverse(
    probability: f64,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    if !probability.is_finite() || probability <= 0.0 || probability >= 1.0 {
        return Err(ErrorKind::Num);
    }
    let distance = (probability - 0.5).abs();
    let tail = probability.min(1.0 - probability);
    let centered = tail >= 0.25;
    let log_tail = tail.ln();
    let mut low = 0.0;
    let mut high = 40.0;
    let mut value = if centered {
        distance * (2.0 * std::f64::consts::PI).sqrt()
    } else {
        (-2.0 * log_tail).sqrt()
    };
    for _ in 0..80 {
        on_iteration()?;
        let (residual, derivative) = if centered {
            (
                0.5 * libm::erf(value / std::f64::consts::SQRT_2) - distance,
                standard_normal_density(value),
            )
        } else {
            let (log_upper, hazard) = normal_log_upper(value, &mut on_iteration)?;
            (log_tail - log_upper, hazard)
        };
        let correction = residual / derivative;
        if correction.abs() <= 4.0 * f64::EPSILON * value {
            return Ok(if probability < 0.5 { -value } else { value });
        }
        if residual < 0.0 {
            low = value;
        } else {
            high = value;
        }
        let candidate = value - correction;
        value = if candidate > low && candidate < high {
            candidate
        } else {
            low + 0.5 * (high - low)
        };
    }
    Err(ErrorKind::Num)
}

/// Returns log(Q(x)) and φ(x)/Q(x) for nonnegative x. Above 8 the
/// Laplace continued fraction Q(x)/φ(x) = 1/(x + 1/(x + 2/(x + ...)))
/// is evaluated backwards with 32 terms. This is DLMF 7.9.1 after substituting
/// z = x/√2: <https://dlmf.nist.gov/7.9.E1>. At x >= 8, adjacent 31/32-term
/// convergents differ by less than 2e-24 relatively in exact arithmetic;
/// positive continued-fraction convergents bound the infinite fraction.
/// No probability or density is exponentiated in this tail branch.
fn normal_log_upper(
    value: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<(f64, f64), ErrorKind> {
    if value < 8.0 {
        let upper = standard_normal_upper(value);
        return Ok((upper.ln(), standard_normal_density(value) / upper));
    }
    let mut fraction = 0.0;
    for numerator in (1..=32).rev() {
        on_iteration()?;
        fraction = f64::from(numerator) / (value + fraction);
    }
    let hazard = value + fraction;
    let log_upper = -0.5 * value * value - 0.5 * (2.0 * std::f64::consts::PI).ln() - hazard.ln();
    Ok((log_upper, hazard))
}

#[cfg(test)]
#[path = "normal_reference_tests.rs"]
mod tests;
