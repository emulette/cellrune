//! BAHTTEXT spells a positive amount in Thai words with the baht and satang suffixes.

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
const ONE_AFTER_TENS: &str = "เอ็ด";
const TWENTY_PREFIX: &str = "ยี่";
const BAHT: &str = "บาท";
const EXACT: &str = "ถ้วน";
const SATANG: &str = "สตางค์";
/// Satang amounts at or above 2^53 are no longer whole numbers in a double.
const MAX_EXACT_SATANG: f64 = 9_007_199_254_740_992.0;

/// BAHTTEXT(number) reads the baht as Thai numerals (สิบ for ten, ยี่สิบ for twenty, เอ็ด for a
/// one after a nonzero tens digit, and ล้าน for each million), then บาท, then ถ้วน for a whole
/// amount or the satang and สตางค์, as Microsoft's example and the Oracle show.
///
/// Microsoft documents no rule for zero or negative amounts, amounts below one baht, more than
/// two decimal places, or a final one after a zero tens digit (101), where Thai usage varies, so
/// those amounts are unsupported rather than guessed.
pub(super) fn baht_text(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [argument] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let number = match required_number(engine, context, argument) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let scaled = number * 100.0;
    let satang_total = scaled.round();
    if number < 1.0
        || satang_total >= MAX_EXACT_SATANG
        || (scaled - satang_total).abs() > 1e-9 * scaled
    {
        return Value::Error(ErrorKind::Unsupported);
    }
    let satang_total = satang_total as u64;
    let (baht, satang) = (satang_total / 100, satang_total % 100);
    let Some(mut text) = thai_number(baht) else {
        return Value::Error(ErrorKind::Unsupported);
    };
    text.push_str(BAHT);
    if satang == 0 {
        text.push_str(EXACT);
    } else {
        let Some(satang) = thai_number(satang) else {
            return Value::Error(ErrorKind::Unsupported);
        };
        text.push_str(&satang);
        text.push_str(SATANG);
    }
    engine.bounded_text(text)
}

/// Reads a positive whole number, or `None` where a final one follows a zero tens digit.
fn thai_number(number: u64) -> Option<String> {
    let (millions, units) = (number / 1_000_000, number % 1_000_000);
    let mut text = if millions > 0 {
        let mut text = thai_number(millions)?;
        text.push_str(MILLION);
        text
    } else {
        String::new()
    };
    if units > 0 {
        let tens = units / 10 % 10;
        if units % 10 == 1 && tens == 0 && number > 1 {
            return None;
        }
        for place in (0..6).rev() {
            let digit = (units / 10_u64.pow(place as u32) % 10) as usize;
            match (place, digit) {
                (_, 0) => {}
                (0, 1) if tens > 0 => text.push_str(ONE_AFTER_TENS),
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
    }
    Some(text)
}
