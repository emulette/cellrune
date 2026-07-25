use crate::{CellAddress, CellRange, CellValue, DiagnosticCode, ValidationError};

/// The formula grammar stored in a workbook snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FormulaDialect {
    /// Excel A1 formula syntax as stored by `SpreadsheetML`.
    ExcelA1,
}

/// Formula text normalized to the XLSX storage form without a leading `=`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FormulaText(Box<str>);

impl FormulaText {
    /// Validates formula text read from an XLSX `<f>` element.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when the formula is empty or begins with `=`.
    pub fn from_xlsx(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ValidationError::FormulaEmpty);
        }
        if value.starts_with('=') {
            return Err(ValidationError::XlsxFormulaHasLeadingEquals);
        }
        Ok(Self(value.into_boxed_str()))
    }

    /// Validates user-entered formula text and removes its leading `=`.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when the formula does not begin with `=` or has no expression.
    pub fn from_user_input(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let Some(normalized) = value.strip_prefix('=') else {
            return Err(ValidationError::UserFormulaMissingLeadingEquals);
        };
        Self::from_xlsx(normalized.to_owned())
    }

    /// Returns XLSX storage-form text without a leading `=`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether a shared formula cell defines or follows its group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SharedFormulaRole {
    /// The cell contains the group's source formula.
    Anchor,
    /// The cell follows a resolved anchor address.
    Follower {
        /// Resolved address of the shared-formula anchor.
        anchor: CellAddress,
    },
}

/// Formula container metadata preserved independently of formula text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum FormulaMetadata {
    /// An ordinary formula.
    #[default]
    Normal,
    /// A shared formula group.
    Shared {
        /// The OOXML shared-formula group index.
        group_index: u32,
        /// Whether this cell is the anchor or a follower.
        role: SharedFormulaRole,
        /// The declared group range, when present.
        range: Option<CellRange>,
    },
    /// A legacy array formula.
    Array {
        /// The declared array result range.
        range: CellRange,
        /// Whether the producer requests full-array recalculation.
        always_calculate: bool,
    },
    /// Dynamic-array metadata with an optional producer-declared spill range.
    DynamicArray {
        /// The declared spill range, when present.
        range: Option<CellRange>,
        /// Whether the producer requests full-array recalculation.
        always_calculate: bool,
    },
    /// Data-table metadata that is preserved but not calculated.
    DataTable {
        /// The declared data-table result range.
        range: CellRange,
        /// First input cell, when retained by the producer.
        input_cell_1: Option<CellAddress>,
        /// Second input cell for a two-dimensional table.
        input_cell_2: Option<CellAddress>,
        /// Whether this is a two-input data table.
        two_dimensional: bool,
        /// Whether the single input is arranged by row rather than column.
        row_oriented: bool,
        /// Whether the first input cell was deleted.
        input_cell_1_deleted: bool,
        /// Whether the second input cell was deleted.
        input_cell_2_deleted: bool,
    },
}

impl FormulaMetadata {
    pub(crate) fn legacy_array_range_at(&self, address: CellAddress) -> Option<CellRange> {
        match self {
            Self::Array { range, .. } if range.start() == address => Some(*range),
            _ => None,
        }
    }

    pub(crate) fn dynamic_array_range_at(&self, address: CellAddress) -> Option<Option<CellRange>> {
        match self {
            Self::DynamicArray { range, .. }
                if range.is_none_or(|range| range.start() == address) =>
            {
                Some(*range)
            }
            _ => None,
        }
    }
}

/// Why a stored formula result could not be interpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedResultIssue {
    code: DiagnosticCode,
    raw_value: Option<Box<str>>,
}

impl SavedResultIssue {
    /// Creates an invalid-result record without changing the raw input.
    pub fn new(code: DiagnosticCode, raw_value: Option<String>) -> Self {
        Self {
            code,
            raw_value: raw_value.map(String::into_boxed_str),
        }
    }

    /// Returns the stable reason code.
    pub const fn code(&self) -> &DiagnosticCode {
        &self.code
    }

    /// Returns the raw stored text, when one existed.
    pub fn raw_value(&self) -> Option<&str> {
        self.raw_value.as_deref()
    }
}

/// A formula's stored result, kept distinct from a blank result.
#[derive(Debug, Clone, PartialEq)]
pub enum SavedResult {
    /// The file contains no usable stored result, including an empty numeric cache marker.
    Missing,
    /// The file contains a valid, typed stored result.
    Present(CellValue),
    /// The file contains a result that could not be interpreted.
    Invalid(SavedResultIssue),
}

/// A formula cell before recalculation.
#[derive(Debug, Clone, PartialEq)]
pub struct FormulaCell {
    dialect: FormulaDialect,
    text: Option<FormulaText>,
    saved_result: SavedResult,
    metadata: FormulaMetadata,
    recalculate_always: bool,
}

impl FormulaCell {
    /// Constructs a formula while preserving text, saved result, and metadata separately.
    pub const fn new(
        dialect: FormulaDialect,
        text: FormulaText,
        saved_result: SavedResult,
        metadata: FormulaMetadata,
    ) -> Self {
        Self {
            dialect,
            text: Some(text),
            saved_result,
            metadata,
            recalculate_always: false,
        }
    }

    /// Constructs a formula container without formula text, such as an OOXML data-table master
    /// cell or an Excel-authored cached placeholder formula.
    pub const fn metadata_only(
        dialect: FormulaDialect,
        saved_result: SavedResult,
        metadata: FormulaMetadata,
    ) -> Self {
        Self {
            dialect,
            text: None,
            saved_result,
            metadata,
            recalculate_always: false,
        }
    }

    pub(crate) const fn from_xlsx_parts(
        dialect: FormulaDialect,
        text: Option<FormulaText>,
        saved_result: SavedResult,
        metadata: FormulaMetadata,
        recalculate_always: bool,
    ) -> Self {
        Self {
            dialect,
            text,
            saved_result,
            metadata,
            recalculate_always,
        }
    }

    /// Returns the formula dialect.
    pub const fn dialect(&self) -> FormulaDialect {
        self.dialect
    }

    /// Returns normalized formula text.
    pub const fn text(&self) -> Option<&FormulaText> {
        self.text.as_ref()
    }

    /// Returns the independent stored result state.
    pub const fn saved_result(&self) -> &SavedResult {
        &self.saved_result
    }

    /// Returns preserved formula metadata.
    pub const fn metadata(&self) -> &FormulaMetadata {
        &self.metadata
    }

    /// Returns whether the producer marked this cell for recalculation on every load.
    pub const fn recalculate_always(&self) -> bool {
        self.recalculate_always
    }

    pub(crate) fn with_text(mut self, text: FormulaText) -> Self {
        self.text = Some(text);
        self
    }
}
