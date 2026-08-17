use super::kernel::DateFunction;
use std::collections::BTreeSet;

use super::super::ast::Expr;
use super::super::coerce::to_number;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::array_common::poll_cancellation;
use super::calendar::{
    Date, date_from_serial, days_from_civil, days_in_month, is_leap_year, serial_from_date,
    serial_from_unix_days, weekday_monday_zero,
};
use super::util::{collect_argument_values, required_number, required_text};
use crate::DateSystem;

pub(super) fn call(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    function: DateFunction,
    args: &[Expr],
) -> Value {
    match function {
        DateFunction::Now => now(engine, args),
        DateFunction::Today => today(engine, args),
        DateFunction::Date => date(engine, context, args),
        DateFunction::DateValue => datevalue(engine, context, args),
        DateFunction::Year => date_part(engine, context, args, DatePart::Year),
        DateFunction::Month => date_part(engine, context, args, DatePart::Month),
        DateFunction::Day => date_part(engine, context, args, DatePart::Day),
        DateFunction::EDate => edate(engine, context, args),
        DateFunction::Eomonth => eomonth(engine, context, args),
        DateFunction::DateDif => datedif(engine, context, args),
        DateFunction::YearFrac => yearfrac(engine, context, args),
        DateFunction::Weekday => weekday(engine, context, args),
        DateFunction::Workday => workday(engine, context, args),
        DateFunction::NetworkDays => networkdays(engine, context, args),
        DateFunction::NetworkDaysIntl => networkdays_intl(engine, context, args),
        DateFunction::TimeValue => timevalue(engine, context, args),
        DateFunction::WorkdayIntl => workday_intl(engine, context, args),
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

fn datevalue(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let text = match exact_ascii_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let date = match parse_iso_date(&text) {
        Some(date) => date,
        None => return Value::Error(ErrorKind::Value),
    };
    if date
        == (Date {
            year: 1900,
            month: 2,
            day: 29,
        })
        && engine.date_system() == DateSystem::Excel1900
    {
        return Value::Number(60.0);
    }
    if date.month == 0
        || date.month > 12
        || date.day == 0
        || date.day > days_in_month(date.year, date.month)
    {
        return Value::Error(ErrorKind::Value);
    }
    serial_from_date(date, engine.date_system())
        .map(Value::Number)
        .unwrap_or(Value::Error(ErrorKind::Num))
}

fn timevalue(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() != 1 {
        return Value::Error(ErrorKind::Value);
    }
    let text = match exact_ascii_text(engine, context, &args[0]) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    parse_iso_time(&text)
        .map(Value::Number)
        .unwrap_or(Value::Error(ErrorKind::Value))
}

fn exact_ascii_text(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<String, ErrorKind> {
    match engine.eval_scalar(context, expr) {
        Value::Text(text) => {
            let text = text.trim_matches([' ', '\t']);
            if text.is_empty() {
                Err(ErrorKind::Value)
            } else {
                Ok(text.to_owned())
            }
        }
        Value::Error(kind) => Err(kind),
        Value::Blank | Value::Number(_) | Value::Logical(_) => Err(ErrorKind::Value),
    }
}

fn parse_iso_date(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    Some(Date {
        year: fixed_ascii_digits(&bytes[0..4])? as i32,
        month: fixed_ascii_digits(&bytes[5..7])?,
        day: fixed_ascii_digits(&bytes[8..10])?,
    })
}

fn parse_iso_time(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    if bytes.len() < 5 || bytes[2] != b':' {
        return None;
    }
    let hour = fixed_ascii_digits(&bytes[0..2])?;
    let minute = fixed_ascii_digits(&bytes[3..5])?;
    if hour > 23 || minute > 59 {
        return None;
    }
    if bytes.len() == 5 {
        return Some(f64::from(hour * 3_600 + minute * 60) / 86_400.0);
    }
    if bytes.len() < 8 || bytes[5] != b':' {
        return None;
    }
    let second = fixed_ascii_digits(&bytes[6..8])?;
    if second > 59 {
        return None;
    }
    let mut nanoseconds = 0_u32;
    if bytes.len() > 8 {
        let fraction_digits = bytes.len().checked_sub(9)?;
        if bytes[8] != b'.' || !(1..=9).contains(&fraction_digits) {
            return None;
        }
        let fraction = fixed_ascii_digits(&bytes[9..])?;
        nanoseconds = fraction.checked_mul(10_u32.pow(u32::try_from(9 - fraction_digits).ok()?))?;
    }
    let seconds = hour * 3_600 + minute * 60 + second;
    Some(f64::from(seconds) / 86_400.0 + f64::from(nanoseconds) / 86_400_000_000_000.0)
}

fn fixed_ascii_digits(bytes: &[u8]) -> Option<u32> {
    bytes.iter().try_fold(0_u32, |value, digit| {
        digit
            .is_ascii_digit()
            .then(|| value.checked_mul(10)?.checked_add(u32::from(*digit - b'0')))
            .flatten()
    })
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
    let start = match date_serial_argument(engine, context, &args[0]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let days = match integer_argument(engine, context, &args[1]) {
        Ok(days) => days,
        Err(kind) => return Value::Error(kind),
    };
    let holidays = match holiday_serials(engine, context, args.get(2)) {
        Ok(holidays) => holidays,
        Err(kind) => return Value::Error(kind),
    };
    workday_from_parts(
        engine,
        context,
        start,
        days,
        WeekendMask::STANDARD,
        holidays,
    )
}

fn networkdays(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 3 {
        return Value::Error(ErrorKind::Value);
    }
    let start = match date_serial_argument(engine, context, &args[0]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let end = match date_serial_argument(engine, context, &args[1]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let holidays = match holiday_serials(engine, context, args.get(2)) {
        Ok(holidays) => holidays,
        Err(kind) => return Value::Error(kind),
    };
    networkdays_from_parts(engine, context, start, end, WeekendMask::STANDARD, holidays)
}

fn workday_intl(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 4 {
        return Value::Error(ErrorKind::Value);
    }
    let start = match date_serial_argument(engine, context, &args[0]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let days = match integer_argument(engine, context, &args[1]) {
        Ok(days) => days,
        Err(kind) => return Value::Error(kind),
    };
    let weekend = match weekend_mask(engine, context, args.get(2)) {
        Ok(mask) => mask,
        Err(kind) => return Value::Error(kind),
    };
    let holidays = match holiday_serials(engine, context, args.get(3)) {
        Ok(holidays) => holidays,
        Err(kind) => return Value::Error(kind),
    };
    workday_from_parts(engine, context, start, days, weekend, holidays)
}

fn networkdays_intl(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    if args.len() < 2 || args.len() > 4 {
        return Value::Error(ErrorKind::Value);
    }
    let start = match date_serial_argument(engine, context, &args[0]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let end = match date_serial_argument(engine, context, &args[1]) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let weekend = match weekend_mask(engine, context, args.get(2)) {
        Ok(mask) => mask,
        Err(kind) => return Value::Error(kind),
    };
    let holidays = match holiday_serials(engine, context, args.get(3)) {
        Ok(holidays) => holidays,
        Err(kind) => return Value::Error(kind),
    };
    networkdays_from_parts(engine, context, start, end, weekend, holidays)
}

fn workday_from_parts(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    start: i64,
    days: i64,
    weekend: WeekendMask,
    holidays: BTreeSet<i64>,
) -> Value {
    if weekend.is_all_nonworking() {
        return Value::Error(ErrorKind::Value);
    }
    if days == 0 {
        return Value::Number(start as f64);
    }
    let mut serial = start;
    let step = if days < 0 { -1 } else { 1 };
    let mut remaining = days.unsigned_abs();
    let mut work = 0_u64;
    while remaining > 0 {
        work = work.saturating_add(1);
        if let Err(kind) = charge_calendar_work(engine, context, work) {
            return Value::Error(kind);
        }
        let Some(next) = serial.checked_add(step) else {
            return Value::Error(ErrorKind::Num);
        };
        if !valid_calendar_serial(next, engine.date_system()) {
            return Value::Error(ErrorKind::Num);
        }
        serial = next;
        if is_workday(serial, engine.date_system(), weekend, &holidays) {
            remaining -= 1;
        }
    }
    Value::Number(serial as f64)
}

fn networkdays_from_parts(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    start: i64,
    end: i64,
    weekend: WeekendMask,
    holidays: BTreeSet<i64>,
) -> Value {
    let (low, high, sign) = if start <= end {
        (start, end, 1.0)
    } else {
        (end, start, -1.0)
    };
    let mut serial = low;
    let mut count = 0_u64;
    let mut work = 0_u64;
    loop {
        work = work.saturating_add(1);
        if let Err(kind) = charge_calendar_work(engine, context, work) {
            return Value::Error(kind);
        }
        if is_workday(serial, engine.date_system(), weekend, &holidays) {
            count += 1;
        }
        if serial == high {
            break;
        }
        serial += 1;
    }
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
    for (index, item) in values.into_iter().enumerate() {
        charge_calendar_work(
            engine,
            context,
            u64::try_from(index)
                .map_err(|_| ErrorKind::Num)?
                .saturating_add(1),
        )?;
        match item.value {
            Value::Number(number) => {
                holidays.insert(date_serial_from_number(number, engine.date_system())?);
            }
            Value::Error(kind) => return Err(kind),
            Value::Blank | Value::Text(_) | Value::Logical(_) => {}
        }
    }
    Ok(holidays)
}

fn is_workday(
    serial: i64,
    system: DateSystem,
    weekend: WeekendMask,
    holidays: &BTreeSet<i64>,
) -> bool {
    let Some(monday_zero) = weekday_monday_zero(serial as f64, system) else {
        return false;
    };
    !weekend.is_nonworking(monday_zero as u8) && !holidays.contains(&serial)
}

#[derive(Debug, Clone, Copy)]
struct WeekendMask(u8);

impl WeekendMask {
    const STANDARD: Self = Self((1 << 5) | (1 << 6));

    fn is_nonworking(self, monday_zero: u8) -> bool {
        self.0 & (1 << monday_zero) != 0
    }

    fn is_all_nonworking(self) -> bool {
        self.0 == 0b111_1111
    }
}

fn weekend_mask(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: Option<&Expr>,
) -> Result<WeekendMask, ErrorKind> {
    let Some(expr) = expr else {
        return Ok(WeekendMask::STANDARD);
    };
    match engine.eval_scalar(context, expr) {
        Value::Text(mask) => parse_weekend_mask(&mask),
        Value::Error(kind) => Err(kind),
        value => {
            let number = to_number(&value)?;
            if !number.is_finite() {
                return Err(ErrorKind::Value);
            }
            numeric_weekend_mask(number.trunc())
        }
    }
}

fn numeric_weekend_mask(code: f64) -> Result<WeekendMask, ErrorKind> {
    if !(i32::MIN as f64..=i32::MAX as f64).contains(&code) {
        return Err(ErrorKind::Num);
    }
    let code = code as i32;
    let bits = match code {
        1..=7 => {
            let start = (code + 4).rem_euclid(7) as u8;
            (1 << start) | (1 << ((start + 1) % 7))
        }
        11..=17 => 1 << ((code - 5).rem_euclid(7) as u8),
        _ => return Err(ErrorKind::Num),
    };
    Ok(WeekendMask(bits))
}

fn parse_weekend_mask(mask: &str) -> Result<WeekendMask, ErrorKind> {
    let bytes = mask.as_bytes();
    if bytes.len() != 7 {
        return Err(ErrorKind::Value);
    }
    let mut bits = 0_u8;
    for (index, value) in bytes.iter().enumerate() {
        match value {
            b'0' => {}
            b'1' => bits |= 1 << index,
            _ => return Err(ErrorKind::Value),
        }
    }
    Ok(WeekendMask(bits))
}

fn date_serial_argument(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<i64, ErrorKind> {
    date_serial_from_number(
        required_number(engine, context, expr)?,
        engine.date_system(),
    )
}

fn date_serial_from_number(number: f64, system: DateSystem) -> Result<i64, ErrorKind> {
    if !number.is_finite() {
        return Err(ErrorKind::Value);
    }
    let serial = number.floor();
    if !(i64::MIN as f64..=i64::MAX as f64).contains(&serial) {
        return Err(ErrorKind::Num);
    }
    let serial = serial as i64;
    valid_calendar_serial(serial, system)
        .then_some(serial)
        .ok_or(ErrorKind::Num)
}

fn integer_argument(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    expr: &Expr,
) -> Result<i64, ErrorKind> {
    let number = required_number(engine, context, expr)?;
    if !number.is_finite() {
        return Err(ErrorKind::Value);
    }
    let number = number.trunc();
    if !(i64::MIN as f64..=i64::MAX as f64).contains(&number) {
        return Err(ErrorKind::Num);
    }
    Ok(number as i64)
}

fn valid_calendar_serial(serial: i64, system: DateSystem) -> bool {
    date_from_serial(serial as f64, system).is_some()
}

fn charge_calendar_work(
    engine: &Engine<'_>,
    context: EvalContext<'_>,
    work: u64,
) -> Result<(), ErrorKind> {
    if work % 256 == 1 {
        poll_cancellation(context)?;
    }
    engine.charge_function_iterations(context, 1)
}

fn previous_month(year: i32, month: u32) -> (i32, u32) {
    if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intl_weekend_codes_cover_every_excel_mapping() {
        for (code, expected) in [
            (1.0, 0b110_0000),
            (2.0, 0b100_0001),
            (3.0, 0b000_0011),
            (4.0, 0b000_0110),
            (5.0, 0b000_1100),
            (6.0, 0b001_1000),
            (7.0, 0b011_0000),
            (11.0, 0b100_0000),
            (12.0, 0b000_0001),
            (13.0, 0b000_0010),
            (14.0, 0b000_0100),
            (15.0, 0b000_1000),
            (16.0, 0b001_0000),
            (17.0, 0b010_0000),
        ] {
            assert_eq!(
                numeric_weekend_mask(code).expect("valid weekend code").0,
                expected,
                "unexpected weekend mask for code {code}"
            );
        }
        for code in [0.0, 8.0, 10.9, 18.0] {
            assert_eq!(numeric_weekend_mask(code).unwrap_err(), ErrorKind::Num);
        }
    }

    #[test]
    fn intl_text_masks_are_exact_ascii_monday_to_sunday_bits() {
        assert_eq!(parse_weekend_mask("0000000").expect("all workdays").0, 0);
        assert_eq!(
            parse_weekend_mask("1111111").expect("all non-workdays").0,
            0b111_1111
        );
        assert_eq!(
            parse_weekend_mask("1000001").expect("Monday and Sunday").0,
            0b100_0001
        );
        for invalid in ["2", "000001", "00000000", "00000x1", "０００００００"] {
            assert_eq!(parse_weekend_mask(invalid).unwrap_err(), ErrorKind::Value);
        }
    }
}
