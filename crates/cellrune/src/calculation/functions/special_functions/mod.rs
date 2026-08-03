//! Pure numeric kernels shared by the probability-distribution evaluators.
//!
//! Every branch-selection, termination, underflow and solver-tolerance rule
//! for this module lives in the constants below so the kernels cannot drift
//! apart. Each unbounded loop charges work through a caller-supplied callback
//! before it runs another step.

mod binomial;
mod incomplete_beta;
mod incomplete_gamma;
mod inverse;
mod log_binomial;
mod log_gamma;

pub(super) use binomial::{
    binomial_pmf, binomial_pmf_sum, negative_binomial_cdf, negative_binomial_pmf,
    smallest_binomial_quantile,
};
pub(super) use incomplete_beta::{ln_beta, regularized_incomplete_beta};
pub(super) use incomplete_gamma::regularized_gamma_p;
pub(super) use inverse::{DomainPolicy, invert_monotone_cdf};
pub(super) use log_gamma::{ln_gamma, signed_gamma};

use super::super::value::ErrorKind;

/// Log-space rounding can leave a probability a few ULP outside [0, 1];
/// clamping keeps every reported value a probability. Shared by all
/// distribution kernels so the policy cannot drift apart.
pub(super) fn bounded_probability(value: f64) -> Result<f64, ErrorKind> {
    if value.is_finite() {
        Ok(value.clamp(0.0, 1.0))
    } else {
        Err(ErrorKind::Num)
    }
}

/// Relative termination threshold for the incomplete-gamma series and
/// continued-fraction refinements.
const CONVERGENCE_EPSILON: f64 = f64::EPSILON;

/// Hard cap on refinement steps for one series or continued-fraction
/// evaluation. The series branch needs ≈ 8.6·√a steps when x is near a, so
/// this cap covers alpha up to ≈ 1.35e8; beyond that the kernel fails closed
/// with a typed error. Every step still charges the caller-supplied work
/// budget and polls cancellation through it, so the engine resource limits
/// govern pathological workloads long before this cap does.
const MAX_REFINEMENT_ITERATIONS: u32 = 100_000;

/// Modified-Lentz floor that keeps continued-fraction denominators away from
/// zero without disturbing converged digits.
const LENTZ_TINY: f64 = f64::MIN_POSITIVE / f64::EPSILON;

/// ln(f64::MIN_POSITIVE): a log-space prefactor below this underflows exp(),
/// so the affected tail probability is reported as exactly zero.
const LN_UNDERFLOW_LIMIT: f64 = -708.3964185322641;

/// Geometric bracket steps that cover the full finite f64 exponent range in
/// both directions before the search is declared impossible.
const MAX_BRACKET_STEPS: u32 = 1_100;

/// Safeguarded solver iterations; alternating with bisection halves the
/// bracket at least every other step, so a convergent problem always finishes
/// within this cap.
const MAX_SOLVER_ITERATIONS: u32 = 256;

/// The solver stops only when the bracket width and the probability residual
/// both meet their absolute + relative tolerances.
const SOLVER_X_ABSOLUTE_TOLERANCE: f64 = 1e-300;
const SOLVER_X_RELATIVE_TOLERANCE: f64 = 4.0 * f64::EPSILON;
const SOLVER_P_ABSOLUTE_TOLERANCE: f64 = 1e-16;
const SOLVER_P_RELATIVE_TOLERANCE: f64 = 1e-9;
