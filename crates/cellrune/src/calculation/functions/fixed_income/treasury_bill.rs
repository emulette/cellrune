/// Treasury-bill closed forms.
///
/// T-bills use a 360-day quotation over the actual settlement-to-maturity day count and are
/// restricted to at most one calendar year. `TBILLEQ` switches from the documented simple formula
/// to the bond-equivalent square-root form above 182 actual days.
use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::util::required_number;
use super::day_count::actual_days;
use super::{coerce_date, date_from_serial_arg, finite_number};

pub(super) fn tbill_eq(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    tbill(engine, context, args, TbillMeasure::Equivalent)
}

pub(super) fn tbill_price(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    tbill(engine, context, args, TbillMeasure::Price)
}

pub(super) fn tbill_yield(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    tbill(engine, context, args, TbillMeasure::Yield)
}

enum TbillMeasure {
    Equivalent,
    Price,
    Yield,
}

fn tbill(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    measure: TbillMeasure,
) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let settlement = match coerce_date(engine, context, &args[0]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let maturity = match coerce_date(engine, context, &args[1]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[2]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };

    let allow_settlement_equals_maturity = !matches!(measure, TbillMeasure::Yield);
    if settlement > maturity || (!allow_settlement_equals_maturity && settlement == maturity) {
        return Value::Error(ErrorKind::Num);
    }
    if rate <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }

    let settlement_date = match date_from_serial_arg(settlement, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let maturity_date = match date_from_serial_arg(maturity, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let days = actual_days(settlement_date, maturity_date);
    if days <= 0.0 || days > 365.0 {
        return Value::Error(ErrorKind::Num);
    }

    let result = match measure {
        TbillMeasure::Price => {
            let price = 100.0 * (1.0 - rate * days / 360.0);
            if price <= 0.0 {
                return Value::Error(ErrorKind::Num);
            }
            price
        }
        TbillMeasure::Yield => (100.0 - rate) / rate * 360.0 / days,
        TbillMeasure::Equivalent => {
            if days > 182.0 {
                let price = 100.0 * (1.0 - rate * days / 360.0);
                if price <= 0.0 {
                    return Value::Error(ErrorKind::Num);
                }
                bond_equivalent(price, days)
            } else {
                let denominator = 360.0 - rate * days;
                if denominator <= 0.0 {
                    return Value::Error(ErrorKind::Num);
                }
                365.0 * rate / denominator
            }
        }
    };
    finite_number(result)
}

fn bond_equivalent(price: f64, days: f64) -> f64 {
    let time = days / 365.0;
    let inside = time * time - (2.0 * time - 1.0) * (1.0 - 100.0 / price);
    2.0 * (inside.sqrt() - time) / (2.0 * time - 1.0)
}
