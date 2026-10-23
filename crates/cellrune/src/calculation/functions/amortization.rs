//! The French accounting depreciation functions AMORLINC and AMORDEGRC.
//!
//! Both take `cost, date_purchased, first_period, salvage, period, rate, [basis]`, prorate period
//! 0 by the YEARFRAC fraction from the purchase date to the end of the first period in the basis,
//! and depreciate each later whole period at the rate. Microsoft documents the bases 0, 1, 3, and
//! 4, and the Oracle a salvage above cost as `#NUM!`. Excel 16.114 for Mac truncates the basis and
//! returns `#NUM!` for basis 2 or 5, a rate of 0, a negative period, and a purchase after the end
//! of the first period, and 0 for a period from 1 on when the salvage equals the cost. The other
//! invalid inputs (a cost that is not positive, a negative salvage, rate, or basis) and the
//! period 0 of a salvage equal to the cost were not measured, so they are unsupported.

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::array_common::poll_cancellation;
use super::date::year_fraction;
use super::financial::{financial_value, scalar_arguments};

struct Asset {
    cost: f64,
    salvage: f64,
    period: f64,
    rate: f64,
    /// The share of a year from the purchase to the end of the first period, or `None` when the
    /// asset was purchased on that date: Excel's `AMORDEGRC` then does not take the share as 0,
    /// and which share it takes was not established.
    first_fraction: Option<f64>,
}

fn asset(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Result<Asset, ErrorKind> {
    let values = scalar_arguments(engine, context, args, 7)?;
    let [cost, purchased, first_period, salvage, period, rate, basis] = values[..] else {
        return Err(ErrorKind::Value);
    };
    let (purchased, first_period, basis) = (purchased.trunc(), first_period.trunc(), basis.trunc());
    if salvage > cost
        || rate == 0.0
        || period < 0.0
        || basis == 2.0
        || basis >= 5.0
        || purchased > first_period
    {
        return Err(ErrorKind::Num);
    }
    if cost <= 0.0 || salvage < 0.0 || rate < 0.0 || basis < 0.0 {
        return Err(ErrorKind::Unsupported);
    }
    let first_fraction = if purchased == first_period {
        None
    } else {
        Some(year_fraction(
            purchased,
            first_period,
            basis as i32,
            engine.date_system(),
        )?)
    };
    Ok(Asset {
        cost,
        salvage,
        period,
        rate,
        first_fraction,
    })
}

/// AMORLINC: period 0 depreciates the prorated cost × rate, each later period cost × rate, and
/// the period after the last whole one the remainder down to salvage; later periods are zero.
/// Excel truncates the period and counts a period between 0 and 1 as period 1. A prorated first
/// period above the depreciable amount is unsupported, and so is a purchase on the end of the
/// first period unless the period's result is the same whether the first share is 0 or 1.
pub(super) fn amorlinc(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let asset = match asset(engine, context, args) {
        Ok(asset) => asset,
        Err(kind) => return Value::Error(kind),
    };
    let period = if asset.period > 0.0 && asset.period < 1.0 {
        1.0
    } else {
        asset.period.trunc()
    };
    let whole = asset.cost * asset.rate;
    let depreciable = asset.cost - asset.salvage;
    if depreciable == 0.0 {
        return if period >= 1.0 {
            Value::Number(0.0)
        } else {
            Value::Error(ErrorKind::Unsupported)
        };
    }
    let linear = |fraction: f64| {
        let first = fraction * whole;
        if first > depreciable {
            return None;
        }
        let whole_periods = ((depreciable - first) / whole).floor();
        Some(if period == 0.0 {
            first
        } else if period <= whole_periods {
            whole
        } else if period == whole_periods + 1.0 {
            depreciable - whole * whole_periods - first
        } else {
            0.0
        })
    };
    let depreciation = match asset.first_fraction {
        Some(fraction) => linear(fraction),
        None => match (linear(0.0), linear(1.0)) {
            (Some(no_share), Some(whole_year)) if no_share == whole_year => Some(no_share),
            _ => None,
        },
    };
    depreciation.map_or(Value::Error(ErrorKind::Unsupported), financial_value)
}

/// AMORDEGRC: the declining-balance counterpart of AMORLINC. The rate is multiplied by the
/// coefficient Microsoft documents for the asset life 1/rate (1.5 from 3 through 4 years, 2 from 5
/// through 6, and 2.5 above 6; `#NUM!` below 3 and strictly between 4 and 5, which Excel confirms
/// for lives of 1 and 2), each period's depreciation is the book value times that rate, rounded to
/// a whole number, and Excel truncates the period.
///
/// Microsoft documents a faster depreciation over the last two periods of the life and a stop
/// once the depreciation passes salvage without their exact rule. Excel's results agree with the
/// rule that the period whose depreciation would take the book value below salvage depreciates
/// half the book value and the later ones nothing, where that period is the one before the last of
/// the life: for a cost of 2400, a salvage of 300, and a rate of 0.15, periods 5 through 7 are 158,
/// 0, and 0. Other end periods are unsupported, as are a period between 0 and 1, where Excel's
/// result is not a defined number, a purchase on the end of the first period, and a rounding tie
/// whose direction changes the result.
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
    } else {
        return Value::Error(ErrorKind::Num);
    };
    if asset.period > 0.0 && asset.period < 1.0 {
        return Value::Error(ErrorKind::Unsupported);
    }
    let period = asset.period.trunc();
    if asset.salvage == asset.cost {
        return if period >= 1.0 {
            Value::Number(0.0)
        } else {
            Value::Error(ErrorKind::Unsupported)
        };
    }
    let Some(first_fraction) = asset.first_fraction else {
        return Value::Error(ErrorKind::Unsupported);
    };
    let schedule = Degressive {
        cost: asset.cost,
        salvage: asset.salvage,
        rate: asset.rate * coefficient,
        first_fraction,
        last_but_one: life.ceil() - 2.0,
    };
    let mut work = || {
        poll_cancellation(context)?;
        engine.charge_function_iterations(context, 1)
    };
    let mut ties = false;
    let result = schedule
        .depreciation(period, Tie::Up, &mut ties, &mut work)
        .and_then(|up| {
            if !ties {
                return Ok(up);
            }
            for tie in [Tie::Down, Tie::Even] {
                if schedule.depreciation(period, tie, &mut ties, &mut work)? != up {
                    return Err(ErrorKind::Unsupported);
                }
            }
            Ok(up)
        });
    match result {
        Ok(depreciation) => financial_value(depreciation),
        Err(kind) => Value::Error(kind),
    }
}

