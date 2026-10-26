use std::fmt;

use super::CalculationLimitKind;
use super::excel_digits::general_text;

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

/// The text a number converts to when it joins text; see [`general_text`].
pub fn number_to_general_text(value: f64) -> String {
    general_text(value)
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

/// The length of `text` in UTF-16 code units, the unit Excel counts characters in.
pub(crate) fn text_length(text: &str) -> usize {
    // ASCII text has one code unit per byte.
    if text.is_ascii() {
        text.len()
    } else {
        text.encode_utf16().count()
    }
}
