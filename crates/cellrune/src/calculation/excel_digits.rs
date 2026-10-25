//! Excel's fifteen-significant-digit rules for comparing numbers and for rounding a number to a
//! decimal place or a multiple.
//!
//! A double such as `1.005` stands for a decimal its binary value only approximates, so scaling it
//! for rounding can leave a residue on the wrong side of a rounding boundary: `1.005 * 100` is
//! `100.49999999999999`. Excel reads the scaled quotient to fifteen significant digits before it
//! rounds, which turns that residue back into the `100.5` it stands for. Comparison likewise treats
//! `0.1 + 0.2` and `0.3`, which differ only past the fifteenth digit, as equal.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt::Write as _;

/// The `Display` text of a finite double, written into a fixed buffer.
struct DecimalText {
    bytes: [u8; 340],
    len: usize,
}

impl Default for DecimalText {
    fn default() -> Self {
        Self {
            bytes: [0; 340],
            len: 0,
        }
    }
}

impl DecimalText {
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).expect("formatted ASCII")
    }
}

impl std::fmt::Write for DecimalText {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = self.len + text.len();
        self.bytes
            .get_mut(self.len..end)
            .ok_or(std::fmt::Error)?
            .copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// The magnitude from which a quotient has no digit below its fifteenth significant one.
const FIFTEEN_DIGIT_QUOTIENT: f64 = 1e15;

/// How a scaled quotient becomes a whole number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Rounding {
    /// Half away from zero, as `ROUND` and `MROUND` round.
    Nearest,
    TowardZero,
    AwayFromZero,
    Down,
    Up,
}

impl Rounding {
    fn apply(self, quotient: f64) -> f64 {
        match self {
            Self::Nearest => quotient.round(),
            Self::TowardZero => quotient.trunc(),
            Self::AwayFromZero if quotient.is_sign_negative() => quotient.floor(),
            Self::AwayFromZero | Self::Up => quotient.ceil(),
            Self::Down => quotient.floor(),
        }
    }
}

/// Rounds `quotient` to a whole number. With `decimal`, a quotient below `1e15` is first read to
/// fifteen significant digits; otherwise the binary value is rounded as it is.
pub(super) fn round_quotient(quotient: f64, rounding: Rounding, decimal: bool) -> f64 {
    let quotient = if decimal && quotient.abs() < FIFTEEN_DIGIT_QUOTIENT {
        fifteen_significant_digits(quotient)
    } else {
        quotient
    };
    rounding.apply(quotient)
}

/// Rounds `value` at `digits` decimal places, or at `10^-digits` when `digits` is negative.
///
/// With `decimal`, a place past the fifteenth significant digit leaves the value as it is, as
/// Excel returns `ROUND(1, 400)`, `ROUND(1E307, 2)`, and `ROUND(1.23456789012345E+20, -2)`
/// unchanged. Without it only a place past the
/// range of a double does. Returns `None` when the rounded value is not a finite double.
pub(super) fn round_to_digits(
    value: f64,
    digits: i32,
    rounding: Rounding,
    decimal: bool,
) -> Option<f64> {
    let scale = 10_f64.powi(digits.saturating_abs());
    if digits >= 0 {
        let quotient = value * scale;
        if !quotient.is_finite() || (decimal && quotient.abs() >= FIFTEEN_DIGIT_QUOTIENT) {
            return Some(value);
        }
        return Some(round_quotient(quotient, rounding, decimal) / scale);
    }
    if !scale.is_finite() {
        // Every finite double is below half of 10^309, so it rounds to zero unless the rounding
        // moves it away from zero, which reaches a place no double holds.
        return match rounding {
            Rounding::AwayFromZero if value != 0.0 => None,
            Rounding::Up if value > 0.0 => None,
            Rounding::Down if value < 0.0 => None,
            _ => Some(0.0),
        };
    }
    let quotient = value / scale;
    if decimal && quotient.abs() >= FIFTEEN_DIGIT_QUOTIENT {
        return Some(value);
    }
    let rounded = round_quotient(quotient, rounding, decimal) * scale;
    rounded.is_finite().then_some(rounded)
}

