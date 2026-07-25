use super::value::{ErrorKind, Value};
use crate::{CellValue, ExcelError, FiniteNumber};

pub(super) fn value_from_cell(value: &CellValue) -> Value {
    match value {
        CellValue::Blank => Value::Blank,
        CellValue::Number(number) => Value::Number(number.get()),
        CellValue::Text(text) => Value::Text(text.clone()),
        CellValue::Logical(logical) => Value::Logical(*logical),
        CellValue::Error(error) => Value::Error(error_from_cell(*error)),
    }
}

fn error_from_cell(error: ExcelError) -> ErrorKind {
    match error {
        ExcelError::Null => ErrorKind::Null,
        ExcelError::DivisionByZero => ErrorKind::Div0,
        ExcelError::Value => ErrorKind::Value,
        ExcelError::Reference => ErrorKind::Ref,
        ExcelError::Name => ErrorKind::Name,
        ExcelError::Number => ErrorKind::Num,
        ExcelError::NotAvailable => ErrorKind::NA,
        ExcelError::Spill => ErrorKind::Spill,
        ExcelError::Calculation => ErrorKind::Calc,
        ExcelError::GettingData => ErrorKind::Unsupported,
    }
}

pub(super) fn cell_from_value(value: Value) -> CellValue {
    match value {
        Value::Blank => CellValue::Blank,
        Value::Number(number) => FiniteNumber::new(number)
            .map(CellValue::Number)
            .unwrap_or(CellValue::Error(ExcelError::Number)),
        Value::Text(text) => CellValue::Text(text),
        Value::Logical(logical) => CellValue::Logical(logical),
        Value::Error(error) => CellValue::Error(match error {
            ErrorKind::Div0 => ExcelError::DivisionByZero,
            ErrorKind::NA => ExcelError::NotAvailable,
            ErrorKind::Name => ExcelError::Name,
            ErrorKind::Null => ExcelError::Null,
            ErrorKind::Num => ExcelError::Number,
            ErrorKind::Ref => ExcelError::Reference,
            ErrorKind::Spill => ExcelError::Spill,
            ErrorKind::Calc => ExcelError::Calculation,
            ErrorKind::Value | ErrorKind::Unsupported | ErrorKind::ResourceLimit(_) => {
                ExcelError::Value
            }
        }),
    }
}
