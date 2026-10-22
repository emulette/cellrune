//! `RAND`, `RANDBETWEEN`, and `RANDARRAY`, drawn from the calculation's random seed.
//!
//! Without a seed each returns the engine's unsupported marker, which the calculation reports as a
//! missing volatile input. Arguments are validated first, so a malformed call still answers with
//! its Excel error.

use super::super::ast::Expr;
use super::super::coerce::to_logical;
use super::super::eval::{Engine, EvalContext};
use super::super::limits::CalculationLimitKind;
use super::super::runtime::Array;
use super::super::value::{ErrorKind, Value};
use super::util::required_number;

pub(super) fn rand(engine: &Engine<'_>, context: EvalContext<'_>) -> Value {
    engine
        .random_draws(context)
        .map_or(Value::Error(ErrorKind::Unsupported), |draws| {
            Value::Number(draws.uniform(0))
        })
}

pub(super) fn randbetween(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [bottom, top] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let bottom = match required_number(engine, context, bottom) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let top = match required_number(engine, context, top) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    if bottom > top {
        return Value::Error(ErrorKind::Num);
    }
    // Excel draws from the integers between the bounds and answers the bottom rounded up when no
    // integer lies between them, as in `RANDBETWEEN(1.5,1.6)`.
    let lowest = bottom.ceil();
    let Some(integers) = IntegerRange::new(lowest, top.floor().max(lowest)) else {
        return Value::Error(ErrorKind::Num);
    };
    engine
        .random_draws(context)
        .map_or(Value::Error(ErrorKind::Unsupported), |draws| {
            Value::Number(integers.select(draws.uniform(0)))
        })
}

pub(super) fn randarray(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
) -> Result<Array, ErrorKind> {
    let [rows, columns, minimum, maximum, whole_number] = args else {
        return Err(ErrorKind::Value);
    };
    let rows = required_number(engine, context, rows)?;
    let columns = required_number(engine, context, columns)?;
    let minimum = required_number(engine, context, minimum)?;
    let maximum = required_number(engine, context, maximum)?;
    let whole_number = to_logical(&engine.eval_scalar(context, whole_number))?;
    // Excel answers `#VALUE!` for a negative dimension, a minimum above the maximum, or a
    // fractional bound with `whole_number`, and `#CALC!` for a dimension that truncates to zero.
    if rows < 0.0
        || columns < 0.0
        || minimum > maximum
        || (whole_number && (minimum.fract() != 0.0 || maximum.fract() != 0.0))
    {
        return Err(ErrorKind::Value);
    }
    let (rows, columns) = (rows.trunc(), columns.trunc());
    if rows == 0.0 || columns == 0.0 {
        return Err(ErrorKind::Calc);
    }
    let too_large = |_| ErrorKind::ResourceLimit(CalculationLimitKind::ArrayCells);
    let rows = u32::try_from(rows as u64).map_err(too_large)?;
    let columns = u32::try_from(columns as u64).map_err(too_large)?;
    let scale = if whole_number {
        Scale::Integers(IntegerRange::new(minimum, maximum).ok_or(ErrorKind::Num)?)
    } else {
        let width = maximum - minimum;
        if !width.is_finite() {
            return Err(ErrorKind::Num);
        }
        Scale::Interval { minimum, width }
    };
    let cells = u64::from(rows) * u64::from(columns);
    engine.ensure_array_cells(cells)?;
    engine.charge_function_iterations(context, cells)?;
    let draws = engine.random_draws(context).ok_or(ErrorKind::Unsupported)?;
    let data = (0..cells)
        .map(|element| {
            let uniform = draws.uniform(element);
            Value::Number(match scale {
                Scale::Integers(integers) => integers.select(uniform),
                Scale::Interval { minimum, width } => minimum + uniform * width,
            })
        })
        .collect();
    Ok(Array {
        rows,
        cols: columns,
        data,
    })
}

#[derive(Clone, Copy)]
enum Scale {
    Integers(IntegerRange),
    Interval { minimum: f64, width: f64 },
}

/// The integers from `bottom` through `top`, both included, with `bottom <= top`.
#[derive(Clone, Copy)]
struct IntegerRange {
    bottom: f64,
    top: f64,
    count: f64,
}

impl IntegerRange {
    /// Returns `None` when the range holds more integers than a finite count.
    fn new(bottom: f64, top: f64) -> Option<Self> {
        let count = top - bottom + 1.0;
        count.is_finite().then_some(Self { bottom, top, count })
    }

    fn select(self, uniform: f64) -> f64 {
        // Past 2^53 the sum can round one step beyond the top.
        (self.bottom + (uniform * self.count).floor()).min(self.top)
    }
}
