//! F-distribution functions: F.DIST, F.DIST.RT, F.INV, F.INV.RT and F.TEST.
//!
//! All five names share one overflow-safe transform to the unit interval:
//! z = d1·x/(d1·x + d2) with w = 1 − z, evaluated directly whenever d1·x is
//! finite (exact at dyadic transforms) and as the ratio r form only when
//! d1·x would overflow, so neither d1·x nor d2/x can overflow. The exact log
//! coordinates ln z, ln w are derived from ln x directly (FORMULAS.md) and
//! handed to the beta kernels so a coordinate that rounds to an endpoint from
//! an interior x still yields the representable subnormal tail.

use super::super::super::ast::Expr;
use super::super::super::coerce::to_logical;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::array_common::poll_cancellation;
use super::super::moments::{NumericMoments, VarianceKind};
use super::super::special_functions::{
    beta_density_exponent, beta_pair, regularized_incomplete_beta_lower,
    regularized_incomplete_beta_upper,
};
use super::super::util::required_number;
use super::{finite, quantile_solver_error};

/// F.DIST(x, df1, df2, cumulative); the cumulative argument is typed logical.
pub(super) fn f_distribution(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 4 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match nonnegative_x(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let df1 = match degrees_of_freedom(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let df2 = match degrees_of_freedom(engine, context, &args[2]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let cumulative = match to_logical(&engine.eval_scalar(context, &args[3])) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let on_iteration = || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    };
    if cumulative {
        match lower_tail(x, df1, df2, on_iteration) {
            Ok(value) => finite(value),
            Err(kind) => Value::Error(kind),
        }
    } else {
        match density(x, df1, df2) {
            Ok(value) => finite(value),
            Err(kind) => Value::Error(kind),
        }
    }
}

/// F.DIST.RT(x, df1, df2) is the direct upper tail; the tail is never
/// computed as 1 − CDF, which would destroy small upper tails.
pub(super) fn f_distribution_rt(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match nonnegative_x(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let df1 = match degrees_of_freedom(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let df2 = match degrees_of_freedom(engine, context, &args[2]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    match upper_tail(x, df1, df2, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    }) {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

/// F.INV(probability, df1, df2): p = 0 is the support origin (x = 0) while
/// p = 1 is documented #NUM!.
pub(super) fn f_inverse(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let probability = match required_number(engine, context, &args[0]) {
        Ok(0.0) => return finite(0.0),
        Ok(value) if value > 0.0 && value < 1.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let df1 = match degrees_of_freedom(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let df2 = match degrees_of_freedom(engine, context, &args[2]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    match beta_pair(df1 / 2.0, df2 / 2.0, probability, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    }) {
        Ok((z, w)) => finite(restore_f_coordinate(z, w, df1, df2)),
        Err(kind) => Value::Error(quantile_solver_error(kind)),
    }
}

/// F.INV.RT(probability, df1, df2): p = 1 is the support origin while p = 0
/// is documented #NUM!. The quantile is solved on the reflected side
/// I_w(df2/2, df1/2), mirroring the lower inverse.
pub(super) fn f_inverse_rt(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let probability = match required_number(engine, context, &args[0]) {
        Ok(1.0) => return finite(0.0),
        Ok(value) if value > 0.0 && value < 1.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let df1 = match degrees_of_freedom(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let df2 = match degrees_of_freedom(engine, context, &args[2]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    match beta_pair(df2 / 2.0, df1 / 2.0, probability, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    }) {
        Ok((w, z)) => finite(restore_f_coordinate(z, w, df1, df2)),
        Err(kind) => Value::Error(quantile_solver_error(kind)),
    }
}

/// F.TEST(left, right): the two-tailed F-test p-value from the ratio of the
/// unbiased sample variances, the larger variance always in the numerator.
pub(in crate::calculation::functions) fn f_test(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let left = match sample_moments(engine, context, &args[0]) {
        Ok(moments) => moments,
        Err(kind) => return Value::Error(kind),
    };
    let right = match sample_moments(engine, context, &args[1]) {
        Ok(moments) => moments,
        Err(kind) => return Value::Error(kind),
    };
    let left_variance = match left.variance(VarianceKind::Sample) {
        Ok(variance) => variance,
        Err(kind) => return Value::Error(kind),
    };
    let right_variance = match right.variance(VarianceKind::Sample) {
        Ok(variance) => variance,
        Err(kind) => return Value::Error(kind),
    };
    // Either zero variance makes the ratio undefined (and a doubled zero
    // variance would otherwise pair with a NaN ratio); this is checked
    // before the ratio, covering both-zero inputs.
    if left_variance == 0.0 || right_variance == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    let (ratio, df1, df2) =
        variance_ratio(left_variance, right_variance, left.count(), right.count());
    match upper_tail(ratio, df1, df2, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    }) {
        Ok(tail) => finite((2.0 * tail).min(1.0)),
        Err(kind) => Value::Error(kind),
    }
}

/// The larger variance is always the numerator, and the degrees of freedom
/// travel with their variances.
fn variance_ratio(
    left_variance: f64,
    right_variance: f64,
    left_count: u64,
    right_count: u64,
) -> (f64, f64, f64) {
    if left_variance >= right_variance {
        (
            left_variance / right_variance,
            (left_count - 1) as f64,
            (right_count - 1) as f64,
        )
    } else {
        (
            right_variance / left_variance,
            (right_count - 1) as f64,
            (left_count - 1) as f64,
        )
    }
}

/// The shared df contract: numeric coercion, truncation toward zero, and the
/// documented 1 ≤ df < 1e10 domain.
pub(super) fn degrees_of_freedom(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    argument: &Expr,
) -> Result<f64, ErrorKind> {
    let df = required_number(engine, context, argument)?.trunc();
    if (1.0..1e10).contains(&df) {
        Ok(df)
    } else {
        Err(ErrorKind::Num)
    }
}

/// x is a finite non-negative number for the distribution evaluators.
pub(super) fn nonnegative_x(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    argument: &Expr,
) -> Result<f64, ErrorKind> {
    match required_number(engine, context, argument)? {
        value if value.is_finite() && value >= 0.0 => Ok(value),
        _ => Err(ErrorKind::Num),
    }
}

/// Overflow-safe F transform to (z, w) on the unit interval (FORMULAS.md).
/// The direct form z = d1·x/(d1·x + d2) is evaluated whenever d1·x is finite;
/// the ratio form (z = r/(1+r)) would round the exact transform away from
/// dyadic points (e.g. x = 1, d1 = 1e6, d2 = 999000000 has the exact
/// z = 0.001, but 1/(1 + d1/d2) rounds to 0.999000999…), and the kernel's
/// mean-residual terms amplify that offset by the shapes. The ratio form is
/// the fallback only when d1·x overflows.
fn f_coordinates(x: f64, df1: f64, df2: f64) -> (f64, f64) {
    if x == 0.0 {
        return (0.0, 1.0);
    }
    let scaled = df1 * x;
    if scaled.is_finite() {
        let z = scaled / (scaled + df2);
        (z, 1.0 - z)
    } else {
        let threshold = df2 / df1;
        if x <= threshold {
            let ratio = x * (df1 / df2);
            (ratio / (1.0 + ratio), 1.0 / (1.0 + ratio))
        } else {
            let ratio = threshold / x;
            (1.0 / (1.0 + ratio), ratio / (1.0 + ratio))
        }
    }
}

/// Exact log coordinates of the F transform, derived from ln x so they
/// survive a coordinate rounding to an endpoint (FORMULAS.md).
fn f_log_coordinates(x: f64, df1: f64, df2: f64) -> (f64, f64) {
    if x == 0.0 {
        return (f64::NEG_INFINITY, 0.0);
    }
    let log_ratio = x.ln() + df1.ln() - df2.ln();
    if log_ratio <= 0.0 {
        let log_w = -log_ratio.exp().ln_1p();
        (log_ratio + log_w, log_w)
    } else {
        let log_z = -(-log_ratio).exp().ln_1p();
        (log_z, -log_ratio + log_z)
    }
}

/// The kernels derive ln from the rounded coordinate unless a coordinate
/// rounded to an endpoint from an interior x, in which case the exact log
/// coordinates must take over so the subnormal tail stays representable.
/// z rounds to 1.0 while w stays subnormal (e.g. x = 1e300, df1 = df2 = 1),
/// so the upper kernel would otherwise early-return a zero tail.
pub(super) fn coordinate_logs(
    x: f64,
    z: f64,
    w: f64,
    log_z: f64,
    log_w: f64,
) -> (Option<f64>, Option<f64>) {
    if (z == 0.0 || w == 0.0 || z == 1.0) && x > 0.0 {
        (Some(log_z), Some(log_w))
    } else {
        (None, None)
    }
}

fn lower_tail(
    x: f64,
    df1: f64,
    df2: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let (z, w) = f_coordinates(x, df1, df2);
    let (log_z, log_w) = f_log_coordinates(x, df1, df2);
    let (log_z, log_w) = coordinate_logs(x, z, w, log_z, log_w);
    regularized_incomplete_beta_lower(df1 / 2.0, df2 / 2.0, z, log_z, log_w, on_iteration)
}

fn upper_tail(
    x: f64,
    df1: f64,
    df2: f64,
    on_iteration: impl FnMut() -> Result<(), ErrorKind>,
) -> Result<f64, ErrorKind> {
    let (z, w) = f_coordinates(x, df1, df2);
    let (log_z, log_w) = f_log_coordinates(x, df1, df2);
    let (log_z, log_w) = coordinate_logs(x, z, w, log_z, log_w);
    regularized_incomplete_beta_upper(df1 / 2.0, df2 / 2.0, z, log_z, log_w, on_iteration)
}

/// F density in log space: a·ln z + b·ln w − ln x − lnB(a, b). The endpoint
/// x = 0 is a pole for df1 < 2, the exact limit 1 for df1 = 2, and zero
/// above.
fn density(x: f64, df1: f64, df2: f64) -> Result<f64, ErrorKind> {
    if x == 0.0 {
        return if df1 < 2.0 {
            Err(ErrorKind::Num)
        } else if df1 == 2.0 {
            Ok(1.0)
        } else {
            Ok(0.0)
        };
    }
    let (z, _) = f_coordinates(x, df1, df2);
    let (log_z, log_w) = f_log_coordinates(x, df1, df2);
    let exponent = beta_density_exponent(df1 / 2.0, df2 / 2.0, z, log_z, log_w)?;
    Ok((exponent + log_z + log_w - x.ln()).exp())
}

/// x = (df2/df1)·z/w; the left-to-right evaluation order keeps a subnormal
/// w from overflowing the quotient prematurely.
fn restore_f_coordinate(z: f64, w: f64, df1: f64, df2: f64) -> f64 {
    (df2 / df1) * z / w
}

/// One-sided numeric sample moments; errors propagate, non-numbers are
/// skipped, and each accepted value charges engine work.
pub(in crate::calculation::functions) fn sample_moments(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    argument: &Expr,
) -> Result<NumericMoments, ErrorKind> {
    let values = engine.eval_array(context, argument)?;
    let mut numbers = Vec::new();
    for item in values.data {
        match item {
            Value::Error(kind) => return Err(kind),
            Value::Number(number) => numbers.push(number),
            _ => {}
        }
    }
    NumericMoments::collect_with_work(numbers, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    })
}

#[cfg(test)]
mod tests {
    use super::{
        coordinate_logs, density, f_coordinates, f_log_coordinates, lower_tail,
        restore_f_coordinate, upper_tail, variance_ratio,
    };
    use crate::calculation::functions::moments::{NumericMoments, VarianceKind};
    use crate::calculation::functions::special_functions::beta_pair;
    use crate::calculation::value::ErrorKind;

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

    fn assert_quantile(actual: f64, expected: f64, what: &str) {
        assert_within(actual, expected, 2e-12, 2e-9, what);
    }

    fn assert_within(actual: f64, expected: f64, abs_tol: f64, rel_tol: f64, what: &str) {
        let diff = (actual - expected).abs();
        let limit = abs_tol + rel_tol * expected.abs();
        assert!(
            diff <= limit,
            "{what}: {actual} vs {expected} (diff {diff:e} > {limit:e})",
        );
    }

    fn sample_variance(values: &[f64]) -> f64 {
        NumericMoments::collect_with_work(values.iter().copied(), || Ok(()))
            .expect("fixture samples are finite")
            .variance(VarianceKind::Sample)
            .expect("fixture samples have at least two values")
    }

    fn f_inverse(p: f64, df1: f64, df2: f64) -> f64 {
        let (z, w) = beta_pair(df1 / 2.0, df2 / 2.0, p, || Ok(())).expect("solver converges");
        restore_f_coordinate(z, w, df1, df2)
    }

    // Cumulative grid. Reference: beta_lib.py (Decimal-110) evaluated at the
    // f64 F-coordinate D.from_float(z_f64) of each x literal; endpoints
    // rounded from interior x use the f64 log-coordinate values exp(log_z) /
    // exp(log_w) (the Decimal transform cannot resolve the complement of
    // x ~ 1e300 at Decimal-110).
    // Fields: (x, df1, df2, lower, upper).
    // The literals are Decimal-110 reference values that must stay byte-exact
    // (several coincide with known constants, e.g. √2·5e-151); they are not
    // code approximations to be replaced by std constants.
    #[allow(clippy::approx_constant)]
    const CUMULATIVE_GRID: &[(f64, f64, f64, f64, f64)] = &[
        (0.0, 1.0, 1.0, 0.0, 1.0),
        (1e-300, 1.0, 1.0, 6.366197723675813e-151, 1.0),
        (1e-10, 1.0, 1.0, 6.366197723463607e-06, 0.9999936338022766),
        (0.5, 1.0, 1.0, 0.39182655203060723, 0.6081734479693928),
        (1.0, 1.0, 1.0, 0.5, 0.5),
        (2.0, 1.0, 1.0, 0.6081734479693927, 0.3918265520306073),
        (10.0, 1.0, 1.0, 0.8050177709578633, 0.1949822290421367),
        (
            1000000.0,
            1.0,
            1.0,
            0.999363380439823,
            0.0006366195601769944,
        ),
        (1e+300, 1.0, 1.0, 1.0, 6.36619772367589e-151),
        (1e+307, 1.0, 1.0, 1.0, 2.013168484179458e-154),
        (0.0, 1.0, 2.0, 0.0, 1.0),
        (1e-300, 1.0, 2.0, 7.071067811865476e-151, 1.0),
        (1e-10, 1.0, 2.0, 7.071067811688699e-06, 0.9999929289321883),
        (0.5, 1.0, 2.0, 0.4472135954999579, 0.552786404500042),
        (1.0, 1.0, 2.0, 0.5773502691896257, 0.42264973081037427),
        (2.0, 1.0, 2.0, 0.7071067811865476, 0.2928932188134525),
        (10.0, 1.0, 2.0, 0.9128709291752769, 0.08712907082472313),
        (1000000.0, 1.0, 2.0, 0.9999990000015, 9.99998500015988e-07),
        (1e+300, 1.0, 2.0, 1.0, 9.999999999999687e-301),
        (1e+307, 1.0, 2.0, 1.0, 9.999999999999218e-308),
        (0.0, 2.0, 5.0, 0.0, 1.0),
        (1e-300, 2.0, 5.0, 1e-300, 1.0),
        (1e-10, 2.0, 5.0, 9.999999999300001e-11, 0.9999999999),
        (0.5, 2.0, 5.0, 0.36606185473939107, 0.633938145260609),
        (1.0, 2.0, 5.0, 0.5687988496283078, 0.43120115037169215),
        (2.0, 2.0, 5.0, 0.7699518541666883, 0.23004814583331173),
        (10.0, 2.0, 5.0, 0.9821114561800017, 0.01788854381999831),
        (
            1000000.0,
            2.0,
            5.0,
            0.9999999999999901,
            9.88205592506318e-15,
        ),
        (1e+300, 2.0, 5.0, 1.0, 0.0),
        (1e+307, 2.0, 5.0, 1.0, 0.0),
        (0.0, 5.0, 30.0, 0.0, 1.0),
        (1e-300, 5.0, 30.0, 0.0, 1.0),
        (1e-10, 5.0, 30.0, 3.3518801847783487e-25, 1.0),
        (0.5, 5.0, 30.0, 0.22626640629640513, 0.7737335937035948),
        (1.0, 5.0, 30.0, 0.5653511236601266, 0.43464887633987337),
        (2.0, 5.0, 30.0, 0.8926646818950412, 0.10733531810495875),
        (10.0, 5.0, 30.0, 0.999989505238318, 1.0494761682015427e-05),
        (1000000.0, 5.0, 30.0, 1.0, 2.316014994196991e-77),
        (1e+300, 5.0, 30.0, 1.0, 0.0),
        (1e+307, 5.0, 30.0, 1.0, 0.0),
        (0.0, 30.0, 5.0, 0.0, 1.0),
        (1e-300, 30.0, 5.0, 0.0, 1.0),
        (1e-10, 30.0, 5.0, 2.3162429647964282e-137, 1.0),
        (0.5, 30.0, 5.0, 0.10733531810495875, 0.8926646818950412),
        (1.0, 30.0, 5.0, 0.4346488763398731, 0.5653511236601269),
        (2.0, 30.0, 5.0, 0.7737335937035952, 0.22626640629640485),
        (10.0, 30.0, 5.0, 0.9913657573262192, 0.008634242673780743),
        (
            1000000.0,
            30.0,
            5.0,
            0.9999999999999967,
            3.3518732046886535e-15,
        ),
        (1e+300, 30.0, 5.0, 1.0, 0.0),
        (1e+307, 30.0, 5.0, 1.0, 0.0),
        (0.0, 1000000.0, 1000000.0, 0.0, 1.0),
        (1e-300, 1000000.0, 1000000.0, 0.0, 1.0),
        (1e-10, 1000000.0, 1000000.0, 0.0, 1.0),
        (0.5, 1000000.0, 1000000.0, 0.0, 1.0),
        (1.0, 1000000.0, 1000000.0, 0.5, 0.5),
        (2.0, 1000000.0, 1000000.0, 1.0, 0.0),
        (10.0, 1000000.0, 1000000.0, 1.0, 0.0),
        (1000000.0, 1000000.0, 1000000.0, 1.0, 0.0),
        (1e+300, 1000000.0, 1000000.0, 1.0, 0.0),
        (1e+307, 1000000.0, 1000000.0, 1.0, 0.0),
        (0.0, 1000000.0, 999000000.0, 0.0, 1.0),
        (1e-300, 1000000.0, 999000000.0, 0.0, 1.0),
        (1e-10, 1000000.0, 999000000.0, 0.0, 1.0),
        (0.5, 1000000.0, 999000000.0, 0.0, 1.0),
        (
            1.0,
            1000000.0,
            999000000.0,
            0.5001877809842447,
            0.4998122190157553,
        ),
        (2.0, 1000000.0, 999000000.0, 1.0, 0.0),
        (10.0, 1000000.0, 999000000.0, 1.0, 0.0),
        (1000000.0, 1000000.0, 999000000.0, 1.0, 0.0),
        (1e+300, 1000000.0, 999000000.0, 1.0, 0.0),
        (1e+307, 1000000.0, 999000000.0, 1.0, 0.0),
        (0.0, 500000000.0, 500000000.0, 0.0, 1.0),
        (1e-300, 500000000.0, 500000000.0, 0.0, 1.0),
        (1e-10, 500000000.0, 500000000.0, 0.0, 1.0),
        (0.5, 500000000.0, 500000000.0, 0.0, 1.0),
        (1.0, 500000000.0, 500000000.0, 0.5, 0.5),
        (2.0, 500000000.0, 500000000.0, 1.0, 0.0),
        (10.0, 500000000.0, 500000000.0, 1.0, 0.0),
        (1000000.0, 500000000.0, 500000000.0, 1.0, 0.0),
        (1e+300, 500000000.0, 500000000.0, 1.0, 0.0),
        (1e+307, 500000000.0, 500000000.0, 1.0, 0.0),
        (0.0, 500000000.0, 4500000000.0, 0.0, 1.0),
        (1e-300, 500000000.0, 4500000000.0, 0.0, 1.0),
        (1e-10, 500000000.0, 4500000000.0, 0.0, 1.0),
        (0.5, 500000000.0, 4500000000.0, 0.0, 1.0),
        (
            1.0,
            500000000.0,
            4500000000.0,
            0.5000070923075768,
            0.4999929076924232,
        ),
        (2.0, 500000000.0, 4500000000.0, 1.0, 0.0),
        (10.0, 500000000.0, 4500000000.0, 1.0, 0.0),
        (1000000.0, 500000000.0, 4500000000.0, 1.0, 0.0),
        (1e+300, 500000000.0, 4500000000.0, 1.0, 0.0),
        (1e+307, 500000000.0, 4500000000.0, 1.0, 0.0),
        (0.0, 9999999999.0, 9999999999.0, 0.0, 1.0),
        (1e-300, 9999999999.0, 9999999999.0, 0.0, 1.0),
        (1e-10, 9999999999.0, 9999999999.0, 0.0, 1.0),
        (0.5, 9999999999.0, 9999999999.0, 0.0, 1.0),
        (1.0, 9999999999.0, 9999999999.0, 0.5, 0.5),
        (2.0, 9999999999.0, 9999999999.0, 1.0, 0.0),
        (10.0, 9999999999.0, 9999999999.0, 1.0, 0.0),
        (1000000.0, 9999999999.0, 9999999999.0, 1.0, 0.0),
        (1e+300, 9999999999.0, 9999999999.0, 1.0, 0.0),
        (1e+307, 9999999999.0, 9999999999.0, 1.0, 0.0),
        (0.0, 3.7, 7.2, 0.0, 1.0),
        (1e-300, 3.7, 7.2, 0.0, 1.0),
        (1e-10, 3.7, 7.2, 6.855029026436743e-19, 1.0),
        (0.5, 3.7, 7.2, 0.27390058075169565, 0.7260994192483043),
        (1.0, 3.7, 7.2, 0.5393832611844304, 0.4606167388155697),
        (2.0, 3.7, 7.2, 0.8023242976920558, 0.1976757023079442),
        (10.0, 3.7, 7.2, 0.9950537532436279, 0.004946246756372084),
        (1000000.0, 3.7, 7.2, 1.0, 1.05350749731921e-20),
        (1e+300, 3.7, 7.2, 1.0, 0.0),
        (1e+307, 3.7, 7.2, 1.0, 0.0),
    ];

    const CENTRAL_BAND: &[(f64, f64, f64, f64, f64)] = &[
        (1.0, 1000000.0, 1000000.0, 0.5, 0.5),
        (
            1.0020020009999997,
            1000000.0,
            1000000.0,
            0.841344625083191,
            0.158655374916809,
        ),
        (
            0.9980019990000002,
            1000000.0,
            1000000.0,
            0.15865537491675527,
            0.8413446250832447,
        ),
        (
            1.0080321244818662,
            1000000.0,
            1000000.0,
            0.9999683304979299,
            3.166950207016052e-05,
        ),
        (
            0.9920318764781482,
            1000000.0,
            1000000.0,
            3.166950207014567e-05,
            0.9999683304979299,
        ),
        (
            1.0161290241285181,
            1000000.0,
            1000000.0,
            0.9999999999999993,
            6.214799597083254e-16,
        ),
        (
            0.984126992000498,
            1000000.0,
            1000000.0,
            6.214799597083254e-16,
            0.9999999999999993,
        ),
        (
            1.0242914856824497,
            1000000.0,
            1000000.0,
            1.0,
            1.7674252088144044e-33,
        ),
        (
            0.9762845966973307,
            1000000.0,
            1000000.0,
            1.7674252088120332e-33,
            1.0,
        ),
        (1.0, 500000000.0, 500000000.0, 0.5, 0.5),
        (
            1.0000894467191894,
            500000000.0,
            500000000.0,
            0.8413447458260245,
            0.15865525417397552,
        ),
        (
            0.9999105612808106,
            500000000.0,
            500000000.0,
            0.15865525417337484,
            0.8413447458266252,
        ),
        (
            1.0003578348874929,
            500000000.0,
            500000000.0,
            0.9999683287616465,
            3.167123835341704e-05,
        ),
        (
            0.9996422931125111,
            500000000.0,
            500000000.0,
            3.167123835341704e-05,
            0.9999683287616465,
        ),
        (
            1.000715797843706,
            500000000.0,
            500000000.0,
            0.9999999999999993,
            6.220948246903931e-16,
        ),
        (
            0.9992847141563586,
            500000000.0,
            500000000.0,
            6.220948246778507e-16,
            0.9999999999999993,
        ),
        (
            1.0010738889374053,
            500000000.0,
            500000000.0,
            1.0,
            1.776463953810829e-33,
        ),
        (
            0.998927263062924,
            500000000.0,
            500000000.0,
            1.776463953810829e-33,
            1.0,
        ),
        (
            1.0,
            500000000.0,
            4500000000.0,
            0.5000070923075768,
            0.4999929076924232,
        ),
        (
            1.0000666671111007,
            500000000.0,
            4500000000.0,
            0.8413447461344663,
            0.15865525386553372,
        ),
        (
            0.9999333337777881,
            500000000.0,
            4500000000.0,
            0.1586552538654145,
            0.8413447461345855,
        ),
        (
            1.000266673777914,
            500000000.0,
            4500000000.0,
            0.9999682930564624,
            3.170694353758117e-05,
        ),
        (
            0.9997333404443082,
            500000000.0,
            4500000000.0,
            3.163556741440673e-05,
            0.9999683644325856,
        ),
        (
            1.0005333617791883,
            500000000.0,
            4500000000.0,
            0.9999999999999993,
            6.277783374421375e-16,
        ),
        (
            0.9994666951097008,
            500000000.0,
            4500000000.0,
            6.164611300301199e-16,
            0.9999999999999993,
        ),
        (
            1.0008000640049604,
            500000000.0,
            4500000000.0,
            1.0,
            1.8318638762876417e-33,
        ),
        (
            0.9992000639950404,
            500000000.0,
            4500000000.0,
            1.7227168249589655e-33,
            1.0,
        ),
        (1.0, 9999999999.0, 9999999999.0, 0.5, 0.5),
        (
            1.000020000200002,
            9999999999.0,
            9999999999.0,
            0.8413447460580296,
            0.1586552539419704,
        ),
        (
            0.999980000199998,
            9999999999.0,
            9999999999.0,
            0.1586552539446568,
            0.8413447460553432,
        ),
        (
            1.000080003200128,
            9999999999.0,
            9999999999.0,
            0.9999683287583414,
            3.167124165860526e-05,
        ),
        (
            0.999920003199872,
            9999999999.0,
            9999999999.0,
            3.167124165860526e-05,
            0.9999683287583414,
        ),
        (
            1.0001600128010242,
            9999999999.0,
            9999999999.0,
            0.9999999999999993,
            6.220959957490522e-16,
        ),
        (
            0.999840012798976,
            9999999999.0,
            9999999999.0,
            6.220959956929608e-16,
            0.9999999999999993,
        ),
        (
            1.0002400288034563,
            9999999999.0,
            9999999999.0,
            1.0,
            1.7764812043765858e-33,
        ),
        (
            0.9997600287965445,
            9999999999.0,
            9999999999.0,
            1.7764812043765858e-33,
            1.0,
        ),
    ];

    const TRANSITION_BOUNDARY: &[(f64, f64, f64, f64, f64)] = &[
        (
            1.024291485679991,
            1000000.0,
            1000000.0,
            1.0,
            1.7674252344418338e-33,
        ),
        (
            1.0242914856849081,
            1000000.0,
            1000000.0,
            1.0,
            1.767425183182233e-33,
        ),
        (
            0.9762845966996742,
            1000000.0,
            1000000.0,
            1.7674252344418338e-33,
            1.0,
        ),
        (
            0.9762845966949873,
            1000000.0,
            1000000.0,
            1.767425183184604e-33,
            1.0,
        ),
        (
            1.001073888937298,
            500000000.0,
            500000000.0,
            1.0,
            1.776463979493804e-33,
        ),
        (
            1.0010738889375128,
            500000000.0,
            500000000.0,
            1.0,
            1.776463928021286e-33,
        ),
        (
            0.9989272630630313,
            500000000.0,
            500000000.0,
            1.7764639795470882e-33,
            1.0,
        ),
        (
            0.9989272630628168,
            500000000.0,
            500000000.0,
            1.776463928021286e-33,
            1.0,
        ),
    ];

    #[test]
    fn cumulative_and_upper_tail_match_the_decimal_reference() {
        for &(x, df1, df2, expected_lower, expected_upper) in CUMULATIVE_GRID
            .iter()
            .chain(CENTRAL_BAND)
            .chain(TRANSITION_BOUNDARY)
        {
            let actual_lower = lower_tail(x, df1, df2, || Ok(())).expect("finite lower tail");
            assert_tail(
                actual_lower,
                expected_lower,
                &format!("F.DIST({x}, {df1}, {df2}, TRUE)"),
            );
            let actual_upper = upper_tail(x, df1, df2, || Ok(())).expect("finite upper tail");
            assert_tail(
                actual_upper,
                expected_upper,
                &format!("F.DIST.RT({x}, {df1}, {df2})"),
            );
        }
    }

    // Density grid. Reference: formula_reference.f_density at the exact
    // Decimal coordinates (the kernel adds exact transform logs), Decimal-110.
    // Fields: (x, df1, df2, density).
    const DENSITY_GRID: &[(f64, f64, f64, f64)] = &[
        (1e-300, 1.0, 1.0, 3.183098861837907e+149),
        (1e-10, 1.0, 1.0, 31830.98861519597),
        (0.5, 1.0, 1.0, 0.30010543871903533),
        (1.0, 1.0, 1.0, 0.15915494309189535),
        (2.0, 1.0, 1.0, 0.07502635967975883),
        (10.0, 1.0, 1.0, 0.009150765837179461),
        (1000000.0, 1.0, 1.0, 3.183095678742228e-10),
        (1e-300, 1.0, 2.0, 3.5355339059327374e+149),
        (1e-10, 1.0, 2.0, 35355.33905667572),
        (0.5, 1.0, 2.0, 0.35777087639996635),
        (1.0, 1.0, 2.0, 0.19245008972987526),
        (2.0, 1.0, 2.0, 0.08838834764831845),
        (10.0, 1.0, 2.0, 0.007607257743127307),
        (1000000.0, 1.0, 2.0, 9.999970000075e-13),
        (1e-300, 2.0, 5.0, 1.0),
        (1e-10, 2.0, 5.0, 0.99999999986),
        (0.5, 2.0, 5.0, 0.5282817877171742),
        (1.0, 2.0, 5.0, 0.3080008216940658),
        (2.0, 2.0, 5.0, 0.12780452546295093),
        (10.0, 2.0, 5.0, 0.0035777087639996636),
        (1000000.0, 2.0, 5.0, 2.4705078049956996e-20),
        (1e-300, 5.0, 30.0, 0.0),
        (1e-10, 5.0, 30.0, 8.379700461247563e-15),
        (0.5, 5.0, 30.0, 0.7300399754408147),
        (1.0, 5.0, 30.0, 0.564494449909015),
        (2.0, 5.0, 30.0, 0.15429277772740163),
        (10.0, 5.0, 30.0, 9.306270678189991e-06),
        (1000000.0, 5.0, 30.0, 3.4739996933709466e-82),
        (1e-300, 30.0, 5.0, 0.0),
        (1e-10, 30.0, 5.0, 3.4743644449145986e-126),
        (0.5, 30.0, 5.0, 0.6171711109096065),
        (1.0, 30.0, 5.0, 0.564494449909015),
        (2.0, 30.0, 5.0, 0.18250999386020367),
        (10.0, 30.0, 5.0, 0.001984281410992701),
        (1000000.0, 30.0, 5.0, 8.379676022936302e-21),
        (1e-300, 1000000.0, 1000000.0, 0.0),
        (1e-10, 1000000.0, 1000000.0, 0.0),
        (0.5, 1000000.0, 1000000.0, 0.0),
        (1.0, 1000000.0, 1000000.0, 199.47109033293754),
        (2.0, 1000000.0, 1000000.0, 0.0),
        (10.0, 1000000.0, 1000000.0, 0.0),
        (1000000.0, 1000000.0, 1000000.0, 0.0),
        (1e-300, 1000000.0, 999000000.0, 0.0),
        (1e-10, 1000000.0, 999000000.0, 0.0),
        (0.5, 1000000.0, 999000000.0, 0.0),
        (1.0, 1000000.0, 999000000.0, 281.9536621061723),
        (2.0, 1000000.0, 999000000.0, 0.0),
        (10.0, 1000000.0, 999000000.0, 0.0),
        (1000000.0, 1000000.0, 999000000.0, 0.0),
        (1e-300, 500000000.0, 500000000.0, 0.0),
        (1e-10, 500000000.0, 500000000.0, 0.0),
        (0.5, 500000000.0, 500000000.0, 0.0),
        (1.0, 500000000.0, 500000000.0, 4460.3102881517725),
        (2.0, 500000000.0, 500000000.0, 0.0),
        (10.0, 500000000.0, 500000000.0, 0.0),
        (1000000.0, 500000000.0, 500000000.0, 0.0),
        (1e-300, 500000000.0, 4500000000.0, 0.0),
        (1e-10, 500000000.0, 4500000000.0, 0.0),
        (0.5, 500000000.0, 4500000000.0, 0.0),
        (1.0, 500000000.0, 4500000000.0, 5984.134204004616),
        (2.0, 500000000.0, 4500000000.0, 0.0),
        (10.0, 500000000.0, 4500000000.0, 0.0),
        (1000000.0, 500000000.0, 4500000000.0, 0.0),
        (1e-300, 9999999999.0, 9999999999.0, 0.0),
        (1e-10, 9999999999.0, 9999999999.0, 0.0),
        (0.5, 9999999999.0, 9999999999.0, 0.0),
        (1.0, 9999999999.0, 9999999999.0, 19947.1140185756),
        (2.0, 9999999999.0, 9999999999.0, 0.0),
        (10.0, 9999999999.0, 9999999999.0, 0.0),
        (1000000.0, 9999999999.0, 9999999999.0, 0.0),
        (1e-300, 3.7, 7.2, 4.010338453498581e-255),
        (1e-10, 3.7, 7.2, 1.2681803697661737e-08),
        (0.5, 3.7, 7.2, 0.639783269709018),
        (1.0, 3.7, 7.2, 0.4184742294240687),
        (2.0, 3.7, 7.2, 0.1533894761971639),
        (10.0, 3.7, 7.2, 0.0014391026209959502),
        (1000000.0, 3.7, 7.2, 3.7926182460237685e-26),
    ];

    #[test]
    fn density_matches_the_decimal_reference_and_enforces_the_origin_contract() {
        for &(x, df1, df2, expected) in DENSITY_GRID {
            let actual = density(x, df1, df2).expect("finite density");
            assert_density(
                actual,
                expected,
                &format!("F.DIST({x}, {df1}, {df2}, FALSE)"),
            );
        }
        // x = 0 is a pole for df1 < 2, the exact limit 1 for df1 = 2, and
        // zero above.
        assert!(matches!(density(0.0, 1.0, 30.0), Err(ErrorKind::Num)));
        assert_eq!(density(0.0, 2.0, 2.0), Ok(1.0));
        assert_eq!(density(0.0, 2.0, 30.0), Ok(1.0));
        assert_eq!(density(0.0, 5.0, 2.0), Ok(0.0));
        assert_eq!(density(0.0, 5.0, 30.0), Ok(0.0));
    }

    // Quantile grid. Reference: bisection of I_z(a, b) = p at Decimal-110
    // with D.from_float(p) (the exact f64 probability); F.INV.RT(p; d1, d2) =
    // 1/F.INV(p; d2, d1).
    // Fields: (p, df1, df2, lower_quantile, upper_quantile).
    const QUANTILE_GRID: &[(f64, f64, f64, f64, f64)] = &[
        (1e-15, 1.0, 1.0, 2.46740110027234e-30, 4.05284734569351e+29),
        (1e-15, 2.0, 5.0, 1.0000000000000009e-15, 2499997.5),
        (1e-06, 5.0, 30.0, 0.0024590889317047692, 12.863164945310249),
        (0.5, 1.0, 1.0, 1.0, 1.0),
        (0.5, 2.0, 5.0, 0.7987697769322356, 0.7987697769322356),
        (0.9, 5.0, 30.0, 2.049246080685769, 0.3150514950189801),
        (0.9, 30.0, 5.0, 3.1740842872043995, 0.48798434186359674),
        (
            0.999999999999999,
            1.0,
            1.0,
            4.059333823527321e+29,
            2.4634583985287003e-30,
        ),
        (
            0.999999999999999,
            2.0,
            5.0,
            2500797.2253150404,
            9.992007221626417e-16,
        ),
        (0.5, 1000000.0, 1000000.0, 1.0, 1.0),
        (
            0.9,
            1000000.0,
            999000000.0,
            1.0018137250507129,
            0.9981871371671881,
        ),
        (
            0.999999999999999,
            500000000.0,
            500000000.0,
            1.0007105567222643,
            0.9992899478100924,
        ),
        (0.5, 9999999999.0, 9999999999.0, 1.0, 1.0),
        (
            1e-06,
            9999999999.0,
            9999999999.0,
            0.9999049360326638,
            1.0000950730053533,
        ),
    ];

    #[test]
    fn quantiles_match_the_decimal_reference() {
        for &(p, df1, df2, expected_lower, expected_upper) in QUANTILE_GRID {
            let actual_lower = f_inverse(p, df1, df2);
            assert_quantile(
                actual_lower,
                expected_lower,
                &format!("F.INV({p}, {df1}, {df2})"),
            );
            let (w, z) = beta_pair(df2 / 2.0, df1 / 2.0, p, || Ok(())).expect("solver converges");
            let actual_upper = restore_f_coordinate(z, w, df1, df2);
            assert_quantile(
                actual_upper,
                expected_upper,
                &format!("F.INV.RT({p}, {df1}, {df2})"),
            );
        }
    }

    #[test]
    fn quantiles_round_trip_through_the_cdf() {
        for &(p, df1, df2, _, _) in QUANTILE_GRID {
            let quantile = f_inverse(p, df1, df2);
            let cdf = lower_tail(quantile, df1, df2, || Ok(())).expect("finite CDF");
            let diff = (cdf - p).abs();
            let limit = 1e-15 + 1e-9 * p;
            assert!(
                diff <= limit,
                "F.DIST(F.INV({p}, {df1}, {df2})) = {cdf} (diff {diff:e} > {limit:e})",
            );
        }
    }

    // F.TEST grid. Reference: min(1, 2·I_w(df2/2, df1/2)) at the ratio of
    // unbiased sample variances. Fields: (left, right, p).
    const F_TEST_GRID: &[(&[f64], &[f64], f64)] = &[
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[10.0, 11.0, 12.0, 13.0, 14.0],
            1.0,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            &[1.0, 2.0, 3.0, 4.0, 100.0],
            1.0324313445360633e-05,
        ),
        (
            &[1.0, 2.0, 3.0, 4.0, 100.0],
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            1.0324313445360633e-05,
        ),
        (&[1.0, 2.0], &[3.0, 4.0], 1.0),
    ];

    #[test]
    fn f_test_p_values_match_the_decimal_reference() {
        for &(left, right, expected_p) in F_TEST_GRID {
            let (ratio, df1, df2) = variance_ratio(
                sample_variance(left),
                sample_variance(right),
                left.len() as u64,
                right.len() as u64,
            );
            let tail = upper_tail(ratio, df1, df2, || Ok(())).expect("finite tail");
            let actual_p = (2.0 * tail).min(1.0);
            let label = format!("F.TEST({left:?}, {right:?})");
            assert_tail(actual_p, expected_p, &label);
            // The larger variance is always the numerator, so swapping the
            // samples cannot change the p-value.
            let (swapped_ratio, swapped_df1, swapped_df2) = variance_ratio(
                sample_variance(right),
                sample_variance(left),
                right.len() as u64,
                left.len() as u64,
            );
            assert_eq!((swapped_ratio, swapped_df1, swapped_df2), (ratio, df1, df2));
        }
    }

    // The expected logs are Decimal-110 reference values (e.g. −ln 2 for the
    // symmetric half-coordinate) that must stay byte-exact; the literals are
    // not approximations to be replaced by std constants.
    #[allow(clippy::approx_constant)]
    #[test]
    fn coordinates_and_log_coordinates_survive_extreme_x() {
        // The direct transform never overflows when d1·x is finite; at
        // x = 1e300 the coordinate rounds to the endpoint (z = 1.0, w = 0.0),
        // and coordinate_logs hands the kernels the exact complement logs
        // (log_z = ln(1 − w) = −1e-300), so tails never lose the subnormal w.
        let (z, w) = f_coordinates(1e300, 1.0, 1.0);
        assert_eq!(z, 1.0);
        assert_eq!(w, 0.0);
        let (log_z, log_w) = f_log_coordinates(1e300, 1.0, 1.0);
        assert_within(log_z, -1e-300, 1e-315, 1e-12, "log_z at x = 1e300");
        assert_within(
            log_w,
            -690.775_527_898_213_7,
            1e-12,
            1e-15,
            "log_w at x = 1e300",
        );

        let (z, w) = f_coordinates(1e-300, 1.0, 1.0);
        assert_eq!(z, 1e-300);
        assert_eq!(w, 1.0);
        let (log_z, log_w) = f_log_coordinates(1e-300, 1.0, 1.0);
        assert_within(
            log_z,
            -690.775_527_898_213_7,
            1e-12,
            1e-15,
            "log_z at x = 1e-300",
        );
        assert_within(log_w, -1e-300, 1e-315, 1e-12, "log_w at x = 1e-300");

        // Endpoint-rounded coordinates from interior x hand their exact logs
        // to the kernels; true endpoints do not.
        let (z, w) = f_coordinates(1e300, 1.0, 1.0);
        let (log_z, log_w) = f_log_coordinates(1e300, 1.0, 1.0);
        assert_eq!(
            coordinate_logs(1e300, z, w, log_z, log_w),
            (Some(log_z), Some(log_w)),
        );
        assert_eq!(
            coordinate_logs(0.0, 0.0, 1.0, f64::NEG_INFINITY, 0.0),
            (None, None)
        );
        assert_eq!(
            coordinate_logs(
                1.0,
                0.5,
                0.5,
                -0.693_147_180_559_945_3,
                -0.693_147_180_559_945_3
            ),
            (None, None)
        );
    }

    #[test]
    fn restore_scales_before_dividing_so_subnormal_coordinates_survive() {
        // x = (df2/df1)·z/w with z ≈ 1 and w subnormal: the intermediate z/w
        // alone would overflow (1/1e-310 = inf), so restore must apply the df
        // ratio first to keep the result finite. Exact equality with 1e300 is
        // unsatisfiable: the subnormal literal 1e-310 carries ~2.3e-14
        // relative rounding (only ~45 significant bits), which the quotient
        // inherits, so assert the scaling order keeps the result ≈ 1e300.
        let restored = restore_f_coordinate(1.0, 1e-310, 1e10, 1.0);
        assert!(restored.is_finite());
        assert_within(
            restored,
            1e300,
            1e286,
            1e-13,
            "restore scales before dividing",
        );
        // A real solve near the lower endpoint stays exact (reference at the
        // exact f64 probability 1e-15, Decimal-110 bisection).
        let (z, w) = beta_pair(1.0, 2.5, 1e-15, || Ok(())).expect("solver converges");
        let quantile = restore_f_coordinate(z, w, 2.0, 5.0);
        assert_quantile(quantile, 1.0000000000000009e-15, "F.INV(1e-15, 2, 5)");
    }
}
