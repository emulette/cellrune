//! WEIBULL.DIST (also spelled WEIBULL): the closed-form Weibull CDF 1 − exp(−(x/β)^α) and
//! density (α/β)·(x/β)^(α−1)·exp(−(x/β)^α).

use super::super::super::ast::Expr;
use super::super::super::coerce::to_logical;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::util::required_number;
use super::f::nonnegative_x;
use super::finite;

pub(super) fn weibull_distribution(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Value {
    let result = (|| {
        let [x, alpha, beta, cumulative] = args else {
            return Err(ErrorKind::Value);
        };
        let x = nonnegative_x(engine, context, x)?;
        let alpha = positive(required_number(engine, context, alpha)?)?;
        let beta = positive(required_number(engine, context, beta)?)?;
        let cumulative = to_logical(&engine.eval_scalar(context, cumulative))?;
        let log_ratio = (x / beta).ln();
        // (x/β)^α through the logarithm, which is how Excel forms the power.
        let power = (alpha * log_ratio).exp();
        if cumulative {
            // −expm1 keeps a CDF far below machine epsilon instead of rounding 1 − exp to zero.
            return Ok(-(-power).exp_m1());
        }
        if x == 0.0 {
            // At the origin the density is a pole for α < 1, 1/β for α = 1, and zero for α > 1.
            return if alpha < 1.0 {
                Err(ErrorKind::Num)
            } else if alpha == 1.0 {
                Ok(1.0 / beta)
            } else {
                Ok(0.0)
            };
        }
        // Log space keeps a large power of x/β from overflowing before exp(−power) shrinks it.
        Ok((alpha.ln() - beta.ln() + (alpha - 1.0) * log_ratio - power).exp())
    })();
    result.map_or_else(Value::Error, finite)
}

fn positive(value: f64) -> Result<f64, ErrorKind> {
    if value > 0.0 {
        Ok(value)
    } else {
        Err(ErrorKind::Num)
    }
}
