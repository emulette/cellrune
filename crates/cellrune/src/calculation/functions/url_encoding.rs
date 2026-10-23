//! ENCODEURL percent-encodes the characters Microsoft documents.

use std::fmt::Write as _;

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::util::required_text;

/// ENCODEURL(text) keeps ASCII letters, digits, and `.`, and replaces `:`, `/`, and the space with
/// `%` and two uppercase hexadecimal digits. Microsoft documents that it replaces "certain
/// non-alphanumeric characters", and its example encodes `:`, `/`, and the space and keeps `.`.
/// Which other characters Excel encodes, and how it encodes text outside ASCII, is neither
/// documented nor measurable (ENCODEURL needs Excel for Windows), and percent-encoding conventions
/// disagree on characters such as `~`, `!`, and `'`, so text with any other character is
/// unsupported rather than guessed.
pub(super) fn encode_url(engine: &Engine<'_>, context: EvalContext<'_>, args: &[Expr]) -> Value {
    let [argument] = args else {
        return Value::Error(ErrorKind::Value);
    };
    let text = match required_text(engine, context, argument) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'.' {
            encoded.push(char::from(byte));
        } else if matches!(byte, b':' | b'/' | b' ') {
            write!(encoded, "%{byte:02X}").expect("writing to String cannot fail");
        } else {
            return Value::Error(ErrorKind::Unsupported);
        }
    }
    engine.bounded_text(encoded)
}
