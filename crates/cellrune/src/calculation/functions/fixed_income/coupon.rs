/// The six regular coupon projection adapters.
use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::schedule::regular_schedule;
use super::{coerce_basis, coerce_date, coerce_frequency, date_from_serial_arg, finite_number};

pub(super) fn coup_day_bs(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    coupon_measure(engine, context, args, Measure::AccruedDays)
}

pub(super) fn coup_days(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    coupon_measure(engine, context, args, Measure::PeriodDays)
}

pub(super) fn coup_days_nc(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    coupon_measure(engine, context, args, Measure::DaysToNext)
}

pub(super) fn coup_ncd(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    coupon_measure(engine, context, args, Measure::NextCoupon)
}

pub(super) fn coup_num(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    coupon_measure(engine, context, args, Measure::CouponCount)
}

pub(super) fn coup_pcd(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    coupon_measure(engine, context, args, Measure::PreviousCoupon)
}

enum Measure {
    AccruedDays,
    PeriodDays,
    DaysToNext,
    NextCoupon,
    CouponCount,
    PreviousCoupon,
}

fn coupon_measure(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    measure: Measure,
) -> Value {
    if args.len() < 3 || args.len() > 4 {
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
    let frequency = match coerce_frequency(engine, context, &args[2]) {
        Ok(frequency) => frequency,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(3)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };
    if settlement >= maturity {
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
    let Some(schedule) = regular_schedule(settlement_date, maturity_date, frequency, basis) else {
        return Value::Error(ErrorKind::Num);
    };
    match measure {
        Measure::AccruedDays => finite_number(schedule.accrued_days),
        Measure::PeriodDays => finite_number(schedule.period_days),
        Measure::DaysToNext => finite_number(schedule.days_to_next),
        Measure::CouponCount => finite_number(schedule.coupon_count as f64),
        Measure::NextCoupon => match schedule.next_coupon_serial(engine.date_system()) {
            Some(serial) => finite_number(serial),
            None => Value::Error(ErrorKind::Num),
        },
        Measure::PreviousCoupon => match schedule.previous_coupon_serial(engine.date_system()) {
            Some(serial) => finite_number(serial),
            None => Value::Error(ErrorKind::Num),
        },
    }
}
