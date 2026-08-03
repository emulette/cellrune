use std::f64::consts::PI;

use super::super::super::value::ErrorKind;

// Lanczos approximation with g = 607/128 and the 14-term Godfrey coefficient
// set (Numerical Recipes 3rd ed., `gammln`). Contract: finite x > 0 only;
// relative error stays below 1e-13 across the representable domain, degrading
// to an absolute error of a few ULP near the zeros at x = 1 and x = 2.
const LANCZOS_G_PLUS_HALF: f64 = 5.242_187_5; // g + 1/2 with g = 607/128
const LANCZOS_SERIES_BASE: f64 = 0.999_999_999_999_997_1;
const SQRT_TWO_PI: f64 = 2.506_628_274_631_000_7;
const LANCZOS_COEFFICIENTS: [f64; 14] = [
    57.156_235_665_862_92,
    -59.597_960_355_475_49,
    14.136_097_974_741_746,
    -0.491_913_816_097_620_2,
    3.399_464_998_481_189e-5,
    4.652_362_892_704_858e-5,
    -9.837_447_530_487_956e-5,
    1.580_887_032_249_125e-4,
    -2.102_644_417_241_048_8e-4,
    2.174_396_181_152_126_5e-4,
    -1.643_181_065_367_639e-4,
    8.441_822_398_385_275e-5,
    -2.619_083_840_158_140_8e-5,
    3.689_918_265_953_162_5e-6,
];

/// Natural log of the gamma function for finite x > 0.
pub(in crate::calculation::functions) fn ln_gamma(x: f64) -> Result<f64, ErrorKind> {
    if !x.is_finite() || x <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let mut series = LANCZOS_SERIES_BASE;
    let mut denominator = x;
    for coefficient in LANCZOS_COEFFICIENTS {
        denominator += 1.0;
        series += coefficient / denominator;
    }
    let shifted = x + LANCZOS_G_PLUS_HALF;
    Ok((x + 0.5) * shifted.ln() - shifted + (SQRT_TWO_PI * series / x).ln())
}

/// Gamma function with sign, for finite non-pole arguments.
///
/// Negative non-integers go through the reflection Γ(x) = π / (sin πx · Γ(1−x))
/// evaluated in log space with the sign recovered from sin πx. Poles (zero and
/// negative integers) and results beyond the f64 range are domain errors, so
/// the caller never sees NaN or infinity.
pub(in crate::calculation::functions) fn signed_gamma(x: f64) -> Result<f64, ErrorKind> {
    if !x.is_finite() {
        return Err(ErrorKind::Num);
    }
    if x > 0.0 {
        let value = ln_gamma(x)?.exp();
        return if value.is_finite() {
            Ok(value)
        } else {
            Err(ErrorKind::Num)
        };
    }
    if x == x.trunc() {
        return Err(ErrorKind::Num);
    }
    let sine = sin_pi(x);
    let magnitude = (PI.ln() - sine.abs().ln() - ln_gamma(1.0 - x)?).exp();
    if !magnitude.is_finite() {
        return Err(ErrorKind::Num);
    }
    Ok(if sine < 0.0 { -magnitude } else { magnitude })
}

/// sin(πx) via range reduction to |r| ≤ 1/2, exact where πx would round.
/// Every non-integer double has |x| < 2^52, so the parity cast cannot saturate.
fn sin_pi(x: f64) -> f64 {
    let nearest = x.round();
    let sine = (PI * (x - nearest)).sin();
    if (nearest as i64) % 2 == 0 {
        sine
    } else {
        -sine
    }
}

#[cfg(test)]
mod tests {
    use super::{ln_gamma, signed_gamma};
    use crate::calculation::value::ErrorKind;

    // reference: mpmath 1.4.1, mp.dps = 30
    const LN_GAMMA_REFERENCES: [(f64, f64); 17] = [
        (0.1, 2.252712651734206),
        (0.25, 1.2880225246980774),
        (0.5, 0.5723649429247001),
        (0.75, 0.20328095143129538),
        (0.99, 0.005854806764709776),
        (1.0, 0.0),
        (1.4616321449683623, -0.12148629053584961),
        (1.5, -0.12078223763524522),
        (2.0, 0.0),
        (2.5, 0.2846828704729192),
        (3.0, std::f64::consts::LN_2),
        (4.5, 2.4537365708424423),
        (10.0, 12.801827480081469),
        (25.0, 54.78472939811232),
        (100.0, 359.1342053695754),
        (170.0, 701.437263808737),
        (171.5, 709.1431630309282),
    ];

    // reference: mpmath 1.4.1, mp.dps = 30
    const SIGNED_GAMMA_REFERENCES: [(f64, f64); 8] = [
        (2.5, 1.329340388179137),
        (0.5, 1.772453850905516),
        (-0.5, -3.544907701811032),
        (-2.5, -0.9453087204829419),
        (-3.75, 0.2678661288614166),
        (5.0, 24.0),
        (170.0, 4.269068009004705e304),
        (-20.2, -1.1995681949848136e-18),
    ];

    #[test]
    fn ln_gamma_matches_mpmath_within_1e13_relative() {
        for (x, expected) in LN_GAMMA_REFERENCES {
            let actual = ln_gamma(x).expect("positive domain");
            // Near the zeros at x = 1 and 2 the bound is absolute, not relative.
            let scale = expected.abs().max(1.0);
            assert!(
                (actual - expected).abs() <= 1e-13 * scale,
                "ln_gamma({x}): {actual} vs {expected}",
            );
        }
    }

    #[test]
    fn ln_gamma_rejects_non_positive_and_non_finite_arguments() {
        for x in [0.0, -0.5, -3.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(ln_gamma(x), Err(ErrorKind::Num), "{x}");
        }
    }

    #[test]
    fn signed_gamma_matches_mpmath_on_both_signs() {
        for (x, expected) in SIGNED_GAMMA_REFERENCES {
            let actual = signed_gamma(x).expect("non-pole domain");
            assert!(
                (actual - expected).abs() <= 1e-13 * expected.abs(),
                "gamma({x}): {actual} vs {expected}",
            );
        }
    }

    #[test]
    fn signed_gamma_rejects_poles_overflow_and_non_finite_arguments() {
        for x in [
            0.0,
            -0.0,
            -1.0,
            -2.0,
            -100.0,
            171.7,
            172.0,
            1.0e3,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            assert_eq!(signed_gamma(x), Err(ErrorKind::Num), "{x}");
        }
    }
}
