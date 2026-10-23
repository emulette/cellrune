use std::error::Error;
use std::fmt;

const MESSAGE_INVALID_DELIMITER: &str =
    "CSV delimiter must be a comma, semicolon, tab, vertical bar, or space";
const MESSAGE_INVALID_UTF8: &str = "CSV input is not UTF-8";
const MESSAGE_UNTERMINATED_QUOTE: &str = "CSV quoted field is not closed before the input ends";
const MESSAGE_INVALID_QUOTE: &str = "CSV quoted field is followed by text before its delimiter";
const MESSAGE_TOO_MANY_ROWS: &str = "CSV input has more rows than a worksheet holds";
const MESSAGE_TOO_MANY_COLUMNS: &str = "CSV input has more columns than a worksheet holds";
const MESSAGE_TOO_MANY_CELLS: &str = "CSV input has more cells than the read limit allows";
const MESSAGE_FIELD_TOO_LONG: &str =
    "CSV field is longer than the 32,767 UTF-16 code units a cell holds";
const MESSAGE_INVALID_VALUE: &str = "CSV input holds a value the workbook rejects";
const MESSAGE_UNKNOWN_SHEET: &str = "workbook does not contain the requested sheet";
const MESSAGE_STALE_CALCULATION: &str = "calculation does not belong to the current workbook";
const MESSAGE_UNAVAILABLE_RESULT: &str = "a formula in the exported sheet has no calculated value";

/// Stable machine-readable CSV failure classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CsvErrorCode {
    /// The delimiter is not a comma, semicolon, tab, vertical bar, or space.
    InvalidDelimiter,
    /// The input is not UTF-8.
    InvalidUtf8,
    /// A quoted field is not closed before the input ends.
    UnterminatedQuote,
    /// A closing quote is followed by text other than a delimiter or line break.
    InvalidQuote,
    /// The input has more rows than a worksheet holds.
    TooManyRows,
    /// The input has more columns than a worksheet holds.
    TooManyColumns,
    /// The input has more cells than the read limit allows.
    TooManyCells,
    /// A field is longer than the 32,767 UTF-16 code units a spreadsheet cell holds.
    FieldTooLong,
    /// The workbook rejects a value or the sheet name.
    InvalidValue,
    /// The exported sheet is not in the workbook.
    UnknownSheet,
    /// The calculation belongs to another workbook revision.
    StaleCalculation,
    /// A formula in the exported sheet has no calculated value.
    UnavailableResult,
}

impl CsvErrorCode {
    /// Returns the stable code.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidDelimiter => "csv.invalid_delimiter",
            Self::InvalidUtf8 => "csv.read.invalid_utf8",
            Self::UnterminatedQuote => "csv.read.unterminated_quote",
            Self::InvalidQuote => "csv.read.invalid_quote",
            Self::TooManyRows => "csv.read.too_many_rows",
            Self::TooManyColumns => "csv.read.too_many_columns",
            Self::TooManyCells => "csv.read.too_many_cells",
            Self::FieldTooLong => "csv.read.field_too_long",
            Self::InvalidValue => "csv.read.invalid_value",
            Self::UnknownSheet => "csv.write.unknown_sheet",
            Self::StaleCalculation => "csv.write.stale_calculation",
            Self::UnavailableResult => "csv.write.unavailable_result",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::InvalidDelimiter => MESSAGE_INVALID_DELIMITER,
            Self::InvalidUtf8 => MESSAGE_INVALID_UTF8,
            Self::UnterminatedQuote => MESSAGE_UNTERMINATED_QUOTE,
            Self::InvalidQuote => MESSAGE_INVALID_QUOTE,
            Self::TooManyRows => MESSAGE_TOO_MANY_ROWS,
            Self::TooManyColumns => MESSAGE_TOO_MANY_COLUMNS,
            Self::TooManyCells => MESSAGE_TOO_MANY_CELLS,
            Self::FieldTooLong => MESSAGE_FIELD_TOO_LONG,
            Self::InvalidValue => MESSAGE_INVALID_VALUE,
            Self::UnknownSheet => MESSAGE_UNKNOWN_SHEET,
            Self::StaleCalculation => MESSAGE_STALE_CALCULATION,
            Self::UnavailableResult => MESSAGE_UNAVAILABLE_RESULT,
        }
    }
}

/// A CSV read or write failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvError {
    code: CsvErrorCode,
    detail: Option<Box<str>>,
}

impl CsvError {
    /// Creates an error of one class without detail.
    pub const fn new(code: CsvErrorCode) -> Self {
        Self { code, detail: None }
    }

    pub(crate) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into().into_boxed_str());
        self
    }

    /// Returns the stable failure class.
    pub const fn code(&self) -> CsvErrorCode {
        self.code
    }

    /// Returns the position or cell the failure concerns, such as `line 3` or `B2`.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

impl fmt::Display for CsvError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code.message())?;
        if let Some(detail) = &self.detail {
            write!(formatter, ": {detail}")?;
        }
        Ok(())
    }
}

impl Error for CsvError {}
