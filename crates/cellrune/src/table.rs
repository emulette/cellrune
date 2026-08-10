use std::num::NonZeroU32;
use std::sync::Arc;
use std::{fmt, result};

use crate::{CellRange, FormulaText, ValidationError};

mod filter;

pub use filter::{
    TableAutoFilter, TableCalendarType, TableColorFilter, TableCustomFilter,
    TableCustomFilterOperator, TableCustomFilters, TableDateGroupItem, TableDateTimeGrouping,
    TableDateTimeValue, TableDynamicFilter, TableDynamicFilterType, TableFilterColumn,
    TableFilterCriteria, TableFilterItem, TableIconFilter, TableIconSet, TableNumericValue,
    TableSortBy, TableSortCondition, TableSortMethod, TableSortState, TableTopFilter,
    TableValueFilters,
};

const MESSAGE_TABLE_METADATA_RANGE_OUTSIDE_TABLE: &str =
    "table filter or sort metadata range extends outside its owner";
const MESSAGE_TABLE_FILTER_COLUMN_OUT_OF_RANGE: &str =
    "table filter column identifier exceeds the filter range width";
const MESSAGE_DUPLICATE_TABLE_FILTER_COLUMN: &str =
    "table auto-filter contains a duplicate column identifier";
const MESSAGE_TOO_MANY_TABLE_SORT_CONDITIONS: &str =
    "table sort state exceeds the 64-condition limit";
const MESSAGE_INVALID_TABLE_SORT_CONDITION: &str =
    "table sort condition is outside or incorrectly oriented for its sort state";
const MESSAGE_INVALID_TABLE_FILTER_CRITERIA: &str =
    "table filter criteria violate OOXML ordering or cardinality";
const MESSAGE_INVALID_TOTALS_ROW_VISIBILITY: &str =
    "a table with one totals row must mark that row as shown";
const MAX_TABLE_SORT_CONDITIONS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TableMetadataValidationError {
    RangeOutsideTable,
    FilterColumnOutOfRange,
    DuplicateFilterColumn,
    TooManySortConditions,
    InvalidSortCondition,
    InvalidFilterCriteria,
    InvalidTotalsRowVisibility,
}

impl fmt::Display for TableMetadataValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::RangeOutsideTable => MESSAGE_TABLE_METADATA_RANGE_OUTSIDE_TABLE,
            Self::FilterColumnOutOfRange => MESSAGE_TABLE_FILTER_COLUMN_OUT_OF_RANGE,
            Self::DuplicateFilterColumn => MESSAGE_DUPLICATE_TABLE_FILTER_COLUMN,
            Self::TooManySortConditions => MESSAGE_TOO_MANY_TABLE_SORT_CONDITIONS,
            Self::InvalidSortCondition => MESSAGE_INVALID_TABLE_SORT_CONDITION,
            Self::InvalidFilterCriteria => MESSAGE_INVALID_TABLE_FILTER_CRITERIA,
            Self::InvalidTotalsRowVisibility => MESSAGE_INVALID_TOTALS_ROW_VISIBILITY,
        })
    }
}

/// A validated, non-zero workbook-local Excel table identifier.
///
/// OOXML requires table IDs to be unique across the workbook. The workbook snapshot enforces
/// that cross-table invariant; this type enforces the non-zero scalar invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableId(NonZeroU32);

impl TableId {
    /// Validates and constructs a table ID.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::TableIdZero`] when `value` is zero.
    pub fn new(value: u32) -> Result<Self, ValidationError> {
        NonZeroU32::new(value)
            .map(Self)
            .ok_or(ValidationError::TableIdZero)
    }

    /// Returns the workbook-local numeric ID.
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl TryFrom<u32> for TableId {
    type Error = ValidationError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// A validated, non-zero identifier for one column within an Excel table.
///
/// OOXML column IDs are stable across column renames and must be unique within their owning
/// table. The [`Table`] constructor enforces that per-table uniqueness; this type enforces the
/// non-zero scalar invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableColumnId(NonZeroU32);

impl TableColumnId {
    /// Validates and constructs a table column ID.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::TableColumnIdZero`] when `value` is zero.
    pub fn new(value: u32) -> Result<Self, ValidationError> {
        NonZeroU32::new(value)
            .map(Self)
            .ok_or(ValidationError::TableColumnIdZero)
    }

    /// Returns the table-local numeric ID.
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl TryFrom<u32> for TableColumnId {
    type Error = ValidationError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// A table name with its original spelling preserved.
///
/// OOXML `name` and `displayName` have the same identifier grammar. Their scopes differ:
/// `displayName` is the workbook-global formula/UI name, while `name` is the worksheet-local
/// programmatic object-model name. Excel compares both case-insensitively; the original spelling
/// is retained for byte-accurate round trips.
///
/// [`TableName::new`] retains the validation contract shipped by CellRune 0.1.8. XLSX readers and
/// canonical writers additionally enforce the complete OOXML identifier grammar at the format
/// boundary.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableName {
    original: Box<str>,
    lookup_key: Box<str>,
}

impl TableName {
    /// Validates the stable core length and character constraints on a table name.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when the name is empty, exceeds 255 UTF-16 code units, or
    /// contains whitespace or control characters. XLSX serialization applies additional OOXML
    /// identifier and cell-reference-conflict rules.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ValidationError::TableNameEmpty);
        }
        let utf16_len = value.encode_utf16().count();
        if utf16_len > 255 {
            return Err(ValidationError::TableNameTooLong { utf16_len });
        }
        if let Some(character) = value
            .chars()
            .find(|character| character.is_control() || character.is_whitespace())
        {
            return Err(ValidationError::TableNameInvalidCharacter { character });
        }
        let lookup_key = case_insensitive_key(&value).into_boxed_str();
        Ok(Self {
            original: value.into_boxed_str(),
            lookup_key,
        })
    }

    pub(crate) fn from_xlsx(value: impl Into<String>) -> Result<Self, ValidationError> {
        let name = Self::new(value)?;
        name.validate_xlsx()?;
        Ok(name)
    }

    pub(crate) fn validate_xlsx(&self) -> Result<(), ValidationError> {
        let mut characters = self.original.chars();
        let first = characters
            .next()
            .expect("validated table name is non-empty");
        if !(first.is_alphabetic() || matches!(first, '_' | '\\')) {
            return Err(ValidationError::TableNameInvalidCharacter { character: first });
        }
        if let Some(character) = characters
            .find(|character| !(character.is_alphanumeric() || matches!(character, '_' | '.')))
        {
            return Err(ValidationError::TableNameInvalidCharacter { character });
        }
        if matches!(self.original.as_ref(), "R" | "r" | "C" | "c")
            || crate::CellAddress::from_a1(&self.original).is_ok()
            || is_r1c1_reference(&self.original)
        {
            return Err(ValidationError::TableNameReferenceConflict);
        }
        Ok(())
    }

    /// Returns the original spelling.
    pub fn as_str(&self) -> &str {
        &self.original
    }

    pub(crate) fn lookup_key(&self) -> &str {
        &self.lookup_key
    }
}

/// The data source represented by an Excel table definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TableType {
    /// A normal worksheet-backed table.
    #[default]
    Worksheet,
    /// An XML-mapped table.
    Xml,
    /// A query-table-backed table.
    QueryTable,
}