/// Returns the result of an addition or subtraction Excel corrects to zero when it cancels: a
/// result smaller than eight units in the last place of the larger operand's binary exponent,
/// `2^(e-49)` for a larger operand in `[2^e, 2^(e+1))`, is `0`. So `0.1+0.2-0.3` is `0`, as are
/// `1+7*2^-52-1` and `1E+15+0.3-1E+15`, while `1+8*2^-52-1` and `100.1-100-0.1` keep their residue
/// (Excel 16.0 build 20326).
pub(super) fn cancel_near_zero(left: f64, right: f64, result: f64) -> f64 {
    let larger = left.abs().max(right.abs());
    if !larger.is_normal() || !result.is_finite() {
        return result;
    }
    let binade = f64::from_bits(larger.to_bits() & 0xfff0_0000_0000_0000);
    if result.abs() < binade * TWO_POW_MINUS_49 {
        0.0
    } else {
        result
    }
}

/// `2^-49`: eight units in the last place of a number in `[1, 2)`.
const TWO_POW_MINUS_49: f64 = 1.0 / 562_949_953_421_312.0;

/// Whether two numbers are equal once each is rounded to fifteen significant digits, as Excel
/// compares them: `=1=1+4E-15` and `=0.1+0.2=0.3` are `TRUE` while `=1+4E-15=1+6E-15` and
/// `=5=5+5E-15` are `FALSE` (Mac Excel 16.114). Numbers of different signs and a zero compare
/// exactly.
pub(super) fn approximately_equal(left: f64, right: f64) -> bool {
    if left == right {
        return true;
    }
    if left == 0.0 || right == 0.0 || left.is_sign_negative() != right.is_sign_negative() {
        return false;
    }
    // Numbers equal to fifteen digits differ by less than a unit in their fifteenth digit, at
    // most `1e-14` of the larger magnitude.
    let larger = left.abs().max(right.abs());
    if (left - right).abs() >= larger * 1e-14 {
        return false;
    }
    fifteen_significant_digits(left) == fifteen_significant_digits(right)
}

/// Orders two numbers as the comparison operators do: `Equal` when they are approximately equal,
/// otherwise by value.
pub(super) fn approximate_ordering(left: f64, right: f64) -> Ordering {
    if approximately_equal(left, right) {
        Ordering::Equal
    } else {
        left.partial_cmp(&right).unwrap_or(Ordering::Equal)
    }
}

/// The longest plain decimal, sign excluded, a number converts to before it is written in
/// scientific notation: `1E+19` converts to twenty digits and `1E+20` to `1E+20`, `1.2345E-14` to
/// `0.000000000000012345` and `1.23456E-14` to `1.23456E-14` (Excel 16.0 build 20326).
const GENERAL_TEXT_WIDTH: usize = 20;

/// The text Excel converts a number to when it joins text (`&`) or reaches a text argument: the
/// number rounded to fifteen significant digits, as a plain decimal when that takes at most
/// twenty characters besides the sign, otherwise in scientific notation with a signed exponent of
/// at least two digits (`3.14159265358979E-05`).
pub(crate) fn general_text(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    if !value.is_finite() {
        return format!("{value}");
    }
    let text = fifteen_digit_text(value.abs());
    let (mantissa, exponent) = text
        .as_str()
        .split_once('e')
        .expect("scientific formatting writes an exponent");
    let exponent: i32 = exponent.parse().expect("formatted exponent");
    let digits = mantissa.replace('.', "");
    let digits = digits.trim_end_matches('0');
    let sign = if value < 0.0 { "-" } else { "" };
    let plain = plain_decimal(digits, exponent);
    if plain.len() <= GENERAL_TEXT_WIDTH {
        return format!("{sign}{plain}");
    }
    let (first, rest) = digits.split_at(1);
    let point = if rest.is_empty() { "" } else { "." };
    let exponent_sign = if exponent < 0 { '-' } else { '+' };
    format!(
        "{sign}{first}{point}{rest}E{exponent_sign}{:02}",
        exponent.unsigned_abs()
    )
}

