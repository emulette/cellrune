use super::{CsvError, CsvErrorCode, unquoted_literal, validate_delimiter};
use crate::{
    CellAddress, CellValue, EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS, EditBatch, ReadLimits, SheetId,
    SheetName, WorkbookChange, WorkbookDraft,
};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Options for reading one CSV sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvReadOptions {
    delimiter: u8,
    sheet_name: SheetName,
    max_cells: u64,
}

impl Default for CsvReadOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            sheet_name: SheetName::new("Sheet1").expect("Sheet1 is a valid sheet name"),
            max_cells: ReadLimits::default().max_cells_per_sheet(),
        }
    }
}

impl CsvReadOptions {
    /// Comma-delimited input read into `Sheet1` under the default per-sheet cell limit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns these options with another field delimiter, such as `;` or a tab.
    ///
    /// # Errors
    ///
    /// Returns [`CsvErrorCode::InvalidDelimiter`] unless the delimiter is one ASCII character
    /// other than a quote or a line break.
    pub fn with_delimiter(self, delimiter: char) -> Result<Self, CsvError> {
        Ok(Self {
            delimiter: validate_delimiter(delimiter)?,
            ..self
        })
    }

    /// Returns these options naming the sheet the input is read into.
    #[must_use]
    pub fn with_sheet_name(self, sheet_name: SheetName) -> Self {
        Self { sheet_name, ..self }
    }

    /// Returns these options with another limit on the cells that hold a value.
    #[must_use]
    pub fn with_max_cells(self, max_cells: u64) -> Self {
        Self { max_cells, ..self }
    }
}

/// Reads RFC 4180 CSV into a new one-sheet workbook draft.
///
/// The input is UTF-8, with an optional byte-order mark. Records end with CRLF, LF, or CR, and a
/// final line break adds no record. Fields become cells by the rule documented on the
/// [`crate::csv`] module: numbers, logicals, and errors unquoted, text otherwise, and never a
/// formula.
///
/// # Errors
///
/// Returns a [`CsvError`] for input that is not UTF-8, a quote the format does not allow, or more
/// rows, columns, or cells than the sheet or the limit holds.
pub fn read_csv(bytes: &[u8], options: &CsvReadOptions) -> Result<WorkbookDraft, CsvError> {
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|error| {
        CsvError::new(CsvErrorCode::InvalidUtf8)
            .with_detail(format!("byte {}", error.valid_up_to()))
    })?;
    let sheet_id = SheetId::new(1).expect("a new draft's first sheet has ID 1");
    let mut changes = Vec::new();
    if options.sheet_name.as_str() != "Sheet1" {
        changes.push(WorkbookChange::rename_sheet(
            sheet_id,
            options.sheet_name.clone(),
        ));
    }
    let mut cells = 0_u64;
    parse_records(text, options.delimiter, |row, column, field| {
        let value = match field {
            Field::Quoted(text) => CellValue::Text(text),
            Field::Unquoted(text) if text.is_empty() => return Ok(()),
            Field::Unquoted(text) => unquoted_literal(&text).unwrap_or(CellValue::Text(text)),
        };
        cells += 1;
        if cells > options.max_cells {
            return Err(CsvError::new(CsvErrorCode::TooManyCells)
                .with_detail(format!("limit {}", options.max_cells)));
        }
        let address = CellAddress::from_indices(row, column).expect("checked bounds");
        changes.push(WorkbookChange::set_cell_value(sheet_id, address, value));
        Ok(())
    })?;
    let mut draft = WorkbookDraft::new();
    draft
        .apply_changes(EditBatch::new(changes))
        .map_err(|error| {
            CsvError::new(CsvErrorCode::InvalidValue).with_detail(error.to_string())
        })?;
    Ok(draft)
}

enum Field {
    Quoted(String),
    Unquoted(String),
}

/// Splits `text` into fields and passes each with its one-based row and column.
fn parse_records(
    text: &str,
    delimiter: u8,
    mut field: impl FnMut(u32, u32, Field) -> Result<(), CsvError>,
) -> Result<(), CsvError> {
    let delimiter = char::from(delimiter);
    let mut characters = text.chars().peekable();
    let mut row = 1_u32;
    let mut column = 1_u32;
    let mut line = 1_u64;
    while characters.peek().is_some() {
        if row > EXCEL_MAX_ROWS {
            return Err(
                CsvError::new(CsvErrorCode::TooManyRows).with_detail(format!("line {line}"))
            );
        }
        if column > EXCEL_MAX_COLUMNS {
            return Err(
                CsvError::new(CsvErrorCode::TooManyColumns).with_detail(format!("line {line}"))
            );
        }
        let value = if characters.peek() == Some(&'"') {
            characters.next();
            let mut value = String::new();
            loop {
                match characters.next() {
                    None => {
                        return Err(CsvError::new(CsvErrorCode::UnterminatedQuote)
                            .with_detail(format!("line {line}")));
                    }
                    Some('"') if characters.peek() == Some(&'"') => {
                        characters.next();
                        value.push('"');
                    }
                    Some('"') => break,
                    Some(character) => {
                        if character == '\n' {
                            line += 1;
                        }
                        value.push(character);
                    }
                }
            }
            if !matches!(characters.peek(), None | Some('\r' | '\n'))
                && characters.peek() != Some(&delimiter)
            {
                return Err(
                    CsvError::new(CsvErrorCode::InvalidQuote).with_detail(format!("line {line}"))
                );
            }
            Field::Quoted(value)
        } else {
            let mut value = String::new();
            while let Some(&character) = characters.peek() {
                if character == delimiter || matches!(character, '\r' | '\n') {
                    break;
                }
                value.push(character);
                characters.next();
            }
            Field::Unquoted(value)
        };
        field(row, column, value)?;
        match characters.next() {
            Some(character) if character == delimiter => {
                column += 1;
                if characters.peek().is_none() {
                    // A trailing delimiter ends the record with one more empty field.
                    break;
                }
            }
            Some('\r') => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                row += 1;
                column = 1;
                line += 1;
            }
            Some('\n') => {
                row += 1;
                column = 1;
                line += 1;
            }
            _ => {}
        }
    }
    Ok(())
}