impl TableType {
    /// Returns the OOXML `ST_TableType` token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Worksheet => "worksheet",
            Self::Xml => "xml",
            Self::QueryTable => "queryTable",
        }
    }

    pub(crate) fn from_xlsx(value: &str) -> Option<Self> {
        match value {
            "worksheet" => Some(Self::Worksheet),
            "xml" => Some(Self::Xml),
            "queryTable" => Some(Self::QueryTable),
            _ => None,
        }
    }
}

/// A calculated-column or totals-row formula stored in a table definition.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableFormula {
    text: FormulaText,
    array: bool,
}

impl TableFormula {
    /// Constructs table-formula metadata from validated XLSX formula text.
    pub const fn new(text: FormulaText, array: bool) -> Self {
        Self { text, array }
    }

    /// Returns storage-form formula text without a leading equals sign.
    pub const fn text(&self) -> &FormulaText {
        &self.text
    }

    /// Returns whether the table formula is declared as an array formula.
    pub const fn is_array(&self) -> bool {
        self.array
    }

    pub(crate) fn with_text(&self, text: FormulaText) -> Self {
        Self {
            text,
            array: self.array,
        }
    }
}

/// The style flags attached to one table.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableStyleInfo {
    name: Option<Arc<str>>,
    show_first_column: bool,
    show_last_column: bool,
    show_row_stripes: bool,
    show_column_stripes: bool,
}

impl TableStyleInfo {
    /// Constructs table style metadata.
    pub fn new(
        name: Option<String>,
        show_first_column: bool,
        show_last_column: bool,
        show_row_stripes: bool,
        show_column_stripes: bool,
    ) -> Self {
        Self {
            name: name.map(Arc::from),
            show_first_column,
            show_last_column,
            show_row_stripes,
            show_column_stripes,
        }
    }

    /// Returns the named table style, when one is declared.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns whether the first-column style is enabled.
    pub const fn show_first_column(&self) -> bool {
        self.show_first_column
    }

    /// Returns whether the last-column style is enabled.
    pub const fn show_last_column(&self) -> bool {
        self.show_last_column
    }

    /// Returns whether alternating row stripes are enabled.
    pub const fn show_row_stripes(&self) -> bool {
        self.show_row_stripes
    }

    /// Returns whether alternating column stripes are enabled.
    pub const fn show_column_stripes(&self) -> bool {
        self.show_column_stripes
    }
}

/// The totals-row aggregation declared for one table column.
///
/// Mirrors the OOXML `ST_TotalsRowFunction` values other than `none`, which is modeled as
/// an absent function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TotalsRowFunction {
    /// `sum`.
    Sum,
    /// `min`.
    Min,
    /// `max`.
    Max,
    /// `average`.
    Average,
    /// `count`.
    Count,
    /// `countNums`.
    CountNumbers,
    /// `stdDev`.
    StdDev,
    /// `var`.
    Var,
    /// `custom` — the totals row uses a stored formula instead of a named aggregation.
    Custom,
}

impl TotalsRowFunction {
    /// Returns the OOXML `ST_TotalsRowFunction` token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Min => "min",
            Self::Max => "max",
            Self::Average => "average",
            Self::Count => "count",
            Self::CountNumbers => "countNums",
            Self::StdDev => "stdDev",
            Self::Var => "var",
            Self::Custom => "custom",
        }
    }
}

/// One table column with the stable XLSX column identifier.
///
/// The `@id` value survives column renames, so consumers that must keep a durable selector
/// across edits hold the identifier rather than the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableColumn {
    id: TableColumnId,
    name: Box<str>,
    totals_row_function: Option<TotalsRowFunction>,
    totals_row_label: Option<Arc<str>>,
    calculated_column_formula: Option<TableFormula>,
    totals_row_formula: Option<TableFormula>,
}

/// A validated table-column name for authoring operations.
///
/// Unlike table display names, column names may contain spaces, punctuation, and strings that
/// resemble A1 or R1C1 references. Structured-reference escaping handles those spellings.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableColumnName(Box<str>);

impl TableColumnName {
    /// Validates a table-column name at the authoring boundary.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when the name is empty, begins or ends with an ASCII space,
    /// exceeds 255 UTF-16 code units, or contains a character forbidden by XML 1.0.
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ValidationError::TableColumnNameEmpty);
        }
        if value.starts_with(' ') || value.ends_with(' ') {
            return Err(ValidationError::TableColumnNameSpaceBoundary);
        }
        let utf16_len = value.encode_utf16().count();
        if utf16_len > 255 {
            return Err(ValidationError::TableColumnNameTooLong { utf16_len });
        }
        if let Some(character) = value
            .chars()
            .find(|character| !is_xml_10_character(*character))
        {
            return Err(ValidationError::TableColumnNameInvalidCharacter { character });
        }
        Ok(Self(value.into_boxed_str()))
    }

    /// Returns the validated original spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_xml_10_character(character: char) -> bool {
    matches!(character, '\u{9}' | '\u{A}' | '\u{D}')
        || matches!(character as u32, 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

impl TableColumn {
    /// Validates and constructs a table column.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::TableColumnIdZero`] when `id` is zero or
    /// [`ValidationError::TableColumnNameEmpty`] when the column name is empty. XLSX readers and
    /// canonical writers additionally enforce OOXML's 255 UTF-16-code-unit column-name limit.
    pub fn new(
        id: u32,
        name: impl Into<String>,
        totals_row_function: Option<TotalsRowFunction>,
    ) -> Result<Self, ValidationError> {
        let id = TableColumnId::new(id)?;
        let name = name.into();
        if name.is_empty() {
            return Err(ValidationError::TableColumnNameEmpty);
        }
        Ok(Self {
            id,
            name: name.into_boxed_str(),
            totals_row_function,
            totals_row_label: None,
            calculated_column_formula: None,
            totals_row_formula: None,
        })
    }

    pub(crate) fn from_xlsx(
        id: u32,
        name: impl Into<String>,
        totals_row_function: Option<TotalsRowFunction>,
    ) -> Result<Self, ValidationError> {
        let column = Self::new(id, name, totals_row_function)?;
        column.validate_xlsx()?;
        Ok(column)
    }

    pub(crate) fn validate_xlsx(&self) -> Result<(), ValidationError> {
        let utf16_len = self.name.encode_utf16().count();
        if utf16_len > 255 {
            return Err(ValidationError::TableColumnNameTooLong { utf16_len });
        }
        Ok(())
    }

    pub(crate) fn with_metadata(
        mut self,
        totals_row_label: Option<String>,
        calculated_column_formula: Option<TableFormula>,
        totals_row_formula: Option<TableFormula>,
    ) -> Self {
        self.totals_row_label = totals_row_label.map(Arc::from);
        self.calculated_column_formula = calculated_column_formula;
        self.totals_row_formula = totals_row_formula;
        self
    }

    /// Returns the stable XLSX column identifier.
    pub const fn id(&self) -> u32 {
        self.id.get()
    }

    /// Returns the typed stable XLSX column identifier.
    ///
    /// [`Self::id`] remains available as the original scalar API.
    pub const fn column_id(&self) -> TableColumnId {
        self.id
    }

    /// Returns the column name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the declared totals-row aggregation, when present.
    pub const fn totals_row_function(&self) -> Option<TotalsRowFunction> {
        self.totals_row_function
    }

    /// Returns the totals-row label, when one is declared.
    pub fn totals_row_label(&self) -> Option<&str> {
        self.totals_row_label.as_deref()
    }

    /// Returns the calculated-column formula, when one is declared.
    pub const fn calculated_column_formula(&self) -> Option<&TableFormula> {
        self.calculated_column_formula.as_ref()
    }

    /// Returns the custom totals-row formula, when one is declared.
    pub const fn totals_row_formula(&self) -> Option<&TableFormula> {
        self.totals_row_formula.as_ref()
    }

    #[cfg(test)]
    fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        if cancelled() {
            return Err(());
        }
        let calculated_column_formula = self.calculated_column_formula.clone();
        if cancelled() {
            return Err(());
        }
        let totals_row_formula = self.totals_row_formula.clone();
        Ok(Self {
            id: self.id,
            name: self.name.clone(),
            totals_row_function: self.totals_row_function,
            totals_row_label: self.totals_row_label.clone(),
            calculated_column_formula,
            totals_row_formula,
        })
    }

    pub(crate) fn rename(&mut self, name: &TableColumnName) {
        self.name = Box::from(name.as_str());
    }

    pub(crate) fn rewrite_formulas(
        &mut self,
        calculated: Option<FormulaText>,
        totals: Option<FormulaText>,
    ) {
        if let (Some(formula), Some(text)) = (&self.calculated_column_formula, calculated) {
            self.calculated_column_formula = Some(formula.with_text(text));
        }
        if let (Some(formula), Some(text)) = (&self.totals_row_formula, totals) {
            self.totals_row_formula = Some(formula.with_text(text));
        }
    }
}

