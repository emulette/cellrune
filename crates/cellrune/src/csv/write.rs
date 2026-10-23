use std::fmt::Write as _;

use super::{CsvError, CsvErrorCode, unquoted_literal, validate_delimiter};
use crate::{
    CalculationCellId, CalculationCellResult, CalculationSnapshot, CellAddress, CellContent,
    CellValue, SheetId, WorkbookSnapshot,
};

/// Options for writing one sheet as CSV.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsvWriteOptions {
    delimiter: u8,
}

impl Default for CsvWriteOptions {
    fn default() -> Self {
        Self { delimiter: b',' }
    }
}

impl CsvWriteOptions {
    /// Comma-delimited output.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns these options with another field delimiter.
    ///
    /// # Errors
    ///
    /// Returns [`CsvErrorCode::InvalidDelimiter`] unless the delimiter is one ASCII character
    /// other than a quote or a line break.
    pub fn with_delimiter(self, delimiter: char) -> Result<Self, CsvError> {
        Ok(Self {
            delimiter: validate_delimiter(delimiter)?,
        })
    }
}

/// Writes one sheet as RFC 4180 CSV: UTF-8 without a byte-order mark, CRLF after every record.
///
/// Formula cells and spilled array cells write their value from `calculation`, which must belong
/// to `workbook`. Values follow the rule documented on the [`crate::csv`] module, so reading the
/// output with [`crate::csv::read_csv`] yields the same literal values at the same addresses.
///
/// # Errors
///
/// Returns [`CsvErrorCode::UnknownSheet`], [`CsvErrorCode::StaleCalculation`], or
/// [`CsvErrorCode::UnavailableResult`] naming the first formula without a value.
pub fn write_csv(
    workbook: &WorkbookSnapshot,
    calculation: &CalculationSnapshot,
    sheet_id: SheetId,
    options: CsvWriteOptions,
) -> Result<Vec<u8>, CsvError> {
    let sheet = workbook
        .sheet_by_id(sheet_id)
        .ok_or_else(|| CsvError::new(CsvErrorCode::UnknownSheet))?;
    if !calculation.matches_workbook(workbook) {
        return Err(CsvError::new(CsvErrorCode::StaleCalculation));
    }
    let mut values = std::collections::BTreeMap::<CellAddress, CellValue>::new();
    for cell in sheet.cells() {
        if let CellContent::Literal(value) = cell.content() {
            values.insert(cell.address(), value.clone());
        }
    }
    let materialized = calculation
        .materialized_cells()
        .filter(|(id, _)| id.sheet_id() == sheet_id);
    for (id, cell) in materialized {
        match cell.result() {
            CalculationCellResult::Value(value) => {
                values.insert(id.address(), value.clone());
            }
            CalculationCellResult::Unavailable(_) => {
                return Err(CsvError::new(CsvErrorCode::UnavailableResult)
                    .with_detail(id.address().to_string()));
            }
        }
    }
    let formula_without_result = sheet.cells().find(|cell| {
        matches!(cell.content(), CellContent::Formula(_))
            && calculation
                .materialized_cell(CalculationCellId::new(sheet_id, cell.address()))
                .is_none()
    });
    if let Some(cell) = formula_without_result {
        return Err(
            CsvError::new(CsvErrorCode::UnavailableResult).with_detail(cell.address().to_string())
        );
    }
    let (Some(last_row), Some(last_column)) = (
        values.keys().map(|address| address.row().get()).max(),
        values.keys().map(|address| address.column().get()).max(),
    ) else {
        return Ok(Vec::new());
    };
    let delimiter = char::from(options.delimiter);
    let mut output = String::new();
    let mut entries = values.into_iter().peekable();
    for row in 1..=last_row {
        for column in 1..=last_column {
            if column > 1 {
                output.push(delimiter);
            }
            if let Some((_, value)) = entries.next_if(|(address, _)| {
                (address.row().get(), address.column().get()) == (row, column)
            }) {
                push_value(&mut output, &value, delimiter);
            }
        }
        output.push_str("\r\n");
    }
    Ok(output.into_bytes())
}

fn push_value(output: &mut String, value: &CellValue, delimiter: char) {
    match value {
        CellValue::Number(number) => {
            let _ = write!(output, "{}", number.get());
        }
        CellValue::Logical(true) => output.push_str("TRUE"),
        CellValue::Logical(false) => output.push_str("FALSE"),
        CellValue::Error(error) => output.push_str(error.as_str()),
        CellValue::Text(text) => {
            let quote = text.is_empty()
                || text.contains([delimiter, '"', '\r', '\n'])
                || unquoted_literal(text).is_some();
            if quote {
                output.push('"');
                output.push_str(&text.replace('"', "\"\""));
                output.push('"');
            } else {
                output.push_str(text);
            }
        }
        _ => {}
    }
}
