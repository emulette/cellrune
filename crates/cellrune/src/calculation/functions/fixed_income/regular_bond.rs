/// Regular coupon bond price, duration, and the shared cash-flow measurements used by `YIELD`.
use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::calendar::Date;
use super::super::util::required_number;
use super::model::{CouponFrequency, DayCountBasis};
use super::schedule::{estimated_periods, regular_schedule};
use super::{
    cash_flow_reduction_with_poll, charge_work, check_cancellation, coerce_basis, coerce_date,
    coerce_frequency, coupon_amount, date_from_serial_arg, finite_number, poll_loop_cancellation,
};
use crate::DateSystem;

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

#[derive(Debug, Clone, Copy)]
pub(super) struct BondTerms {
    pub(super) settlement: Date,
    pub(super) maturity: Date,
    pub(super) rate: f64,
    pub(super) redemption: f64,
    pub(super) frequency: CouponFrequency,
    pub(super) basis: DayCountBasis,
    pub(super) date_system: DateSystem,
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
    let Some(work) = estimated_periods(settlement_date, maturity_date, frequency) else {
        return Value::Error(ErrorKind::Num);
    };
    if let Err(kind) = charge_work(engine, context, work.saturating_mul(2)) {
        return Value::Error(kind);
    }
    let mut poll = || check_cancellation(context);
    let measurements = match bond_measurements(
        BondTerms {
            settlement: settlement_date,
            maturity: maturity_date,
            rate,
            redemption,
            frequency,
            basis,
            date_system: engine.date_system(),
        },
        &mut poll,
    ) {
        Ok(Some(measurements)) => measurements,
        Ok(None) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };

    let price = if measurements.coupon_count == 1 {
        direct_price_n1(&measurements, yield_)
    } else {
        let (value, _) = match cash_flow_reduction_with_poll(
            &measurements.flows,
            measurements.frequency,
            yield_,
            &mut poll,
        ) {
            Ok(reduction) => reduction,
            Err(kind) => return Value::Error(kind),
        };
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
    let Some(work) = estimated_periods(settlement_date, maturity_date, frequency) else {
        return Value::Error(ErrorKind::Num);
    };
    if let Err(kind) = charge_work(engine, context, work.saturating_mul(2)) {
        return Value::Error(kind);
    }
    let mut poll = || check_cancellation(context);
    let measurements = match bond_measurements(
        BondTerms {
            settlement: settlement_date,
            maturity: maturity_date,
            rate,
            redemption: 100.0,
            frequency,
            basis,
            date_system: engine.date_system(),
        },
        &mut poll,
    ) {
        Ok(Some(measurements)) => measurements,
        Ok(None) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };

    let (value, time_weighted) = match cash_flow_reduction_with_poll(
        &measurements.flows,
        measurements.frequency,
        yield_,
        &mut poll,
    ) {
        Ok(reduction) => reduction,
        Err(kind) => return Value::Error(kind),
    };
    let macaulay = time_weighted / measurements.frequency / value;
    if !modified {
        return finite_number(macaulay);
    }
    finite_number(macaulay / (1.0 + yield_ / measurements.frequency))
}

pub(super) fn bond_measurements(
    terms: BondTerms,
    poll: &mut impl FnMut() -> Result<(), ErrorKind>,
) -> Result<Option<BondMeasurements>, ErrorKind> {
    let Some(schedule) = regular_schedule(
        terms.settlement,
        terms.maturity,
        terms.frequency,
        terms.basis,
        terms.date_system,
    ) else {
        return Ok(None);
    };
    if terms.date_system == DateSystem::Excel1900
        && schedule.previous_coupon_precedes_epoch(terms.date_system)
    {
        return Ok(None);
    }
    let coupon = coupon_amount(terms.rate, terms.frequency);
    let accrued_interest = coupon * schedule.accrued_days / schedule.period_days;
    let alpha = schedule.days_to_next / schedule.period_days;

    let mut flows = Vec::with_capacity(schedule.coupon_count.max(0) as usize);
    for k in 1..=schedule.coupon_count {
        poll_loop_cancellation(k as usize, poll)?;
        let time = (k - 1) as f64 + alpha;
        let cash_flow = if k == schedule.coupon_count {
            coupon + terms.redemption
        } else {
            coupon
        };
        flows.push((time, cash_flow));
    }

    Ok(Some(BondMeasurements {
        frequency: terms.frequency.as_f64(),
        coupon,
        redemption: terms.redemption,
        accrued_interest,
        flows,
        coupon_count: schedule.coupon_count,
        period_days: schedule.period_days,
        accrued_days: schedule.accrued_days,
    }))
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
    use std::cell::Cell;

    use super::super::super::calendar::Date;
    use super::super::cash_flow_reduction;
    use super::*;
    use crate::calculation::limits::CalculationLimitKind;

    const fn date(year: i32, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn one_coupon_price_and_yield_match_frozen_literals() {
        let measurements = bond_measurements(
            BondTerms {
                settlement: date(2025, 3, 15),
                maturity: date(2025, 7, 1),
                rate: 0.05,
                redemption: 100.0,
                frequency: CouponFrequency::Semiannual,
                basis: DayCountBasis::Us30360,
                date_system: DateSystem::Excel1900,
            },
            &mut || Ok(()),
        )
        .unwrap();
        let measurements = measurements.unwrap();
        assert_eq!(measurements.coupon_count, 1);
        assert!((direct_price_n1(&measurements, 0.04) - 100.279_052_883_324_8).abs() < 1e-12);
        assert!((direct_yield_n1(&measurements, 99.0) - 0.083_938_947_776_561_02).abs() < 1e-12);
    }

    #[test]
    fn regular_price_n_greater_than_one_matches_frozen_reference() {
        let measurements = bond_measurements(
            BondTerms {
                settlement: date(2025, 1, 1),
                maturity: date(2030, 1, 1),
                rate: 0.05,
                redemption: 100.0,
                frequency: CouponFrequency::Semiannual,
                basis: DayCountBasis::Us30360,
                date_system: DateSystem::Excel1900,
            },
            &mut || Ok(()),
        )
        .unwrap();
        let measurements = measurements.unwrap();
        assert_eq!(measurements.coupon_count, 10);
        let (value, _) = cash_flow_reduction(&measurements.flows, 2.0, 0.04);
        assert!((value - measurements.accrued_interest - 104.491_292_503_121_13).abs() < 1e-12);
    }

    #[test]
    fn long_cash_flow_construction_observes_bounded_cancellation_polls() {
        let polls = Cell::new(0_usize);
        let mut poll = || {
            let next = polls.get() + 1;
            polls.set(next);
            if next == 2 {
                Err(ErrorKind::ResourceLimit(
                    CalculationLimitKind::FunctionIterations,
                ))
            } else {
                Ok(())
            }
        };
        let result = bond_measurements(
            BondTerms {
                settlement: date(1901, 1, 1),
                maturity: date(9999, 1, 1),
                rate: 0.05,
                redemption: 100.0,
                frequency: CouponFrequency::Quarterly,
                basis: DayCountBasis::ActualActual,
                date_system: DateSystem::Excel1900,
            },
            &mut poll,
        );
        assert_eq!(
            result.err(),
            Some(ErrorKind::ResourceLimit(
                CalculationLimitKind::FunctionIterations
            ))
        );
        assert_eq!(polls.get(), 2);
    }
}