/// An Excel table (ListObject) definition owned by its worksheet.
///
/// The sheet owns its tables because the OOXML table part is a worksheet relationship and
/// `@ref` addresses that sheet's range; a `sheet_id` field would duplicate state that could
/// drift. Global name lookup lives on the workbook snapshot instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    id: TableId,
    name: TableName,
    display_name: TableName,
    range: CellRange,
    table_type: TableType,
    header_row_count: u32,
    totals_row_count: u32,
    totals_row_shown: bool,
    columns: Vec<TableColumn>,
    auto_filter: Option<TableAutoFilter>,
    sort_state: Option<TableSortState>,
    style_info: Option<TableStyleInfo>,
    opaque_source_xml: Option<Arc<[u8]>>,
}

impl Table {
    #[cfg(test)]
    pub(crate) fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        let mut columns = Vec::with_capacity(self.columns.len());
        for column in &self.columns {
            if cancelled() {
                return Err(());
            }
            columns.push(column.clone_cancellable(cancelled)?);
        }
        Ok(Self {
            id: self.id,
            name: self.name.clone(),
            display_name: self.display_name.clone(),
            range: self.range,
            table_type: self.table_type,
            header_row_count: self.header_row_count,
            totals_row_count: self.totals_row_count,
            totals_row_shown: self.totals_row_shown,
            columns,
            auto_filter: self
                .auto_filter
                .as_ref()
                .map(|filter| filter.clone_cancellable(cancelled))
                .transpose()?,
            sort_state: self
                .sort_state
                .as_ref()
                .map(|sort| sort.clone_cancellable(cancelled))
                .transpose()?,
            style_info: self.style_info.clone(),
            opaque_source_xml: self.opaque_source_xml.clone(),
        })
    }

    /// Validates internal consistency and constructs a table.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when the table declares no columns, the column count
    /// does not match the range width, column names repeat case-insensitively, column
    /// identifiers repeat, or the header and totals rows do not fit inside the range.
    pub fn new(
        id: TableId,
        name: TableName,
        display_name: TableName,
        range: CellRange,
        header_row_count: u32,
        totals_row_count: u32,
        columns: Vec<TableColumn>,
    ) -> Result<Self, ValidationError> {
        if columns.is_empty() {
            return Err(ValidationError::TableColumnsEmpty);
        }
        if columns.len() as u64 != u64::from(range.width()) {
            return Err(ValidationError::TableColumnCountMismatch {
                columns: columns.len(),
                width: range.width(),
            });
        }
        let mut column_names = std::collections::BTreeSet::new();
        let mut column_ids = std::collections::BTreeSet::new();
        for column in &columns {
            if !column_names.insert(case_insensitive_key(column.name())) {
                return Err(ValidationError::DuplicateTableColumnName {
                    name: column.name().to_owned(),
                });
            }
            if !column_ids.insert(column.id()) {
                return Err(ValidationError::DuplicateTableColumnId { id: column.id() });
            }
            let totals_formula = column.totals_row_formula().is_some();
            let totals_metadata_is_consistent = if column.totals_row_label().is_some() {
                column.totals_row_function().is_none() && !totals_formula
            } else {
                match column.totals_row_function() {
                    Some(TotalsRowFunction::Custom) => totals_formula,
                    Some(_) | None => !totals_formula,
                }
            };
            if !totals_metadata_is_consistent {
                return Err(ValidationError::InvalidTableTotalsMetadata);
            }
        }
        if u64::from(header_row_count) + u64::from(totals_row_count) > u64::from(range.height()) {
            return Err(ValidationError::TableRowCountsExceedRange {
                header_row_count,
                totals_row_count,
                height: range.height(),
            });
        }
        Ok(Self {
            id,
            name,
            display_name,
            range,
            table_type: TableType::Worksheet,
            header_row_count,
            totals_row_count,
            totals_row_shown: true,
            columns,
            auto_filter: None,
            sort_state: None,
            style_info: None,
            opaque_source_xml: None,
        })
    }

    pub(crate) fn try_with_metadata(
        mut self,
        table_type: TableType,
        totals_row_shown: bool,
        auto_filter: Option<TableAutoFilter>,
        sort_state: Option<TableSortState>,
        style_info: Option<TableStyleInfo>,
        opaque_source_xml: Option<Vec<u8>>,
    ) -> result::Result<Self, TableMetadataValidationError> {
        if self.totals_row_count == 1 && !totals_row_shown {
            return Err(TableMetadataValidationError::InvalidTotalsRowVisibility);
        }
        validate_table_metadata(self.range, auto_filter.as_ref(), sort_state.as_ref())?;
        self.table_type = table_type;
        self.totals_row_shown = totals_row_shown;
        self.auto_filter = auto_filter;
        self.sort_state = sort_state;
        self.style_info = style_info;
        self.opaque_source_xml = opaque_source_xml.map(Arc::from);
        Ok(self)
    }

    /// Returns the stable workbook-local OOXML table ID.
    pub const fn id(&self) -> TableId {
        self.id
    }

    /// Returns the worksheet-local programmatic object-model name (`@name`).
    pub const fn name(&self) -> &TableName {
        &self.name
    }

    /// Returns the workbook-global formula and UI name (`@displayName`).
    pub const fn display_name(&self) -> &TableName {
        &self.display_name
    }

    /// Returns the full table range including header and totals rows.
    pub const fn range(&self) -> CellRange {
        self.range
    }

    /// Returns the table's declared data-source type.
    pub const fn table_type(&self) -> TableType {
        self.table_type
    }

    /// Returns the declared header row count (Excel writes 0 or 1).
    pub const fn header_row_count(&self) -> u32 {
        self.header_row_count
    }

    /// Returns the declared totals row count (Excel writes 0 or 1).
    pub const fn totals_row_count(&self) -> u32 {
        self.totals_row_count
    }

    /// Returns whether the totals row is shown.
    pub const fn totals_row_shown(&self) -> bool {
        self.totals_row_shown
    }

    /// Returns columns in XLSX declaration order.
    pub fn columns(&self) -> &[TableColumn] {
        &self.columns
    }

    /// Returns table-owned auto-filter metadata, when present.
    pub const fn auto_filter(&self) -> Option<&TableAutoFilter> {
        self.auto_filter.as_ref()
    }

    /// Returns table-level sort metadata, when present.
    pub const fn sort_state(&self) -> Option<&TableSortState> {
        self.sort_state.as_ref()
    }

    /// Returns table style metadata, when present.
    pub const fn style_info(&self) -> Option<&TableStyleInfo> {
        self.style_info.as_ref()
    }

    /// Returns whether unmodeled source metadata must be preserved by a source-linked writer.
    pub const fn has_opaque_metadata(&self) -> bool {
        self.opaque_source_xml.is_some()
    }

    pub(crate) fn opaque_source_xml(&self) -> Option<&[u8]> {
        self.opaque_source_xml.as_deref()
    }

    pub(crate) fn semantic_eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.display_name == other.display_name
            && self.range == other.range
            && self.table_type == other.table_type
            && self.header_row_count == other.header_row_count
            && self.totals_row_count == other.totals_row_count
            && self.totals_row_shown == other.totals_row_shown
            && self.columns == other.columns
            && self.auto_filter == other.auto_filter
            && self.sort_state == other.sort_state
            && self.style_info == other.style_info
    }

    pub(crate) fn rename(&mut self, name: TableName) {
        self.name = name.clone();
        self.display_name = name;
    }

    pub(crate) fn rename_column(
        &mut self,
        column_id: TableColumnId,
        name: &TableColumnName,
    ) -> bool {
        let Some(column) = self
            .columns
            .iter_mut()
            .find(|column| column.column_id() == column_id)
        else {
            return false;
        };
        column.rename(name);
        true
    }

    pub(crate) fn columns_mut(&mut self) -> &mut [TableColumn] {
        &mut self.columns
    }

    pub(crate) fn resize_data_rows(
        &mut self,
        first_data_row: crate::Row,
        last_data_row: crate::Row,
    ) -> Result<(), ()> {
        let start_row = first_data_row
            .get()
            .checked_sub(self.header_row_count)
            .and_then(|row| crate::Row::new(row).ok())
            .ok_or(())?;
        let end_row = last_data_row
            .get()
            .checked_add(self.totals_row_count)
            .and_then(|row| crate::Row::new(row).ok())
            .ok_or(())?;
        let old_data_range = self.data_range();
        let new_data_range = CellRange::from_ordered(
            crate::CellAddress::new(first_data_row, self.range.start().column()),
            crate::CellAddress::new(last_data_row, self.range.end().column()),
        );
        let new_range = CellRange::from_ordered(
            crate::CellAddress::new(start_row, self.range.start().column()),
            crate::CellAddress::new(end_row, self.range.end().column()),
        );
        match old_data_range {
            Some(old_data_range) => {
                self.auto_filter = self
                    .auto_filter
                    .as_ref()
                    .map(|filter| {
                        filter.resized(
                            new_filter_range(new_range, self.totals_row_count),
                            old_data_range,
                            new_data_range,
                        )
                    })
                    .transpose()?;
                self.sort_state = self
                    .sort_state
                    .as_ref()
                    .map(|sort| sort.resized(old_data_range, new_data_range))
                    .transpose()?;
            }
            None => {
                if self.sort_state.is_some()
                    || self
                        .auto_filter
                        .as_ref()
                        .is_some_and(|filter| filter.sort_state().is_some())
                {
                    return Err(());
                }
                self.auto_filter = self.auto_filter.as_ref().map(|filter| {
                    filter.resized_from_empty(new_filter_range(new_range, self.totals_row_count))
                });
            }
        }
        self.range = new_range;
        Ok(())
    }

    pub(crate) fn data_range(&self) -> Option<CellRange> {
        let first = self
            .range
            .start()
            .row()
            .get()
            .checked_add(self.header_row_count)
            .and_then(|row| crate::Row::new(row).ok())?;
        let last = self
            .range
            .end()
            .row()
            .get()
            .checked_sub(self.totals_row_count)
            .and_then(|row| crate::Row::new(row).ok())?;
        (first <= last).then(|| {
            CellRange::from_ordered(
                crate::CellAddress::new(first, self.range.start().column()),
                crate::CellAddress::new(last, self.range.end().column()),
            )
        })
    }
}

