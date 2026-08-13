/// Odd-first and odd-last price/yield kernels and adapters.
use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::util::required_number;
use super::model::CouponFrequency;
use super::odd_schedule::{OddFirstMeasures, odd_first_measures, odd_last_measures};
use super::{
    cash_flow_reduction, charge_work, coerce_basis, coerce_date, coerce_frequency, coupon_amount,
    date_from_serial_arg, finite_number,
};

pub(super) fn odd_f_price(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 8 || args.len() > 9 {
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
    let issue = match coerce_date(engine, context, &args[2]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let first_coupon = match coerce_date(engine, context, &args[3]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[4]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let yield_ = match required_number(engine, context, &args[5]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let redemption = match required_number(engine, context, &args[6]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let frequency = match coerce_frequency(engine, context, &args[7]) {
        Ok(frequency) => frequency,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(8)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };

    if !(maturity > first_coupon && first_coupon > settlement && settlement > issue) {
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
    let issue_date = match date_from_serial_arg(issue, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let first_coupon_date = match date_from_serial_arg(first_coupon, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };

    let Some(measures) = odd_first_measures(
        issue_date,
        first_coupon_date,
        settlement_date,
        maturity_date,
        frequency,
        basis,
    ) else {
        return Value::Error(ErrorKind::Num);
    };
    let flows = odd_first_flows(&measures, rate, redemption, frequency);
    let accrued = flows.accrued_interest;
    if let Err(kind) = charge_work(engine, context, flows.flows.len()) {
        return Value::Error(kind);
    }
    let (value, _) = cash_flow_reduction(&flows.flows, frequency.as_f64(), yield_);
    finite_number(value - accrued)
}

pub(super) fn odd_l_price(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 7 || args.len() > 8 {
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
    let last_interest = match coerce_date(engine, context, &args[2]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[3]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let yield_ = match required_number(engine, context, &args[4]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let redemption = match required_number(engine, context, &args[5]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let frequency = match coerce_frequency(engine, context, &args[6]) {
        Ok(frequency) => frequency,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(7)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };

    if !(maturity > settlement && settlement > last_interest) {
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
    let last_interest_date = match date_from_serial_arg(last_interest, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let Some(measures) = odd_last_measures(
        last_interest_date,
        settlement_date,
        maturity_date,
        frequency,
        basis,
    ) else {
        return Value::Error(ErrorKind::Num);
    };
    let coupon = coupon_amount(rate, frequency);
    let result = (redemption + coupon * measures.coupon_days_fraction)
        / (1.0 + (yield_ / frequency.as_f64()) * measures.to_maturity_fraction)
        - coupon * measures.accrued_fraction;
    finite_number(result)
}

pub(super) fn odd_l_yield(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 7 || args.len() > 8 {
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
    let last_interest = match coerce_date(engine, context, &args[2]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[3]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let price = match required_number(engine, context, &args[4]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let redemption = match required_number(engine, context, &args[5]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let frequency = match coerce_frequency(engine, context, &args[6]) {
        Ok(frequency) => frequency,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(7)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };

    if !(maturity > settlement && settlement > last_interest) {
        return Value::Error(ErrorKind::Num);
    }
    if rate < 0.0 || price <= 0.0 || redemption <= 0.0 {
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
    let last_interest_date = match date_from_serial_arg(last_interest, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let Some(measures) = odd_last_measures(
        last_interest_date,
        settlement_date,
        maturity_date,
        frequency,
        basis,
    ) else {
        return Value::Error(ErrorKind::Num);
    };
    let coupon = coupon_amount(rate, frequency);
    let result = ((redemption + coupon * measures.coupon_days_fraction)
        / (price + coupon * measures.accrued_fraction)
        - 1.0)
        * frequency.as_f64()
        / measures.to_maturity_fraction;
    finite_number(result)
}

pub(super) struct OddFirstFlows {
    pub(super) flows: Vec<(f64, f64)>,
    pub(super) accrued_interest: f64,
}

pub(super) fn odd_first_flows(
    measures: &OddFirstMeasures,
    rate: f64,
    redemption: f64,
    frequency: CouponFrequency,
) -> OddFirstFlows {
    let coupon = coupon_amount(rate, frequency);
    let alpha = measures.days_to_first_coupon / measures.period_days;
    let first_cash_flow = coupon * measures.first_period_days / measures.period_days;

    let mut flows = Vec::with_capacity(measures.coupon_count.max(0) as usize + 1);
    flows.push((alpha, first_cash_flow));
    for k in 2..=measures.coupon_count {
        flows.push(((k - 1) as f64 + alpha, coupon));
    }
    flows.push(((measures.coupon_count - 1) as f64 + alpha, redemption));

    OddFirstFlows {
        flows,
        accrued_interest: coupon * measures.accrued_days / measures.period_days,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::calendar::Date;
    use super::super::model::DayCountBasis;
    use super::*;

    const fn date(year: i32, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn odd_first_short_price_matches_frozen_reference() {
        let measures = odd_first_measures(
            date(2024, 11, 15),
            date(2025, 3, 1),
            date(2025, 2, 1),
            date(2030, 3, 1),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
        )
        .unwrap();
        let flows = odd_first_flows(&measures, 0.05, 100.0, CouponFrequency::Semiannual);
        let (value, _) = cash_flow_reduction(&flows.flows, 2.0, 0.06);
        assert!((value - flows.accrued_interest - 95.673_855_249_014_57).abs() < 1e-12);
    }

    #[test]
    fn odd_last_price_and_yield_match_frozen_literals() {
        let measures = odd_last_measures(
            date(2024, 10, 15),
            date(2025, 2, 1),
            date(2025, 6, 15),
            CouponFrequency::Semiannual,
            DayCountBasis::Us30360,
        )
        .unwrap();
        let coupon = coupon_amount(0.05, CouponFrequency::Semiannual);
        let price = (100.0 + coupon * measures.coupon_days_fraction)
            / (1.0 + (0.06 / 2.0) * measures.to_maturity_fraction)
            - coupon * measures.accrued_fraction;
        assert!((price - 99.603_747_781_038_28).abs() < 1e-12);

        let yield_ = ((100.0 + coupon * measures.coupon_days_fraction)
            / (99.0 + coupon * measures.accrued_fraction)
            - 1.0)
            * 2.0
            / measures.to_maturity_fraction;
        assert!((yield_ - 0.076_504_400_859_952_39).abs() < 1e-12);
    }
}
