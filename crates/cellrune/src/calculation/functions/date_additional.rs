use super::super::ast::Expr;
use super::super::coerce::to_logical;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::calendar::{
    Date, date_from_serial, days_from_civil, is_leap_year, serial_from_date, weekday_monday_zero,
};
use super::date::{days_360_european, days_360_us};
use super::kernel::DateAdditionalFunction;
use super::util::required_number;

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: DateAdditionalFunction,
    args: &[Expr],
) -> Value {
    match function {
        DateAdditionalFunction::Days => days(engine, context, args),
        DateAdditionalFunction::Days360 => days360(engine, context, args),
        DateAdditionalFunction::Hour => time_part(engine, context, args, TimePart::Hour),
        DateAdditionalFunction::Minute => time_part(engine, context, args, TimePart::Minute),
        DateAdditionalFunction::Second => time_part(engine, context, args, TimePart::Second),
        DateAdditionalFunction::Time => time(engine, context, args),
        DateAdditionalFunction::IsoWeekNum => iso_week_num(engine, context, args),
        DateAdditionalFunction::WeekNum => week_num(engine, context, args),
    }
}

fn days(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 2 {
        return Value::Error(ErrorKind::Value);
    }
    let end = match required_number(engine, context, &args[0]) {
        Ok(number) => number.trunc(),
        Err(kind) => return Value::Error(kind),
    };
    let start = match required_number(engine, context, &args[1]) {
        Ok(number) => number.trunc(),
        Err(kind) => return Value::Error(kind),
    };
    if date_from_serial(end, engine.date_system()).is_none()
        || date_from_serial(start, engine.date_system()).is_none()
    {
        Value::Error(ErrorKind::Num)
    } else {
        Value::Number(end - start)
    }
}

fn days360(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
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
    let european = match args.get(2) {
        Some(expr) => match to_logical(&engine.eval_scalar(context, expr)) {
            Ok(value) => value,
            Err(kind) => return Value::Error(kind),
        },
        None => false,
    };
    let Some(start) = date_from_serial(start_serial, engine.date_system()) else {
        return Value::Error(ErrorKind::Num);
    };
    let Some(end) = date_from_serial(end_serial, engine.date_system()) else {
        return Value::Error(ErrorKind::Num);
    };
    Value::Number(if european {
        days_360_european(start, end)
    } else {
        days_360_us(start, end)
    })
}

#[derive(Debug, Clone, Copy)]
enum TimePart {
    Hour,
    Minute,
    Second,
}

fn time_part(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    args: &[Expr],
    part: TimePart,
) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let serial = match required_number(engine, context, &args[0]) {
        Ok(number) if number >= 0.0 => number,
        Ok(_) => return Value::Error(ErrorKind::Num),
        Err(kind) => return Value::Error(kind),
    };
    let seconds = ((serial.fract() * 86_400.0).round() as i64).rem_euclid(86_400);
    let value = match part {
        TimePart::Hour => seconds / 3_600,
        TimePart::Minute => (seconds / 60) % 60,
        TimePart::Second => seconds % 60,
    };
    Value::Number(value as f64)
}

fn time(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 3 {
        return Value::Error(ErrorKind::Value);
    }
    let mut components = [0_i64; 3];
    for (target, expr) in components.iter_mut().zip(args) {
        *target = match required_number(engine, context, expr) {
            Ok(number) if (0.0..=32_767.0).contains(&number) => number.trunc() as i64,
            Ok(_) => return Value::Error(ErrorKind::Num),
            Err(kind) => return Value::Error(kind),
        };
    }
    let Some(seconds) = components[0]
        .checked_mul(3_600)
        .and_then(|value| value.checked_add(components[1] * 60))
        .and_then(|value| value.checked_add(components[2]))
    else {
        return Value::Error(ErrorKind::Num);
    };
    Value::Number(seconds.rem_euclid(86_400) as f64 / 86_400.0)
}

fn iso_week_num(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let serial = match required_number(engine, context, &args[0]) {
        Ok(number) => number.trunc(),
        Err(kind) => return Value::Error(kind),
    };
    let Some(date) = date_from_serial(serial, engine.date_system()) else {
        return Value::Error(ErrorKind::Num);
    };
    let Some(weekday) = weekday_monday_zero(serial, engine.date_system()) else {
        return Value::Error(ErrorKind::Num);
    };
    let ordinal =
        days_from_civil(date.year, date.month, date.day) - days_from_civil(date.year, 1, 1) + 1;
    let mut week = (ordinal - i64::from(weekday) + 10).div_euclid(7);
    if week < 1 {
        week = i64::from(iso_weeks_in_year(date.year - 1));
    } else if week > i64::from(iso_weeks_in_year(date.year)) {
        week = 1;
    }
    Value::Number(week as f64)
}

fn iso_weeks_in_year(year: i32) -> i32 {
    let january_first = (days_from_civil(year, 1, 1) + 3).rem_euclid(7) as i32;
    if january_first == 3 || (january_first == 2 && is_leap_year(year)) {
        53
    } else {
        52
    }
}

fn week_num(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.is_empty() || args.len() > 2 {
        return Value::Error(ErrorKind::Value);
    }
    let serial = match required_number(engine, context, &args[0]) {
        Ok(number) => number.trunc(),
        Err(kind) => return Value::Error(kind),
    };
    if date_from_serial(serial, engine.date_system()).is_none() {
        return Value::Error(ErrorKind::Num);
    }
    let return_type = match args.get(1) {
        Some(expr) => match required_number(engine, context, expr) {
            Ok(number) => number.trunc() as i32,
            Err(kind) => return Value::Error(kind),
        },
        None => 1,
    };
    if return_type == 21 {
        return iso_week_num(engine, context, &args[..1]);
    }
    let week_start = match return_type {
        1 | 17 => 6,
        2 | 11 => 0,
        12..=16 => return_type - 11,
        _ => return Value::Error(ErrorKind::Num),
    };
    let Some(date) = date_from_serial(serial, engine.date_system()) else {
        return Value::Error(ErrorKind::Num);
    };
    let Some(year_start) = serial_from_date(
        Date {
            year: date.year,
            month: 1,
            day: 1,
        },
        engine.date_system(),
    ) else {
        return Value::Error(ErrorKind::Num);
    };
    let Some(january_weekday) = weekday_monday_zero(year_start, engine.date_system()) else {
        return Value::Error(ErrorKind::Num);
    };
    let offset = (january_weekday - week_start).rem_euclid(7);
    Value::Number(((serial - year_start + f64::from(offset)) / 7.0).floor() + 1.0)
}
