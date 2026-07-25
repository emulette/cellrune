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

/// Collapses `-0.0` to `+0.0` on the way out of calculation.
///
/// IEEE 754 keeps the two zeros apart and float kernels reach `-0.0` by several routes:
/// `Iterator::sum` folds from `-0.0`, `f64::min` and `f64::max` may return either operand when
/// both compare equal, and `Iterator::product` inherits the sign of an odd number of negative
/// factors. Excel's number model has no negative zero, so no workbook may observe one. Normalizing
/// at the boundary every calculated value crosses makes that an invariant of the engine rather
/// than a rule each kernel has to remember, which is how `SUM` and `MIN` came to disagree.
fn normalize_negative_zero(number: f64) -> f64 {
    if number == 0.0 { 0.0 } else { number }
}

pub(super) fn cell_from_value(value: Value) -> CellValue {
    match value {
        Value::Blank => CellValue::Blank,
        Value::Number(number) => FiniteNumber::new(normalize_negative_zero(number))
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
