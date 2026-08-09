use super::super::super::value::ErrorKind;
use super::bounded_probability;
use super::double_double::DoubleDouble;
use super::inverse::{DomainPolicy, invert_monotone_cdf};
use super::log_gamma::ln_gamma;
use super::{LENTZ_TINY, MAX_REFINEMENT_ITERATIONS};

/// Shape threshold above which `ln_beta`, the direct-tail prefactor, and the
/// density exponent switch to the cancellation-resistant large-shape
/// piecewise forms (FORMULAS.md §2.1, DESIGN.md). Below it everything keeps
/// the established Lanczos path bit for bit.
const LARGE_SHAPE: f64 = 2048.0;

/// Shape threshold at and above which the uniform-asymptotic central path
/// applies: min(a, b) >= 1_000_000 and |x − p| ≤ 12·σ (DESIGN.md).
const CENTRAL_MIN_SHAPE: f64 = 1_000_000.0;
const CENTRAL_SIGMA_MULTIPLE: f64 = 12.0;

/// g-series truncation order (DESIGN.md): with |δ| ≤ 12σ and min(a,b) ≥ 1e6,
/// |t| = |δ|/p ≤ 12/√a ≤ 0.012, so the k = 24 remainder is below 2⁻⁶⁰
/// relative.
const CENTRAL_SERIES_TERMS: u32 = 24;

/// Termination threshold of the double-double Lentz recurrence:
/// abs((δ − 1).hi) + abs((δ − 1).lo) < 2⁻¹⁰⁰ (FORMULAS.md §2.2).
const DOUBLE_DOUBLE_TOLERANCE: f64 = 7.888_609_052_210_118e-31;

/// Regularized incomplete beta I_x(a, b) for finite a > 0, b > 0, x ∈ [0, 1]:
/// the lower-tail entrypoint without log coordinates, kept for the 0.1.12
/// consumers (BETA.DIST, the binomial kernels, the solver).
pub(in crate::calculation::functions) fn regularized_incomplete_beta(
    a: f64,
    b: f64,
    x: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    regularized_incomplete_beta_lower(a, b, x, None, None, on_iteration)
}

/// Lower-tail entrypoint I_x(a, b). F/t consumers pass `log_x`/`log_w` (their
/// transform's exact log coordinates) whenever their coordinate underflowed
/// to 0; a rounded-zero coordinate is then an interior point, not an
/// endpoint, and the direct tail still yields the representable subnormal.
pub(in crate::calculation::functions) fn regularized_incomplete_beta_lower(
    a: f64,
    b: f64,
    x: f64,
    log_x: Option<f64>,
    log_w: Option<f64>,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    validate_domain(a, b, x)?;
    if x == 0.0 && log_x.is_none() {
        return Ok(0.0);
    }
    if x == 1.0 && log_w.is_none() {
        return Ok(1.0);
    }
    if a == 1.0 && b == 1.0 && log_x.is_none() {
        // The uniform beta CDF is exactly the identity. Avoiding the generic
        // lnGamma path matters when a unit-coordinate ULP would later be
        // magnified by a very wide custom support interval.
        return Ok(x);
    }
    if a == b && x == 0.5 {
        // Exact-0.5 fast path (DESIGN.md): the Lanczos reference bias floor
        // (≈ 2.9e-15) previously made I_0.5(a, a) drift below 0.5.
        return Ok(0.5);
    }
    if in_central_band(a, b, x) {
        return Ok(central_lower_upper(a, b, x, &mut on_iteration)?.0);
    }
    if x < (a + 1.0) / (a + b + 2.0) {
        // Direct evaluation at the rounded coordinate itself: the caller's
        // logs are exact only at a rounded-zero endpoint (where the
        // coordinate's own log is meaningless), so they are used there and
        // nowhere else. Anywhere else the coordinate's own logs describe the
        // same point the continued fraction sees.
        let endpoint_log_x = if x > 0.0 { None } else { log_x };
        direct_tail(a, b, x, endpoint_log_x, None, &mut on_iteration)
    } else {
        // Reflected evaluation at the fine complement when the caller
        // supplied its exact log: the t transform hands over ln of the
        // untruncated small coordinate, and exponentiating it keeps the
        // reflected argument on the fine grid instead of the complement of a
        // rounded near-one coordinate (whose unit-interval ULP would
        // quantize every CDF value into ~f·2⁻⁵³ steps, too coarse for the
        // solver to hit a small target). The generic complement 1 − x is
        // used otherwise.
        let w = log_w.map_or(1.0 - x, f64::exp);
        Ok(1.0 - direct_tail(b, a, w, log_w, log_x, &mut on_iteration)?)
    }
}

/// Upper-tail entrypoint 1 − I_x(a, b), symmetric to the lower entrypoint.
pub(in crate::calculation::functions) fn regularized_incomplete_beta_upper(
    a: f64,
    b: f64,
    x: f64,
    log_x: Option<f64>,
    log_w: Option<f64>,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    validate_domain(a, b, x)?;
    if x == 0.0 && log_x.is_none() {
        return Ok(1.0);
    }
    if x == 1.0 && log_w.is_none() {
        return Ok(0.0);
    }
    if a == 1.0 && b == 1.0 && log_x.is_none() {
        return Ok(1.0 - x);
    }
    if a == b && x == 0.5 {
        return Ok(0.5);
    }
    if in_central_band(a, b, x) {
        return Ok(central_lower_upper(a, b, x, &mut on_iteration)?.1);
    }
    if x < (a + 1.0) / (a + b + 2.0) {
        Ok(1.0 - direct_tail(a, b, x, log_x, log_w, &mut on_iteration)?)
    } else {
        let w = log_w.map_or(1.0 - x, f64::exp);
        direct_tail(b, a, w, log_w, log_x, &mut on_iteration)
    }
}

