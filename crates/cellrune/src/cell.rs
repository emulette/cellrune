use std::fmt;

use crate::{CellAddress, FormulaCell, ValidationError};

/// Semantic category inferred from an XLSX number format without changing the stored value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NumberFormatKind {
    /// The General format or an omitted style.
    #[default]
    General,
    /// A non-date numeric display format.
    Number,
    /// A calendar date display format.
    Date,
    /// A wall-clock time display format.
    Time,
    /// A combined calendar date and wall-clock time display format.
    DateTime,
    /// An elapsed-time display format such as `[h]:mm:ss`.
    Duration,
}

/// Number-format metadata attached to a cell while preserving its raw numeric value.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct NumberFormat {
    id: u32,
    code: Option<Box<str>>,
    kind: NumberFormatKind,
}

impl NumberFormat {
    /// Constructs a validated built-in Excel number format.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::BuiltInNumberFormatId`] when `id` is in the custom-format
    /// range.
    pub fn built_in(id: u32, kind: NumberFormatKind) -> Result<Self, ValidationError> {
        if id >= 164 {
            return Err(ValidationError::BuiltInNumberFormatId { value: id });
        }
        Ok(Self {
            id,
            code: None,
            kind,
        })
    }

    /// Constructs a validated custom Excel number format.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::CustomNumberFormatId`] when `id` is reserved for built-in
    /// formats, or [`ValidationError::NumberFormatCodeEmpty`] when `code` is empty.
    pub fn custom(
        id: u32,
        code: impl Into<String>,
        kind: NumberFormatKind,
    ) -> Result<Self, ValidationError> {
        if id < 164 {
            return Err(ValidationError::CustomNumberFormatId { value: id });
        }
        let code = code.into();
        if code.is_empty() {
            return Err(ValidationError::NumberFormatCodeEmpty);
        }
        Ok(Self {
            id,
            code: Some(code.into_boxed_str()),
            kind,
        })
    }

    pub(crate) fn new(id: u32, code: Option<Box<str>>, kind: NumberFormatKind) -> Self {
        Self { id, code, kind }
    }

    /// Returns the workbook-local OOXML number-format identifier.
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Returns the custom or known built-in format code, when retained.
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// Returns the style-derived semantic category.
    pub const fn kind(&self) -> NumberFormatKind {
        self.kind
    }
}

/// A finite IEEE-754 number accepted at the workbook boundary.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct FiniteNumber(f64);

impl FiniteNumber {
    /// Rejects NaN and positive or negative infinity.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::NonFiniteNumber`] when `value` is not finite.
    pub fn new(value: f64) -> Result<Self, ValidationError> {
        if !value.is_finite() {
            return Err(ValidationError::NonFiniteNumber);
        }
        Ok(Self(value))
    }

    /// Returns the underlying finite number.
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for FiniteNumber {
    type Error = ValidationError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// An error value stored by a spreadsheet cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ExcelError {
    /// `#NULL!`
    Null,
    /// `#DIV/0!`
    DivisionByZero,
    /// `#VALUE!`
    Value,
    /// `#REF!`
    Reference,
    /// `#NAME?`
    Name,
    /// `#NUM!`
    Number,
    /// `#N/A`
    NotAvailable,
    /// `#GETTING_DATA`
    GettingData,
    /// `#SPILL!`
    Spill,
    /// `#CALC!`
    Calculation,
}

impl ExcelError {
    /// Returns the canonical display form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Null => "#NULL!",
            Self::DivisionByZero => "#DIV/0!",
            Self::Value => "#VALUE!",
            Self::Reference => "#REF!",
            Self::Name => "#NAME?",
            Self::Number => "#NUM!",
            Self::NotAvailable => "#N/A",
            Self::GettingData => "#GETTING_DATA",
            Self::Spill => "#SPILL!",
            Self::Calculation => "#CALC!",
        }
    }
}

impl fmt::Display for ExcelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A literal or saved cell value.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub enum CellValue {
    /// No value is present.
    #[default]
    Blank,
    /// A finite numeric value.
    Number(FiniteNumber),
    /// A Unicode string.
    Text(String),
    /// A logical value.
    Logical(bool),
    /// An Excel error value.
    Error(ExcelError),
}

impl CellValue {
    /// Validates and constructs a numeric cell value.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::NonFiniteNumber`] when `value` is not finite.
    pub fn number(value: f64) -> Result<Self, ValidationError> {
        Ok(Self::Number(FiniteNumber::new(value)?))
    }
}

/// The mutually exclusive content stored at a sparse cell address.
#[derive(Debug, Clone, PartialEq)]
pub enum CellContent {
    /// A non-formula literal.
    Literal(CellValue),
    /// A formula and its independent saved result.
    Formula(FormulaCell),
}

/// A sparse cell with a validated address and content.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    address: CellAddress,
    content: CellContent,
    number_format: NumberFormat,
}

impl Cell {
    /// Constructs a cell from already validated parts.
    pub const fn new(address: CellAddress, content: CellContent) -> Self {
        Self {
            address,
            content,
            number_format: NumberFormat {
                id: 0,
                code: None,
                kind: NumberFormatKind::General,
            },
        }
    }

    pub(crate) const fn with_number_format(
        address: CellAddress,
        content: CellContent,
        number_format: NumberFormat,
    ) -> Self {
        Self {
            address,
            content,
            number_format,
        }
    }

    /// Returns the address.
    pub const fn address(&self) -> CellAddress {
        self.address
    }

    /// Returns the cell content.
    pub const fn content(&self) -> &CellContent {
        &self.content
    }

    /// Returns number-format metadata without converting the raw cell value.
    pub const fn number_format(&self) -> &NumberFormat {
        &self.number_format
    }

    pub(crate) const fn with_content_and_number_format(
        address: CellAddress,
        content: CellContent,
        number_format: NumberFormat,
    ) -> Self {
        Self {
            address,
            content,
            number_format,
        }
    }
}