/// Writes the significant `digits` of a number whose first digit stands at `10^exponent`.
fn plain_decimal(digits: &str, exponent: i32) -> String {
    let places = usize::try_from(exponent.unsigned_abs()).unwrap_or(usize::MAX);
    if exponent < 0 {
        return format!("0.{}{digits}", "0".repeat(places - 1));
    }
    let whole = places + 1;
    if digits.len() <= whole {
        format!("{digits}{}", "0".repeat(whole - digits.len()))
    } else {
        format!("{}.{}", &digits[..whole], &digits[whole..])
    }
}

/// Returns a numeric literal as Excel reads it: digits past the fifteenth significant one become
/// zeros, so `1234567890123456` reads as `1234567890123450` and `0.30000000000000004` as `0.3`.
pub(crate) fn fifteen_digit_literal(literal: &str) -> Cow<'_, str> {
    let mantissa_end = literal.find(['e', 'E']).unwrap_or(literal.len());
    let (mantissa, exponent) = literal.split_at(mantissa_end);
    let mut significant = 0;
    let mut excess = false;
    for byte in mantissa.bytes().filter(u8::is_ascii_digit) {
        if significant > 0 || byte != b'0' {
            significant += 1;
            excess |= significant > EXCEL_SIGNIFICANT_DIGITS;
        }
    }
    if !excess {
        return Cow::Borrowed(literal);
    }
    let point = mantissa.find('.').unwrap_or(mantissa.len());
    let mut kept = String::with_capacity(literal.len());
    let mut significant = 0;
    for (index, character) in mantissa.char_indices() {
        if !character.is_ascii_digit() {
            kept.push(character);
            continue;
        }
        if significant > 0 || character != '0' {
            significant += 1;
        }
        if significant <= EXCEL_SIGNIFICANT_DIGITS {
            kept.push(character);
        } else if index < point {
            kept.push('0');
        }
    }
    kept.push_str(exponent);
    Cow::Owned(kept)
}

/// The significant digits Excel keeps of a number.
const EXCEL_SIGNIFICANT_DIGITS: usize = 15;

/// Writes a finite `value` in scientific notation with fifteen significant digits, rounded to
/// the nearest and a tie toward zero, as Excel rounds: `(1234567890123450+5)&""` is
/// `1234567890123450` and `ROUND(1234567890123470+5,-1)` is `1234567890123470` (Mac Excel
/// 16.114). Rust rounds a tie to even, which moves it away from zero only to an even fifteenth
/// digit, so such a result is checked against the exact value.
fn fifteen_digit_text(value: f64) -> DecimalText {
    let mut text = DecimalText::default();
    write!(text, "{value:.14e}").expect("a finite double fits the buffer");
    if !(EXACT_TIE_MAGNITUDES.contains(&value.abs())) {
        return text;
    }
    let last = text
        .as_str()
        .find('e')
        .expect("scientific formatting writes an exponent")
        - 1;
    if (text.bytes[last] - b'0') % 2 == 1 {
        return text;
    }
    let mut sixteen = DecimalText::default();
    write!(sixteen, "{value:.15e}").expect("a finite double fits the buffer");
    let exponent_start = sixteen
        .as_str()
        .find('e')
        .expect("scientific formatting writes an exponent");
    if sixteen.bytes[exponent_start - 1] != b'5' {
        return text;
    }
    // The tie is exact only when no digit past the sixteenth is nonzero; a double in the range of
    // exact ties has at most 97 significant digits.
    let mut exact = DecimalText::default();
    write!(exact, "{value:.100e}").expect("a finite double fits the buffer");
    let (digits, _) = exact
        .as_str()
        .split_once('e')
        .expect("scientific formatting writes an exponent");
    // `d.` and the fifteen digits after the point come first.
    if digits
        .trim_start_matches('-')
        .bytes()
        .skip(17)
        .any(|digit| digit != b'0')
    {
        return text;
    }
    let mut toward_zero = DecimalText::default();
    let (mantissa, exponent) = sixteen.as_str().split_at(exponent_start);
    write!(toward_zero, "{}{exponent}", &mantissa[..mantissa.len() - 1])
        .expect("the digits fit the buffer");
    toward_zero
}

