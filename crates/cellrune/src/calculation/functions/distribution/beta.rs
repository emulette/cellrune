use super::super::super::ast::Expr;
use super::super::super::coerce::to_logical;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::array_common::poll_cancellation;
use super::super::special_functions::{
    DomainPolicy, invert_monotone_cdf, ln_beta, regularized_incomplete_beta,
};
use super::super::util::required_number;
use super::{finite, quantile_solver_error};

/// BETA.DIST(x, alpha, beta, cumulative, [A], [B]); the contract materializes
/// the documented defaults A = 0, B = 1 before dispatch.
pub(super) fn beta_distribution(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 6 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match required_number(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let alpha = match positive_parameter(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let beta = match positive_parameter(engine, context, &args[2]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let cumulative = match to_logical(&engine.eval_scalar(context, &args[3])) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let interval = match support_interval(engine, context, &args[4], &args[5])
        .and_then(|(lower, upper)| unit_interval(x, lower, upper))
    {
        Ok(interval) => interval,
        Err(kind) => return Value::Error(kind),
    };
    if cumulative {
        cumulative_probability(engine, context, alpha, beta, interval.position)
    } else {
        density(alpha, beta, interval)
    }
}

/// BETADIST(x, alpha, beta, [A], [B]) is the CDF-only legacy signature; it
/// adapts the arguments and reuses the canonical kernel path bit for bit.
pub(super) fn beta_distribution_legacy(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 5 {
        return Value::Error(ErrorKind::Value);
    }
    let x = match required_number(engine, context, &args[0]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let alpha = match positive_parameter(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let beta = match positive_parameter(engine, context, &args[2]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let interval = match support_interval(engine, context, &args[3], &args[4])
        .and_then(|(lower, upper)| unit_interval(x, lower, upper))
    {
        Ok(interval) => interval,
        Err(kind) => return Value::Error(kind),
    };
    cumulative_probability(engine, context, alpha, beta, interval.position)
}

/// BETA.INV(probability, alpha, beta, [A], [B]); p = 0 is documented #NUM!
/// (unlike GAMMA.INV) while p = 1 is the finite upper bound B.
pub(super) fn beta_inverse(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 5 {
        return Value::Error(ErrorKind::Value);
    }
    let probability = match required_number(engine, context, &args[0]) {
        Ok(value) if value > 0.0 && value <= 1.0 => value,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let alpha = match positive_parameter(engine, context, &args[1]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let beta = match positive_parameter(engine, context, &args[2]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let (lower, upper) = match support_interval(engine, context, &args[3], &args[4]) {
        Ok(bounds) => bounds,
        Err(kind) => return Value::Error(kind),
    };
    if probability == 1.0 {
        // The support is bounded, so the p = 1 quantile is exactly B.
        return finite(upper);
    }
    let solved = invert_monotone_cdf(
        |position| {
            regularized_incomplete_beta(alpha, beta, position, || {
                poll_cancellation(context)?;
                engine.charge_function_iterations(context, 1)
            })
        },
        probability,
        DomainPolicy::FiniteInterval {
            low: 0.0,
            high: 1.0,
        },
        || {
            poll_cancellation(context)?;
            engine.charge_function_iterations(context, 1)
        },
    );
    match solved {
        Ok(position) => finite(interpolate_support(lower, upper, position)),
        Err(kind) => Value::Error(quantile_solver_error(kind)),
    }
}

fn positive_parameter(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    argument: &Expr,
) -> Result<f64, ErrorKind> {
    match required_number(engine, context, argument)? {
        value if value > 0.0 => Ok(value),
        _ => Err(ErrorKind::Num),
    }
}

/// The one home of the beta-family interval rule, shared by all four names:
/// a support interval is valid only when A < B, so A = B and the reversed
/// A > B are both #NUM! before any kernel or solver work runs.
fn support_interval(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    lower_argument: &Expr,
    upper_argument: &Expr,
) -> Result<(f64, f64), ErrorKind> {
    let lower = required_number(engine, context, lower_argument)?;
    let upper = required_number(engine, context, upper_argument)?;
    if lower >= upper {
        return Err(ErrorKind::Num);
    }
    Ok((lower, upper))
}

/// x mapped onto the unit interval of a validated support [lower, upper].
/// Endpoint identity is retained separately from rounded coordinates so an
/// interior ratio that underflows cannot be mistaken for a density endpoint.
#[derive(Debug, Clone, Copy)]
struct UnitInterval {
    position: f64,
    log_position: f64,
    log_complement: f64,
    log_width: f64,
    width: f64,
    at_lower: bool,
    at_upper: bool,
}

fn unit_interval(x: f64, lower: f64, upper: f64) -> Result<UnitInterval, ErrorKind> {
    if x < lower || x > upper {
        return Err(ErrorKind::Num);
    }
    let width = upper - lower;
    let at_lower = x == lower;
    let at_upper = x == upper;
    if width.is_finite() {
        let lower_distance = x - lower;
        let upper_distance = upper - x;
        let position = lower_distance / width;
        let complement = upper_distance / width;
        return Ok(UnitInterval {
            position,
            // Preserve the established finite-width evaluation order bit for
            // bit; only overflowing widths take the scaled log-ratio path.
            log_position: position.ln(),
            log_complement: complement.ln(),
            log_width: width.ln(),
            width,
            at_lower,
            at_upper,
        });
    }

    // A finite ordered interval can have a non-finite width only when it
    // crosses zero and the two magnitudes add past f64::MAX. Normalize all
    // three coordinates before subtracting, keeping the ratios and log-width
    // finite without changing the accepted public domain.
    let scale = lower.abs().max(upper.abs());
    let scaled_lower = lower / scale;
    let scaled_upper = upper / scale;
    let scaled_x = x / scale;
    let scaled_width = scaled_upper - scaled_lower;
    let scaled_lower_distance = scaled_x - scaled_lower;
    let scaled_upper_distance = scaled_upper - scaled_x;
    Ok(UnitInterval {
        position: scaled_lower_distance / scaled_width,
        log_position: scaled_lower_distance.ln() - scaled_width.ln(),
        log_complement: scaled_upper_distance.ln() - scaled_width.ln(),
        log_width: scale.ln() + scaled_width.ln(),
        width,
        at_lower,
        at_upper,
    })
}

/// Maps a unit-interval quantile back onto finite support without ever
/// materializing an overflowing `upper - lower` width.
fn interpolate_support(lower: f64, upper: f64, position: f64) -> f64 {
    if position == 0.0 {
        return lower;
    }
    if position == 1.0 {
        return upper;
    }
    let width = upper - lower;
    if width.is_finite() {
        lower + width * position
    } else {
        lower * (1.0 - position) + upper * position
    }
}

fn cumulative_probability(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    alpha: f64,
    beta: f64,
    position: f64,
) -> Value {
    match regularized_incomplete_beta(alpha, beta, position, || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    }) {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

fn density(alpha: f64, beta: f64, interval: UnitInterval) -> Value {
    if interval.at_lower {
        // The endpoint contract mirrors GAMMA.DIST at the origin: a pole for
        // alpha < 1, the exact limit for alpha = 1, and zero above.
        return if alpha < 1.0 {
            Value::Error(ErrorKind::Num)
        } else if alpha == 1.0 {
            if interval.width.is_finite() {
                finite(beta / interval.width)
            } else {
                finite((beta.ln() - interval.log_width).exp())
            }
        } else {
            Value::Number(0.0)
        };
    }
    if interval.at_upper {
        return if beta < 1.0 {
            Value::Error(ErrorKind::Num)
        } else if beta == 1.0 {
            if interval.width.is_finite() {
                finite(alpha / interval.width)
            } else {
                finite((alpha.ln() - interval.log_width).exp())
            }
        } else {
            Value::Number(0.0)
        };
    }
    let ln_beta_factor = match ln_beta(alpha, beta) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    // Excel rescales the unit-interval density by the interval width:
    // BETA.DIST(2,8,10,FALSE,1,3) = 1.4837646 = pdf(0.5; 8, 10) / 2 in the
    // documented example, i.e. the 1/(B − A) Jacobian is applied.
    let log_density = (alpha - 1.0) * interval.log_position
        + (beta - 1.0) * interval.log_complement
        - ln_beta_factor
        - interval.log_width;
    finite(log_density.exp())
}

#[cfg(test)]
mod tests {
    use super::{density, interpolate_support, unit_interval};
    use crate::calculation::value::Value;

    #[test]
    fn finite_supports_wider_than_f64_keep_their_unit_coordinates_and_jacobian() {
        let interval = unit_interval(0.0, -1e308, 1e308).expect("ordered finite support");
        assert_eq!(interval.position, 0.5);
        assert_eq!(interpolate_support(-1e308, 1e308, 0.5), 0.0);

        let Value::Number(actual_density) = density(1.0, 1.0, interval) else {
            panic!("uniform density must remain representable");
        };
        let expected_density = 5e-309;
        assert!(
            (actual_density - expected_density).abs() <= 1e-12 * expected_density,
            "uniform wide-support density: {actual_density} vs {expected_density}",
        );
    }
}
