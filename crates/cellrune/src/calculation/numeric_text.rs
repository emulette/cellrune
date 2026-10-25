//! Text read as the number it stands for, where a formula needs a number.
//!
//! Excel reads such text as it reads a typed entry: `"1,234"`, `"50%"`, `"(5)"`, `"$5"`, `"1 1/2"`,
//! `"2024-01-15"`, and `"2:30 PM"` are numbers to `VALUE`, the arithmetic operators, function
//! arguments, and criteria. Its reader also takes forms whose number depends on the current date
//! (`"1/2"` is the second of January of this year) or that it reads loosely (`"12:99"`), and
//! CellRune leaves those unread. Every accepted form below was measured in Mac Excel 16.114; text
//! outside them reads as no number, so it fails explicitly instead of reading differently.
//!
//! The currency symbol and whether a date may begin with its month follow the [`Locale`]; the
//! decimal separator is `.` and the thousands separator `,` in every built-in locale.

use std::borrow::Cow;

use super::excel_digits::fifteen_digit_literal;
use super::functions::{calendar_date_exists, serial_from_calendar_date};
use super::locale::Locale;
use crate::DateSystem;

/// The magnitude from which text reads as no number: `9.99999999999999E+307` reads, `1E+308`
/// does not.
const MAX_MAGNITUDE: f64 = 1e308;
/// Seconds in a day.
const DAY_SECONDS: f64 = 86_400.0;

/// The words, in lowercase, Excel may read within a number, a date, or a time: an exponent, the
/// halves of the day, and the English month names and their abbreviations.
const NUMBER_WORDS: [&str; 29] = [
    "e",
    "a",
    "p",
    "am",
    "pm",
    "jan",
    "feb",
    "mar",
    "apr",
    "may",
    "jun",
    "jul",
    "aug",
    "sep",
    "sept",
    "oct",
    "nov",
    "dec",
    "january",
    "february",
    "march",
    "april",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// How text reads as a number: the locale's conventions and the workbook's date system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::calculation) struct NumericText {
    locale: Locale,
    date_system: DateSystem,
}

impl NumericText {
    pub(in crate::calculation) const fn new(locale: Locale, date_system: DateSystem) -> Self {
        Self {
            locale,
            date_system,
        }
    }

    /// The number `text` stands for, or `None` when it stands for none.
    ///
    /// Spaces around the text are ignored; other whitespace is not. A number is a decimal with
    /// optional thousands separators and exponent (`1,234.5`, `.5`, `1e5`), or a whole number and
    /// a fraction (`1 1/2`). It may carry one sign, the currency symbol, a percent sign before or
    /// after it, or parentheses that negate it, but not both a currency symbol and a percent sign
    /// or both a sign and parentheses. Digits past the fifteenth significant one read as zeros, as
    /// in a formula. Otherwise the text is read as [`Self::date_time`] reads it.
    pub(in crate::calculation) fn number(self, text: &str) -> Option<f64> {
        let text = text.trim_matches(' ');
        signed_number(text, self.locale.currency_symbol()).or_else(|| self.read_date_time(text))
    }

    /// Whether Excel may read `text`, which [`Self::number`] leaves unread, as a number: text that
    /// holds a digit and nothing but the characters of numbers, dates, and times (`1/2`, `12:99`,
    /// `1,0000`, `2:30 p`) or month names (`Jan 2, 2024`, `15-Jan-2024`). Excel reads those as
    /// numbers (`TEXT("1/2","mm-dd")` is `01-02`, Mac Excel 16.114) while it keeps `Jan`, `abc`,
    /// and `2024년 1월 15일` as text, so a caller that would show such text as written reports it as
    /// unsupported instead.
    pub(in crate::calculation) fn may_read_otherwise(self, text: &str) -> bool {
        let text = text.trim_matches(' ');
        if !text.bytes().any(|byte| byte.is_ascii_digit()) {
            return false;
        }
        let currency = self.locale.currency_symbol();
        let mut rest = text;
        while let Some(character) = rest.chars().next() {
            if character.is_ascii_alphabetic() {
                let end = rest
                    .find(|character: char| !character.is_ascii_alphabetic())
                    .unwrap_or(rest.len());
                let word = rest[..end].to_ascii_lowercase();
                if !NUMBER_WORDS.contains(&word.as_str()) {
                    return false;
                }
                rest = &rest[end..];
            } else if let Some(after) = rest.strip_prefix(currency) {
                rest = after;
            } else if character.is_ascii_digit() || " ,.:/-+%()".contains(character) {
                rest = &rest[1..];
            } else {
                return false;
            }
        }
        true
    }

