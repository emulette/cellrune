//! RFC 4180 CSV import and export of one sheet.
//!
//! # Value interpretation
//!
//! Reading treats every field as a literal; it never creates a formula. An empty unquoted field
//! is an empty cell. A quoted field is always text, so `"007"` stays the text `007`. An unquoted
//! field is a number when it is a decimal number such as `-12`, `3.5`, `.5`, or `1e-3`; a logical
//! when it is `TRUE` or `FALSE` in any letter case; an error when it is exactly an Excel error such
//! as `#N/A`; and text otherwise, with no date or locale parsing.
//!
//! Writing quotes a text field when it would read back as anything but the same text, including
//! text that starts with U+FEFF, which would otherwise read back as a byte-order mark. Numbers use
//! the shortest decimal that reads back to the same value, logicals are `TRUE`/`FALSE`, errors their
//! Excel spelling, and formulas their calculated value. Every row from row 1 through the last used
//! row is written with every column from column A through the last used column, so cell addresses
//! survive a write and a read.

mod error;
mod read;
mod write;

#[cfg(test)]
mod tests;

pub use error::{CsvError, CsvErrorCode};
pub use read::{CsvReadOptions, read_csv};
pub use write::{CsvWriteOptions, write_csv};

/// The accepted field delimiters: comma, semicolon, tab, vertical bar, and space.
///
/// None of them occurs in a number, logical, or error spelling the writer leaves unquoted, so a
/// write and a read with the same delimiter keep every value. A delimiter such as `.`, `-`, `E`, a
/// digit, or `#` would split such a value instead.
const DELIMITERS: [char; 5] = [',', ';', '\t', '|', ' '];

/// Validates a field delimiter against [`DELIMITERS`].
fn validate_delimiter(delimiter: char) -> Result<u8, CsvError> {
    if !DELIMITERS.contains(&delimiter) {
        return Err(CsvError::new(CsvErrorCode::InvalidDelimiter));
    }
    Ok(delimiter as u8)
}

/// Returns the literal an unquoted field reads as, or `None` for text.
fn unquoted_literal(field: &str) -> Option<crate::CellValue> {
    use crate::{CellValue, ExcelError};

    if is_decimal_number(field) {
        return field
            .parse::<f64>()
            .ok()
            .and_then(|value| CellValue::number(value).ok());
    }
    if field.eq_ignore_ascii_case("TRUE") {
        return Some(CellValue::Logical(true));
    }
    if field.eq_ignore_ascii_case("FALSE") {
        return Some(CellValue::Logical(false));
    }
    ExcelError::from_spelling(field).map(CellValue::Error)
}

/// Matches `[+-]? (digits [. digits?] | . digits) ([eE] [+-]? digits)?`.
fn is_decimal_number(field: &str) -> bool {
    let bytes = field.as_bytes();
    let mut index = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let digits = |index: &mut usize| {
        let start = *index;
        while bytes.get(*index).is_some_and(u8::is_ascii_digit) {
            *index += 1;
        }
        *index - start
    };
    let integer = digits(&mut index);
    let mut fraction = 0;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        fraction = digits(&mut index);
    }
    if integer == 0 && fraction == 0 {
        return false;
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        if digits(&mut index) == 0 {
            return false;
        }
    }
    index == bytes.len()
}