fn new_filter_range(table_range: CellRange, totals_row_count: u32) -> CellRange {
    let end_row = crate::Row::new(
        table_range
            .end()
            .row()
            .get()
            .saturating_sub(totals_row_count),
    )
    .expect("validated table range retains at least one non-totals row");
    CellRange::from_ordered(
        crate::CellAddress::new(table_range.start().row(), table_range.start().column()),
        crate::CellAddress::new(end_row, table_range.end().column()),
    )
}

fn validate_table_metadata(
    table_range: CellRange,
    auto_filter: Option<&TableAutoFilter>,
    sort_state: Option<&TableSortState>,
) -> result::Result<(), TableMetadataValidationError> {
    if let Some(filter) = auto_filter {
        if !range_contains(table_range, filter.range()) {
            return Err(TableMetadataValidationError::RangeOutsideTable);
        }
        let mut column_ids = std::collections::BTreeSet::new();
        for column in filter.filter_columns() {
            if column.column_id() >= filter.range().width() {
                return Err(TableMetadataValidationError::FilterColumnOutOfRange);
            }
            if !column_ids.insert(column.column_id()) {
                return Err(TableMetadataValidationError::DuplicateFilterColumn);
            }
            if let Some(criteria) = column.criteria() {
                validate_filter_criteria(criteria)?;
            }
        }
        if let Some(sort) = filter.sort_state() {
            validate_sort_state(filter.range(), sort, false)?;
        }
    }
    if let Some(sort) = sort_state {
        validate_sort_state(table_range, sort, true)?;
    }
    Ok(())
}

