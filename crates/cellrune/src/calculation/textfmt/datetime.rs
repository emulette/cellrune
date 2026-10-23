//! Date and time layouts: `yyyy-mm-dd`, `h:mm AM/PM`, `[h]:mm:ss.00`, and the like.

use super::super::functions::{date_from_serial, weekday_monday_zero};
use super::super::value::ErrorKind;
use super::FormatContext;
use super::lexer::{Meridiem, Placeholder, Token};
use crate::Locale;

/// The locale identifier of English (United States) in a `[$-409]` code.
const EN_US_LOCALE_ID: u32 = 0x409;

/// Month and weekday names and the half-day markers.
pub(super) struct DateNames {
    months: [&'static str; 12],
    month_abbreviations: [&'static str; 12],
    weekdays: [&'static str; 7],
    weekday_abbreviations: [&'static str; 7],
    morning: &'static str,
    evening: &'static str,
}

/// The English (United States) names Microsoft documents for `mmm`, `mmmm`, `ddd`, `dddd`, and
/// `AM/PM`.
pub(super) const EN_US_NAMES: DateNames = DateNames {
    months: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    month_abbreviations: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    weekdays: [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ],
    weekday_abbreviations: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    morning: "AM",
    evening: "PM",
};

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Literal(String),
    /// The year in 2 or 4 digits: `y` and `yy` show two, `yyy`, `yyyy`, `e`, and `ee` four.
    Year(usize),
    /// `m` or `mm` before resolving to a month or a minute; `mmm` and longer are months.
    MonthOrMinute(usize),
    Month(usize),
    Minute(usize),
    Day(usize),
    Hour(usize),
    Second(usize),
    Elapsed {
        unit: char,
        width: usize,
    },
    /// The `00` of `ss.00`.
    SubSecond(usize),
    Meridiem(Meridiem),
}

/// Whether a section shows a date or a time rather than digits.
pub(super) fn is_date_section(tokens: &[Token]) -> bool {
    let has_placeholders = tokens
        .iter()
        .any(|token| matches!(token, Token::Placeholder(_)));
    tokens.iter().any(|token| match token {
        Token::Letters { letter, .. } => matches!(letter, 'd' | 'm' | 'y' | 'h' | 's' | 'e'),
        Token::Elapsed { .. } => true,
        // 0.1.26 showed `A/P` beside digit placeholders as literal text.
        Token::Meridiem(Meridiem::Letter { .. }) => !has_placeholders,
        Token::Meridiem(_) => true,
        _ => false,
    })
}

/// Renders a non-negative serial date-time with a date section.
pub(super) fn render(
    tokens: &[Token],
    value: f64,
    locale_id: Option<u32>,
    context: FormatContext,
) -> Result<String, ErrorKind> {
    let parts = resolve_minutes(parts(tokens)?);
    // Names are known only in English; the Korean names have not been measured.
    let english = match locale_id {
        Some(EN_US_LOCALE_ID) => true,
        Some(_) => false,
        None => context.locale == Locale::EnUs,
    };
    if !english && parts.iter().any(uses_names) {
        return Err(ErrorKind::Unsupported);
    }
    let names = &EN_US_NAMES;
    let decimals = parts
        .iter()
        .filter_map(|part| match part {
            Part::SubSecond(digits) => Some(*digits),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let ticks_per_second = 10_u64.pow(decimals as u32);
    let ticks_per_day = 86_400 * ticks_per_second;
    let ticks = (value * ticks_per_day as f64).round();
    if !ticks.is_finite() || ticks >= u64::MAX as f64 {
        return Err(ErrorKind::Unsupported);
    }
    let ticks = ticks as u64;
    let day = ticks / ticks_per_day;
    let of_day = ticks % ticks_per_day;
    let date = date_from_serial(day as f64, context.date_system).ok_or(ErrorKind::Unsupported)?;
    let twelve_hour = parts.iter().any(|part| matches!(part, Part::Meridiem(_)));
    let hour = of_day / (3_600 * ticks_per_second);
    let minute = of_day / (60 * ticks_per_second) % 60;
    let second = of_day / ticks_per_second % 60;
    let sub_second = of_day % ticks_per_second;

    let mut output = String::new();
    for part in &parts {
        match part {
            Part::Literal(text) => output.push_str(text),
            Part::Year(2) => output.push_str(&format!("{:02}", date.year.rem_euclid(100))),
            Part::Year(_) => output.push_str(&format!("{:04}", date.year)),
            Part::Month(1) => output.push_str(&date.month.to_string()),
            Part::Month(2) => output.push_str(&format!("{:02}", date.month)),
            Part::Month(count) => {
                let index = date.month as usize - 1;
                match count {
                    3 => output.push_str(names.month_abbreviations[index]),
                    4 => output.push_str(names.months[index]),
                    _ => output.extend(names.months[index].chars().next()),
                }
            }
            Part::Day(count @ (1 | 2)) => output.push_str(&padded(u64::from(date.day), *count)),
            Part::Day(count) => {
                let weekday = weekday_monday_zero(day as f64, context.date_system)
                    .ok_or(ErrorKind::Unsupported)?;
                let index = (weekday as usize + 1) % 7;
                output.push_str(if *count == 3 {
                    names.weekday_abbreviations[index]
                } else {
                    names.weekdays[index]
                });
            }
            Part::Hour(count) => {
                let shown = if twelve_hour {
                    match hour % 12 {
                        0 => 12,
                        other => other,
                    }
                } else {
                    hour
                };
                output.push_str(&padded(shown, *count));
            }
            Part::Minute(count) => output.push_str(&padded(minute, *count)),
            Part::Second(count) => output.push_str(&padded(second, *count)),
            Part::Elapsed { unit, width } => {
                let total = match unit {
                    'h' => ticks / (3_600 * ticks_per_second),
                    'm' => ticks / (60 * ticks_per_second),
                    _ => ticks / ticks_per_second,
                };
                output.push_str(&padded(total, *width));
            }
            Part::SubSecond(digits) => {
                let text = format!("{:0width$}", sub_second, width = decimals);
                output.push_str(&text[..*digits]);
            }
            Part::Meridiem(Meridiem::Full) => {
                output.push_str(if hour < 12 {
                    names.morning
                } else {
                    names.evening
                });
            }
            Part::Meridiem(Meridiem::Letter { morning, evening }) => {
                output.push(if hour < 12 { *morning } else { *evening });
            }
            Part::Meridiem(Meridiem::UnmeasuredCase) | Part::MonthOrMinute(_) => {
                unreachable!("parts rejects unmeasured markers and resolves months and minutes")
            }
        }
    }
    Ok(output)
}

fn padded(value: u64, width: usize) -> String {
    format!("{value:0width$}")
}

fn uses_names(part: &Part) -> bool {
    matches!(
        part,
        Part::Month(3..) | Part::Day(3..) | Part::Meridiem(Meridiem::Full)
    )
}

/// Reads date section tokens; constructs whose output has not been established are rejected.
fn parts(tokens: &[Token]) -> Result<Vec<Part>, ErrorKind> {
    let mut parts = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        index += 1;
        let after_second = matches!(
            parts.last(),
            Some(Part::Second(_) | Part::Elapsed { unit: 's', .. })
        );
        let part = match token {
            Token::Literal(text) => Part::Literal(text.clone()),
            Token::Letters { letter, count, .. } => match (letter, count) {
                // Excel 16.114 shows `y yyy ee` as `24 2024 2024` (excel-measure RESULTS.md,
                // section B).
                ('y', 1 | 2) => Part::Year(2),
                ('y', 3 | 4) | ('e', 1 | 2) => Part::Year(4),
                ('m', 1..=5) => Part::MonthOrMinute(*count),
                ('d', 1..=4) => Part::Day(*count),
                ('h', 1..=2) => Part::Hour(*count),
                ('s', 1..=2) => Part::Second(*count),
                // Other lengths, the era, Buddhist-year, and weekday-letter codes, and letters
                // that are not codes, which some hosts read as codes, are unmeasured.
                _ => return Err(ErrorKind::Unsupported),
            },
            Token::Elapsed { unit, width } => Part::Elapsed {
                unit: *unit,
                width: *width,
            },
            Token::DecimalPoint
                if after_second
                    && tokens.get(index) == Some(&Token::Placeholder(Placeholder::Zero)) =>
            {
                let digits = tokens[index..]
                    .iter()
                    .take_while(|token| **token == Token::Placeholder(Placeholder::Zero))
                    .count();
                if digits > 3 {
                    return Err(ErrorKind::Unsupported);
                }
                index += digits;
                parts.push(Part::Literal(".".to_owned()));
                Part::SubSecond(digits)
            }
            Token::DecimalPoint => Part::Literal(".".to_owned()),
            Token::Comma => Part::Literal(",".to_owned()),
            Token::Slash => Part::Literal("/".to_owned()),
            Token::Skip => Part::Literal(" ".to_owned()),
            Token::Meridiem(Meridiem::UnmeasuredCase) => return Err(ErrorKind::Unsupported),
            Token::Meridiem(meridiem) => Part::Meridiem(meridiem.clone()),
            Token::Placeholder(_)
            | Token::Percent
            | Token::Exponent { .. }
            | Token::TextValue
            | Token::Fill
            | Token::General => return Err(ErrorKind::Unsupported),
        };
        parts.push(part);
    }
    Ok(parts)
}

/// Resolves `m` and `mm`: minutes right after an hour or right before a second, months otherwise.
fn resolve_minutes(mut parts: Vec<Part>) -> Vec<Part> {
    let is_code = |part: &Part| !matches!(part, Part::Literal(_));
    for index in 0..parts.len() {
        let Part::MonthOrMinute(count) = parts[index] else {
            continue;
        };
        let after_hour = parts[..index]
            .iter()
            .rev()
            .find(|part| is_code(part))
            .is_some_and(|part| matches!(part, Part::Hour(_) | Part::Elapsed { unit: 'h', .. }));
        let before_second = parts[index + 1..]
            .iter()
            .find(|part| is_code(part))
            .is_some_and(|part| matches!(part, Part::Second(_) | Part::Elapsed { unit: 's', .. }));
        parts[index] = if count <= 2 && (after_hour || before_second) {
            Part::Minute(count)
        } else {
            Part::Month(count)
        };
    }
    parts
}
