//! The regularized incomplete gamma functions for large shapes, where the series and continued
//! fraction need on the order of √a steps and lose digits to the size of their terms.
//!
//! With t = (a − 1) + s·√(a − 1), the integrand t^(a−1)·e^(−t)/Γ(a) becomes
//! e^(−c)/√(2π) · exp((a − 1)·(ln(1 + u) − u)) with u = s/√(a − 1), where c is the Stirling
//! correction of ln Γ(a) = ln (a − 1)!. It is a smooth bell in s, so composite Gauss–Legendre
//! quadrature from the tail's edge outward, on panels narrowed to the bell's steepness, resolves
//! either tail to full precision. The smaller tail is integrated directly.

use super::super::super::value::ErrorKind;
use super::bounded_probability;

/// Shapes from which callers use this kernel.
pub(in crate::calculation::functions) const LARGE_SHAPE: f64 = 1e4;

/// The integrand is followed outward from the tail's edge until it falls this far, in natural
/// logarithm, below its value at the edge; the rest of the tail is below e^(−50) of the integral.
const TAIL_DECAY: f64 = 50.0;

/// Ten-point Gauss–Legendre nodes and weights on [−1, 1], positive half.
const NODES: [f64; 5] = [
    0.148_874_338_981_631_2,
    0.433_395_394_129_247_2,
    0.679_409_568_299_024_4,
    0.865_063_366_688_984_5,
    0.973_906_528_517_171_7,
];
const WEIGHTS: [f64; 5] = [
    0.295_524_224_714_752_9,
    0.269_266_719_309_996_4,
    0.219_086_362_515_982,
    0.149_451_349_150_580_6,
    0.066_671_344_308_688_1,
];

/// Returns (P(a, x), Q(a, x)) for a ≥ [`LARGE_SHAPE`] and x ≥ 0. `on_iteration` is charged before
/// every quadrature panel.
pub(in crate::calculation::functions) fn regularized_gamma_large_shape(
    a: f64,
    x: f64,
    mut on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<(f64, f64), ErrorKind> {
    if !a.is_finite() || a < LARGE_SHAPE || !x.is_finite() || x < 0.0 {
        return Err(ErrorKind::Num);
    }
    let bell = Bell::new(a - 1.0);
    let edge = (x - bell.m) / bell.root;
    // t = 0 is s = −√m; the lower tail ends there.
    let lower = edge <= 0.0;
    let limit = if lower { -bell.root } else { f64::INFINITY };
    let tail = bell.tail(edge, limit, &mut on_iteration)?;
    let tail = bounded_probability(tail)?;
    Ok(if lower {
        (tail, 1.0 - tail)
    } else {
        (1.0 - tail, tail)
    })
}

/// The integrand of P(a, x) in s, for m = a − 1.
struct Bell {
    m: f64,
    root: f64,
    /// ln(e^(−c)/√(2π)).
    log_scale: f64,
}

impl Bell {
    fn new(m: f64) -> Self {
        Self {
            m,
            root: m.sqrt(),
            log_scale: -stirling_correction(m) - 0.5 * (2.0 * std::f64::consts::PI).ln(),
        }
    }

    /// The natural logarithm of the integrand without its scale.
    fn log_density(&self, s: f64) -> f64 {
        self.m * ln_1p_minus(s / self.root)
    }

    /// How steeply the logarithm falls at s: |d/ds| = |s|/(1 + s/√m).
    fn steepness(&self, s: f64) -> f64 {
        s.abs() / (1.0 + s / self.root)
    }

    /// Integrates the bell from `edge` outward to `limit` (left when `limit < edge`). Each panel is
    /// at most one unit wide and narrow enough that the logarithm falls by about one across it,
    /// and the integrand is divided by its value at the edge, so a tail far below f64's range
    /// keeps its digits until the final scale.
    fn tail(
        &self,
        edge: f64,
        limit: f64,
        on_iteration: &mut impl FnMut() -> Result<(), ErrorKind>,
    ) -> Result<f64, ErrorKind> {
        let direction = if limit < edge { -1.0 } else { 1.0 };
        let peak = self.log_density(edge);
        let mut near = edge;
        let mut sum = 0.0;
        while (limit - near) * direction > 0.0 {
            on_iteration()?;
            let width = (1.0 / (self.steepness(near) + 1.0)).min((limit - near).abs());
            let far = near + direction * width;
            let center = (near + far) / 2.0;
            let half = width / 2.0;
            let mut panel = 0.0;
            for (node, weight) in NODES.iter().zip(WEIGHTS) {
                panel += weight
                    * ((self.log_density(center - half * node) - peak).exp()
                        + (self.log_density(center + half * node) - peak).exp());
            }
            sum += panel * half;
            if self.log_density(far) - peak < -TAIL_DECAY {
                break;
            }
            near = far;
        }
        if sum == 0.0 {
            return Ok(0.0);
        }
        Ok((self.log_scale + peak + sum.ln()).exp())
    }
}

/// ln(1 + u) − u, without the cancellation of the two terms for small u.
fn ln_1p_minus(u: f64) -> f64 {
    if u.abs() >= 0.01 {
        return u.ln_1p() - u;
    }
    // −u²/2 + u³/3 − u⁴/4 + …; |u| < 0.01 makes each term 100 times smaller than the last.
    let mut power = u * u;
    let mut sum = 0.0;
    for k in 2..=10 {
        let term = power / f64::from(k);
        sum += if k % 2 == 0 { -term } else { term };
        power *= u;
    }
    sum
}

/// ln m! − (m·ln m − m + ½·ln(2πm)): the Stirling series, exact to f64 for m ≥ 10⁴.
fn stirling_correction(m: f64) -> f64 {
    let inverse = m.recip();
    let square = inverse * inverse;
    inverse * (1.0 / 12.0 - square * (1.0 / 360.0 - square * (1.0 / 1260.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free() -> Result<(), ErrorKind> {
        Ok(())
    }

    #[test]
    fn ten_point_gauss_legendre_is_exact_for_degree_nineteen() {
        let value = NODES
            .iter()
            .zip(WEIGHTS)
            .map(|(node, weight)| weight * 2.0 * node.powi(18))
            .sum::<f64>();
        assert!((value - 2.0 / 19.0).abs() < 1e-15, "{value}");
    }

    #[test]
    fn a_far_tail_keeps_its_digits() {
        // The power series summed in mpmath at 50 digits:
        // P(5e8, 998344117.6852958 / 2) = 9.999999999376541275e-301.
        let (p, _) =
            regularized_gamma_large_shape(5e8, 998_344_117.685_295_8 / 2.0, free).expect("tail");
        assert!((p / 9.999_999_999_376_541e-301 - 1.0).abs() < 1e-10, "{p}");
    }

    #[test]
    fn tails_match_high_precision_quadrature() {
        // mpmath at 50 digits puts the medians here. Rounding x to f64 moves P by up to the
        // density times half an ULP of x over √a, about 4e-13 at a = 5e8.
        for (a, x) in [(5e8, 499_999_999.666_666_7), (5e5, 499_999.666_666_706_2)] {
            let (p, q) = regularized_gamma_large_shape(a, x, free).expect("tails");
            assert!((p - 0.5).abs() < 5e-13, "{a} {p}");
            assert!((p + q - 1.0).abs() < 1e-15);
        }
    }
}
