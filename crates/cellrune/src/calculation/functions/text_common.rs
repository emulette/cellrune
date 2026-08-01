use super::super::coerce::to_text;
use super::super::value::{ErrorKind, Value};

pub(super) fn value_to_text(value: &Value, strict: bool) -> Result<String, ErrorKind> {
    match value {
        Value::Text(text) if strict => Ok(format!("\"{}\"", text.replace('"', "\"\""))),
        Value::Text(text) => Ok(text.clone()),
        Value::Error(kind) if kind.is_engine_issue() => Err(*kind),
        Value::Error(kind) => Ok(kind.as_str().to_owned()),
        other => to_text(other),
    }
}
