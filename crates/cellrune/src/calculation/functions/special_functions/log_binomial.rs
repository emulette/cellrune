use super::super::super::value::ErrorKind;
use super::log_gamma::ln_gamma;

/// Natural log of the binomial coefficient C(n, k) for integers 0 ≤ k ≤ n.
///
/// Evaluated as lnΓ(n+1) − lnΓ(k+1) − lnΓ(n−k+1), so large combinations
/// never pass through factorial products. The edge columns k = 0 and k = n
/// return exactly zero, which keeps downstream log-space masses free of the
/// small lnΓ(1) residue. Non-integer, out-of-order and non-finite arguments
/// are domain errors, as is a coefficient beyond the finite f64 range.
pub(in crate::calculation::functions) fn ln_binomial(n: f64, k: f64) -> Result<f64, ErrorKind> {
    if !n.is_finite() || n != n.trunc() || k != k.trunc() || k < 0.0 || k > n {
        return Err(ErrorKind::Num);
    }
    if k == 0.0 || k == n {
        return Ok(0.0);
    }
    let value = ln_gamma(n + 1.0)? - ln_gamma(k + 1.0)? - ln_gamma(n - k + 1.0)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(ErrorKind::Num)
    }
}

#[cfg(test)]
mod tests {
    use super::ln_binomial;
    use crate::calculation::value::ErrorKind;

    // reference: mpmath 1.4.1, mp.dps = 30 — log(binomial(n, k)) covering
    // small exact coefficients, the central column, and large n where any
    // factorial-product formulation would overflow immediately.
    const LN_BINOMIAL_REFERENCES: [(f64, f64, f64); 8] = [
        (10.0, 3.0, 4.787491742782046),
        (52.0, 5.0, 14.77062192297037),
        (100.0, 50.0, 66.78384165201743),
        (1000.0, 500.0, 689.4672615678512),
        (100000.0, 100.0, 787.5036545157869),
        (100000.0, 50000.0, 69308.7357994094),
        (100000.0, 99900.0, 787.5036545157869),
        (1000000.0, 123456.0, 373748.0244124986),
    ];

    #[test]
    fn ln_binomial_matches_mpmath_including_large_n() {
        for (n, k, expected) in LN_BINOMIAL_REFERENCES {
            let actual = ln_binomial(n, k).expect("valid domain");
            assert!(
                (actual - expected).abs() <= 1e-12 * expected.abs().max(1.0),
                "ln C({n}, {k}): {actual} vs {expected}",
            );
        }
    }

    #[test]
    fn edge_columns_are_exactly_zero() {
        for (n, k) in [
            (0.0, 0.0),
            (1.0, 0.0),
            (1.0, 1.0),
            (100000.0, 0.0),
            (100000.0, 100000.0),
        ] {
            assert_eq!(ln_binomial(n, k), Ok(0.0), "n={n} k={k}");
        }
    }

    #[test]
    fn invalid_domains_are_rejected() {
        for (n, k) in [
            (10.0, -1.0),
            (10.0, 11.0),
            (-1.0, 0.0),
            (10.5, 3.0),
            (10.0, 2.5),
            (f64::NAN, 1.0),
            (10.0, f64::NAN),
            (f64::INFINITY, 1.0),
            (1e308, 0.5e308),
        ] {
            assert_eq!(ln_binomial(n, k), Err(ErrorKind::Num), "n={n} k={k}");
        }
    }
}
