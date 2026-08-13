/// Regular coupon bond price, duration, and the shared cash-flow measurements used by `YIELD`.
use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::calendar::Date;
use super::super::util::required_number;
use super::model::{CouponFrequency, DayCountBasis};
use super::schedule::regular_schedule;
use super::{
    cash_flow_reduction, charge_work, coerce_basis, coerce_date, coerce_frequency, coupon_amount,
    date_from_serial_arg, finite_number,
};

pub(super) struct BondMeasurements {
    pub(super) frequency: f64,
    pub(super) coupon: f64,
    pub(super) redemption: f64,
    pub(super) accrued_interest: f64,
    pub(super) flows: Vec<(f64, f64)>,
    pub(super) coupon_count: i64,
    pub(super) period_days: f64,
    pub(super) accrued_days: f64,
}

pub(super) fn price(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 6 || args.len() > 7 {
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
    let yield_ = match required_number(engine, context, &args[3]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let redemption = match required_number(engine, context, &args[4]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let frequency = match coerce_frequency(engine, context, &args[5]) {
        Ok(frequency) => frequency,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(6)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };

    if settlement >= maturity {
        return Value::Error(ErrorKind::Num);
    }
    if rate < 0.0 || yield_ < 0.0 || redemption <= 0.0 {
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
    let Some(measurements) = bond_measurements(
        settlement_date,
        maturity_date,
        rate,
        redemption,
        frequency,
        basis,
    ) else {
        return Value::Error(ErrorKind::Num);
    };

    let price = if measurements.coupon_count == 1 {
        direct_price_n1(&measurements, yield_)
    } else {
        if let Err(kind) = charge_work(engine, context, measurements.flows.len()) {
            return Value::Error(kind);
        }
        let (value, _) = cash_flow_reduction(&measurements.flows, measurements.frequency, yield_);
        value - measurements.accrued_interest
    };
    finite_number(price)
}

pub(super) fn duration(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    duration_like(engine, context, args, false)
}

pub(super) fn m_duration(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    duration_like(engine, context, args, true)
}

fn duration_like(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    modified: bool,
) -> Value {
    if args.len() < 5 || args.len() > 6 {
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
    let yield_ = match required_number(engine, context, &args[3]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let frequency = match coerce_frequency(engine, context, &args[4]) {
        Ok(frequency) => frequency,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(5)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };

    if settlement >= maturity {
        return Value::Error(ErrorKind::Num);
    }
    if rate < 0.0 || yield_ < 0.0 {
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
    let Some(measurements) = bond_measurements(
        settlement_date,
        maturity_date,
        rate,
        100.0,
        frequency,
        basis,
    ) else {
        return Value::Error(ErrorKind::Num);
    };

    if let Err(kind) = charge_work(engine, context, measurements.flows.len()) {
        return Value::Error(kind);
    }
    let (value, time_weighted) =
        cash_flow_reduction(&measurements.flows, measurements.frequency, yield_);
    let macaulay = time_weighted / measurements.frequency / value;
    if !modified {
        return finite_number(macaulay);
    }
    finite_number(macaulay / (1.0 + yield_ / measurements.frequency))
}

pub(super) fn bond_measurements(
    settlement: Date,
    maturity: Date,
    rate: f64,
    redemption: f64,
    frequency: CouponFrequency,
    basis: DayCountBasis,
) -> Option<BondMeasurements> {
    let schedule = regular_schedule(settlement, maturity, frequency, basis)?;
    let coupon = coupon_amount(rate, frequency);
    let accrued_interest = coupon * schedule.accrued_days / schedule.period_days;
    let alpha = schedule.days_to_next / schedule.period_days;

    let mut flows = Vec::with_capacity(schedule.coupon_count.max(0) as usize);
    for k in 1..=schedule.coupon_count {
        let time = (k - 1) as f64 + alpha;
        let cash_flow = if k == schedule.coupon_count {
            coupon + redemption
        } else {
            coupon
        };
        flows.push((time, cash_flow));
    }

    Some(BondMeasurements {
        frequency: frequency.as_f64(),
        coupon,
        redemption,
        accrued_interest,
        flows,
        coupon_count: schedule.coupon_count,
        period_days: schedule.period_days,
        accrued_days: schedule.accrued_days,
    })
}

pub(super) fn direct_price_n1(measurements: &BondMeasurements, yield_: f64) -> f64 {
    let dsr = measurements.period_days - measurements.accrued_days;
    let t1 = measurements.redemption + measurements.coupon;
    let t2 = 1.0 + (yield_ / measurements.frequency) * dsr / measurements.period_days;
    let t3 = measurements.coupon * measurements.accrued_days / measurements.period_days;
    t1 / t2 - t3
}

pub(super) fn direct_yield_n1(measurements: &BondMeasurements, price: f64) -> f64 {
    let base = price / 100.0
        + (measurements.accrued_days / measurements.period_days) * measurements.coupon / 100.0;
    let dsr = measurements.period_days - measurements.accrued_days;
    ((measurements.redemption / 100.0 + measurements.coupon / 100.0) - base) / base
        * measurements.frequency
        * measurements.period_days
        / dsr
}

#[cfg(test)]
mod tests {
    use super::super::super::calendar::Date;
    use super::*;

    const fn date(year: i32, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn one_coupon_price_and_yield_match_frozen_literals() {
        let measurements = bond_measurements(
            date(2025, 3, 15),
            date(2025, 7, 1),
            0.05,
            100.0,
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
        )
        .unwrap();
        assert_eq!(measurements.coupon_count, 1);
        assert!((direct_price_n1(&measurements, 0.04) - 100.279_052_883_324_8).abs() < 1e-12);
        assert!((direct_yield_n1(&measurements, 99.0) - 0.083_938_947_776_561_02).abs() < 1e-12);
    }

    #[test]
    fn regular_price_n_greater_than_one_matches_frozen_reference() {
        let measurements = bond_measurements(
            date(2025, 1, 1),
            date(2030, 1, 1),
            0.05,
            100.0,
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
        )
        .unwrap();
        assert_eq!(measurements.coupon_count, 10);
        let (value, _) = cash_flow_reduction(&measurements.flows, 2.0, 0.04);
        assert!((value - measurements.accrued_interest - 104.491_292_503_121_13).abs() < 1e-12);
    }
}
