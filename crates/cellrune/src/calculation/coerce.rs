use std::cmp::Ordering;

use super::value::{ErrorKind, Value, number_to_general_text};

pub fn to_number(value: &Value) -> Result<f64, ErrorKind> {
    match value {
        Value::Number(number) => Ok(*number),
        Value::Blank => Ok(0.0),
        Value::Logical(logical) => Ok(if *logical { 1.0 } else { 0.0 }),
        Value::Text(text) => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .ok_or(ErrorKind::Value),
        Value::Error(kind) => Err(*kind),
    }
}

pub fn to_text(value: &Value) -> Result<String, ErrorKind> {
    match value {
        Value::Text(text) => Ok(text.clone()),
        Value::Blank => Ok(String::new()),
        Value::Number(number) => Ok(number_to_general_text(*number)),
        Value::Logical(true) => Ok("TRUE".to_owned()),
        Value::Logical(false) => Ok("FALSE".to_owned()),
        Value::Error(kind) => Err(*kind),
    }
}

pub fn to_logical(value: &Value) -> Result<bool, ErrorKind> {
    match value {
        Value::Logical(logical) => Ok(*logical),
        Value::Number(number) => Ok(*number != 0.0),
        Value::Blank => Ok(false),
        Value::Text(text) => {
            if text.eq_ignore_ascii_case("TRUE") {
                Ok(true)
            } else if text.eq_ignore_ascii_case("FALSE") {
                Ok(false)
            } else {
                Err(ErrorKind::Value)
            }
        }
        Value::Error(kind) => Err(*kind),
    }
}

fn type_rank(value: &Value) -> u8 {
    match value {
        Value::Number(_) => 0,
        Value::Text(_) => 1,
        Value::Logical(_) => 2,
        Value::Blank | Value::Error(_) => 3,
    }
}

fn blank_substitute(other: &Value) -> Value {
    match other {
        Value::Number(_) => Value::Number(0.0),
        Value::Text(_) => Value::Text(String::new()),
        Value::Logical(_) => Value::Logical(false),
        _ => Value::Blank,
    }
}

pub fn compare(left: &Value, right: &Value) -> Result<Ordering, ErrorKind> {
    if let Some(kind) = left.error() {
        return Err(kind);
    }
    if let Some(kind) = right.error() {
        return Err(kind);
    }
    match (left, right) {
        (Value::Blank, Value::Blank) => Ok(Ordering::Equal),
        (Value::Blank, other) => compare(&blank_substitute(other), other),
        (other, Value::Blank) => {
            let substitute = blank_substitute(other);
            compare(other, &substitute)
        }
        (Value::Number(a), Value::Number(b)) => Ok(a.partial_cmp(b).unwrap_or(Ordering::Equal)),
        (Value::Text(a), Value::Text(b)) => Ok(compare_text_case_insensitive(a, b)),
        (Value::Logical(a), Value::Logical(b)) => Ok(a.cmp(b)),
        _ => Ok(type_rank(left).cmp(&type_rank(right))),
    }
}

pub fn compare_text_case_insensitive(left: &str, right: &str) -> Ordering {
    left.chars()
        .flat_map(char::to_lowercase)
        .cmp(right.chars().flat_map(char::to_lowercase))
}

pub fn values_equal(left: &Value, right: &Value) -> Result<bool, ErrorKind> {
    Ok(compare(left, right)? == Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::{ErrorKind, Value, to_number};

    #[test]
    fn numeric_text_rejects_non_finite_rust_spellings() {
        assert_eq!(
            to_number(&Value::Text("NaN".to_owned())),
            Err(ErrorKind::Value)
        );
        assert_eq!(
            to_number(&Value::Text("inf".to_owned())),
            Err(ErrorKind::Value)
        );
        assert_eq!(to_number(&Value::Text(" 12.5 ".to_owned())), Ok(12.5));
    }
}