/// How a whole-number rounding resolves an amount within rounding error of a half.
#[derive(Debug, Clone, Copy)]
enum Tie {
    Up,
    Down,
    Even,
}

struct Degressive {
    cost: f64,
    salvage: f64,
    rate: f64,
    first_fraction: f64,
    /// The period before the last of the life, ⌈life⌉ − 2.
    last_but_one: f64,
}

impl Degressive {
    /// The depreciation of `period` with ties rounded by `tie`, setting `ties` when one occurs.
    fn depreciation(
        &self,
        period: f64,
        tie: Tie,
        ties: &mut bool,
        work: &mut impl FnMut() -> Result<(), ErrorKind>,
    ) -> Result<f64, ErrorKind> {
        let mut depreciation = whole_amount(self.first_fraction * self.rate * self.cost, tie, ties);
        let mut book = self.cost - depreciation;
        let mut remaining = book - self.salvage;
        if remaining < 0.0 {
            return Err(ErrorKind::Unsupported);
        }
        let mut current = 0.0;
        while current < period {
            work()?;
            current += 1.0;
            depreciation = whole_amount(self.rate * book, tie, ties);
            remaining -= depreciation;
            if remaining < 0.0 {
                if current != self.last_but_one {
                    return Err(ErrorKind::Unsupported);
                }
                return Ok(if current == period {
                    whole_amount(book * 0.5, tie, ties)
                } else {
                    0.0
                });
            }
            if current >= self.last_but_one {
                return Err(ErrorKind::Unsupported);
            }
            book -= depreciation;
        }
        Ok(depreciation)
    }
}

fn whole_amount(amount: f64, tie: Tie, ties: &mut bool) -> f64 {
    let floor = amount.floor();
    if (amount - floor - 0.5).abs() >= f64::EPSILON * amount.abs().max(1.0) {
        return amount.round();
    }
    *ties = true;
    match tie {
        Tie::Down => floor,
        Tie::Up => floor + 1.0,
        Tie::Even if floor % 2.0 == 0.0 => floor,
        Tie::Even => floor + 1.0,
    }
}
