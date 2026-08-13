/// Treasury-bill closed forms.
///
/// T-bills use a 360-day quotation over the actual settlement-to-maturity day count and are
/// restricted to at most one calendar year. Excel switches `TBILLEQ` from the simple quotation
/// formula to a compound bond-equivalent branch after 182 actual days.
use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::util::required_number;
use super::day_count::actual_days;
use super::schedule::add_months;
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

    if settlement >= maturity {
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
    if !within_one_calendar_year(settlement_date, maturity_date) {
        return Value::Error(ErrorKind::Num);
    }
    let days = actual_days(settlement_date, maturity_date);
    if days <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }

    let result = match measure {
        TbillMeasure::Price => {
            let Some(price) = price_from_discount(rate, days) else {
                return Value::Error(ErrorKind::Num);
            };
            price
        }
        TbillMeasure::Yield => (100.0 - rate) / rate * 360.0 / days,
        TbillMeasure::Equivalent => {
            let Some(equivalent) = equivalent_from_discount(rate, days) else {
                return Value::Error(ErrorKind::Num);
            };
            equivalent
        }
    };
    finite_number(result)
}

fn within_one_calendar_year(
    settlement: super::super::calendar::Date,
    maturity: super::super::calendar::Date,
) -> bool {
    add_months(settlement, 12, false).is_some_and(|limit| maturity <= limit)
}

fn price_from_discount(discount: f64, days: f64) -> Option<f64> {
    let price = 100.0 * (1.0 - discount * days / 360.0);
    (price > 0.0).then_some(price)
}

fn equivalent_from_discount(discount: f64, days: f64) -> Option<f64> {
    if days <= 182.0 {
        let denominator = 360.0 - discount * days;
        return (denominator > 0.0).then_some(365.0 * discount / denominator);
    }

    let price = 1.0 - discount * days / 360.0;
    if price <= 0.0 {
        return None;
    }
    let year = if days == 366.0 { 366.0 } else { 365.0 };
    let extra_days = days - year / 2.0;
    let a = extra_days * price / (2.0 * year);
    let b = price * (0.5 + extra_days / year);
    let c = price - 1.0;
    let discriminant = b.mul_add(b, -4.0 * a * c);
    if a <= 0.0 || discriminant < 0.0 {
        return None;
    }
    // The direct `-b + sqrt(discriminant)` numerator loses precision for small discounts.
    // This is the algebraically equivalent, cancellation-resistant positive root.
    let denominator = -b - discriminant.sqrt();
    let result = 2.0 * c / denominator;
    (denominator != 0.0 && result.is_finite()).then_some(result)
}

#[cfg(test)]
mod tests {
    use super::super::super::calendar::Date;
    use super::*;

    const fn date(year: i32, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn calendar_year_limit_can_span_a_leap_day() {
        let settlement = date(2023, 7, 1);
        let maturity = date(2024, 7, 1);
        assert_eq!(actual_days(settlement, maturity), 366.0);
        assert!(within_one_calendar_year(settlement, maturity));
        assert!(within_one_calendar_year(
            date(2023, 2, 28),
            date(2024, 2, 28)
        ));
        assert!(!within_one_calendar_year(
            date(2023, 2, 28),
            date(2024, 2, 29)
        ));
        assert!(within_one_calendar_year(
            date(2024, 2, 29),
            date(2025, 2, 28)
        ));
    }

    #[test]
    fn equivalent_switches_to_the_excel_compound_branch_after_182_days() {
        let discount = 0.04;
        for days in [1.0, 181.0, 182.0] {
            let expected: f64 = 365.0 * discount / (360.0 - discount * days);
            assert_eq!(equivalent_from_discount(discount, days), Some(expected));
        }
        for (days, expected) in [
            (183.0, 0.041_394_959_763_901),
            (365.0, 0.041_832_345_790_172),
            (366.0, 0.041_950_586_074_360),
        ] {
            let actual = equivalent_from_discount(discount, days).unwrap();
            assert!((actual - expected).abs() < 1e-12, "{days}: {actual}");
        }
    }
}