/// Solves the lower-tail problem: the (z, w) pair with I_z(a, b) = p, for
/// 0 ≤ p ≤ 1. When p ≤ 0.5 the quantile is solved on the lower side; the
/// reflected problem I_w(b, a) = 1 − p is solved above, so the near-one
/// coordinate survives rounding (FORMULAS.md §2.3).
pub(in crate::calculation::functions) fn beta_pair(
    a: f64,
    b: f64,
    probability: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind> + Clone,
) -> Result<(f64, f64), ErrorKind> {
    if !a.is_finite() || a <= 0.0 || !b.is_finite() || b <= 0.0 || !probability.is_finite() {
        return Err(ErrorKind::Num);
    }
    if probability == 0.0 {
        return Ok((0.0, 1.0));
    }
    if probability == 1.0 {
        return Ok((1.0, 0.0));
    }
    if probability <= 0.5 {
        let mut iterations = on_iteration.clone();
        let z = invert_monotone_cdf(
            |position| regularized_incomplete_beta(a, b, position, &mut iterations),
            probability,
            DomainPolicy::FiniteInterval {
                low: 0.0,
                high: 1.0,
            },
            on_iteration,
        )?;
        Ok((z, 1.0 - z))
    } else {
        let mut iterations = on_iteration.clone();
        let w = invert_monotone_cdf(
            |position| regularized_incomplete_beta(b, a, position, &mut iterations),
            1.0 - probability,
            DomainPolicy::FiniteInterval {
                low: 0.0,
                high: 1.0,
            },
            on_iteration,
        )?;
        Ok((1.0 - w, w))
    }
}

/// ln of the beta density (a−1)·ln x + (b−1)·ln(1−x) − lnB(a, b), evaluated
/// cancellation-resistantly when any shape is large (DESIGN.md "PDF
/// exponent"); the returned value is the log density. `log_x`/`log_w` are
/// the caller's exact log coordinates (ln x, ln(1−x)).
pub(in crate::calculation::functions) fn beta_density_exponent(
    a: f64,
    b: f64,
    x: f64,
    log_x: f64,
    log_w: f64,
) -> Result<f64, ErrorKind> {
    validate_domain(a, b, x)?;
    let total = a + b;
    let p = a / total;
    let q = b / total;
    // p and q are divided separately here; a large shape never derives the
    // small ratio as 1 − p (DESIGN.md, PDF-exponent section). The mean
    // residual correction applies whenever any shape is large, so a rounded
    // p cannot be amplified by the large coefficients.
    let (ratio_x, ratio_w) = if a >= LARGE_SHAPE || b >= LARGE_SHAPE {
        let p_low = f64::mul_add(-p, total, a) / total;
        let delta = (x - p) - p_low;
        let scaled_p = delta / p;
        let scaled_q = delta / q;
        (
            if scaled_p.abs() < 0.5 {
                scaled_p.ln_1p()
            } else {
                log_x - p.ln()
            },
            if scaled_q.abs() < 0.5 {
                (-scaled_q).ln_1p()
            } else {
                log_w - q.ln()
            },
        )
    } else {
        (log_x, log_w)
    };
    let exponent = if a >= LARGE_SHAPE && b >= LARGE_SHAPE {
        (a - 1.0) * ratio_x + (b - 1.0) * ratio_w + 0.5 * (total / (p * q)).ln()
            - 0.5 * (2.0 * std::f64::consts::PI).ln()
            - stirling_correction(a)
            - stirling_correction(b)
            + stirling_correction(total)
    } else if a >= LARGE_SHAPE {
        (b - 1.0) * q.ln() - 0.5 * p.ln() + (a - 1.0) * ratio_x + (b - 1.0) * ratio_w - ln_gamma(b)?
            + b * total.ln()
            - b
            - stirling_correction(a)
            + stirling_correction(total)
    } else if b >= LARGE_SHAPE {
        (a - 1.0) * p.ln() - 0.5 * q.ln() + (b - 1.0) * ratio_w + (a - 1.0) * ratio_x - ln_gamma(a)?
            + a * total.ln()
            - a
            - stirling_correction(b)
            + stirling_correction(total)
    } else {
        (a - 1.0) * log_x + (b - 1.0) * log_w - ln_beta(a, b)?
    };
    if exponent.is_nan() {
        Err(ErrorKind::Num)
    } else {
        Ok(exponent)
    }
}

fn validate_domain(a: f64, b: f64, x: f64) -> Result<(), ErrorKind> {
    if !a.is_finite() || a <= 0.0 || !b.is_finite() || b <= 0.0 || !(0.0..=1.0).contains(&x) {
        return Err(ErrorKind::Num);
    }
    Ok(())
}

/// ln B(a, b) = lnΓ(a) + lnΓ(b) − lnΓ(a + b) for finite a > 0, b > 0; shared
/// with the density evaluators so they cannot drift from this kernel. The
/// two large-shape forms below are cancellation-resistant (FORMULAS.md
/// §2.1): the Lanczos lnΓ terms would otherwise lose the whole difference
/// when a, b ≳ 2048.
pub(in crate::calculation::functions) fn ln_beta(a: f64, b: f64) -> Result<f64, ErrorKind> {
    if !a.is_finite() || a <= 0.0 || !b.is_finite() || b <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let total = a + b;
    if a >= LARGE_SHAPE && b >= LARGE_SHAPE {
        let p = a / total;
        let q = b / total;
        return Ok(
            total * (p * p.ln() + q * q.ln()) - 0.5 * (p * q * total).ln()
                + 0.5 * (2.0 * std::f64::consts::PI).ln()
                + stirling_correction(a)
                + stirling_correction(b)
                - stirling_correction(total),
        );
    }
    let large = a.max(b);
    let small = a.min(b);
    if large >= LARGE_SHAPE {
        return Ok(
            ln_gamma(small)? + (large - 0.5) * (-(small / total)).ln_1p() - small * total.ln()
                + small
                + stirling_correction(large)
                - stirling_correction(total),
        );
    }
    Ok(ln_gamma(a)? + ln_gamma(b)? - ln_gamma(total)?)
}

/// One-term Stirling correction corr(z) = 1/(12z) − 1/(360z³) + 1/(1260z⁵).
fn stirling_correction(value: f64) -> f64 {
    let inverse = 1.0 / value;
    let inverse2 = inverse * inverse;
    inverse / 12.0 - inverse * inverse2 / 360.0 + inverse * inverse2 * inverse2 / 1260.0
}