/// The magnitudes a double can hold an exact sixteen-digit tie in. A tie is `N·10^q` with `N` a
/// sixteen-digit odd multiple of 5: below `10^-7` it needs `5^-q` to divide `N`, which no `q`
/// below -22 allows, and from `10^17` its odd part passes 2^53. Such a double has at most 79
/// binary places after the point, so 97 significant digits write it exactly.
const EXACT_TIE_MAGNITUDES: std::ops::Range<f64> = 1e-8..1e17;

/// The numbers that round to the same fifteen significant digits as a number, and so are
/// [`approximately_equal`] to it, as one interval of doubles: computed once, it compares a
/// criterion with many cells without writing each cell's digits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FifteenDigitBucket {
    low: f64,
    high: f64,
}

impl FifteenDigitBucket {
    pub(super) fn of(value: f64) -> Self {
        if value == 0.0 || !value.is_finite() {
            return Self {
                low: value,
                high: value,
            };
        }
        let rounded = fifteen_significant_digits(value);
        let same = |other: f64| fifteen_significant_digits(other) == rounded;
        // The midpoints next to the fifteen-digit value bound the interval; the doubles nearest
        // to them are then moved to the last ones that round to it.
        let text = fifteen_digit_text(value.abs());
        let (mantissa, exponent) = text
            .as_str()
            .split_once('e')
            .expect("scientific formatting writes an exponent");
        let units: u64 = mantissa.replace('.', "").parse().expect("formatted digits");
        let exponent: i32 = exponent.parse().expect("formatted exponent");
        let midpoint = |numerator: u64| -> f64 {
            format!("{numerator}e{}", exponent - 15)
                .parse::<f64>()
                .expect("formatted midpoint")
                .copysign(value)
        };
        // Near the largest double a midpoint passes the range, as the rounding itself may.
        let finite = |bound: f64| bound.clamp(-f64::MAX, f64::MAX);
        let mut low = finite(midpoint(units * 10 - 5));
        let mut high = finite(midpoint(units * 10 + 5));
        if low > high {
            (low, high) = (high, low);
        }
        while !same(low) {
            low = low.next_up();
        }
        while low.next_down().is_finite() && same(low.next_down()) {
            low = low.next_down();
        }
        while !same(high) {
            high = high.next_down();
        }
        while high.next_up().is_finite() && same(high.next_up()) {
            high = high.next_up();
        }
        Self { low, high }
    }

    /// Whether `other` rounds to the same fifteen significant digits.
    pub(super) fn contains(self, other: f64) -> bool {
        self.low <= other && other <= self.high
    }

    /// The least and greatest numbers in the interval.
    pub(super) const fn bounds(self) -> (f64, f64) {
        (self.low, self.high)
    }
}

/// Returns the double nearest to `value` written with fifteen significant digits.
pub(super) fn fifteen_significant_digits(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    fifteen_digit_text(value).as_str().parse().unwrap_or(value)
}

