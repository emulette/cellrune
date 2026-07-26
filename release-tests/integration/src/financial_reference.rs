//! Independent reference calculations for iterative financial functions.
//!
//! CellRune's production solvers use Newton–Raphson. This module deliberately uses bisection:
//! it needs only a sign-changing bracket and never consults a derivative. Agreement therefore
//! compares two different search algorithms instead of asking the production implementation to
//! certify itself.

/// Maximum bisection passes used by the test-only reference.
const BISECTION_ITERATIONS: u32 = 256;

/// Finds a root inside a sign-changing bracket using bisection.
///
/// Returns `None` for a non-finite bound, an unordered bracket, a non-finite residual, or a
/// bracket whose endpoints have the same sign.
pub fn bisect_root(
    mut lower: f64,
    mut upper: f64,
    tolerance: f64,
    residual: impl Fn(f64) -> f64,
) -> Option<f64> {
    if !lower.is_finite()
        || !upper.is_finite()
        || lower >= upper
        || !tolerance.is_finite()
        || tolerance <= 0.0
    {
        return None;
    }
    let mut lower_value = residual(lower);
    let upper_value = residual(upper);
    if !lower_value.is_finite() || !upper_value.is_finite() {
        return None;
    }
    if lower_value == 0.0 {
        return Some(lower);
    }
    if upper_value == 0.0 {
        return Some(upper);
    }
    if lower_value.is_sign_positive() == upper_value.is_sign_positive() {
        return None;
    }

    for _ in 0..BISECTION_ITERATIONS {
        let midpoint = lower + (upper - lower) / 2.0;
        let midpoint_value = residual(midpoint);
        if !midpoint_value.is_finite() {
            return None;
        }
        if midpoint_value == 0.0 || (upper - lower).abs() <= tolerance {
            return Some(midpoint);
        }
        if midpoint_value.is_sign_positive() == lower_value.is_sign_positive() {
            lower = midpoint;
            lower_value = midpoint_value;
        } else {
            upper = midpoint;
        }
    }
    Some(lower + (upper - lower) / 2.0)
}

/// Returns the periodic discounted-cash-flow residual solved by `IRR`.
pub fn irr_residual(cashflows: &[f64], rate: f64) -> f64 {
    cashflows
        .iter()
        .enumerate()
        .map(|(period, cashflow)| {
            cashflow / (1.0 + rate).powi(i32::try_from(period).expect("test period fits i32"))
        })
        .sum()
}

/// Returns the dated discounted-cash-flow residual solved by `XIRR`.
pub fn xirr_residual(cashflows: &[f64], dates: &[f64], rate: f64) -> f64 {
    assert_eq!(
        cashflows.len(),
        dates.len(),
        "reference cashflows and dates must pair"
    );
    let start = dates[0];
    cashflows
        .iter()
        .zip(dates)
        .map(|(cashflow, date)| cashflow / (1.0 + rate).powf((date - start) / 365.0))
        .sum()
}

/// Returns the annuity-balance residual solved by `RATE`.
pub fn rate_residual(
    periods: f64,
    payment: f64,
    present: f64,
    future: f64,
    payment_type: f64,
    rate: f64,
) -> f64 {
    let power = (1.0 + rate).powf(periods);
    let factor = if rate == 0.0 {
        periods
    } else {
        (power - 1.0) / rate
    };
    present * power + payment * (1.0 + rate * payment_type) * factor + future
}

#[cfg(test)]
mod tests {
    use super::{bisect_root, irr_residual, rate_residual, xirr_residual};

    #[test]
    fn bisection_finds_each_supported_financial_root_without_a_derivative() {
        let irr = bisect_root(-0.9, 1.0, 1e-12, |rate| {
            irr_residual(&[-100.0, 60.0, 60.0], rate)
        })
        .expect("IRR bracket");
        assert!(irr_residual(&[-100.0, 60.0, 60.0], irr).abs() < 1e-9);

        let xirr = bisect_root(-0.9, 1.0, 1e-12, |rate| {
            xirr_residual(&[-100.0, 120.0], &[45_000.0, 45_365.0], rate)
        })
        .expect("XIRR bracket");
        assert!(xirr_residual(&[-100.0, 120.0], &[45_000.0, 45_365.0], xirr).abs() < 1e-9);

        let rate = bisect_root(-0.9, 1.0, 1e-12, |value| {
            rate_residual(10.0, -100.0, 800.0, 0.0, 0.0, value)
        })
        .expect("RATE bracket");
        assert!(rate_residual(10.0, -100.0, 800.0, 0.0, 0.0, rate).abs() < 1e-7);
    }

    #[test]
    fn bisection_rejects_a_non_bracket() {
        assert_eq!(bisect_root(1.0, 2.0, 1e-12, |value| value + 1.0), None);
    }
}
