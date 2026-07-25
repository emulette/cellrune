use std::fmt;

use super::CalculationLimitKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    Div0,
    NA,
    Name,
    Null,
    Num,
    Ref,
    Value,
    Spill,
    Calc,
    Unsupported,
    ResourceLimit(CalculationLimitKind),
}

impl ErrorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Div0 => "#DIV/0!",
            ErrorKind::NA => "#N/A",
            ErrorKind::Name => "#NAME?",
            ErrorKind::Null => "#NULL!",
            ErrorKind::Num => "#NUM!",
            ErrorKind::Ref => "#REF!",
            ErrorKind::Value => "#VALUE!",
            ErrorKind::Spill => "#SPILL!",
            ErrorKind::Calc => "#CALC!",
            ErrorKind::Unsupported => "#UNSUPPORTED!",
            ErrorKind::ResourceLimit(_) => "#RESOURCE!",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_uppercase().as_str() {
            "#DIV/0!" => Some(ErrorKind::Div0),
            "#N/A" => Some(ErrorKind::NA),
            "#NAME?" => Some(ErrorKind::Name),
            "#NULL!" => Some(ErrorKind::Null),
            "#NUM!" => Some(ErrorKind::Num),
            "#REF!" => Some(ErrorKind::Ref),
            "#VALUE!" => Some(ErrorKind::Value),
            "#SPILL!" => Some(ErrorKind::Spill),
            "#CALC!" => Some(ErrorKind::Calc),
            "#UNSUPPORTED!" => Some(ErrorKind::Unsupported),
            _ => None,
        }
    }

    pub const fn is_engine_issue(self) -> bool {
        matches!(self, Self::Unsupported | Self::ResourceLimit(_))
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    #[default]
    Blank,
    Number(f64),
    Text(String),
    Logical(bool),
    Error(ErrorKind),
}

impl Value {
    pub fn is_blank_like(&self) -> bool {
        match self {
            Value::Blank => true,
            Value::Text(text) => text.is_empty(),
            _ => false,
        }
    }

    pub fn error(&self) -> Option<ErrorKind> {
        match self {
            Value::Error(kind) => Some(*kind),
            _ => None,
        }
    }
}

pub fn number_to_general_text(value: f64) -> String {
    let truncated = truncate_to_excel_precision(value);
    if truncated == truncated.trunc() && truncated.abs() < 1e15 {
        format!("{}", truncated as i64)
    } else {
        format!("{truncated}")
    }
}

fn truncate_to_excel_precision(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let decimal_exponent = value.abs().log10().floor() as i32;
    let scale = 10_f64.powi(14 - decimal_exponent);
    if !scale.is_finite() || scale == 0.0 {
        return value;
    }
    let scaled = value * scale;
    if !scaled.is_finite() {
        return value;
    }
    let truncated = scaled.trunc() / scale;
    if truncated.is_finite() {
        truncated
    } else {
        value
    }
}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Blank => Ok(()),
            Value::Number(number) => formatter.write_str(&number_to_general_text(*number)),
            Value::Text(text) => formatter.write_str(text),
            Value::Logical(true) => formatter.write_str("TRUE"),
            Value::Logical(false) => formatter.write_str("FALSE"),
            Value::Error(kind) => formatter.write_str(kind.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::number_to_general_text;

    #[test]
    fn general_text_hides_binary_noise_at_excel_precision() {
        assert_eq!(number_to_general_text(0.1 + 0.2), "0.3");
        assert_eq!(
            number_to_general_text(1.234_567_890_123_456),
            "1.23456789012345"
        );
        assert_eq!(number_to_general_text(-0.0), "0");
    }
}
