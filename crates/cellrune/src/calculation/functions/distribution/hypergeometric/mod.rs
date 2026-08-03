//! Excel contract layer for HYPGEOM.DIST and its legacy spelling: argument
//! coercion, truncation and the documented domain rules. The numeric kernels
//! live in [`mass`], mirroring how the gamma family keeps its Excel entry
//! points apart from the pure special-function code.

use super::super::super::ast::Expr;
use super::super::super::coerce::to_logical;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::array_common::poll_cancellation;
use super::super::util::required_number;
use super::finite;
use mass::{cumulative_probability, probability_mass};

mod mass;

pub(super) fn hypergeometric_distribution(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 5 {
        return Value::Error(ErrorKind::Value);
    }
    let parameters = match parameters(engine, context, [&args[0], &args[1], &args[2], &args[3]]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    let cumulative = match to_logical(&engine.eval_scalar(context, &args[4])) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    distribution(engine, context, parameters, cumulative)
}

/// HYPGEOMDIST is the pre-2010 spelling. It carries no cumulative argument, so
/// it maps onto the same kernel with the mass branch selected; the results are
/// bit-identical to HYPGEOM.DIST(..., FALSE) by construction.
pub(super) fn hypergeometric_distribution_legacy(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    if args.len() != 4 {
        return Value::Error(ErrorKind::Value);
    }
    let parameters = match parameters(engine, context, [&args[0], &args[1], &args[2], &args[3]]) {
        Ok(value) => value,
        Err(kind) => return Value::Error(kind),
    };
    distribution(engine, context, parameters, false)
}

/// Hypergeometric parameters that already satisfy every documented domain rule,
/// so the log-space mass kernels cannot leave their domain.
#[derive(Debug, Clone, Copy)]
struct Parameters {
    sample_successes: f64,
    sample: f64,
    population_successes: f64,
    population: f64,
}

fn distribution(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    parameters: Parameters,
    cumulative: bool,
) -> Value {
    let mut on_iteration = || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    };
    let probability = if cumulative {
        cumulative_probability(parameters, on_iteration)
    } else {
        probability_mass(parameters, &mut on_iteration)
    };
    match probability {
        Ok(value) => finite(value),
        Err(kind) => Value::Error(kind),
    }
}

fn parameters(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: [&Expr; 4],
) -> Result<Parameters, ErrorKind> {
    let mut coerced = [0.0_f64; 4];
    for (slot, expr) in coerced.iter_mut().zip(args) {
        *slot = required_number(engine, context, expr)?;
    }
    validated(coerced)
}

/// Truncation and the #NUM! conditions follow Microsoft's HYPGEOM.DIST page
/// (<https://support.microsoft.com/en-us/office/hypgeom-dist-function-6dbd547f-1d12-4b1f-8ae5-b0d9e3d22fbf>);
/// the legacy HYPGEOMDIST page documents the identical rules.
fn validated(arguments: [f64; 4]) -> Result<Parameters, ErrorKind> {
    if !arguments.iter().all(|argument| argument.is_finite()) {
        return Err(ErrorKind::Num);
    }
    // All four arguments are truncated to integers before any domain test.
    let [sample_successes, sample, population_successes, population] = arguments.map(f64::trunc);
    if population <= 0.0
        || sample <= 0.0
        || sample > population
        || population_successes <= 0.0
        || population_successes > population
    {
        return Err(ErrorKind::Num);
    }
    let parameters = Parameters {
        sample_successes,
        sample,
        population_successes,
        population,
    };
    // The support floor is never negative, so it subsumes the documented
    // sample_s < 0 rule.
    if sample_successes < support_floor(parameters)
        || sample_successes > sample.min(population_successes)
    {
        return Err(ErrorKind::Num);
    }
    Ok(parameters)
}

/// Smallest drawable success count: once the population's failures are
/// exhausted every further draw has to be a success.
fn support_floor(parameters: Parameters) -> f64 {
    (parameters.sample - parameters.population + parameters.population_successes).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::validated;
    use crate::calculation::value::ErrorKind;

    #[test]
    fn documented_domain_violations_are_rejected() {
        for arguments in [
            // sample_s below zero and above min(number_sample, population_s).
            [-1.0, 4.0, 8.0, 20.0],
            [5.0, 4.0, 8.0, 20.0],
            [9.0, 20.0, 8.0, 20.0],
            // sample_s below max(0, number_sample − number_pop + population_s).
            [3.0, 6.0, 8.0, 10.0],
            // number_sample outside (0, number_pop].
            [1.0, 0.0, 8.0, 20.0],
            [1.0, -4.0, 8.0, 20.0],
            [1.0, 21.0, 8.0, 20.0],
            // population_s outside (0, number_pop].
            [1.0, 4.0, 0.0, 20.0],
            [1.0, 4.0, -8.0, 20.0],
            [1.0, 4.0, 21.0, 20.0],
            // number_pop at or below zero.
            [1.0, 4.0, 8.0, 0.0],
            [1.0, 4.0, 8.0, -20.0],
            // Non-finite arguments can never name a population.
            [f64::NAN, 4.0, 8.0, 20.0],
            [1.0, 4.0, 8.0, f64::INFINITY],
        ] {
            assert_eq!(
                validated(arguments).err(),
                Some(ErrorKind::Num),
                "{arguments:?}",
            );
        }
    }

    #[test]
    fn arguments_are_truncated_toward_zero_before_the_domain_test() {
        let truncated = validated([1.9, 4.7, 8.2, 20.9]).expect("documented domain");
        assert_eq!(truncated.sample_successes, 1.0);
        assert_eq!(truncated.sample, 4.0);
        assert_eq!(truncated.population_successes, 8.0);
        assert_eq!(truncated.population, 20.0);
        // Truncation runs first, so a fractional sample_s above −1 reaches the
        // support floor at zero instead of failing the domain test.
        let negative = validated([-0.5, 4.0, 8.0, 20.0]).expect("documented domain");
        assert_eq!(negative.sample_successes, 0.0);
    }
}
