//! CSV import into a new session and export of one calculated sheet.

use cellrune::{
    CsvError, CsvErrorCode, CsvReadOptions, CsvWriteOptions, SheetName, WorkbookCalculationSession,
    read_csv, write_csv,
};

use super::WorkbookSession;
use crate::{CsvReadOptionsDto, CsvWriteRequestDto, InteropError};

impl WorkbookSession {
    /// Opens RFC 4180 CSV as a new one-sheet workbook by the value rule documented on
    /// `cellrune::csv`.
    ///
    /// # Errors
    ///
    /// Returns a typed `csv.*` error for an invalid delimiter or input, or a validation error for
    /// an invalid sheet name.
    pub fn open_csv_bytes(bytes: &[u8], options: &CsvReadOptionsDto) -> Result<Self, InteropError> {
        let mut read_options = CsvReadOptions::new();
        if let Some(name) = &options.sheet_name {
            read_options = read_options.with_sheet_name(SheetName::new(name.as_str())?);
        }
        if let Some(delimiter) = &options.delimiter {
            read_options = read_options.with_delimiter(delimiter_char(delimiter)?)?;
        }
        let draft = read_csv(bytes, &read_options)?;
        Ok(Self::from_engine(WorkbookCalculationSession::new(draft)))
    }

    /// Exports one sheet as CSV bytes, formulas as their values in the current calculated
    /// revision. A sheet that holds no formula needs no calculation.
    ///
    /// # Errors
    ///
    /// Returns `interop.calculation.required` for a sheet with a formula and no current
    /// calculation, `interop.sheet.not_found` for an unknown sheet, or a typed `csv.*` error.
    pub fn save_csv_bytes(&self, request: &CsvWriteRequestDto) -> Result<Vec<u8>, InteropError> {
        let calculation = self.current_calculation();
        let workbook = self.engine.workbook();
        let sheet = workbook
            .sheet_by_name(&request.sheet)
            .ok_or_else(InteropError::sheet_not_found)?;
        let mut options = CsvWriteOptions::new();
        if let Some(delimiter) = &request.delimiter {
            options = options.with_delimiter(delimiter_char(delimiter)?)?;
        }
        write_csv(workbook, calculation, sheet.id(), options).map_err(|error| {
            // Without a calculation, the only formula without a value is one never calculated.
            if calculation.is_none() && error.code() == CsvErrorCode::UnavailableResult {
                InteropError::calculation_required()
            } else {
                error.into()
            }
        })
    }
}

/// Reads a delimiter string that must hold exactly one character.
fn delimiter_char(value: &str) -> Result<char, InteropError> {
    let mut characters = value.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) => Ok(character),
        _ => Err(CsvError::new(CsvErrorCode::InvalidDelimiter).into()),
    }
}