    /// The serial of a date, a time of day, or a date and a time, as `DATEVALUE` and `TIMEVALUE`
    /// read their text; numbers in other forms are `None`.
    ///
    /// A date is a four-digit year, month, and day (`2024-01-15`, `2024/1/5`), or where the locale
    /// writes the month first, a month, day, and two- or four-digit year (`1/15/2024`); `-` and
    /// `/` separate its parts. A two-digit year below 30 is in the 2000s, otherwise in the 1900s.
    /// A time is hours and minutes with optional seconds and fraction (`14:30:15.5`), read to the
    /// millisecond, or hours alone before `AM` or `PM`; `AM` and `PM` follow a space and take
    /// hours up to 12, and hours without them may pass 24 (`25:00`). A time may follow a date
    /// after spaces.
    pub(in crate::calculation) fn date_time(self, text: &str) -> Option<f64> {
        self.read_date_time(text.trim_matches(' '))
    }

    fn read_date_time(self, text: &str) -> Option<f64> {
        if let Some(time) = time_of_day(text) {
            return Some(time);
        }
        let (date, time) = text.split_once(' ').map_or((text, None), |(date, time)| {
            (date, Some(time.trim_start_matches(' ')))
        });
        let serial = self.date(date)?;
        match time {
            Some(time) => Some(serial + time_of_day(time)?),
            None => Some(serial),
        }
    }

    fn date(self, text: &str) -> Option<f64> {
        let mut parts = text.split(['-', '/']);
        let (first, second, third) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        let (year, month, day) = if first.len() == 4 {
            (digits(first, 4)?, digits(second, 2)?, digits(third, 2)?)
        } else if self.locale.writes_month_first() {
            let year = match third.len() {
                4 => digits(third, 4)?,
                1 | 2 => two_digit_year(digits(third, 2)?),
                _ => return None,
            };
            (year, digits(first, 2)?, digits(second, 2)?)
        } else {
            return None;
        };
        let year = i32::try_from(year).ok()?;
        if !calendar_date_exists(year, month, day) {
            return None;
        }
        serial_from_calendar_date(year, month, day, self.date_system)
    }
}

/// A number with its sign, currency symbol, percent sign, and parentheses, as Mac Excel 16.114
/// reads them: spaces may follow each of them and precede a trailing percent sign or a closing
/// parenthesis (`- 5`, `% 5`, `( 5 )`, `₩ -5`), and the percent sign may stand inside the
/// parentheses (`(5%)`).
fn signed_number(text: &str, currency: &str) -> Option<f64> {
    let (mut rest, mut percent) = match text.strip_suffix('%') {
        Some(rest) => (rest.trim_end_matches(' '), true),
        None => (text, false),
    };
    let mut percent_before = false;
    let mut negative = false;
    let mut signed = false;
    let mut has_currency = false;
    let mut parenthesized = false;
    loop {
        if !has_currency && let Some(after) = rest.strip_prefix(currency) {
            has_currency = true;
            rest = after;
        } else if !signed
            && !parenthesized
            && let Some(after) = rest.strip_prefix(['+', '-'])
        {
            signed = true;
            negative = rest.starts_with('-');
            rest = after;
        } else if !percent && let Some(after) = rest.strip_prefix('%') {
            percent = true;
            percent_before = true;
            rest = after;
        } else if !signed
            && !parenthesized
            && !percent_before
            && let Some(after) = rest.strip_prefix('(')
        {
            // `(5)`, `(₩5)`, and `₩(5)` read; a parenthesis after a sign or a percent sign does not.
            parenthesized = true;
            rest = after.strip_suffix(')')?.trim_end_matches(' ');
            if !percent && let Some(inner) = rest.strip_suffix('%') {
                percent = true;
                rest = inner.trim_end_matches(' ');
            }
        } else {
            break;
        }
        rest = rest.trim_start_matches(' ');
    }
    if has_currency && percent {
        return None;
    }
    let magnitude = match rest.split_once(' ') {
        Some((whole, fraction)) => mixed_number(whole, fraction, percent)?,
        None => decimal(rest, percent)?,
    };
    let value = if negative || parenthesized {
        -magnitude
    } else {
        magnitude
    };
    // Negative zero reads as zero.
    Some(value + 0.0)
}