/// True when x lies in the central band |x − p| ≤ 12·σ around the mean
/// p = a/(a+b) with min(a, b) ≥ 1_000_000 (DESIGN.md). σ uses the f64
/// variance; the grid rows that straddle the boundary are placed at
/// 12σ·(1 ± 1e-10), far outside any rounding ambiguity.
fn in_central_band(a: f64, b: f64, x: f64) -> bool {
    if a.min(b) < CENTRAL_MIN_SHAPE {
        return false;
    }
    let total = a + b;
    let p = a / total;
    let variance = a * b / (total * total * (total + 1.0));
    if !variance.is_finite() || variance <= 0.0 {
        return false;
    }
    (x - p).abs() <= CENTRAL_SIGMA_MULTIPLE * variance.sqrt()
}

/// Uniform-asymptotic evaluation of I_x(a, b) and its upper complement in the
/// central band (DESIGN.md). Returns the (lower, upper) pair computed from
/// the same leading erfc term and correction, so the pair is consistent by
/// construction.
fn central_lower_upper(
    a: f64,
    b: f64,
    x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<(f64, f64), ErrorKind> {
    let total = a + b;
    let p = a / total;
    // p = a/total rounds to f64 up to 0.5 ulp away from the exact mean; the
    // displacement x − p would carry that error and du/dδ = √(N/(2pq))
    // amplifies it into the leading erfc term. Recover the exact mean with a
    // double-double residual: fma(−p, total, a) is exact, so
    // (x − p) − p_low is the exact displacement (DESIGN.md evaluation
    // appendix).
    let p_low = f64::mul_add(-p, total, a) / total;
    let delta = (x - p) - p_low;
    let t = 1.0 / total;
    let g = central_g(p, delta, on_iteration)?;
    // Round-off can push g a few ULP above 0 when δ is tiny; u = 0 there is
    // the exact δ → 0 limit.
    let u = ((-total * g).max(0.0)).sqrt().copysign(delta);
    let p_term = (-u * u).exp() / (2.0 * std::f64::consts::PI * total).sqrt();
    let coefficients = central_coefficients(p);
    // Series evaluated in the reference order: coefficient · δ^k / k!
    // (final_validation.py c_series). The coefficient forms themselves are
    // transcribed verbatim from smalldelta_forms.json.
    let c0 = coefficients.c0
        + coefficients.c0_prime * delta
        + coefficients.c0_double_prime * delta.powi(2) / 2.0
        + coefficients.c0_triple_prime * delta.powi(3) / 6.0
        + coefficients.c0_quadruple_prime * delta.powi(4) / 24.0;
    let c1 = coefficients.c1
        + coefficients.c1_prime * delta
        + coefficients.c1_double_prime * delta.powi(2) / 2.0;
    let correction = c0 - c1 * t + coefficients.c2 * t * t;
    Ok((
        0.5 * libm::erfc(-u) + p_term * correction,
        0.5 * libm::erfc(u) - p_term * correction,
    ))
}

/// g(δ) = Σ_{k≥2} (−1)^(k+1)·(p·t^k + (−1)^k·q·s^k)/k with t = δ/p, s = δ/q
/// (DESIGN.md evaluation appendix). The log1p form p·log1p(δ/p) +
/// q·log1p(−δ/q) loses ~10 digits to cancellation when |δ| is small
/// (g ≈ −δ²/(2pq)).
fn central_g(
    p: f64,
    delta: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let q = 1.0 - p;
    let t = delta / p;
    let s = delta / q;
    let mut total = 0.0;
    let mut t_power = t * t;
    let mut s_power = s * s;
    for k in 2..=CENTRAL_SERIES_TERMS {
        on_iteration()?;
        let sign = if k % 2 == 0 { -1.0 } else { 1.0 };
        let inner = p * t_power + (if k % 2 == 0 { 1.0 } else { -1.0 }) * q * s_power;
        total += sign * inner / f64::from(k);
        t_power *= t;
        s_power *= s;
    }
    Ok(total)
}

/// The δ-expansion coefficients evaluated at x0 = p. The forms are taken
/// verbatim from smalldelta_forms.json (validated 751/751); only the
/// identical `sqrt(-x0*(x0-1))` factor is shared between coefficients. The
/// coefficients are evaluated against p_hi (the f64 mean); the ≈1e-16
/// relative difference from the exact mean is negligible (DESIGN.md).
struct CentralCoefficients {
    c0: f64,
    c0_prime: f64,
    c0_double_prime: f64,
    c0_triple_prime: f64,
    c0_quadruple_prime: f64,
    c1: f64,
    c1_prime: f64,
    c1_double_prime: f64,
    c2: f64,
}

fn central_coefficients(x0: f64) -> CentralCoefficients {
    let base = (-x0 * (x0 - 1.0)).sqrt();
    let quadratic = x0 * x0 - x0 + 1.0;
    let x0_squared = x0 * x0;
    let x0_cubed = x0_squared * x0;
    let x0_fourth = x0_squared * x0_squared;
    let complement_squared = (x0 - 1.0) * (x0 - 1.0);
    let complement_cubed = complement_squared * (x0 - 1.0);
    let complement_fourth = complement_squared * complement_squared;
    CentralCoefficients {
        // c0(0) = sqrt(-x0*(x0 - 1))*(2*x0 - 1)/(3*x0*(x0 - 1))
        c0: base * (2.0 * x0 - 1.0) / (3.0 * x0 * (x0 - 1.0)),
        // c0'(0) = -sqrt(-x0*(x0 - 1))*(x0**2 - x0 + 1)/(12*x0**2*(x0 - 1)**2)
        c0_prime: -(base * quadratic) / (12.0 * x0_squared * complement_squared),
        // c0''(0) = sqrt(-x0*(x0 - 1))*(2*x0 - 1)*(11*x0**2 - 11*x0 + 23)
        //           /(270*x0**3*(x0 - 1)**3)
        c0_double_prime: base * (2.0 * x0 - 1.0) * (11.0 * x0_squared - 11.0 * x0 + 23.0)
            / (270.0 * x0_cubed * complement_cubed),
        // c0'''(0) = -sqrt(-x0*(x0 - 1))*(329*x0**4 - 658*x0**3 + 1587*x0**2
        //             - 1258*x0 + 353)/(2160*x0**4*(x0 - 1)**4)
        c0_triple_prime: -(base
            * (329.0 * x0_fourth - 658.0 * x0_cubed + 1587.0 * x0_squared - 1258.0 * x0 + 353.0))
            / (2160.0 * x0_fourth * complement_fourth),
        // c0''''(0) = -(2*x0 - 1)*(987*x0**4 - 1974*x0**3 + 7277*x0**2
        //              - 6290*x0 + 2471)/(4320*(x0*(1 - x0))**4.5)
        c0_quadruple_prime: -(2.0 * x0 - 1.0)
            * (987.0 * x0_fourth - 1974.0 * x0_cubed + 7277.0 * x0_squared - 6290.0 * x0 + 2471.0)
            / (4320.0 * (x0 * (1.0 - x0)).powf(4.5)),
        // c1(0) = -sqrt(-x0*(x0 - 1))*(2*x0 - 1)*(23*x0**2 - 23*x0 - 1)
        //         /(540*x0**2*(x0 - 1)**2)
        c1: -(base * (2.0 * x0 - 1.0) * (23.0 * x0_squared - 23.0 * x0 - 1.0))
            / (540.0 * x0_squared * complement_squared),
        // c1'(0) = sqrt(-x0*(x0 - 1))*(x0**2 - x0 + 1)**2/(288*x0**3*(x0 - 1)**3)
        c1_prime: base * quadratic * quadratic / (288.0 * x0_cubed * complement_cubed),
        // c1''(0) = sqrt(-x0*(x0 - 1))*(2*x0 - 1)*(x0**2 - x0 - 23)
        //           *(x0**2 - x0 + 1)/(3024*x0**4*(x0 - 1)**4)
        c1_double_prime: base * (2.0 * x0 - 1.0) * (x0_squared - x0 - 23.0) * quadratic
            / (3024.0 * x0_fourth * complement_fourth),
        // c2(0) = sqrt(-x0*(x0 - 1))*(2*x0 - 1)*(x0**2 - x0 + 1)
        //         *(23*x0**2 - 23*x0 - 25)/(6048*x0**3*(x0 - 1)**3)
        c2: base * (2.0 * x0 - 1.0) * quadratic * (23.0 * x0_squared - 23.0 * x0 - 25.0)
            / (6048.0 * x0_cubed * complement_cubed),
    }
}

/// Direct (small-tail) evaluation of x^a·(1−x)^b/(a·B(a,b))·F in log space
/// (FORMULAS.md §2.2). `log_x`/`log_w` are the caller's log coordinates
/// whenever the coordinate rounded to 0; a rounded-zero coordinate keeps the
/// fraction at 1 and evaluates the prefactor from the logs, so the
/// representable subnormal tail survives. The exponent is never pre-truncated
/// at ln(f64::MIN_POSITIVE): exp() rounds to 0 only when the value itself is
/// below the subnormal range.
fn direct_tail(
    a: f64,
    b: f64,
    x: f64,
    log_x: Option<f64>,
    log_w: Option<f64>,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let fraction = if x > 0.0 {
        beta_fraction_dd(a, b, x, on_iteration)?
    } else {
        1.0
    };
    let actual_log_x = log_x.unwrap_or_else(|| x.ln());
    let actual_log_w = log_w.unwrap_or_else(|| (-x).ln_1p());
    let total = a + b;
    let p = a / total;
    let q = b / total;
    // Same mean-residual correction as the central path: the prefactor
    // a·ratio_x + b·ratio_w has coefficients as large as the shapes, so a
    // rounded p would be amplified into the exponent.
    let (ratio_x, ratio_w) = if x > 0.0 {
        let p_low = f64::mul_add(-p, total, a) / total;
        let delta = (x - p) - p_low;
        let scaled_p = delta / p;
        let scaled_q = delta / q;
        (
            if scaled_p.abs() < 0.5 {
                scaled_p.ln_1p()
            } else {
                actual_log_x - p.ln()
            },
            if scaled_q.abs() < 0.5 {
                (-scaled_q).ln_1p()
            } else {
                actual_log_w - q.ln()
            },
        )
    } else {
        (actual_log_x - p.ln(), actual_log_w - q.ln())
    };
    let prefactor = if a >= LARGE_SHAPE && b >= LARGE_SHAPE {
        a * ratio_x + b * ratio_w + 0.5 * (p * q * total).ln()
            - 0.5 * (2.0 * std::f64::consts::PI).ln()
            - stirling_correction(a)
            - stirling_correction(b)
            + stirling_correction(total)
    } else if a >= LARGE_SHAPE {
        a * ratio_x + b * ratio_w + 0.5 * p.ln() + b * b.ln()
            - b
            - ln_gamma(b)?
            - stirling_correction(a)
            + stirling_correction(total)
    } else if b >= LARGE_SHAPE {
        a * ratio_x + b * ratio_w + 0.5 * q.ln() + a * a.ln()
            - a
            - ln_gamma(a)?
            - stirling_correction(b)
            + stirling_correction(total)
    } else {
        a * actual_log_x + b * actual_log_w - ln_beta(a, b)?
    };
    bounded_probability((prefactor + fraction.ln() - a.ln()).exp())
}

/// Modified-Lentz evaluation of the incomplete-beta continued fraction with
/// every recurrence variable in double-double arithmetic (FORMULAS.md §2.2);
/// only the final fraction rounds to f64. Each loop pass applies one even
/// and one odd fraction step, and the recurrence stops when
/// abs((δ−1).hi) + abs((δ−1).lo) < 2⁻¹⁰⁰.
fn beta_fraction_dd(
    a: f64,
    b: f64,
    x: f64,
    on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let a_dd = DoubleDouble::new(a);
    let b_dd = DoubleDouble::new(b);
    let x_dd = DoubleDouble::new(x);
    let one = DoubleDouble::new(1.0);
    let total = a_dd.add(b_dd);
    let above = a_dd.add(one);
    let below = a_dd.sub(one);
    let mut c = one;
    let mut d = protect_lentz(one.sub(total.mul(x_dd).div(above))).reciprocal();
    let mut fraction = d;
    for index in 1..=MAX_REFINEMENT_ITERATIONS {
        on_iteration()?;
        let m = DoubleDouble::new(f64::from(index));
        let doubled = DoubleDouble::new(f64::from(2 * index));
        let coefficient = m
            .mul(b_dd.sub(m))
            .mul(x_dd)
            .div(below.add(doubled).mul(a_dd.add(doubled)));
        d = protect_lentz(one.add(coefficient.mul(d))).reciprocal();
        c = protect_lentz(one.add(coefficient.div(c)));
        fraction = fraction.mul(d).mul(c);

        let coefficient = a_dd
            .add(m)
            .neg()
            .mul(total.add(m))
            .mul(x_dd)
            .div(a_dd.add(doubled).mul(above.add(doubled)));
        d = protect_lentz(one.add(coefficient.mul(d))).reciprocal();
        c = protect_lentz(one.add(coefficient.div(c)));
        let change = d.mul(c);
        fraction = fraction.mul(change);
        let difference = change.sub(one);
        if difference.magnitude() < DOUBLE_DOUBLE_TOLERANCE {
            return Ok(fraction.as_f64());
        }
    }
    Err(ErrorKind::Num)
}

/// Modified-Lentz floor in double-double: when abs(hi) + abs(lo) drops below
/// LENTZ_TINY, replace the value with ±LENTZ_TINY (sign from hi, falling
/// back to lo).
fn protect_lentz(value: DoubleDouble) -> DoubleDouble {
    if value.magnitude() >= LENTZ_TINY {
        value
    } else {
        let sign_source = if value.hi != 0.0 { value.hi } else { value.lo };
        DoubleDouble::new(sign_source.copysign(LENTZ_TINY))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        beta_density_exponent, ln_beta, regularized_incomplete_beta,
        regularized_incomplete_beta_upper,
    };
    use crate::calculation::limits::CalculationLimitKind;
    use crate::calculation::value::ErrorKind;

    fn incomplete_beta(a: f64, b: f64, x: f64) -> f64 {
        regularized_incomplete_beta(a, b, x, || Ok(())).expect("valid domain")
    }

    // The plan's §6.2 tolerance policy, as applied by the 0.1.13 validation
    // scripts (final_validation.py `allowed_cdf`).
    fn allowed_cdf(reference: f64) -> f64 {
        if reference > 0.0 && reference < 1e-12 {
            let two_subnormal = 2.0 * f64::from_bits(1);
            two_subnormal.max(5e-9 * reference.abs())
        } else {
            2e-14_f64.max(2e-12 * reference.abs())
        }
    }

    fn allowed_density(reference: f64) -> f64 {
        2e-14_f64.max(2e-11 * reference.abs())
    }

    // Fixtures generated by 0.1.13-math-prep/generate_fixtures.py:
    // central-path grid and transition rows (beta_lib.py, Decimal-110),
    // ln_beta (mpmath gammaln, dps=60), PDF exponent (Decimal-110), and
    // double-double direct-CF regressions (Decimal-110). Fields match the
    // generator's comments per section.
    const CENTRAL_GRID: [(f64, f64, f64, f64, f64); 53] = [
        (1000000.0, 1000000.0, 0.5, 0.5, 0.5),
        (
            1000000.0,
            1000000.0,
            0.500353553302205,
            0.8413446855758903,
            0.1586553144241098,
        ),
        (
            1000000.0,
            1000000.0,
            0.49964644669779507,
            0.15865531442414776,
            0.8413446855758522,
        ),
        (
            1000000.0,
            1000000.0,
            0.5014142132088198,
            0.9999683296280558,
            3.1670371944138676e-05,
        ),
        (
            1000000.0,
            1000000.0,
            0.49858578679118015,
            3.1670371944138676e-05,
            0.9999683296280558,
        ),
        (
            1000000.0,
            1000000.0,
            0.5028284264176397,
            0.9999999999999993,
            6.217879387386642e-16,
        ),
        (
            1000000.0,
            1000000.0,
            0.4971715735823603,
            6.217879387386642e-16,
            0.9999999999999993,
        ),
        (
            1000000.0,
            1000000.0,
            0.5042426396264595,
            1.0,
            1.7719480898505355e-33,
        ),
        (
            1000000.0,
            1000000.0,
            0.4957573603735405,
            1.771948089853897e-33,
            1.0,
        ),
        (
            1000000.0,
            999000000.0,
            0.001,
            0.5001327812065861,
            0.4998672187934139,
        ),
        (
            1000000.0,
            999000000.0,
            0.0010009994998744377,
            0.8413447861265592,
            0.15865521387344086,
        ),
        (
            1000000.0,
            999000000.0,
            0.0009990005001255624,
            0.15865521381641715,
            0.8413447861835829,
        ),
        (
            1000000.0,
            999000000.0,
            0.0010039979994977508,
            0.9999676555737744,
            3.234442622557855e-05,
        ),
        (
            1000000.0,
            999000000.0,
            0.0009960020005022492,
            3.100810558381563e-05,
            0.9999689918944162,
        ),
        (
            1000000.0,
            999000000.0,
            0.0010079959989955016,
            0.9999999999999992,
            7.36840775469319e-16,
        ),
        (
            1000000.0,
            999000000.0,
            0.0009920040010044982,
            5.241475082445706e-16,
            0.9999999999999994,
        ),
        (
            1000000.0,
            999000000.0,
            0.0010119939984932526,
            1.0,
            3.141075373613254e-33,
        ),
        (
            1000000.0,
            999000000.0,
            0.0009880060015067475,
            9.943725437781412e-34,
            1.0,
        ),
        (500000000.0, 500000000.0, 0.5, 0.5, 0.5),
        (
            500000000.0,
            500000000.0,
            0.500015811388293,
            0.841344745948313,
            0.15865525405168704,
        ),
        (
            500000000.0,
            500000000.0,
            0.49998418861170707,
            0.15865525405253655,
            0.8413447459474634,
        ),
        (
            500000000.0,
            500000000.0,
            0.5000632455531717,
            0.9999683287599065,
            3.167124009353529e-05,
        ),
        (
            500000000.0,
            500000000.0,
            0.4999367544468283,
            3.167124009353529e-05,
            0.9999683287599065,
        ),
        (
            500000000.0,
            500000000.0,
            0.5001264911063434,
            0.9999999999999993,
            6.220954410661108e-16,
        ),
        (
            500000000.0,
            500000000.0,
            0.4998735088936565,
            6.220954410483732e-16,
            0.9999999999999993,
        ),
        (
            500000000.0,
            500000000.0,
            0.5001897366595153,
            1.0,
            1.77647303284631e-33,
        ),
        (
            500000000.0,
            500000000.0,
            0.4998102633404848,
            1.7764730329216658e-33,
            1.0,
        ),
        (
            500000000.0,
            4500000000.0,
            0.1,
            0.5000050150190426,
            0.4999949849809574,
        ),
        (
            500000000.0,
            4500000000.0,
            0.10000424264068669,
            0.8413447461014903,
            0.15865525389850965,
        ),
        (
            500000000.0,
            4500000000.0,
            0.0999957573593133,
            0.15865525389834875,
            0.8413447461016512,
        ),
        (
            500000000.0,
            4500000000.0,
            0.10001697056274678,
            0.9999683035160755,
            3.169648392452147e-05,
        ),
        (
            500000000.0,
            4500000000.0,
            0.09998302943725322,
            3.1646013384613006e-05,
            0.9999683539866154,
        ),
        (
            500000000.0,
            4500000000.0,
            0.10003394112549356,
            0.9999999999999993,
            6.261091113651963e-16,
        ),
        (
            500000000.0,
            4500000000.0,
            0.09996605887450644,
            6.1810667974939e-16,
            0.9999999999999993,
        ),
        (
            500000000.0,
            4500000000.0,
            0.10005091168824035,
            1.0,
            1.815472812413005e-33,
        ),
        (
            500000000.0,
            4500000000.0,
            0.09994908831175967,
            1.7382996240072936e-33,
            1.0,
        ),
        (4999999999.5, 4999999999.5, 0.5, 0.5, 0.5),
        (
            4999999999.5,
            4999999999.5,
            0.500005,
            0.8413447460580296,
            0.1586552539419704,
        ),
        (
            4999999999.5,
            4999999999.5,
            0.499995,
            0.1586552539446568,
            0.8413447460553432,
        ),
        (
            4999999999.5,
            4999999999.5,
            0.50002,
            0.9999683287583414,
            3.167124165860526e-05,
        ),
        (
            4999999999.5,
            4999999999.5,
            0.49998,
            3.167124165860526e-05,
            0.9999683287583414,
        ),
        (
            4999999999.5,
            4999999999.5,
            0.50004,
            0.9999999999999993,
            6.220959957490522e-16,
        ),
        (
            4999999999.5,
            4999999999.5,
            0.49996,
            6.220959958051437e-16,
            0.9999999999999993,
        ),
        (
            4999999999.5,
            4999999999.5,
            0.50006,
            1.0,
            1.7764812043765858e-33,
        ),
        (
            4999999999.5,
            4999999999.5,
            0.49994,
            1.7764812041382894e-33,
            1.0,
        ),
        (
            1000000.0,
            1000000.0,
            0.5042426396260352,
            1.0,
            1.77194811554706e-33,
        ),
        (
            1000000.0,
            1000000.0,
            0.5042426396268838,
            1.0,
            1.771948064160735e-33,
        ),
        (
            1000000.0,
            1000000.0,
            0.49575736037396473,
            1.771948115543698e-33,
            1.0,
        ),
        (
            1000000.0,
            1000000.0,
            0.49575736037311624,
            1.771948064160735e-33,
            1.0,
        ),
        (
            500000000.0,
            500000000.0,
            0.5001897366594963,
            1.0,
            1.77647305861792e-33,
        ),
        (
            500000000.0,
            500000000.0,
            0.5001897366595343,
            1.0,
            1.7764730070747005e-33,
        ),
        (
            500000000.0,
            500000000.0,
            0.4998102633405037,
            1.77647305861792e-33,
            1.0,
        ),
        (
            500000000.0,
            500000000.0,
            0.4998102633404658,
            1.7764730071500562e-33,
            1.0,
        ),
    ];

    const LN_BETA_REFERENCES: [(f64, f64, f64); 16] = [
        (2048.0, 2048.0, -2841.6775879079755),
        (2047.9, 3000.0, -3411.241410180502),
        (2048.1, 3000.0, -3411.4218648857996),
        (2500.0, 3000.0, -3792.240782232465),
        (1000000.0, 1000000.0, -1386300.003362921),
        (1000000.0, 999000000.0, -7907261.100548499),
        (500000000.0, 500000000.0, -693147189.3094925),
        (500000000.0, 4500000000.0, -1625414876.0006816),
        (4999999999.5, 4999999999.5, -6931471814.807146),
        (0.5, 500000000.0, -9.442694385018532),
        (500000000.0, 0.5, -9.442694385018532),
        (0.5, 4999999999.5, -10.593986931690555),
        (0.001, 1000000000.0, 6.886455619547407),
        (1000000.0, 0.5, -6.335390211057437),
        (0.001, 0.001, 7.600900817008347),
        (2.0, 3.0, -2.4849066497880004),
    ];

    const DENSITY_REFERENCES: [(f64, f64, f64, f64); 25] = [
        (500000000.0, 500000000.0, 0.5, 25231.325213893768),
        (
            500000000.0,
            500000000.0,
            0.500015811388293,
            15303.572346488681,
        ),
        (
            500000000.0,
            500000000.0,
            0.49998418861170707,
            15303.572346542409,
        ),
        (
            500000000.0,
            500000000.0,
            0.5000632455531717,
            8.464166323201736,
        ),
        (
            500000000.0,
            500000000.0,
            0.4999367544468283,
            8.464166323201736,
        ),
        (
            500000000.0,
            500000000.0,
            0.5001897366595153,
            1.3574855231040293e-27,
        ),
        (
            500000000.0,
            500000000.0,
            0.4998102633404848,
            1.3574855231612202e-27,
        ),
        (500000000.0, 4500000000.0, 0.1, 94031.5972421133),
        (
            500000000.0,
            4500000000.0,
            0.10000424264068669,
            57031.612861111935,
        ),
        (
            500000000.0,
            4500000000.0,
            0.0999957573593133,
            57034.480662257,
        ),
        (
            500000000.0,
            4500000000.0,
            0.10005091168824035,
            5.167790190342687e-27,
        ),
        (
            500000000.0,
            4500000000.0,
            0.09994908831175967,
            4.952564965131098e-27,
        ),
        (1000000.0, 999000000.0, 0.001, 399141.86800791015),
        (
            1000000.0,
            999000000.0,
            0.0010009994998744377,
            241930.74239055743,
        ),
        (
            1000000.0,
            999000000.0,
            0.0009990005001255624,
            242253.04721491568,
        ),
        (0.5, 500000000.0, 9.99999999e-10, 241970724.88209945),
        (0.5, 500000000.0, 6.6568542343502444e-09, 5543167.598478089),
        (500000000.0, 0.5, 0.999999999, 241970731.48352832),
        (500000000.0, 0.5, 0.9999999933431458, 5543167.774933952),
        (0.5, 2.0, 0.3, 0.9585144756340407),
        (0.5, 2.0, 0.4, 0.7115124735378853),
        (2.0, 3.0, 0.3, 1.764),
        (2.0, 3.0, 0.4, 1.728),
        (0.001, 1000000000.0, 9.99999999999e-13, 992695447.1976395),
        (
            0.001,
            1000000000.0,
            3.262277658582397e-11,
            29585235.575548504,
        ),
    ];

    const DIRECT_CF_REFERENCES: [(f64, f64, f64, f64); 7] = [
        (2.0, 3.0, 0.3, 0.3483),
        (500.0, 700.0, 0.05, 1.1093543784e-314),
        (500000000.0, 0.5, 0.9999999933431458, 0.009877513491937773),
        (1000000.0, 2048.0, 0.9974147920074132, 7.160127527544618e-29),
        (
            1000000000.0,
            2048.0,
            0.9999979067494994,
            0.15863586726165863,
        ),
        (
            5000000000.0,
            100000.0,
            0.9999799371563362,
            0.158654851712051,
        ),
        (
            5000000000.0,
            999999.0,
            0.9997998402520065,
            0.1586552136372943,
        ),
    ];

    #[test]
    fn central_path_and_transition_match_the_decimal_reference() {
        for (a, b, x, lower, upper) in CENTRAL_GRID {
            let actual_lower = incomplete_beta(a, b, x);
            let actual_upper =
                regularized_incomplete_beta_upper(a, b, x, None, None, || Ok(())).expect("domain");
            assert!(
                (actual_lower - lower).abs() <= allowed_cdf(lower),
                "lower I_x({a}, {b}, {x}): {actual_lower} vs {lower}",
            );
            assert!(
                (actual_upper - upper).abs() <= allowed_cdf(upper),
                "upper I_x({a}, {b}, {x}): {actual_upper} vs {upper}",
            );
            assert!(
                (actual_lower + actual_upper - 1.0).abs() <= allowed_cdf(lower),
                "I + I' pair inconsistent at ({a}, {b}, {x})",
            );
        }
    }

    #[test]
    fn cancellation_resistant_ln_beta_matches_mpmath() {
        for (a, b, expected) in LN_BETA_REFERENCES {
            let actual = ln_beta(a, b).expect("valid domain");
            assert!(
                (actual - expected).abs() <= 2e-12 * expected.abs(),
                "ln B({a}, {b}): {actual} vs {expected}",
            );
        }
    }

    #[test]
    fn large_shape_density_exponent_matches_the_decimal_reference() {
        for (a, b, x, expected) in DENSITY_REFERENCES {
            let log_density =
                beta_density_exponent(a, b, x, x.ln(), (-x).ln_1p()).expect("valid domain");
            let actual = log_density.exp();
            assert!(
                (actual - expected).abs() <= allowed_density(expected),
                "density({a}, {b}, {x}): {actual} vs {expected}",
            );
        }
    }

    #[test]
    fn double_double_direct_cf_matches_the_decimal_reference() {
        for (a, b, x, expected) in DIRECT_CF_REFERENCES {
            let actual = incomplete_beta(a, b, x);
            assert!(
                (actual - expected).abs() <= allowed_cdf(expected),
                "direct I_x({a}, {b}, {x}): {actual} vs {expected}",
            );
        }
    }

    // reference: mpmath 1.4.1, mp.dps = 30 — (a, b, x, I, tolerance) covering
    // both symmetry branches (the switch point is (a + 1)/(a + b + 2)), tiny
    // and large shape parameters, and both extreme tails. The two deep-tail
    // rows carry relative tolerances: at a = 500 the accuracy limit is the
    // ULP of the log-space exponent a·ln x (≈ 1e-13 relative), not machine
    // epsilon, and an absolute bound there would have no discriminating power.
    const GRID_REFERENCES: [(f64, f64, f64, f64, f64); 15] = [
        (0.001, 0.001, 1e-9, 0.4897457971281709, 1e-12),
        (0.001, 0.001, 0.5, 0.5, 1e-12),
        (0.001, 0.001, 0.999, 0.5034406643608944, 1e-12),
        (0.5, 0.5, 0.25, 0.3333333333333333, 1e-12),
        (1.0, 1.0, 0.3, 0.3, 1e-12),
        (1.0, 5.0, 0.2, 0.67232, 1e-12),
        (2.0, 3.0, 0.3, 0.3483, 1e-12),
        (2.0, 3.0, 0.6, 0.8208, 1e-12),
        (8.0, 10.0, 0.05, 6.313621707851916e-7, 1e-18),
        (8.0, 10.0, 0.6, 0.9081007458287615, 1e-12),
        (8.0, 10.0, 0.99, 0.9999999999999998, 1e-15),
        (500.0, 700.0, 0.28, 1.9508843506706205e-24, 2e-35),
        (500.0, 700.0, 0.35, 8.924518076445037e-7, 1e-17),
        (500.0, 700.0, 0.4, 0.12051628908943207, 2e-12),
        (500.0, 700.0, 0.5, 0.9999999964603646, 1e-12),
    ];

    #[test]
    fn incomplete_beta_matches_mpmath_on_both_branches() {
        for (a, b, x, expected, tolerance) in GRID_REFERENCES {
            let actual = incomplete_beta(a, b, x);
            assert!(
                (actual - expected).abs() <= tolerance,
                "I_{x}({a}, {b}): {actual} vs {expected}",
            );
        }
    }

    #[test]
    fn uniform_beta_cdf_is_the_exact_identity() {
        for x in [f64::MIN_POSITIVE, 0.25, 0.5, 0.75, 1.0 - f64::EPSILON] {
            assert_eq!(incomplete_beta(1.0, 1.0, x), x);
        }
    }

    // reference: mpmath 1.4.1, mp.dps = 30 — one f64 step below, at, and
    // above the exact switch points 0.4 (for a=3, b=5) and 0.45 (for a=8,
    // b=10), so the branch selection itself is under test.
    const SWITCH_REFERENCES: [(f64, f64, f64, f64); 6] = [
        (3.0, 5.0, 0.39999999999999997, 0.580096),
        (3.0, 5.0, 0.4, 0.5800960000000001),
        (3.0, 5.0, 0.4000000000000001, 0.5800960000000002),
        (8.0, 10.0, 0.44999999999999996, 0.5256918104178496),
        (8.0, 10.0, 0.45, 0.5256918104178498),
        (8.0, 10.0, 0.45000000000000007, 0.52569181041785),
    ];

    #[test]
    fn both_branches_agree_across_the_symmetry_switch_point() {
        for (a, b, x, expected) in SWITCH_REFERENCES {
            let actual = incomplete_beta(a, b, x);
            assert!(
                (actual - expected).abs() <= 1e-14,
                "I_{x}({a}, {b}): {actual} vs {expected}",
            );
        }
    }

    #[test]
    fn tails_obey_the_reflection_identity_across_the_grid() {
        for (a, b) in [
            (0.001, 0.001),
            (0.5, 2.5),
            (2.0, 3.0),
            (8.0, 10.0),
            (500.0, 700.0),
        ] {
            for step in 1..16 {
                // Dyadic abscissae keep x and 1 − x exact in binary.
                let x = f64::from(step) / 16.0;
                let direct = incomplete_beta(a, b, x);
                let reflected = incomplete_beta(b, a, 1.0 - x);
                assert!(
                    (direct + reflected - 1.0).abs() <= 1e-12,
                    "a={a} b={b} x={x}: I + I' = {}",
                    direct + reflected,
                );
            }
        }
    }

    #[test]
    fn lower_tail_is_monotone_in_x_including_the_branch_boundary() {
        for (a, b) in [(0.5, 2.5), (2.0, 3.0), (8.0, 10.0)] {
            let mut previous = 0.0;
            for step in 0..=200 {
                let x = f64::from(step) / 200.0;
                let probability = incomplete_beta(a, b, x);
                assert!(
                    probability + 1e-13 >= previous,
                    "a={a} b={b} x={x}: {probability} dropped below {previous}",
                );
                previous = probability;
            }
        }
    }

    #[test]
    fn deep_tails_preserve_representable_subnormals() {
        // reference: mpmath 1.4.1 — I_0.05(500, 700) ≈ 1.11e-314 is
        // representable as a subnormal. The 0.1.13 log-space direct tail
        // (FORMULAS.md §2.2) no longer truncates at ln(f64::MIN_POSITIVE), so
        // the tail survives instead of being reported as exact zero (0.1.12
        // reported 0.0 here).
        let tail = incomplete_beta(500.0, 700.0, 0.05);
        assert!(tail > 0.0 && tail < 5e-314, "deep tail: {tail}");
        // The complement rounds back to exactly one; the symmetry floor
        // remains 1 − subnormal = 1.
        assert_eq!(incomplete_beta(500.0, 700.0, 0.9), 1.0);
        assert_eq!(incomplete_beta(2.0, 3.0, 0.0), 0.0);
        assert_eq!(incomplete_beta(2.0, 3.0, 1.0), 1.0);
    }

    #[test]
    fn invalid_domains_are_rejected() {
        for (a, b, x) in [
            (0.0, 1.0, 0.5),
            (-1.0, 1.0, 0.5),
            (1.0, 0.0, 0.5),
            (1.0, -2.0, 0.5),
            (1.0, 1.0, -0.1),
            (1.0, 1.0, 1.1),
            (f64::NAN, 1.0, 0.5),
            (1.0, f64::NAN, 0.5),
            (1.0, 1.0, f64::NAN),
            (f64::INFINITY, 1.0, 0.5),
            (1.0, f64::INFINITY, 0.5),
        ] {
            assert_eq!(
                regularized_incomplete_beta(a, b, x, || Ok(())),
                Err(ErrorKind::Num),
                "a={a} b={b} x={x}",
            );
        }
    }

    #[test]
    fn central_asymptotic_series_charges_work_and_stops_on_callback_errors() {
        let mut calls = 0_u32;
        regularized_incomplete_beta(1_000_000.0, 1_000_000.0, 0.5001, || {
            calls += 1;
            Ok(())
        })
        .expect("central-band input");
        assert_eq!(calls, 23);

        let budget_error = ErrorKind::ResourceLimit(CalculationLimitKind::FunctionIterations);
        let mut remaining = 1_u32;
        let result = regularized_incomplete_beta(1_000_000.0, 1_000_000.0, 0.5001, || {
            if remaining == 0 {
                return Err(budget_error);
            }
            remaining -= 1;
            Ok(())
        });
        assert_eq!(result, Err(budget_error));
    }

    #[test]
    fn both_branches_charge_work_and_stop_on_callback_errors() {
        let budget_error = ErrorKind::ResourceLimit(CalculationLimitKind::FunctionIterations);
        for x in [0.2, 0.9] {
            let mut calls = 0_u32;
            regularized_incomplete_beta(2.0, 3.0, x, || {
                calls += 1;
                Ok(())
            })
            .expect("valid domain");
            assert!(calls > 0, "x={x} charged no work");

            let mut remaining = 1_u32;
            let result = regularized_incomplete_beta(2.0, 3.0, x, || {
                if remaining == 0 {
                    return Err(budget_error);
                }
                remaining -= 1;
                Ok(())
            });
            assert_eq!(result, Err(budget_error), "x={x}");
        }
    }
}
