use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::{CsvError, CsvErrorCode, unquoted_literal, validate_delimiter};
use crate::{
    CalculationCellId, CalculationCellResult, CalculationSnapshot, CellAddress, CellContent,
    CellValue, SheetId, WorkbookSnapshot,
};

/// U+FEFF, which reading skips at the start of the input as a byte-order mark.
const BYTE_ORDER_MARK: char = '\u{feff}';

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

    /// Returns these options with another field delimiter: `;`, a tab, `|`, or a space.
    ///
    /// # Errors
    ///
    /// Returns [`CsvErrorCode::InvalidDelimiter`] unless the delimiter is a comma, semicolon,
    /// tab, vertical bar, or space.
    pub fn with_delimiter(self, delimiter: char) -> Result<Self, CsvError> {
        Ok(Self {
            delimiter: validate_delimiter(delimiter)?,
        })
    }
}

/// Writes one sheet as RFC 4180 CSV: UTF-8 without a byte-order mark, CRLF after every record.
///
/// Formula cells and spilled array cells write their value from `calculation`, which must belong
/// to `workbook`; a sheet that holds no formula needs no calculation and accepts `None`. Values
/// follow the rule documented on the [`crate::csv`] module, so reading the output with
/// [`crate::csv::read_csv`] yields the same literal values at the same addresses.
///
/// # Errors
///
/// Returns [`CsvErrorCode::UnknownSheet`], [`CsvErrorCode::StaleCalculation`], or
/// [`CsvErrorCode::UnavailableResult`] naming the first formula without a value, which is the
/// first formula of the sheet when `calculation` is `None`.
pub fn write_csv(
    workbook: &WorkbookSnapshot,
    calculation: Option<&CalculationSnapshot>,
    sheet_id: SheetId,
    options: CsvWriteOptions,
) -> Result<Vec<u8>, CsvError> {
    let sheet = workbook
        .sheet_by_id(sheet_id)
        .ok_or_else(|| CsvError::new(CsvErrorCode::UnknownSheet))?;
    let Some(calculation) = calculation else {
        if let Some(address) = sheet.formula_addresses().first() {
            return Err(
                CsvError::new(CsvErrorCode::UnavailableResult).with_detail(address.to_string())
            );
        }
        let values = sheet.cells().filter_map(|cell| match cell.content() {
            CellContent::Literal(value) => Some((cell.address(), value.clone())),
            CellContent::Formula(_) => None,
        });
        return Ok(render(values.collect(), options));
    };
    if !calculation.matches_workbook(workbook) {
        return Err(CsvError::new(CsvErrorCode::StaleCalculation));
    }
    let mut values = BTreeMap::<CellAddress, CellValue>::new();
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
    Ok(render(values, options))
}

/// Renders every row from 1 through the last used one with every column from A through the last
/// used one.
fn render(values: BTreeMap<CellAddress, CellValue>, options: CsvWriteOptions) -> Vec<u8> {
    let (Some(last_row), Some(last_column)) = (
        values.keys().map(|address| address.row().get()).max(),
        values.keys().map(|address| address.column().get()).max(),
    ) else {
        return Vec::new();
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
    output.into_bytes()
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
            // A leading U+FEFF at the start of the output would read back as a byte-order mark.
            let quote = text.is_empty()
                || text.starts_with(BYTE_ORDER_MARK)
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
