//! The French accounting depreciation functions AMORLINC and AMORDEGRC.
//!
//! Both take `cost, date_purchased, first_period, salvage, period, rate, [basis]`, prorate period
//! 0 by the YEARFRAC fraction from the purchase date to the end of the first period in the basis,
//! and depreciate each later whole period at the rate. Microsoft documents the bases 0, 1, 3, and 4
//! and, with the Oracle, a salvage above cost as `#NUM!`; it documents no other argument errors, so
//! the remaining invalid inputs (a cost, rate, or period that is not positive, a fractional period,
//! a purchase after the first period, another basis, or a salvage equal to the cost) are
//! unsupported rather than guessed.

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::date::year_fraction;
use super::financial::{financial_value, scalar_arguments};

struct Asset {
    cost: f64,
    salvage: f64,
    period: f64,
    rate: f64,
    first_fraction: f64,
}

fn asset(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Result<Asset, ErrorKind> {
    let values = scalar_arguments(engine, context, args, 7)?;
    let [cost, purchased, first_period, salvage, period, rate, basis] = values[..] else {
        return Err(ErrorKind::Value);
    };
    if salvage > cost {
        return Err(ErrorKind::Num);
    }
    if cost <= 0.0
        || salvage < 0.0
        || salvage == cost
        || rate <= 0.0
        || period < 0.0
        || period.fract() != 0.0
        || !matches!(basis, 0.0 | 1.0 | 3.0 | 4.0)
    {
        return Err(ErrorKind::Unsupported);
    }
    let (purchased, first_period) = (purchased.trunc(), first_period.trunc());
    if purchased > first_period {
        return Err(ErrorKind::Unsupported);
    }
    let first_fraction =
        year_fraction(purchased, first_period, basis as i32, engine.date_system())?;
    Ok(Asset {
        cost,
        salvage,
        period,
        rate,
        first_fraction,
    })
}

/// AMORLINC: period 0 depreciates the prorated cost × rate, each later period cost × rate, and
/// the period after the last whole one the remainder down to salvage; later periods are zero. A
/// prorated first period above the depreciable amount is unsupported.
pub(super) fn amorlinc(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let asset = match asset(engine, context, args) {
        Ok(asset) => asset,
        Err(kind) => return Value::Error(kind),
    };
    let whole = asset.cost * asset.rate;
    let depreciable = asset.cost - asset.salvage;
    let first = asset.first_fraction * whole;
    if first > depreciable {
        return Value::Error(ErrorKind::Unsupported);
    }
    let whole_periods = ((depreciable - first) / whole).floor();
    financial_value(if asset.period == 0.0 {
        first
    } else if asset.period <= whole_periods {
        whole
    } else if asset.period == whole_periods + 1.0 {
        depreciable - whole * whole_periods - first
    } else {
        0.0
    })
}

/// AMORDEGRC: the declining-balance counterpart of AMORLINC. The rate is multiplied by the
/// coefficient Microsoft documents for the asset life 1/rate (1.5 from 3 through 4 years, 2 from 5
/// through 6, and 2.5 above 6; `#NUM!` strictly between 0 and 3 other than 1 and 2, and strictly
/// between 4 and 5), and each period's depreciation is the book value times that rate, rounded to
/// a whole number. Microsoft documents a faster depreciation over the last two periods of the
/// life and a stop once the depreciation would pass salvage, but not their exact rule, so those
/// periods, a life of exactly 1 or 2, and a rounding tie are unsupported.
pub(super) fn amordegrc(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let asset = match asset(engine, context, args) {
        Ok(asset) => asset,
        Err(kind) => return Value::Error(kind),
    };
    let life = asset.rate.recip();
    let coefficient = if (3.0..=4.0).contains(&life) {
        1.5
    } else if (5.0..=6.0).contains(&life) {
        2.0
    } else if life > 6.0 {
        2.5
    } else if life == 1.0 || life == 2.0 {
        return Value::Error(ErrorKind::Unsupported);
    } else {
        return Value::Error(ErrorKind::Num);
    };
    if asset.period + 2.0 >= life {
        return Value::Error(ErrorKind::Unsupported);
    }
    if let Err(kind) = engine.charge_function_iterations(context, asset.period as u64 + 1) {
        return Value::Error(kind);
    }
    let rate = asset.rate * coefficient;
    let mut depreciation = match whole_amount(asset.first_fraction * rate * asset.cost) {
        Ok(amount) => amount,
        Err(kind) => return Value::Error(kind),
    };
    let mut book = asset.cost - depreciation;
    for _ in 0..asset.period as u64 {
        depreciation = match whole_amount(rate * book) {
            Ok(amount) => amount,
            Err(kind) => return Value::Error(kind),
        };
        book -= depreciation;
    }
    if book < asset.salvage {
        return Value::Error(ErrorKind::Unsupported);
    }
    financial_value(depreciation)
}

fn whole_amount(amount: f64) -> Result<f64, ErrorKind> {
    if (amount.fract().abs() - 0.5).abs() < f64::EPSILON * amount.abs().max(1.0) {
        Err(ErrorKind::Unsupported)
    } else {
        Ok(amount.round())
    }
}
