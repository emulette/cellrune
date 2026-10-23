//! ENCODEURL percent-encodes text the way RFC 3986 encodes a URI component.

use std::fmt::Write as _;

use super::super::ast::Expr;
use super::super::eval::{Engine, EvalContext};
use super::super::value::{ErrorKind, Value};
use super::util::required_text;

/// ENCODEURL(text) keeps the RFC 3986 unreserved characters (ASCII letters and digits, `-`, `.`,
/// `_`, and `~`) and replaces every other UTF-8 byte with `%` and two uppercase hexadecimal
/// digits. Microsoft's example encodes `:`, `/`, and the space and keeps `.`.
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
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            write!(encoded, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    engine.bounded_text(encoded)
}
