//! BAHTTEXT spells an amount in Thai words with the baht and satang suffixes.

use std::cmp::Ordering;

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::util::required_number;

const DIGITS: [&str; 10] = [
    "ศูนย์",
    "หนึ่ง",
    "สอง",
    "สาม",
    "สี่",
    "ห้า",
    "หก",
    "เจ็ด",
    "แปด",
    "เก้า",
];
const PLACES: [&str; 6] = ["", "สิบ", "ร้อย", "พัน", "หมื่น", "แสน"];
const MILLION: &str = "ล้าน";
const ONE_AFTER_HIGHER_DIGIT: &str = "เอ็ด";
const TWENTY_PREFIX: &str = "ยี่";
const BAHT: &str = "บาท";
const EXACT: &str = "ถ้วน";
const SATANG: &str = "สตางค์";
const NEGATIVE: &str = "ลบ";
/// Satang amounts at or above 2^53 are no longer whole numbers in a double.
const MAX_EXACT_SATANG: u64 = 9_007_199_254_740_992;

/// BAHTTEXT(number) reads the baht as Thai numerals (สิบ for ten, ยี่สิบ for twenty, เอ็ด for a
/// final one after any higher digit, and ล้าน for each million), then บาท, then ถ้วน for a whole
/// amount or the satang and สตางค์, as Microsoft's example, the Oracle, and Excel 16.114 for Mac
/// show: zero is ศูนย์บาทถ้วน, an amount below one baht has only its satang, and a negative amount
/// is prefixed with ลบ. Excel rounds to the nearest satang (1.235 is 24 satang); a half satang
/// whose direction depends on the rounding rule, and a negative amount that rounds to zero, are
/// unsupported rather than guessed.
pub(super) fn baht_text(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [argument] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let number = match required_number(engine, context, argument) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let Some(satang_total) = whole_satang(number.abs()).filter(|total| *total < MAX_EXACT_SATANG)
    else {
        return Value::Error(ErrorKind::Unsupported);
    };
    let (baht, satang) = (satang_total / 100, satang_total % 100);
    let mut text = String::new();
    if number < 0.0 {
        if satang_total == 0 {
            return Value::Error(ErrorKind::Unsupported);
        }
        text.push_str(NEGATIVE);
    }
    if baht > 0 || satang == 0 {
        text.push_str(&thai_number(baht));
        text.push_str(BAHT);
    }
    if satang == 0 {
        text.push_str(EXACT);
    } else {
        text.push_str(&thai_number(satang));
        text.push_str(SATANG);
    }
    engine.bounded_text(text)
}

/// The whole satang nearest to `amount` read to 15 significant digits, as Excel reads numbers. At
/// a half satang the exact binary value, rounding half up, and rounding half to even must agree on
/// the next satang, as for 1.235; otherwise the direction is unknown and this returns `None`.
fn whole_satang(amount: f64) -> Option<u64> {
    let text = format!("{amount:.14e}");
    let (mantissa, exponent) = text.split_once('e')?;
    let coefficient = mantissa.replace('.', "").parse::<u128>().ok()?;
    // The power of ten of the last significant digit, counted in satang.
    let shift = exponent.parse::<i32>().ok()? - 12;
    if shift >= 0 {
        return u64::try_from(coefficient.checked_mul(10_u128.checked_pow(shift.unsigned_abs())?)?)
            .ok();
    }
    if shift < -30 {
        return Some(0);
    }
    let unit = 10_u128.pow(shift.unsigned_abs());
    let (whole, rest) = (coefficient / unit, coefficient % unit);
    let rounded = match rest.cmp(&(unit / 2)) {
        Ordering::Less => whole,
        Ordering::Greater => whole + 1,
        Ordering::Equal => {
            let half = format!("{}.{:02}5{}", whole / 100, whole % 100, "0".repeat(22));
            let up = whole + 1;
            if up % 2 != 0 || format!("{amount:.25}") < half {
                return None;
            }
            up
        }
    };
    u64::try_from(rounded).ok()
}

/// Reads a whole number in Thai numerals.
fn thai_number(number: u64) -> String {
    let (millions, units) = (number / 1_000_000, number % 1_000_000);
    let mut text = if millions > 0 {
        let mut text = thai_number(millions);
        text.push_str(MILLION);
        text
    } else {
        String::new()
    };
    if number == 0 {
        text.push_str(DIGITS[0]);
    }
    for place in (0..6).rev() {
        let digit = (units / 10_u64.pow(place as u32) % 10) as usize;
        match (place, digit) {
            (_, 0) => {}
            (0, 1) if number > 1 => text.push_str(ONE_AFTER_HIGHER_DIGIT),
            (1, 1) => text.push_str(PLACES[1]),
            (1, 2) => {
                text.push_str(TWENTY_PREFIX);
                text.push_str(PLACES[1]);
            }
            _ => {
                text.push_str(DIGITS[digit]);
                text.push_str(PLACES[place]);
            }
        }
    }
    text
}
