use std::num::NonZeroU32;

use crate::{CellRange, ValidationError};

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

/// A validated Excel table name with its original spelling preserved.
///
/// Both OOXML `name` and `displayName` use the same character and length constraints. Their
/// scopes differ: `displayName` is the workbook-global formula/UI name, while `name` is the
/// worksheet-local programmatic object-model name. Excel compares both case-insensitively; the
/// original spelling is retained for byte-accurate round trips.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TableName {
    original: Box<str>,
    lookup_key: Box<str>,
}

impl TableName {
    /// Validates length and character constraints on a table name.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when the name is empty, exceeds 255 UTF-16 code units,
    /// or contains whitespace or control characters.
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

    /// Returns the original spelling.
    pub fn as_str(&self) -> &str {
        &self.original
    }

    pub(crate) fn lookup_key(&self) -> &str {
        &self.lookup_key
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
    id: u32,
    name: Box<str>,
    totals_row_function: Option<TotalsRowFunction>,
}

impl TableColumn {
    /// Validates and constructs a table column.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::TableColumnNameEmpty`] when the column name is empty.
    pub fn new(
        id: u32,
        name: impl Into<String>,
        totals_row_function: Option<TotalsRowFunction>,
    ) -> Result<Self, ValidationError> {
        let name = name.into();
        if name.is_empty() {
            return Err(ValidationError::TableColumnNameEmpty);
        }
        Ok(Self {
            id,
            name: name.into_boxed_str(),
            totals_row_function,
        })
    }

    /// Returns the stable XLSX column identifier.
    pub const fn id(&self) -> u32 {
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
    header_row_count: u32,
    totals_row_count: u32,
    columns: Vec<TableColumn>,
}

impl Table {
    pub(crate) fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        let mut columns = Vec::with_capacity(self.columns.len());
        for column in &self.columns {
            if cancelled() {
                return Err(());
            }
            columns.push(column.clone());
        }
        Ok(Self {
            id: self.id,
            name: self.name.clone(),
            display_name: self.display_name.clone(),
            range: self.range,
            header_row_count: self.header_row_count,
            totals_row_count: self.totals_row_count,
            columns,
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
            header_row_count,
            totals_row_count,
            columns,
        })
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

    /// Returns the declared header row count (Excel writes 0 or 1).
    pub const fn header_row_count(&self) -> u32 {
        self.header_row_count
    }

    /// Returns the declared totals row count (Excel writes 0 or 1).
    pub const fn totals_row_count(&self) -> u32 {
        self.totals_row_count
    }

    /// Returns columns in XLSX declaration order.
    pub fn columns(&self) -> &[TableColumn] {
        &self.columns
    }
}

fn case_insensitive_key(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

#[cfg(test)]
mod tests {
    use super::{Table, TableColumn, TableId, TableName, TotalsRowFunction};
    use crate::{
        CalculationHints, CellAddress, CellRange, DateSystem, DefinedName, DefinedNameScope,
        FormulaText, Provenance, ProviderIdentity, Sheet, SheetId, SheetName, SheetVisibility,
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

    #[test]
    fn table_name_is_case_insensitive_and_preserves_spelling() {
        assert_eq!(TableId::new(0), Err(ValidationError::TableIdZero));
        assert_eq!(TableId::new(7).expect("table id").get(), 7);
        let name = TableName::new("SalesTable").expect("name");
        assert_eq!(name.as_str(), "SalesTable");
        assert_eq!(name.lookup_key(), "salestable");
        assert_eq!(TableName::new(""), Err(ValidationError::TableNameEmpty));
        assert_eq!(
            TableName::new("has space"),
            Err(ValidationError::TableNameInvalidCharacter { character: ' ' })
        );
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
    }
}
