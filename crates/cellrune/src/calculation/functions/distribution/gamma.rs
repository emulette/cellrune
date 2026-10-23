use super::super::super::ast::Expr;
use super::super::super::coerce::to_logical;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::array_common::poll_cancellation;
use super::super::special_functions::{
    DomainPolicy, LARGE_SHAPE, invert_monotone_cdf, ln_gamma, regularized_gamma_large_shape,
    regularized_gamma_p, regularized_gamma_p_from_log, regularized_gamma_q, signed_gamma,
};
use super::super::util::required_number;
use super::f::nonnegative_x;
use super::{finite, quantile_solver_error};

pub(super) fn gamma(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [argument] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let x = match required_number(engine, context, argument) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    match signed_gamma(x) {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn log_gamma_precise(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    let [argument] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let x = match required_number(engine, context, argument) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    match ln_gamma(x) {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

pub(super) fn gamma_distribution(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 4 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match required_number(engine, context, &args[0]) {
        Ok(value) if value >= 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let alpha = match required_number(engine, context, &args[1]) {
        Ok(value) if value > 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let beta = match required_number(engine, context, &args[2]) {
        Ok(value) if value > 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let cumulative = match to_logical(&engine.eval_scalar(context, &args[3])) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    if cumulative {
        if x == 0.0 {
            return Value::Number(0.0);
        }
        let log_scaled = x.ln() - beta.ln();
        match regularized_gamma_p_from_log(alpha, log_scaled, || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        }) {
            Ok(value) => finite(value),
            Err(kind) => Value::Error(kind),
        }
    } else {
        density(x, alpha, beta)
    }
}

/// CHISQ.DIST(x, df, cumulative): the chi-square distribution is the gamma distribution with
/// shape df/2 and scale 2.
pub(super) fn chi_square_distribution(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    let [x, df, cumulative] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let x = match nonnegative_x(engine, context, x) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let df = match chi_square_degrees_of_freedom(engine, context, df) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let cumulative = match to_logical(&engine.eval_scalar(context, cumulative)) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    if !cumulative {
        return density(x, df / 2.0, 2.0);
    }
    match regularized_gamma_p(df / 2.0, x / 2.0, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    }) {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

/// CHISQ.DIST.RT(x, df), also spelled CHIDIST: the right tail is the upper regularized gamma
/// Q(df/2, x/2) evaluated directly, so small tails are not lost to 1 − P.
pub(super) fn chi_square_distribution_rt(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    let [x, df] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let result = nonnegative_x(engine, context, x).and_then(|x| {
        let df = chi_square_degrees_of_freedom(engine, context, df)?;
        regularized_gamma_q(df / 2.0, x / 2.0, || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        })
    });
    match result {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

/// CHISQ.INV(probability, df): the left-tail chi-square quantile, twice the gamma quantile with
/// shape df/2. A zero probability is the origin; a probability of one is the infinite upper end.
/// From shape 10⁴ the lower tail comes from the large-shape kernel, which keeps every digit up to
/// Excel's 10¹⁰ degrees of freedom.
pub(super) fn chi_square_inverse(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    let [probability, df] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let result = chi_square_probability(engine, context, probability).and_then(|probability| {
        let shape = chi_square_degrees_of_freedom(engine, context, df)? / 2.0;
        if probability == 0.0 {
            return Ok(0.0);
        }
        if probability == 1.0 {
            return Err(ErrorKind::Num);
        }
        let charge = || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        };
        invert_monotone_cdf(
            |x| {
                if shape >= LARGE_SHAPE {
                    regularized_gamma_large_shape(shape, x, charge).map(|(lower, _)| lower)
                } else {
                    regularized_gamma_p(shape, x, charge)
                }
            },
            probability,
            DomainPolicy::PositiveHalfLine {
                initial_guess: shape,
            },
            || {
                poll_cancellation(context)?;
                engine.charge_function_iterations(context, 1)
            },
        )
        .map(|half| 2.0 * half)
        .map_err(quantile_solver_error)
    });
    match result {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

/// CHISQ.INV.RT(probability, df), also spelled CHIINV: solves Q(df/2, x/2) = probability. The
/// right tail falls as x grows, so the solver searches the rising Q(df/2, 1/t) over t = 1/(x/2),
/// which keeps small tails at full relative precision instead of inverting 1 − P.
pub(super) fn chi_square_inverse_rt(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    let [probability, df] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let result = chi_square_probability(engine, context, probability).and_then(|probability| {
        let shape = chi_square_degrees_of_freedom(engine, context, df)? / 2.0;
        if probability == 1.0 {
            return Ok(0.0);
        }
        if probability == 0.0 {
            return Err(ErrorKind::Num);
        }
        let charge = || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        };
        invert_monotone_cdf(
            |reciprocal| {
                if shape >= LARGE_SHAPE {
                    regularized_gamma_large_shape(shape, reciprocal.recip(), charge)
                        .map(|(_, upper)| upper)
                } else {
                    regularized_gamma_q(shape, reciprocal.recip(), charge)
                }
            },
            probability,
            DomainPolicy::PositiveHalfLine {
                initial_guess: shape.recip(),
            },
            || {
                poll_cancellation(context)?;
                engine.charge_function_iterations(context, 1)
            },
        )
        .map(|reciprocal| 2.0 / reciprocal)
        .map_err(quantile_solver_error)
    });
    match result {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

/// CHISQ.TEST(actual_range, expected_range), also spelled CHITEST: the right-tail probability of
/// χ² = Σ (A − E)² / E. Ranges with different numbers of values, or a single value, return `#N/A`.
///
/// As Excel 16.114 for Mac returns it, the ranges only need the same number of values, and the
/// degrees of freedom come from the shape of `actual_range`, text cells included: (r − 1)(c − 1),
/// or one less than the count for a single row or column. A pair with text drops out of the sum,
/// an expected 0 returns `#DIV/0!`, and a negative expected value `#NUM!`. Blanks, logical values,
/// and errors in the ranges, text beside an expected value that is not positive, and expected
/// values that would give both errors were not measured, so they are unsupported.
pub(in crate::calculation::functions) fn chi_square_test(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    let [actual, expected] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let result = engine.eval_array(context, actual).and_then(|actual| {
        let expected = engine.eval_array(context, expected)?;
        if actual.data.len() != expected.data.len() || actual.data.len() == 1 {
            return Err(ErrorKind::NA);
        }
        engine.charge_function_iterations(
            context,
            u64::try_from(actual.data.len()).map_err(|_| ErrorKind::Num)?,
        )?;
        let mut statistic = 0.0;
        let mut error = None;
        let mut unsupported = false;
        for pair in actual.data.iter().zip(&expected.data) {
            poll_cancellation(context)?;
            match pair {
                (Value::Error(kind), _) | (_, Value::Error(kind)) if kind.is_engine_issue() => {
                    return Err(*kind);
                }
                (Value::Number(actual), Value::Number(expected)) if *expected > 0.0 => {
                    statistic += (actual - expected) * (actual - expected) / expected;
                }
                (Value::Number(_), Value::Number(expected)) => {
                    let kind = if *expected == 0.0 {
                        ErrorKind::Div0
                    } else {
                        ErrorKind::Num
                    };
                    unsupported |= error.is_some_and(|first| first != kind);
                    error = Some(kind);
                }
                (Value::Text(_), Value::Number(expected)) if *expected > 0.0 => {}
                (Value::Text(_) | Value::Number(_), Value::Text(_)) => {}
                _ => unsupported = true,
            }
        }
        if unsupported {
            return Err(ErrorKind::Unsupported);
        }
        if let Some(kind) = error {
            return Err(kind);
        }
        let rows = f64::from(actual.rows);
        let cols = f64::from(actual.cols);
        let df = if actual.rows > 1 && actual.cols > 1 {
            (rows - 1.0) * (cols - 1.0)
        } else {
            rows.max(cols) - 1.0
        };
        if !statistic.is_finite() {
            return Err(ErrorKind::Num);
        }
        regularized_gamma_q(df / 2.0, statistic / 2.0, || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        })
    });
    match result {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

/// The chi-square quantiles accept probabilities from 0 through 1.
fn chi_square_probability(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    argument: &Expr,
) -> Result<f64, ErrorKind> {
    match required_number(engine, context, argument)? {
        value if (0.0..=1.0).contains(&value) => Ok(value),
        _ => Err(ErrorKind::Num),
    }
}

/// The chi-square functions truncate their degrees of freedom and accept 1 through 10^10.
fn chi_square_degrees_of_freedom(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    argument: &Expr,
) -> Result<f64, ErrorKind> {
    let df = required_number(engine, context, argument)?.trunc();
    if (1.0..=1e10).contains(&df) {
        Ok(df)
    } else {
        Err(ErrorKind::Num)
    }
}

fn density(x: f64, alpha: f64, beta: f64) -> Value {
    if x == 0.0 {
        // Microsoft's GAMMA.DIST contract at the origin: the density is a pole
        // for alpha < 1, 1/beta for alpha = 1, and zero for alpha > 1.
        return if alpha < 1.0 {
            Value::Error(ErrorKind::Num)
        } else if alpha == 1.0 {
            finite(1.0 / beta)
        } else {
            Value::Number(0.0)
        };
    }
    let log_scaled = x.ln() - beta.ln();
    if log_scaled > f64::MAX.ln() {
        // A scaled point beyond the largest finite double is also beyond every
        // finite shape parameter; its density is below the representable tail.
        return Value::Number(0.0);
    }
    let scaled = log_scaled.exp();
    let ln_gamma_alpha = match ln_gamma(alpha) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let log_density = (alpha - 1.0) * log_scaled - scaled - ln_gamma_alpha - beta.ln();
    finite(log_density.exp())
}

pub(super) fn gamma_inverse(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let probability = match required_number(engine, context, &args[0]) {
        Ok(value) if (0.0..=1.0).contains(&value) => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let alpha = match required_number(engine, context, &args[1]) {
        Ok(value) if value > 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let beta = match required_number(engine, context, &args[2]) {
        Ok(value) if value > 0.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    if probability == 0.0 {
        return Value::Number(0.0);
    }
    if probability == 1.0 {
        // The upper quantile is infinite and cannot materialize as a number.
        return Value::Error(ErrorKind::Num);
    }
    let solved = invert_monotone_cdf(
        |x| {
            regularized_gamma_p(alpha, x, || {
                poll_cancellation(context)?;
                engine.charge_function_iterations(context, 1)
            })
        },
        probability,
        DomainPolicy::PositiveHalfLine {
            initial_guess: alpha,
        },
        || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        },
    );
    match solved {
        Ok(value) => finite(beta * value),
        Err(kind) => Value::Error(quantile_solver_error(kind)),
    }
}

#[cfg(test)]
mod tests {
    use super::density;
    use crate::calculation::value::Value;

    #[test]
    fn positive_x_is_not_mistaken_for_the_origin_when_scale_division_underflows() {
        let Value::Number(actual) = density(1e-308, 0.5, 1e308) else {
            panic!("finite density must stay numeric");
        };
        let expected = 0.564_189_583_547_784_1;
        assert!(
            (actual - expected).abs() <= 1e-12 * expected,
            "underflow-scale density: {actual} vs {expected}",
        );
    }
}
