use std::collections::BTreeSet;

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::calendar::{
    Date, date_from_serial, days_from_civil, days_in_month, is_leap_year, serial_from_date,
    serial_from_unix_days, weekday_monday_zero,
};
use super::util::{collect_argument_values, required_number, required_text};
use crate::DateSystem;

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    name: &str,
    args: &[Expr],
) -> Value {
    match name {
        "NOW" => now(engine, args),
        "TODAY" => today(engine, args),
        "DATE" => date(engine, context, args),
        "YEAR" => date_part(engine, context, args, DatePart::Year),
        "MONTH" => date_part(engine, context, args, DatePart::Month),
        "DAY" => date_part(engine, context, args, DatePart::Day),
        "EDATE" => edate(engine, context, args),
        "EOMONTH" => eomonth(engine, context, args),
        "DATEDIF" => datedif(engine, context, args),
        "YEARFRAC" => yearfrac(engine, context, args),
        "WEEKDAY" => weekday(engine, context, args),
        "WORKDAY" => workday(engine, context, args),
        "NETWORKDAYS" => networkdays(engine, context, args),
        _ => Value::Error(ErrorKind::Unsupported),
    }
}

fn now(engine: &Engine<'_>, args: &[Expr]) -> Value {
    if !args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    engine
        .now_serial()
        .map(Value::Number)
        .unwrap_or(Value::Error(ErrorKind::Unsupported))
}

fn yearfrac(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let start_serial = match required_number(engine, context, &args[0]) {
        Ok(number) => number.trunc(),
        Err(kind) => return Value::Error(kind),
    };
    let end_serial = match required_number(engine, context, &args[1]) {
        Ok(number) => number.trunc(),
        Err(kind) => return Value::Error(kind),
    };
    let basis = match args.get(2) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number.trunc() as i32,
            Err(kind) => return Value::Error(kind),
        },
        None => 0,
    };
    if !(0..=4).contains(&basis) {
        return Value::Error(ErrorKind::Num);
    }

    let (low_serial, high_serial, sign) = if start_serial <= end_serial {
        (start_serial, end_serial, 1.0)
    } else {
        (end_serial, start_serial, -1.0)
    };
    let Some(start) = date_from_serial(low_serial, engine.date_system()) else {
        return Value::Error(ErrorKind::Num);
    };
    let Some(end) = date_from_serial(high_serial, engine.date_system()) else {
        return Value::Error(ErrorKind::Num);
    };
    let actual_days = high_serial - low_serial;
    let fraction = match basis {
        0 => days_360_us(start, end) / 360.0,
        1 => actual_days / actual_actual_year_length(start, end),
        2 => actual_days / 360.0,
        3 => actual_days / 365.0,
        4 => days_360_european(start, end) / 360.0,
        _ => unreachable!("basis was validated"),
    };
    Value::Number(fraction * sign)
}

fn actual_actual_year_length(start: Date, end: Date) -> f64 {
    if start.year == end.year {
        return if is_leap_year(start.year) {
            366.0
        } else {
            365.0
        };
    }
    let no_more_than_one_year =
        end.year == start.year + 1 && (end.month, end.day) <= (start.month, start.day);
    if no_more_than_one_year {
        let includes_leap_day = (is_leap_year(start.year) && (start.month, start.day) <= (2, 29))
            || (is_leap_year(end.year) && (end.month, end.day) >= (2, 29));
        return if includes_leap_day { 366.0 } else { 365.0 };
    }
    let year_count = i64::from(end.year - start.year + 1);
    let days = (start.year..=end.year)
        .map(|year| if is_leap_year(year) { 366_i64 } else { 365_i64 })
        .sum::<i64>();
    days as f64 / year_count as f64
}

pub(super) fn days_360_us(start: Date, end: Date) -> f64 {
    let start_is_last_february =
        start.month == 2 && start.day == days_in_month(start.year, start.month);
    let end_is_last_february = end.month == 2 && end.day == days_in_month(end.year, end.month);
    let start_day = if start.day == 31 || start_is_last_february {
        30
    } else {
        start.day
    };
    let end_day =
        if (end.day == 31 && start_day == 30) || (end_is_last_february && start_is_last_february) {
            30
        } else {
            end.day
        };
    days_360_components(
        start.year,
        start.month,
        start_day,
        end.year,
        end.month,
        end_day,
    )
}

pub(super) fn days_360_european(start: Date, end: Date) -> f64 {
    days_360_components(
        start.year,
        start.month,
        start.day.min(30),
        end.year,
        end.month,
        end.day.min(30),
    )
}

fn days_360_components(
    start_year: i32,
    start_month: u32,
    start_day: u32,
    end_year: i32,
    end_month: u32,
    end_day: u32,
) -> f64 {
    let years = i64::from(end_year - start_year) * 360;
    let months = (i64::from(end_month) - i64::from(start_month)) * 30;
    let days = i64::from(end_day) - i64::from(start_day);
    (years + months + days) as f64
}

