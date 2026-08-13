/// `ACCRINT` — periodic accrued interest over a quasi-coupon schedule.
use super::super::super::ast::Expr;
use super::super::super::eval::{Engine, EvalContext};
use super::super::super::value::{ErrorKind, Value};
use super::super::util::required_number;
use super::day_count::days_between;
use super::model::{CouponFrequency, DayCountBasis};
use super::schedule::{add_months, is_end_of_month, normal_period_days};
use super::{coerce_basis, coerce_date, coerce_frequency, date_from_serial_arg, finite_number};

pub(super) fn call(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 6 || args.len() > 8 {
        return Value::Error(ErrorKind::Value);
    }
    let issue = match coerce_date(engine, context, &args[0]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let first_interest = match coerce_date(engine, context, &args[1]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let settlement = match coerce_date(engine, context, &args[2]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let rate = match required_number(engine, context, &args[3]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let par = match args.get(4) {
        Some(Expr::Missing) | None => 1000.0,
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number,
            Err(kind) => return Value::Error(kind),
        },
    };
    let frequency = match coerce_frequency(engine, context, &args[5]) {
        Ok(frequency) => frequency,
        Err(kind) => return Value::Error(kind),
    };
    let basis = match coerce_basis(engine, context, args.get(6)) {
        Ok(basis) => basis,
        Err(kind) => return Value::Error(kind),
    };
    let calc_method = match args.get(7) {
        None => true,
        Some(Expr::Missing) => false,
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number != 0.0,
            Err(kind) => return Value::Error(kind),
        },
    };

    if issue >= settlement {
        return Value::Error(ErrorKind::Num);
    }
    if rate <= 0.0 || par <= 0.0 {
        return Value::Error(ErrorKind::Num);
    }

    let issue_date = match date_from_serial_arg(issue, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let first_interest_date = match date_from_serial_arg(first_interest, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };
    let settlement_date = match date_from_serial_arg(settlement, engine) {
        Ok(date) => date,
        Err(kind) => return Value::Error(kind),
    };

    let Some(fraction) = accrued_fraction(
        issue_date,
        first_interest_date,
        settlement_date,
        frequency,
        basis,
        calc_method,
    ) else {
        return Value::Error(ErrorKind::Num);
    };

    finite_number(par * rate / frequency.as_f64() * fraction)
}

fn accrued_fraction(
    issue: super::super::calendar::Date,
    first_interest: super::super::calendar::Date,
    settlement: super::super::calendar::Date,
    frequency: CouponFrequency,
    basis: DayCountBasis,
    calc_method: bool,
) -> Option<f64> {
    let months = frequency.months();
    let end_of_month = is_end_of_month(first_interest);
    let accrual_start = if calc_method { issue } else { first_interest };

    let mut fraction = 0.0;
    let mut period_start = issue;
    let mut period_end = first_interest;
    loop {
        let normal_days = if period_start == issue {
            normal_period_days(first_interest, -months, basis, frequency, end_of_month)?
        } else {
            normal_period_days(period_start, months, basis, frequency, end_of_month)?
        };
        let accrued_end = period_end.min(settlement);
        let accrued_start = period_start.max(accrual_start);
        if accrued_start < accrued_end {
            fraction += days_between(accrued_start, accrued_end, basis) / normal_days;
        }
        if period_end >= settlement {
            break;
        }
        period_start = period_end;
        period_end = add_months(period_end, months, end_of_month)?;
    }
    Some(fraction)
}