fn validate_sort_state(
    parent_range: CellRange,
    sort: &TableSortState,
    honor_column_sort: bool,
) -> result::Result<(), TableMetadataValidationError> {
    if !range_contains(parent_range, sort.range()) {
        return Err(TableMetadataValidationError::RangeOutsideTable);
    }
    if sort.conditions().len() > MAX_TABLE_SORT_CONDITIONS {
        return Err(TableMetadataValidationError::TooManySortConditions);
    }
    let column_sort = honor_column_sort && sort.column_sort();
    for condition in sort.conditions() {
        let range = condition.range();
        if !range_contains(sort.range(), range)
            || if column_sort {
                range.height() != 1
            } else {
                range.width() != 1
            }
        {
            return Err(TableMetadataValidationError::InvalidSortCondition);
        }
        let sort_by = condition.sort_by().unwrap_or(TableSortBy::Value);
        let attributes_are_consistent = match sort_by {
            TableSortBy::Value => {
                condition.differential_format_id().is_none()
                    && condition.icon_set().is_none()
                    && condition.icon_id().is_none()
            }
            TableSortBy::CellColor | TableSortBy::FontColor => {
                condition.icon_set().is_none() && condition.icon_id().is_none()
            }
            TableSortBy::Icon => {
                condition.differential_format_id().is_none()
                    && condition.icon_id().is_none_or(|icon_id| {
                        icon_id
                            < condition
                                .icon_set()
                                .unwrap_or(crate::TableIconSet::ThreeArrows)
                                .icon_count()
                    })
            }
        };
        if !attributes_are_consistent {
            return Err(TableMetadataValidationError::InvalidSortCondition);
        }
    }
    Ok(())
}

fn validate_filter_criteria(
    criteria: &TableFilterCriteria,
) -> result::Result<(), TableMetadataValidationError> {
    match criteria {
        TableFilterCriteria::Values(filters) => {
            let mut saw_date_group = false;
            for item in filters.items() {
                match item {
                    TableFilterItem::Value(_) if saw_date_group => {
                        return Err(TableMetadataValidationError::InvalidFilterCriteria);
                    }
                    TableFilterItem::Value(_) => {}
                    TableFilterItem::DateGroup(_) => saw_date_group = true,
                }
            }
        }
        TableFilterCriteria::Custom(filters) => {
            if !(1..=2).contains(&filters.filters().len()) {
                return Err(TableMetadataValidationError::InvalidFilterCriteria);
            }
        }
        TableFilterCriteria::Icon(filter)
            if filter
                .icon_id()
                .is_some_and(|icon_id| icon_id >= filter.icon_set().icon_count()) =>
        {
            return Err(TableMetadataValidationError::InvalidFilterCriteria);
        }
        TableFilterCriteria::Dynamic(_)
        | TableFilterCriteria::Color(_)
        | TableFilterCriteria::Icon(_)
        | TableFilterCriteria::Top(_) => {}
    }
    Ok(())
}

fn range_contains(parent: CellRange, child: CellRange) -> bool {
    parent.contains(child.start()) && parent.contains(child.end())
}

fn is_r1c1_reference(value: &str) -> bool {
    let value = value.as_bytes();
    if !matches!(value.first(), Some(b'R' | b'r')) {
        return false;
    }
    let mut index = 1;
    let row_start = index;
    while value.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    if index == row_start || !matches!(value.get(index), Some(b'C' | b'c')) {
        return false;
    }
    index += 1;
    let column_start = index;
    while value.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    index > column_start && index == value.len()
}