fn today(engine: &Engine<'_>, args: &[Expr]) -> Value {
    if !args.is_empty() {
        return Value::Error(ErrorKind::Value);
    }
    engine
        .today_serial()
        .map(Value::Number)
        .unwrap_or(Value::Error(ErrorKind::Unsupported))
}

fn date(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let mut year = match required_number(engine, context, &args[0]) {
        Ok(number) => number.trunc() as i64,
        Err(kind) => return Value::Error(kind),
    };
    let month = match required_number(engine, context, &args[1]) {
        Ok(number) => number.trunc() as i64,
        Err(kind) => return Value::Error(kind),
    };
    let day = match required_number(engine, context, &args[2]) {
        Ok(number) => number.trunc() as i64,
        Err(kind) => return Value::Error(kind),
    };
    if (0..1900).contains(&year) {
        year += 1900;
    }
    let Some(month_zero) = month.checked_sub(1) else {
        return Value::Error(ErrorKind::Num);
    };
    let Some(normalized_year) = year.checked_add(month_zero.div_euclid(12)) else {
        return Value::Error(ErrorKind::Num);
    };
    if !(0..=9_999).contains(&normalized_year) {
        return Value::Error(ErrorKind::Num);
    }
    let normalized_month = month_zero.rem_euclid(12) as u32 + 1;
    let Some(month_start) = serial_from_unix_days(
        days_from_civil(normalized_year as i32, normalized_month, 1),
        engine.date_system(),
    ) else {
        return Value::Error(ErrorKind::Num);
    };
    let Some(serial) = (month_start as i64)
        .checked_add(day)
        .and_then(|value| value.checked_sub(1))
    else {
        return Value::Error(ErrorKind::Num);
    };
    let serial = serial as f64;
    if date_from_serial(serial, engine.date_system()).is_some() {
        Value::Number(serial)
    } else {
        Value::Error(ErrorKind::Num)
    }
}

#[derive(Debug, Clone, Copy)]
enum DatePart {
    Year,
    Month,
    Day,
}

fn date_part(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    part: DatePart,
) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let serial = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    match date_from_serial(serial, engine.date_system()) {
        Some(date) => Value::Number(match part {
            DatePart::Year => date.year as f64,
            DatePart::Month => date.month as f64,
            DatePart::Day => date.day as f64,
        }),
        None => Value::Error(ErrorKind::Num),
    }
}

fn edate(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    shift_month(engine, context, args, ShiftedDay::Preserve)
}

fn eomonth(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    shift_month(engine, context, args, ShiftedDay::Last)
}

#[derive(Debug, Clone, Copy)]
enum ShiftedDay {
    Preserve,
    Last,
}

fn shift_month(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    shifted_day: ShiftedDay,
) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let serial = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let months = match required_number(engine, context, &args[1]) {
        Ok(number) => number.trunc() as i64,
        Err(kind) => return Value::Error(kind),
    };
    let Some(source) = date_from_serial(serial, engine.date_system()) else {
        return Value::Error(ErrorKind::Num);
    };
    let Some(month_zero) = i64::from(source.year)
        .checked_mul(12)
        .and_then(|value| value.checked_add(i64::from(source.month) - 1))
        .and_then(|value| value.checked_add(months))
    else {
        return Value::Error(ErrorKind::Num);
    };
    let year = month_zero.div_euclid(12);
    if !(0..=9_999).contains(&year) {
        return Value::Error(ErrorKind::Num);
    }
    let month = month_zero.rem_euclid(12) as u32 + 1;
    let last_day = days_in_month(year as i32, month);
    let day = match shifted_day {
        ShiftedDay::Preserve => source.day.min(last_day),
        ShiftedDay::Last => last_day,
    };
    serial_from_date(
        Date {
            year: year as i32,
            month,
            day,
        },
        engine.date_system(),
    )
    .map(Value::Number)
    .unwrap_or(Value::Error(ErrorKind::Num))
}

fn datedif(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let start_serial = match required_number(engine, context, &args[0]) {
        Ok(number) => number.floor(),
        Err(kind) => return Value::Error(kind),
    };
    let end_serial = match required_number(engine, context, &args[1]) {
        Ok(number) => number.floor(),
        Err(kind) => return Value::Error(kind),
    };
    if end_serial < start_serial {
        return Value::Error(ErrorKind::Num);
    }
    let unit = match required_text(engine, context, &args[2]) {
        Ok(text) => text.to_ascii_uppercase(),
        Err(kind) => return Value::Error(kind),
    };
    let Some(start) = date_from_serial(start_serial, engine.date_system()) else {
        return Value::Error(ErrorKind::Num);
    };
    let Some(end) = date_from_serial(end_serial, engine.date_system()) else {
        return Value::Error(ErrorKind::Num);
    };
    let months = (end.year - start.year) * 12 + end.month as i32
        - start.month as i32
        - i32::from(end.day < start.day);
    let value = match unit.as_str() {
        "Y" => months / 12,
        "M" => months,
        "D" => (end_serial - start_serial) as i32,
        "YM" => months.rem_euclid(12),
        "MD" => day_difference_ignoring_months(start, end),
        "YD" => day_difference_ignoring_years(start, end),
        _ => return Value::Error(ErrorKind::Num),
    };
    Value::Number(value as f64)
}