/// A whole number and a fraction separated by one space, such as `1 1/2`.
fn mixed_number(whole: &str, fraction: &str, percent: bool) -> Option<f64> {
    let whole = grouped_digits(whole).filter(|whole| !whole.is_empty())?;
    let (numerator, denominator) = fraction.split_once('/')?;
    if !is_digits(numerator) || !is_digits(denominator) {
        return None;
    }
    let denominator = denominator.parse::<f64>().ok()?;
    if denominator == 0.0 {
        return None;
    }
    // The whole part keeps fifteen significant digits, as a literal does:
    // `VALUE("1234567890123456 1/2")` is `1234567890123450.5`.
    let whole = fifteen_digit_literal(&whole).parse::<f64>().ok()?;
    let mut value = whole + numerator.parse::<f64>().ok()? / denominator;
    if percent {
        value /= 100.0;
    }
    in_range(value, value != 0.0)
}

/// A decimal with optional thousands separators and exponent. A percent sign moves the decimal
/// point, so `1.1%` reads as `0.011` rather than `1.1 / 100`.
fn decimal(text: &str, percent: bool) -> Option<f64> {
    let (mantissa, exponent) = match text.find(['e', 'E']) {
        Some(index) => (&text[..index], Some(&text[index + 1..])),
        None => (text, None),
    };
    let (integer, fraction) = mantissa
        .split_once('.')
        .map_or((mantissa, None), |(integer, fraction)| {
            (integer, Some(fraction))
        });
    let integer = grouped_digits(integer)?;
    let fraction = fraction.unwrap_or("");
    if !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || (integer.is_empty() && fraction.is_empty())
    {
        return None;
    }
    let mut exponent = match exponent {
        Some(exponent) => {
            let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
            if !is_digits(digits) {
                return None;
            }
            exponent.parse::<i32>().ok()?
        }
        None => 0,
    };
    let nonzero = integer
        .bytes()
        .chain(fraction.bytes())
        .any(|byte| matches!(byte, b'1'..=b'9'));
    let value = match integer {
        // Text without separators or a percent sign is already a Rust float literal.
        Cow::Borrowed(_) if !percent => fifteen_digit_literal(text).parse::<f64>().ok()?,
        _ => {
            if percent {
                exponent = exponent.checked_sub(2)?;
            }
            let digits = format!("{integer}.{fraction}");
            format!("{}e{exponent}", fifteen_digit_literal(&digits))
                .parse::<f64>()
                .ok()?
        }
    };
    in_range(value, nonzero)
}

/// Digits with optional thousands separators, which must follow a first group that does not
/// begin with zero and separate groups of three (`1,234`, `1234,567`), returned without them.
fn grouped_digits(text: &str) -> Option<Cow<'_, str>> {
    if !text.contains(',') {
        return text
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then_some(Cow::Borrowed(text));
    }
    let mut groups = text.split(',');
    let first = groups.next()?;
    if !is_digits(first) || first.starts_with('0') {
        return None;
    }
    let mut joined = first.to_owned();
    for group in groups {
        if group.len() != 3 || !is_digits(group) {
            return None;
        }
        joined.push_str(group);
    }
    Some(Cow::Owned(joined))
}

/// `value` when text may read as it: finite, below [`MAX_MAGNITUDE`], and neither subnormal nor
/// a nonzero number that underflowed to zero.
fn in_range(value: f64, nonzero: bool) -> Option<f64> {
    let magnitude = value.abs();
    let readable = magnitude < MAX_MAGNITUDE
        && (magnitude >= f64::MIN_POSITIVE || (magnitude == 0.0 && !nonzero));
    readable.then_some(value)
}

