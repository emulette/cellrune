use std::fmt;
use std::str::FromStr;

use crate::ValidationError;

/// Maximum row supported by an Excel worksheet.
pub const EXCEL_MAX_ROWS: u32 = 1_048_576;
/// Maximum column supported by an Excel worksheet.
pub const EXCEL_MAX_COLUMNS: u32 = 16_384;

/// A validated, one-based Excel row index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Row(u32);

impl Row {
    /// Validates and constructs a row index.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::RowOutOfRange`] when `value` is outside Excel's row bounds.
    pub fn new(value: u32) -> Result<Self, ValidationError> {
        if !(1..=EXCEL_MAX_ROWS).contains(&value) {
            return Err(ValidationError::RowOutOfRange { value });
        }
        Ok(Self(value))
    }

    /// Returns the one-based index.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for Row {
    type Error = ValidationError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// A validated, one-based Excel column index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Column(u32);

impl Column {
    /// Validates and constructs a column index.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::ColumnOutOfRange`] when `value` is outside Excel's column bounds.
    pub fn new(value: u32) -> Result<Self, ValidationError> {
        if !(1..=EXCEL_MAX_COLUMNS).contains(&value) {
            return Err(ValidationError::ColumnOutOfRange { value });
        }
        Ok(Self(value))
    }

    /// Returns the one-based index.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for Column {
    type Error = ValidationError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// A validated cell address ordered in row-major order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellAddress {
    row: Row,
    column: Column,
}

impl CellAddress {
    /// Constructs an address from validated coordinates.
    pub const fn new(row: Row, column: Column) -> Self {
        Self { row, column }
    }

    /// Validates raw one-based coordinates and constructs an address.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when either coordinate is outside Excel's worksheet bounds.
    pub fn from_indices(row: u32, column: u32) -> Result<Self, ValidationError> {
        Ok(Self::new(Row::new(row)?, Column::new(column)?))
    }

    /// Parses an unqualified A1 address such as `A1` or `XFD1048576`.
    ///
    /// Sheet-qualified references and absolute markers such as `$A$1` belong to formula syntax
    /// and are deliberately rejected by this workbook lookup helper.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when the text is not an unqualified address within Excel's
    /// worksheet bounds.
    pub fn from_a1(value: &str) -> Result<Self, ValidationError> {
        let bytes = value.as_bytes();
        let Some(row_start) = bytes.iter().position(u8::is_ascii_digit) else {
            return Err(ValidationError::CellAddressInvalid);
        };
        if row_start == 0
            || !bytes[..row_start].iter().all(u8::is_ascii_alphabetic)
            || !bytes[row_start..].iter().all(u8::is_ascii_digit)
        {
            return Err(ValidationError::CellAddressInvalid);
        }

        let mut column = 0_u32;
        for byte in &bytes[..row_start] {
            column = column * 26 + u32::from(byte.to_ascii_uppercase() - b'A' + 1);
            if column > EXCEL_MAX_COLUMNS {
                return Err(ValidationError::ColumnOutOfRange { value: column });
            }
        }

        let mut row = 0_u32;
        for byte in &bytes[row_start..] {
            row = row * 10 + u32::from(*byte - b'0');
            if row > EXCEL_MAX_ROWS {
                return Err(ValidationError::RowOutOfRange { value: row });
            }
        }
        Self::from_indices(row, column)
    }

    /// Returns the row.
    pub const fn row(self) -> Row {
        self.row
    }

    /// Returns the column.
    pub const fn column(self) -> Column {
        self.column
    }
}

impl FromStr for CellAddress {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_a1(value)
    }
}

impl fmt::Display for CellAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&column_label(self.column.get()))?;
        write!(formatter, "{}", self.row.get())
    }
}

/// A non-empty rectangular range with validated, inclusive endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellRange {
    start: CellAddress,
    end: CellAddress,
}

impl CellRange {
    /// Constructs a range when the start is above and to the left of the end.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::RangeStartAfterEnd`] when either start coordinate exceeds the
    /// corresponding end coordinate.
    pub fn new(start: CellAddress, end: CellAddress) -> Result<Self, ValidationError> {
        if start.row > end.row || start.column > end.column {
            return Err(ValidationError::RangeStartAfterEnd);
        }
        Ok(Self { start, end })
    }

    pub(crate) const fn from_ordered(start: CellAddress, end: CellAddress) -> Self {
        Self { start, end }
    }

    /// Returns the inclusive start address.
    pub const fn start(self) -> CellAddress {
        self.start
    }

    /// Returns the inclusive end address.
    pub const fn end(self) -> CellAddress {
        self.end
    }

    /// Returns the number of rows in the range.
    pub const fn height(self) -> u32 {
        self.end.row.get() - self.start.row.get() + 1
    }

    /// Returns the number of columns in the range.
    pub const fn width(self) -> u32 {
        self.end.column.get() - self.start.column.get() + 1
    }

    /// Returns whether the range contains the address.
    pub const fn contains(self, address: CellAddress) -> bool {
        address.row.get() >= self.start.row.get()
            && address.row.get() <= self.end.row.get()
            && address.column.get() >= self.start.column.get()
            && address.column.get() <= self.end.column.get()
    }
}

fn column_label(mut column: u32) -> String {
    let mut reversed = Vec::with_capacity(3);
    while column > 0 {
        column -= 1;
        reversed.push(b'A' + (column % 26) as u8);
        column /= 26;
    }
    reversed.reverse();
    reversed.into_iter().map(char::from).collect()
}