fn day_difference_ignoring_months(start: Date, end: Date) -> i32 {
    if end.day >= start.day {
        (end.day - start.day) as i32
    } else {
        let (year, month) = previous_month(end.year, end.month);
        (days_in_month(year, month) - start.day + end.day) as i32
    }
}

fn day_difference_ignoring_years(start: Date, end: Date) -> i32 {
    let anniversary_year = if (end.month, end.day) >= (start.month, start.day) {
        end.year
    } else {
        end.year - 1
    };
    let anniversary_day = start.day.min(days_in_month(anniversary_year, start.month));
    (days_from_civil(end.year, end.month, end.day)
        - days_from_civil(anniversary_year, start.month, anniversary_day)) as i32
}

fn weekday(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let serial = match required_number(engine, context, &args[0]) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let return_type = match args.get(1) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number.trunc() as i32,
            Err(kind) => return Value::Error(kind),
        },
        None => 1,
    };
    let Some(monday_zero) = weekday_monday_zero(serial, engine.date_system()) else {
        return Value::Error(ErrorKind::Num);
    };
    let result = match return_type {
        1 | 17 => (monday_zero + 1).rem_euclid(7) + 1,
        2 | 11 => monday_zero + 1,
        3 => monday_zero,
        12..=16 => (monday_zero - (return_type - 11)).rem_euclid(7) + 1,
        _ => return Value::Error(ErrorKind::Num),
    };
    Value::Number(result as f64)
}

fn workday(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let start = match required_number(engine, context, &args[0]) {
        Ok(number) => number.floor() as i64,
        Err(kind) => return Value::Error(kind),
    };
    let days = match required_number(engine, context, &args[1]) {
        Ok(number) => number.trunc() as i64,
        Err(kind) => return Value::Error(kind),
    };
    let holidays = match holiday_serials(engine, context, args.get(2)) {
        Ok(holidays) => holidays,
        Err(kind) => return Value::Error(kind),
    };
    let mut serial = start;
    let step = if days < 0 { -1 } else { 1 };
    let mut remaining = days.unsigned_abs();
    if let Err(kind) = engine.ensure_function_iterations(remaining) {
        return Value::Error(kind);
    }
    let mut iterations = 0_u64;
    while remaining > 0 {
        iterations += 1;
        if let Err(kind) = engine.ensure_function_iterations(iterations) {
            return Value::Error(kind);
        }
        let Some(next) = serial.checked_add(step) else {
            return Value::Error(ErrorKind::Num);
        };
        serial = next;
        if is_workday(serial, engine.date_system(), &holidays) {
            remaining -= 1;
        }
    }
    Value::Number(serial as f64)
}

fn networkdays(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let start = match required_number(engine, context, &args[0]) {
        Ok(number) => number.floor() as i64,
        Err(kind) => return Value::Error(kind),
    };
    let end = match required_number(engine, context, &args[1]) {
        Ok(number) => number.floor() as i64,
        Err(kind) => return Value::Error(kind),
    };
    let holidays = match holiday_serials(engine, context, args.get(2)) {
        Ok(holidays) => holidays,
        Err(kind) => return Value::Error(kind),
    };
    let (low, high, sign) = if start <= end {
        (start, end, 1.0)
    } else {
        (end, start, -1.0)
    };
    let iterations = i128::from(high) - i128::from(low) + 1;
    let Ok(iterations) = u64::try_from(iterations) else {
        return Value::Error(ErrorKind::Num);
    };
    if let Err(kind) = engine.ensure_function_iterations(iterations) {
        return Value::Error(kind);
    }
    let count = (low..=high)
        .filter(|serial| is_workday(*serial, engine.date_system(), &holidays))
        .count();
    Value::Number(count as f64 * sign)
}

fn holiday_serials(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
) -> Result<BTreeSet<i64>, ErrorKind> {
    let Some(expr) = expr else {
        return Ok(BTreeSet::new());
    };
    let values = collect_argument_values(engine, context, std::slice::from_ref(expr))?;
    let mut holidays = BTreeSet::new();
    for item in values {
        match item.value {
            Value::Number(number) => {
                holidays.insert(number.floor() as i64);
            }
            Value::Error(kind) => return Err(kind),
            Value::Blank | Value::Text(_) | Value::Logical(_) => {}
        }
    }
    Ok(holidays)
}

fn is_workday(serial: i64, system: DateSystem, holidays: &BTreeSet<i64>) -> bool {
    let Some(monday_zero) = weekday_monday_zero(serial as f64, system) else {
        return false;
    };
    monday_zero < 5 && !holidays.contains(&serial)
}

fn previous_month(year: i32, month: u32) -> (i32, u32) {
    if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    }
}