/// A time of day, as a fraction of a day that may pass one.
fn time_of_day(text: &str) -> Option<f64> {
    let (clock, afternoon) = match text.rsplit_once(' ') {
        Some((clock, marker)) if marker.eq_ignore_ascii_case("AM") => (clock, Some(false)),
        Some((clock, marker)) if marker.eq_ignore_ascii_case("PM") => (clock, Some(true)),
        _ => (text, None),
    };
    let clock = clock.trim_end_matches(' ');
    let mut fields = clock.split(':');
    let hour = digits(fields.next()?, 4)?;
    let minute = fields.next();
    let second = fields.next();
    if fields.next().is_some() || (minute.is_none() && afternoon.is_none()) {
        return None;
    }
    let minute = match minute {
        Some(minute) => digits(minute, 2).filter(|minute| *minute < 60)?,
        None => 0,
    };
    let (second, milliseconds) = match second {
        Some(second) => seconds(second)?,
        None => (0, 0),
    };
    let hour = match afternoon {
        Some(_) if hour > 12 => return None,
        Some(afternoon) => hour % 12 + if afternoon { 12 } else { 0 },
        None => hour,
    };
    let whole_seconds = f64::from(hour * 3_600 + minute * 60 + second);
    Some((whole_seconds + f64::from(milliseconds) / 1_000.0) / DAY_SECONDS)
}

/// Seconds below 60 with an optional fraction, as whole seconds and the fraction rounded half up
/// to milliseconds, which may carry into the seconds (`1.9999` is two seconds).
fn seconds(text: &str) -> Option<(u32, u32)> {
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let whole = digits(whole, 2).filter(|whole| *whole < 60)?;
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut milliseconds = 0;
    for (place, byte) in fraction.bytes().take(3).enumerate() {
        milliseconds += u32::from(byte - b'0') * 10_u32.pow(2 - place as u32);
    }
    if fraction.as_bytes().get(3).is_some_and(|byte| *byte >= b'5') {
        milliseconds += 1;
    }
    Some((whole + milliseconds / 1_000, milliseconds % 1_000))
}