/// The fifteen significant digits of a nonzero finite magnitude, as ASCII, and the power of ten
/// the first one stands at.
pub(super) fn fifteen_significant_decimal(magnitude: f64) -> ([u8; 15], i32) {
    let text = fifteen_digit_text(magnitude);
    let (mantissa, exponent) = text
        .as_str()
        .split_once('e')
        .expect("scientific formatting writes an exponent");
    let mut digits = [b'0'; EXCEL_SIGNIFICANT_DIGITS];
    for (digit, byte) in digits
        .iter_mut()
        .zip(mantissa.bytes().filter(u8::is_ascii_digit))
    {
        *digit = byte;
    }
    (digits, exponent.parse().expect("formatted exponent"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifteen_significant_digits_reads_binary_residues_as_their_decimals() {
        assert_eq!(fifteen_significant_digits(1.005 * 100.0), 100.5);
        assert_eq!(fifteen_significant_digits(0.3 / 0.1), 3.0);
        assert_eq!(fifteen_significant_digits((0.1 + 0.2) * 10.0), 3.0);
        assert_eq!(fifteen_significant_digits(-1.15 * 100.0), -115.0);
        assert_eq!(
            fifteen_significant_digits(123_456_789_012.345_67),
            123_456_789_012.346
        );
    }

    #[test]
    fn fifteen_digits_round_an_exact_tie_toward_zero() {
        assert_eq!(
            fifteen_significant_digits(1_234_567_890_123_455.0),
            1_234_567_890_123_450.0
        );
        assert_eq!(
            fifteen_significant_digits(-1_234_567_890_123_455.0),
            -1_234_567_890_123_450.0
        );
        assert_eq!(
            fifteen_significant_digits(123_456_789_012_347.5),
            123_456_789_012_347.0
        );
        assert_eq!(
            fifteen_significant_digits(1_234_567_890_123_456.0),
            1_234_567_890_123_460.0
        );
        assert_eq!(
            fifteen_significant_digits(2_f64.powi(60)),
            1_152_921_504_606_850_000.0
        );
        assert_eq!(general_text(1_234_567_890_123_475.0), "1234567890123470");
        assert_eq!(general_text(123_456_789_012_344.5), "123456789012344");
    }

    #[test]
    fn a_fifteen_digit_bucket_holds_exactly_the_approximately_equal_numbers() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut values = vec![
            0.3,
            0.1 + 0.2,
            1.0,
            1e15,
            999_999_999_999_999.6,
            1_234_567_890_123_455.0,
            1.234_567_890_123_464_5e-300,
            f64::MAX,
            -f64::MAX,
            f64::MIN_POSITIVE,
            f64::from_bits(1),
            -0.3,
        ];
        for _ in 0..2_000 {
            let bits = next();
            let value = f64::from_bits(bits);
            if value.is_finite() {
                values.push(value);
            }
        }
        for value in values {
            let bucket = FifteenDigitBucket::of(value);
            let (low, high) = bucket.bounds();
            for other in [
                value,
                low,
                high,
                low.next_down(),
                high.next_up(),
                value.next_up(),
                value.next_down(),
            ] {
                if other.is_finite() {
                    assert_eq!(
                        bucket.contains(other),
                        approximately_equal(other, value),
                        "{value:e} {other:e}"
                    );
                }
            }
        }
    }

    #[test]
    fn general_text_rounds_to_fifteen_digits_and_switches_to_scientific_past_twenty_characters() {
        for (value, text) in [
            (2.0 / 3.0, "0.666666666666667"),
            (-2.0 / 3.0, "-0.666666666666667"),
            (1.0 - 1e-16, "1"),
            (0.1 + 0.2, "0.3"),
            (-0.0, "0"),
            (1_234_567_890_123_458.0, "1234567890123460"),
            (9_007_199_254_740_998.0, "9007199254741000"),
            (1e15, "1000000000000000"),
            (1e19, "10000000000000000000"),
            (-1e19, "-10000000000000000000"),
            (1e20, "1E+20"),
            (1e300, "1E+300"),
            (9.999_999_999_999_99e20, "9.99999999999999E+20"),
            (1e-10, "0.0000000001"),
            (1.5e-17, "0.000000000000000015"),
            (-1.2345e-14, "-0.000000000000012345"),
            (1.23456e-14, "1.23456E-14"),
            (1e-19, "1E-19"),
            (1e-300, "1E-300"),
            (std::f64::consts::PI * 1e-5, "3.14159265358979E-05"),
            (std::f64::consts::PI * 1e15, "3141592653589790"),
            (1_234_567.890_123_45, "1234567.89012345"),
        ] {
            assert_eq!(general_text(value), text, "{value:e}");
        }
    }

    #[test]
    fn literals_keep_fifteen_significant_digits() {
        for (literal, read) in [
            ("1234567890123456", "1234567890123450"),
            ("0.30000000000000004", "0.300000000000000"),
            ("0.99999999999999999", "0.999999999999999"),
            ("123456789012345678.5e3", "123456789012345000.e3"),
            ("1.23456789012345678E+20", "1.23456789012345E+20"),
            ("0.000123", "0.000123"),
            ("123456789012345", "123456789012345"),
        ] {
            assert_eq!(fifteen_digit_literal(literal), read, "{literal}");
        }
    }

    #[test]
    fn near_zero_cancellation_stops_at_eight_units_in_the_last_place() {
        let unit = f64::EPSILON;
        assert_eq!(cancel_near_zero(0.1 + 0.2, 0.3, 0.1 + 0.2 - 0.3), 0.0);
        assert_eq!(cancel_near_zero(1.0 + 7.0 * unit, 1.0, 7.0 * unit), 0.0);
        assert_eq!(
            cancel_near_zero(1.0 + 8.0 * unit, 1.0, 8.0 * unit),
            8.0 * unit
        );
        assert_eq!(
            cancel_near_zero(3.0 + 4.0 * unit * 2.0, 3.0, 8.0 * unit),
            0.0
        );
        assert_eq!(cancel_near_zero(1e15 + 0.25, 1e15, 0.25), 0.0);
        let residue = 100.1 - 100.0 - 0.1;
        assert_eq!(cancel_near_zero(100.1 - 100.0, 0.1, residue), residue);
        assert_eq!(cancel_near_zero(0.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn approximate_equality_ignores_differences_past_the_fifteenth_digit() {
        assert!(approximately_equal(0.1 + 0.2, 0.3));
        assert!(approximately_equal(1.1 - 1.0, 0.1));
        assert!(approximately_equal(-(0.1 + 0.2), -0.3));
        assert!(!approximately_equal(0.3, 0.300_000_000_000_01));
        assert!(!approximately_equal(5.551_115_123_125_783e-17, 0.0));
        assert!(!approximately_equal(1e-300, -1e-300));
        assert!(approximately_equal(1e15 + 1.0, 1e15));
        assert!(!approximately_equal(1e14 + 1.0, 1e14));
        // Mac Excel 16.114 compares the fifteen-digit roundings.
        assert!(approximately_equal(1.0, 1.0 + 4e-15));
        assert!(!approximately_equal(1.0 + 4e-15, 1.0 + 6e-15));
        assert!(!approximately_equal(9.0, 9.0 + 3e-14));
        assert!(!approximately_equal(5.0, 5.0 + 5e-15));
        assert!(approximately_equal(2.0, 2.0 + 5e-15));
        assert!(!approximately_equal(0.3, 0.3 + 6e-16));
        assert!(approximately_equal(1e15, 1e15 + 4.0));
        assert!(!approximately_equal(1e15, 1e15 + 6.0));
        assert_eq!(approximate_ordering(0.1 + 0.2, 0.3), Ordering::Equal);
        assert_eq!(approximate_ordering(0.3, 0.31), Ordering::Less);
    }

    #[test]
    fn rounding_at_a_place_reads_the_scaled_quotient_to_fifteen_digits() {
        assert_eq!(
            round_to_digits(1.005, 2, Rounding::Nearest, true),
            Some(1.01)
        );
        assert_eq!(
            round_to_digits(1.005, 2, Rounding::Nearest, false),
            Some(1.0)
        );
        assert_eq!(
            round_to_digits(1.15, 2, Rounding::TowardZero, true),
            Some(1.15)
        );
        assert_eq!(
            round_to_digits(0.1 + 0.2, 1, Rounding::AwayFromZero, true),
            Some(0.3)
        );
        assert_eq!(
            round_to_digits(-2.5, 0, Rounding::Nearest, true),
            Some(-3.0)
        );
        assert_eq!(
            round_to_digits(1234.5, -2, Rounding::Nearest, true),
            Some(1200.0)
        );
    }

    #[test]
    fn places_beyond_the_digits_or_the_range_leave_or_zero_the_value() {
        assert_eq!(
            round_to_digits(1.0, 400, Rounding::Nearest, true),
            Some(1.0)
        );
        assert_eq!(
            round_to_digits(1e307, 2, Rounding::Nearest, true),
            Some(1e307)
        );
        assert_eq!(
            round_to_digits(1.0, 400, Rounding::Nearest, false),
            Some(1.0)
        );
        assert_eq!(
            round_to_digits(1.5, i32::MIN, Rounding::Nearest, true),
            Some(0.0)
        );
        assert_eq!(
            round_to_digits(1.5, i32::MIN, Rounding::TowardZero, true),
            Some(0.0)
        );
        assert_eq!(
            round_to_digits(1.5, i32::MIN, Rounding::AwayFromZero, true),
            None
        );
        assert_eq!(
            round_to_digits(0.0, i32::MIN, Rounding::AwayFromZero, true),
            Some(0.0)
        );
    }
}
