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