/// The value of one to `most` digits.
fn digits(text: &str, most: usize) -> Option<u32> {
    if text.len() > most || !is_digits(text) {
        return None;
    }
    text.parse().ok()
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// The year a two-digit year stands for: below 30 in the 2000s, otherwise in the 1900s.
const fn two_digit_year(year: u32) -> u32 {
    if year < 30 {
        2_000 + year
    } else {
        1_900 + year
    }
}

#[cfg(test)]
mod tests {
    use super::NumericText;
    use crate::DateSystem;
    use crate::calculation::locale::Locale;

    const US: NumericText = NumericText::new(Locale::EnUs, DateSystem::Excel1900);
    const KOREA: NumericText = NumericText::new(Locale::KoKr, DateSystem::Excel1900);

    #[test]
    fn reads_numbers_in_the_forms_excel_accepts() {
        for (text, expected) in [
            (" 5 ", 5.0),
            ("+5", 5.0),
            (".5", 0.5),
            ("5.", 5.0),
            ("1e5", 1e5),
            ("1E+5", 1e5),
            ("5e0003", 5e3),
            ("00005", 5.0),
            ("-0", 0.0),
            ("1,234", 1_234.0),
            ("1,234,567", 1_234_567.0),
            ("1234,567", 1_234_567.0),
            ("1,234.", 1_234.0),
            ("1,000e3", 1e6),
            ("(5)", -5.0),
            ("( 5 )", -5.0),
            ("(5)%", -0.05),
            ("(%5)", -0.05),
            ("50%", 0.5),
            ("50 %", 0.5),
            ("%5", 0.05),
            ("-5%", -0.05),
            ("%-5", -0.05),
            ("1.1%", 0.011),
            ("5e3%", 50.0),
            ("$5", 5.0),
            ("$ 5", 5.0),
            ("-$5", -5.0),
            ("$-5", -5.0),
            ("($5)", -5.0),
            ("$(5)", -5.0),
            ("$1,000", 1_000.0),
            ("$.5", 0.5),
            ("1 1/2", 1.5),
            ("0 1/2", 0.5),
            ("-0 1/2", -0.5),
            ("+1 1/2", 1.5),
            ("1 3/2", 2.5),
            ("1,000 1/2", 1_000.5),
            ("(1 1/2)", -1.5),
            ("(1 1/2%)", -0.015),
            ("(5%)", -0.05),
            ("( 5% )", -0.05),
            ("(% 5)", -0.05),
            ("(1,000%)", -10.0),
            ("- 5", -5.0),
            ("+ 5", 5.0),
            ("% 5", 0.05),
            ("% -5", -0.05),
            ("- 5%", -0.05),
            ("$ -5", -5.0),
            ("- $5", -5.0),
            ("$( 5)", -5.0),
            ("- 1 1/2", -1.5),
            ("1234567890123456 1/2", 1_234_567_890_123_450.5),
            ("1 1/2%", 0.015),
            ("$1 1/2", 1.5),
            ("1234567890123456", 1_234_567_890_123_450.0),
            ("1.2345678901234567", 1.234_567_890_123_45),
            ("0.30000000000000004", 0.3),
            ("9.999999999999999E+307", 9.999_999_999_999_99e307),
            ("2.3E-308", 2.3e-308),
        ] {
            assert_eq!(US.number(text), Some(expected), "{text:?}");
        }
        assert_eq!(US.number("-0").map(f64::is_sign_negative), Some(false));
    }

    #[test]
    fn leaves_forms_excel_rejects_or_reads_from_the_current_date_unread() {
        for text in [
            "", " ", ".", "-", "+", "%", "()", "$", "1e", "e5", "5e+", "--5", "+-5", "-+5", "5-",
            "5+", "(-5)", "-(5)", "%(5)", "5%%", "%5%", "5 0", "5$", "$5%", "%$5", "$%5", "$$5",
            "1,2", "12,34", "1,23,456", ",123", "0,001", "1e3,000", "1, 234", "1.5,000", "inf",
            "nan", "Infinity", "1e400", "5e-400", "1E+308", "0x10", "1_000", "５", "TRUE", "1.2.3",
            "\u{a0}5", "5\u{a0}", "\t5", "5\n", "1 1/0", "1  1/2", "1 1.5/2", "1 1/2.5", "1e2 1/2",
            "1.5 1/2", "1 -1/2", "1/2", "5-3", "€5", "₩5", "(5%%)", "-(5%)", "(-5%)", "$5 %",
            "$(5%)", "5 -",
        ] {
            assert_eq!(US.number(text), None, "{text:?}");
        }
    }

    #[test]
    fn reads_times_of_day() {
        let time = |hours: f64, minutes: f64, seconds: f64| {
            (hours * 3_600.0 + minutes * 60.0 + seconds) / 86_400.0
        };
        for (text, expected) in [
            ("12:30", time(12.0, 30.0, 0.0)),
            ("1:2", time(1.0, 2.0, 0.0)),
            ("14:30:15", time(14.0, 30.0, 15.0)),
            ("14:30:15.5", time(14.0, 30.0, 15.5)),
            ("0:0:59.9", time(0.0, 0.0, 59.9)),
            ("2:30 PM", time(14.0, 30.0, 0.0)),
            ("2:30 pm", time(14.0, 30.0, 0.0)),
            ("12:30:15 PM", time(12.0, 30.0, 15.0)),
            ("2:30 AM", time(2.0, 30.0, 0.0)),
            ("12:30 AM", time(0.0, 30.0, 0.0)),
            ("2 PM", time(14.0, 0.0, 0.0)),
            ("12 PM", 0.5),
            ("0 PM", 0.5),
            ("12 AM", 0.0),
            ("25:00", 25.0 / 24.0),
            ("9999:59", time(9_999.0, 59.0, 0.0)),
            ("  12:30", time(12.0, 30.0, 0.0)),
        ] {
            assert_eq!(US.number(text), Some(expected), "{text:?}");
        }
        assert_eq!(
            US.number("0:0:1.0025"),
            Some((1.0 + 3.0 / 1_000.0) / 86_400.0)
        );
        assert_eq!(
            US.number("0:0:1.2344"),
            Some((1.0 + 234.0 / 1_000.0) / 86_400.0)
        );
        assert_eq!(US.number("0:0:1.9999"), Some(2.0 / 86_400.0));
        for text in [
            "2:30PM",
            "2PM",
            "13:30 PM",
            "13 AM",
            "10000:00",
            "12:",
            ":30",
            "1:2:3:4",
            "12:60",
            "0:0:60",
            "1.5:30",
            "-1:30",
            "+1:30",
            "(1:30)",
            "1:30%",
            "$1:30",
            "12:30 A.M.",
        ] {
            assert_eq!(US.number(text), None, "{text:?}");
        }
    }

    #[test]
    fn reads_dates_in_the_order_the_locale_writes_them() {
        for (text, expected) in [
            ("2024-01-15", 45_306.0),
            ("2024-1-5", 45_296.0),
            ("2024/01/15", 45_306.0),
            ("2024-1/15", 45_306.0),
            ("1/15/2024", 45_306.0),
            ("01/05/2024", 45_296.0),
            ("1-15-2024", 45_306.0),
            ("1/15/24", 45_306.0),
            ("1/15/29", 47_133.0),
            ("1/15/30", 10_973.0),
            ("1/15/0", 36_540.0),
            ("2/29/2024", 45_351.0),
            ("1900-01-01", 1.0),
            ("1900-02-29", 60.0),
            ("9999-12-31", 2_958_465.0),
        ] {
            assert_eq!(US.number(text), Some(expected), "{text:?}");
        }
        for text in [
            "15/1/2024",
            "13/1/2024",
            "1/32/2024",
            "2/29/2023",
            "2024-02-30",
            "2024-13-01",
            "1899-12-31",
            "10000-01-01",
            "02024-01-15",
            "2024-001-15",
            "2024-01-015",
            "24-01-15",
            "1/15/024",
            "1/15/100",
            "2024.01.15",
            "2024-01-15T12:30",
            "(2024-01-15)",
            "-2024-01-15",
            "2024-01-15%",
        ] {
            assert_eq!(US.number(text), None, "{text:?}");
        }
        assert_eq!(KOREA.number("2024-01-15"), Some(45_306.0));
        assert_eq!(KOREA.number("1/15/2024"), None);
        assert_eq!(KOREA.number("₩5"), Some(5.0));
        assert_eq!(KOREA.number("$5"), None);
        let system_1904 = NumericText::new(Locale::EnUs, DateSystem::Excel1904);
        assert_eq!(system_1904.number("1904-01-02"), Some(1.0));
        assert_eq!(system_1904.number("1903-12-31"), None);
    }

    #[test]
    fn reads_a_date_followed_by_a_time() {
        let at =
            |hours: f64, minutes: f64| 45_306.0 + (hours * 3_600.0 + minutes * 60.0) / 86_400.0;
        for (text, expected) in [
            ("2024-01-15 12:30", at(12.0, 30.0)),
            ("2024-01-15  12:30", at(12.0, 30.0)),
            ("2024-01-15 2:30 PM", at(14.0, 30.0)),
            ("2024-1-15 1 PM", at(13.0, 0.0)),
            ("1/15/2024 12:30", at(12.0, 30.0)),
            ("2024-01-15 25:00", at(25.0, 0.0)),
        ] {
            assert_eq!(US.number(text), Some(expected), "{text:?}");
        }
        assert_eq!(US.number("2024-01-15 2:30PM"), None);
        assert_eq!(US.number("12:30 2024-01-15"), None);
    }

    #[test]
    fn date_time_reads_only_dates_and_times() {
        assert_eq!(US.date_time(" 2024-01-15 "), Some(45_306.0));
        assert_eq!(US.date_time("12:30"), Some(45_000.0 / 86_400.0));
        for text in ["5", "1 1/2", "50%", "$5"] {
            assert_eq!(US.date_time(text), None, "{text:?}");
        }
    }
}