fn case_insensitive_key(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{
        Table, TableAutoFilter, TableColumn, TableColumnId, TableColumnName, TableCustomFilters,
        TableDateGroupItem, TableDateTimeGrouping, TableFilterColumn, TableFilterCriteria,
        TableFilterItem, TableFormula, TableIconFilter, TableIconSet, TableId,
        TableMetadataValidationError, TableName, TableSortBy, TableSortCondition, TableSortState,
        TableValueFilters, TotalsRowFunction,
    };
    use crate::{
        CalculationHints, CellAddress, CellRange, DateSystem, DefinedName, DefinedNameScope,
        FormulaText, Provenance, ProviderIdentity, Row, Sheet, SheetId, SheetName, SheetVisibility,
        ValidationError, WorkbookSnapshot, WorkbookSource,
    };

    fn range(a1: &str, b1: &str) -> CellRange {
        CellRange::new(
            CellAddress::from_a1(a1).expect("start"),
            CellAddress::from_a1(b1).expect("end"),
        )
        .expect("range")
    }

    fn column(id: u32, name: &str) -> TableColumn {
        TableColumn::new(id, name, None).expect("column")
    }

    fn table(id: u32, name: &str, display_name: &str) -> Table {
        Table::new(
            TableId::new(id).expect("table id"),
            TableName::new(name).expect("name"),
            TableName::new(display_name).expect("display name"),
            range("A1", "B3"),
            1,
            0,
            vec![column(1, "First"), column(2, "Second")],
        )
        .expect("table")
    }

    fn sort_condition(range: CellRange) -> TableSortCondition {
        TableSortCondition::from_xlsx(range, false, None, None, None, None, None)
    }

    #[test]
    fn extended_table_metadata_is_validated_by_the_core_aggregate() {
        let outside_filter = TableAutoFilter::from_xlsx(range("A1", "C3"), true, Vec::new(), None);
        assert_eq!(
            table(1, "Sales", "Sales")
                .try_with_metadata(
                    super::TableType::Worksheet,
                    true,
                    Some(outside_filter),
                    None,
                    None,
                    None,
                )
                .expect_err("filter must stay inside table"),
            TableMetadataValidationError::RangeOutsideTable
        );

        let out_of_range_column = TableAutoFilter::from_xlsx(
            range("A1", "B3"),
            true,
            vec![TableFilterColumn::from_xlsx(2, false, true, None)],
            None,
        );
        assert_eq!(
            table(1, "Sales", "Sales")
                .try_with_metadata(
                    super::TableType::Worksheet,
                    true,
                    Some(out_of_range_column),
                    None,
                    None,
                    None,
                )
                .expect_err("filter column must fit range"),
            TableMetadataValidationError::FilterColumnOutOfRange
        );

        let duplicate_columns = TableAutoFilter::from_xlsx(
            range("A1", "B3"),
            true,
            vec![
                TableFilterColumn::from_xlsx(1, false, true, None),
                TableFilterColumn::from_xlsx(1, false, true, None),
            ],
            None,
        );
        assert_eq!(
            table(1, "Sales", "Sales")
                .try_with_metadata(
                    super::TableType::Worksheet,
                    true,
                    Some(duplicate_columns),
                    None,
                    None,
                    None,
                )
                .expect_err("filter column IDs must be unique"),
            TableMetadataValidationError::DuplicateFilterColumn
        );

        let too_many_conditions = TableSortState::from_xlsx(
            range("A1", "B3"),
            false,
            false,
            None,
            vec![sort_condition(range("A2", "A3")); 65],
        );
        assert_eq!(
            table(1, "Sales", "Sales")
                .try_with_metadata(
                    super::TableType::Worksheet,
                    true,
                    None,
                    Some(too_many_conditions),
                    None,
                    None,
                )
                .expect_err("sort state must enforce cardinality"),
            TableMetadataValidationError::TooManySortConditions
        );

        let incorrectly_oriented = TableSortState::from_xlsx(
            range("A1", "B3"),
            false,
            false,
            None,
            vec![sort_condition(range("A2", "B3"))],
        );
        assert_eq!(
            table(1, "Sales", "Sales")
                .try_with_metadata(
                    super::TableType::Worksheet,
                    true,
                    None,
                    Some(incorrectly_oriented),
                    None,
                    None,
                )
                .expect_err("row-oriented sort conditions must be single-column"),
            TableMetadataValidationError::InvalidSortCondition
        );

        let invalid_custom_filter = TableAutoFilter::from_xlsx(
            range("A1", "B3"),
            true,
            vec![TableFilterColumn::from_xlsx(
                0,
                false,
                true,
                Some(TableFilterCriteria::Custom(TableCustomFilters::from_xlsx(
                    false,
                    Vec::new(),
                ))),
            )],
            None,
        );
        assert_eq!(
            table(1, "Sales", "Sales")
                .try_with_metadata(
                    super::TableType::Worksheet,
                    true,
                    Some(invalid_custom_filter),
                    None,
                    None,
                    None,
                )
                .expect_err("custom filters must contain one or two comparisons"),
            TableMetadataValidationError::InvalidFilterCriteria
        );

        let date = TableDateGroupItem::from_xlsx(
            2026,
            None,
            None,
            None,
            None,
            None,
            TableDateTimeGrouping::Year,
        )
        .expect("date group");
        let invalid_value_order = TableAutoFilter::from_xlsx(
            range("A1", "B3"),
            true,
            vec![TableFilterColumn::from_xlsx(
                0,
                false,
                true,
                Some(TableFilterCriteria::Values(TableValueFilters::from_xlsx(
                    false,
                    None,
                    vec![
                        TableFilterItem::DateGroup(date),
                        TableFilterItem::Value(None),
                    ],
                ))),
            )],
            None,
        );
        assert_eq!(
            table(1, "Sales", "Sales")
                .try_with_metadata(
                    super::TableType::Worksheet,
                    true,
                    Some(invalid_value_order),
                    None,
                    None,
                    None,
                )
                .expect_err("literal values must precede grouped dates"),
            TableMetadataValidationError::InvalidFilterCriteria
        );

        let invalid_icon = TableAutoFilter::from_xlsx(
            range("A1", "B3"),
            true,
            vec![TableFilterColumn::from_xlsx(
                0,
                false,
                true,
                Some(TableFilterCriteria::Icon(TableIconFilter::from_xlsx(
                    TableIconSet::ThreeArrows,
                    Some(3),
                ))),
            )],
            None,
        );
        assert_eq!(
            table(1, "Sales", "Sales")
                .try_with_metadata(
                    super::TableType::Worksheet,
                    true,
                    Some(invalid_icon),
                    None,
                    None,
                    None,
                )
                .expect_err("icon IDs must fit their icon set"),
            TableMetadataValidationError::InvalidFilterCriteria
        );

        let invalid_value_sort = TableSortState::from_xlsx(
            range("A1", "B3"),
            false,
            false,
            None,
            vec![TableSortCondition::from_xlsx(
                range("A2", "A3"),
                false,
                Some(TableSortBy::Value),
                None,
                Some(4),
                None,
                None,
            )],
        );
        assert_eq!(
            table(1, "Sales", "Sales")
                .try_with_metadata(
                    super::TableType::Worksheet,
                    true,
                    None,
                    Some(invalid_value_sort),
                    None,
                    None,
                )
                .expect_err("value sorts cannot reference differential formats"),
            TableMetadataValidationError::InvalidSortCondition
        );

        let color_sort_without_dxf = TableSortState::from_xlsx(
            range("A1", "B3"),
            false,
            false,
            None,
            vec![TableSortCondition::from_xlsx(
                range("A2", "A3"),
                false,
                Some(TableSortBy::CellColor),
                None,
                None,
                None,
                None,
            )],
        );
        table(1, "Sales", "Sales")
            .try_with_metadata(
                super::TableType::Worksheet,
                true,
                None,
                Some(color_sort_without_dxf),
                None,
                None,
            )
            .expect("optional OOXML sort attributes remain valid when absent");
    }

    #[test]
    fn table_resize_preserves_row_and_column_sort_orientation() {
        let row_sort = TableSortState::from_xlsx(
            range("A2", "B3"),
            false,
            false,
            None,
            vec![sort_condition(range("B2", "B3"))],
        );
        let filter =
            TableAutoFilter::from_xlsx(range("A1", "B3"), true, Vec::new(), Some(row_sort));
        let column_sort = TableSortState::from_xlsx(
            range("A2", "B3"),
            false,
            true,
            None,
            vec![sort_condition(range("A2", "B2"))],
        );
        let mut table = table(1, "Sales", "Sales")
            .try_with_metadata(
                super::TableType::Worksheet,
                true,
                Some(filter),
                Some(column_sort),
                None,
                None,
            )
            .expect("valid row- and column-oriented sort metadata");

        table
            .resize_data_rows(Row::new(2).expect("first"), Row::new(5).expect("last"))
            .expect("resize with both sort orientations");

        let row_sort = table
            .auto_filter()
            .and_then(TableAutoFilter::sort_state)
            .expect("row sort");
        assert!(!row_sort.column_sort());
        assert_eq!(row_sort.range(), range("A2", "B5"));
        assert_eq!(row_sort.conditions()[0].range(), range("B2", "B5"));

        let column_sort = table.sort_state().expect("column sort");
        assert!(column_sort.column_sort());
        assert_eq!(column_sort.range(), range("A2", "B5"));
        assert_eq!(column_sort.conditions()[0].range(), range("A2", "B2"));
    }

    #[test]
    fn table_clone_polls_cancellation_inside_filter_criteria() {
        let items = (0..32)
            .map(|index| TableFilterItem::Value(Some(format!("value-{index}").into())))
            .collect();
        let filter = TableAutoFilter::from_xlsx(
            range("A1", "B3"),
            true,
            vec![TableFilterColumn::from_xlsx(
                0,
                false,
                true,
                Some(TableFilterCriteria::Values(TableValueFilters::from_xlsx(
                    false, None, items,
                ))),
            )],
            None,
        );
        let table = table(1, "Sales", "Sales")
            .try_with_metadata(
                super::TableType::Worksheet,
                true,
                Some(filter),
                None,
                None,
                None,
            )
            .expect("valid table metadata");
        let polls = Cell::new(0_u32);
        let cancelled = || {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 6
        };

        assert_eq!(table.clone_cancellable(&cancelled), Err(()));
        assert!(polls.get() >= 6);
    }

    #[test]
    fn table_name_is_case_insensitive_and_preserves_spelling() {
        assert_eq!(TableId::new(0), Err(ValidationError::TableIdZero));
        assert_eq!(
            TableId::new(u32::MAX).expect("maximum table id").get(),
            u32::MAX
        );
        assert_eq!(TableId::new(7).expect("table id").get(), 7);
        assert_eq!(
            TableColumnId::new(0),
            Err(ValidationError::TableColumnIdZero)
        );
        assert_eq!(TableColumnId::new(9).expect("column id").get(), 9);
        let name = TableName::new("SalesTable").expect("name");
        assert_eq!(name.as_str(), "SalesTable");
        assert_eq!(name.lookup_key(), "salestable");
        assert_eq!(TableName::new(""), Err(ValidationError::TableNameEmpty));
        assert_eq!(
            TableName::new("has space"),
            Err(ValidationError::TableNameInvalidCharacter { character: ' ' })
        );
        assert!(TableName::new("A1").is_ok());
        assert!(TableName::new("r1c1").is_ok());
        assert!(TableName::new("1Sales").is_ok());
        assert!(TableName::new("Sales-2026").is_ok());
        assert_eq!(
            TableName::from_xlsx("A1"),
            Err(ValidationError::TableNameReferenceConflict)
        );
        assert_eq!(
            TableName::from_xlsx("r1c1"),
            Err(ValidationError::TableNameReferenceConflict)
        );
        assert_eq!(
            TableName::from_xlsx("1Sales"),
            Err(ValidationError::TableNameInvalidCharacter { character: '1' })
        );
        assert_eq!(
            TableName::from_xlsx("Sales-2026"),
            Err(ValidationError::TableNameInvalidCharacter { character: '-' })
        );
        assert!(TableName::new("_매출.2026").is_ok());
        assert!(TableName::new("\\Local").is_ok());
        assert!(matches!(
            TableName::new("x".repeat(256)),
            Err(ValidationError::TableNameTooLong { utf16_len: 256 })
        ));
    }

    #[test]
    fn table_validation_rejects_inconsistent_definitions() {
        let id = || TableId::new(1).expect("id");
        let name = || TableName::new("T").expect("name");
        assert_eq!(
            Table::new(id(), name(), name(), range("A1", "B3"), 1, 0, Vec::new()),
            Err(ValidationError::TableColumnsEmpty)
        );
        assert_eq!(
            Table::new(
                id(),
                name(),
                name(),
                range("A1", "B3"),
                1,
                0,
                vec![column(1, "Only")],
            ),
            Err(ValidationError::TableColumnCountMismatch {
                columns: 1,
                width: 2,
            })
        );
        assert_eq!(
            Table::new(
                id(),
                name(),
                name(),
                range("A1", "B3"),
                1,
                0,
                vec![column(1, "Dup"), column(2, "DUP")],
            ),
            Err(ValidationError::DuplicateTableColumnName {
                name: "DUP".to_owned(),
            })
        );
        assert_eq!(
            Table::new(
                id(),
                name(),
                name(),
                range("A1", "B3"),
                1,
                0,
                vec![column(7, "First"), column(7, "Second")],
            ),
            Err(ValidationError::DuplicateTableColumnId { id: 7 })
        );
        assert_eq!(
            Table::new(
                id(),
                name(),
                name(),
                range("A1", "B1"),
                1,
                1,
                vec![column(1, "First"), column(2, "Second")],
            ),
            Err(ValidationError::TableRowCountsExceedRange {
                header_row_count: 1,
                totals_row_count: 1,
                height: 1,
            })
        );
        assert_eq!(
            TableColumn::new(1, "", Some(TotalsRowFunction::Sum)),
            Err(ValidationError::TableColumnNameEmpty)
        );
        assert_eq!(
            TableColumn::new(0, "First", None),
            Err(ValidationError::TableColumnIdZero)
        );
        assert_eq!(
            TableColumnName::new(" Amount"),
            Err(ValidationError::TableColumnNameSpaceBoundary)
        );
        assert_eq!(
            TableColumnName::new("Amount "),
            Err(ValidationError::TableColumnNameSpaceBoundary)
        );
        assert!(TableColumnName::new("Gross Amount").is_ok());
        assert!(TableColumn::new(1, "😀".repeat(128), None).is_ok());
        assert_eq!(
            TableColumn::from_xlsx(1, "😀".repeat(128), None),
            Err(ValidationError::TableColumnNameTooLong { utf16_len: 256 })
        );

        let formula = || {
            TableFormula::new(
                FormulaText::from_xlsx("SUBTOTAL(109,[Value])").expect("formula"),
                false,
            )
        };
        let single_column_table =
            |column| Table::new(id(), name(), name(), range("A1", "A3"), 1, 1, vec![column]);
        assert_eq!(
            single_column_table(
                TableColumn::new(1, "Value", Some(TotalsRowFunction::Sum))
                    .expect("column")
                    .with_metadata(Some("Total".to_owned()), None, None),
            ),
            Err(ValidationError::InvalidTableTotalsMetadata)
        );
        assert_eq!(
            single_column_table(
                TableColumn::new(1, "Value", Some(TotalsRowFunction::Custom)).expect("column"),
            ),
            Err(ValidationError::InvalidTableTotalsMetadata)
        );
        assert_eq!(
            single_column_table(
                TableColumn::new(1, "Value", None)
                    .expect("column")
                    .with_metadata(None, None, Some(formula())),
            ),
            Err(ValidationError::InvalidTableTotalsMetadata)
        );

        let counted_totals = Table::new(
            id(),
            name(),
            name(),
            range("A1", "A3"),
            1,
            1,
            vec![column(1, "Value")],
        )
        .expect("table");
        assert_eq!(
            counted_totals
                .try_with_metadata(super::TableType::Worksheet, false, None, None, None, None,)
                .expect_err("a single totals row must be marked as shown"),
            TableMetadataValidationError::InvalidTotalsRowVisibility
        );

        Table::new(
            id(),
            name(),
            name(),
            range("A1", "A4"),
            1,
            2,
            vec![column(1, "Value")],
        )
        .expect("table")
        .try_with_metadata(super::TableType::Worksheet, false, None, None, None, None)
        .expect("the totals-row history flag is independent for non-Excel row counts");
    }

    #[test]
    fn snapshot_indexes_display_names_and_enforces_table_identity_scopes() {
        let sheet_name = |value: &str| SheetName::new(value).expect("sheet name");
        let mut first = Sheet::new(
            SheetId::new(1).expect("id"),
            sheet_name("One"),
            SheetVisibility::Visible,
        );
        first.set_tables(vec![table(1, "Local", "Alpha")]);
        let mut second = Sheet::new(
            SheetId::new(2).expect("id"),
            sheet_name("Two"),
            SheetVisibility::Visible,
        );
        second.set_tables(vec![table(2, "Local", "Beta")]);
        let snapshot = WorkbookSnapshot::new_with_metadata(
            vec![first.clone(), second],
            Vec::new(),
            Vec::new(),
            DateSystem::Excel1900,
            CalculationHints::default(),
            WorkbookSource::default(),
            Provenance::new(ProviderIdentity::new("test", "0").expect("provider"), None),
        )
        .expect("snapshot");
        assert_eq!(
            snapshot
                .table("ALPHA")
                .expect("alpha")
                .display_name()
                .as_str(),
            "Alpha"
        );
        let alpha_id = TableId::new(1).expect("table id");
        let first_column_id = TableColumnId::new(1).expect("column id");
        assert_eq!(
            snapshot
                .table_by_id(alpha_id)
                .expect("table by stable id")
                .display_name()
                .as_str(),
            "Alpha"
        );
        assert_eq!(
            snapshot
                .table_column_by_id(alpha_id, first_column_id)
                .expect("column by stable id")
                .name(),
            "First"
        );
        assert_eq!(
            snapshot
                .table_column(alpha_id, "fIRSt")
                .expect("column by case-insensitive name")
                .column_id(),
            first_column_id
        );
        assert_eq!(
            snapshot
                .containing_table(
                    SheetId::new(1).expect("sheet id"),
                    CellAddress::from_a1("B2").expect("address"),
                )
                .expect("containing table")
                .id(),
            alpha_id
        );
        assert!(
            snapshot
                .containing_table(
                    SheetId::new(1).expect("sheet id"),
                    CellAddress::from_a1("C2").expect("address"),
                )
                .is_none()
        );
        assert!(
            snapshot
                .table_by_id(TableId::new(999).expect("missing table id"))
                .is_none()
        );
        assert!(
            snapshot
                .table_column_by_id(
                    alpha_id,
                    TableColumnId::new(999).expect("missing column id"),
                )
                .is_none()
        );
        assert!(snapshot.table_column(alpha_id, "Missing").is_none());
        let beta_id = TableId::new(2).expect("table id");
        assert_eq!(
            snapshot
                .table_column_by_id(beta_id, first_column_id)
                .expect("same column id belongs to the second table")
                .name(),
            "First"
        );
        assert_eq!(
            snapshot
                .containing_table(
                    SheetId::new(2).expect("sheet id"),
                    CellAddress::from_a1("B2").expect("address"),
                )
                .expect("same geometry is indexed per sheet")
                .id(),
            beta_id
        );
        assert_eq!(
            snapshot
                .table("beta")
                .expect("beta")
                .display_name()
                .as_str(),
            "Beta"
        );
        assert!(snapshot.table("Gamma").is_none());
        assert!(snapshot.table("Local").is_none());
        let cloned = snapshot.clone();
        assert_eq!(
            cloned
                .table_column_by_id(alpha_id, first_column_id)
                .expect("cloned snapshot keeps the stable index")
                .name(),
            "First"
        );
        assert_eq!(
            cloned
                .containing_table(
                    SheetId::new(2).expect("sheet id"),
                    CellAddress::from_a1("B2").expect("address"),
                )
                .expect("cloned snapshot keeps the spatial index")
                .id(),
            beta_id
        );
        let clone_polls = Cell::new(0_u32);
        let cancelled = || {
            let next = clone_polls.get() + 1;
            clone_polls.set(next);
            next >= 14
        };
        assert!(
            snapshot.clone_cancellable(&cancelled).is_err(),
            "snapshot clone must propagate cancellation from the table index"
        );
        assert_eq!(clone_polls.get(), 14);

        let mut duplicate = Sheet::new(
            SheetId::new(2).expect("id"),
            sheet_name("Two"),
            SheetVisibility::Visible,
        );
        duplicate.set_tables(vec![table(3, "Other", "ALPHA")]);
        let error = WorkbookSnapshot::new_with_metadata(
            vec![first.clone(), duplicate],
            Vec::new(),
            Vec::new(),
            DateSystem::Excel1900,
            CalculationHints::default(),
            WorkbookSource::default(),
            Provenance::new(ProviderIdentity::new("test", "0").expect("provider"), None),
        )
        .expect_err("duplicate display names must be rejected");
        assert_eq!(
            error,
            ValidationError::DuplicateTableDisplayName {
                name: "ALPHA".to_owned(),
            }
        );

        let mut duplicate_id = Sheet::new(
            SheetId::new(2).expect("id"),
            sheet_name("Two"),
            SheetVisibility::Visible,
        );
        duplicate_id.set_tables(vec![table(1, "Other", "Beta")]);
        let error = WorkbookSnapshot::new_with_metadata(
            vec![first.clone(), duplicate_id],
            Vec::new(),
            Vec::new(),
            DateSystem::Excel1900,
            CalculationHints::default(),
            WorkbookSource::default(),
            Provenance::new(ProviderIdentity::new("test", "0").expect("provider"), None),
        )
        .expect_err("duplicate table IDs must be rejected");
        assert_eq!(error, ValidationError::DuplicateTableId { id: 1 });

        first.set_tables(vec![table(1, "Local", "Alpha"), table(2, "LOCAL", "Beta")]);
        let error = WorkbookSnapshot::new_with_metadata(
            vec![first],
            Vec::new(),
            Vec::new(),
            DateSystem::Excel1900,
            CalculationHints::default(),
            WorkbookSource::default(),
            Provenance::new(ProviderIdentity::new("test", "0").expect("provider"), None),
        )
        .expect_err("programmatic names must be unique within one sheet");
        assert_eq!(
            error,
            ValidationError::DuplicateTableProgrammaticName {
                name: "LOCAL".to_owned(),
            }
        );

        let mut first = Sheet::new(
            SheetId::new(1).expect("id"),
            sheet_name("One"),
            SheetVisibility::Visible,
        );
        first.set_tables(vec![table(1, "Local", "Alpha")]);
        let defined_name = DefinedName::new(
            "ALPHA",
            DefinedNameScope::Workbook,
            FormulaText::from_xlsx("1").expect("formula"),
            false,
        )
        .expect("defined name");
        let error = WorkbookSnapshot::new_with_metadata(
            vec![first],
            vec![defined_name],
            Vec::new(),
            DateSystem::Excel1900,
            CalculationHints::default(),
            WorkbookSource::default(),
            Provenance::new(ProviderIdentity::new("test", "0").expect("provider"), None),
        )
        .expect_err("table display names must not conflict with defined names");
        assert_eq!(
            error,
            ValidationError::TableDisplayNameConflictsWithDefinedName {
                name: "Alpha".to_owned(),
            }
        );

        let mut overlapping = Sheet::new(
            SheetId::new(1).expect("id"),
            sheet_name("One"),
            SheetVisibility::Visible,
        );
        overlapping.set_tables(vec![
            table(1, "FirstTable", "FirstDisplay"),
            table(2, "SecondTable", "SecondDisplay"),
        ]);
        let error = WorkbookSnapshot::new_with_metadata(
            vec![overlapping],
            Vec::new(),
            Vec::new(),
            DateSystem::Excel1900,
            CalculationHints::default(),
            WorkbookSource::default(),
            Provenance::new(ProviderIdentity::new("test", "0").expect("provider"), None),
        )
        .expect_err("overlapping tables make containing-table resolution ambiguous");
        assert_eq!(
            error,
            ValidationError::OverlappingTables {
                sheet_id: 1,
                first_table_id: 1,
                second_table_id: 2,
            }
        );
    }
}
