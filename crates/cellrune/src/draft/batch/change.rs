use crate::{
    CalculationHints, CellAddress, CellFormatEdit, CellRange, CellValue, Column, DateSystem,
    DefinedName, DefinedNameScope, FormulaText, NumberFormat, Row, SheetId, SheetName,
    SheetVisibility, TableColumnId, TableColumnName, TableId, TableName, ValidationError,
};

/// One validated workbook mutation in an atomic [`EditBatch`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum WorkbookChange {
    /// Sets a literal value, or clears the sparse cell when the value is blank.
    SetCellValue {
        /// Target sheet.
        sheet_id: SheetId,
        /// Target address.
        address: CellAddress,
        /// New typed value.
        value: CellValue,
    },
    /// Sets a normal formula with no trusted saved result.
    SetCellFormula {
        /// Target sheet.
        sheet_id: SheetId,
        /// Target address.
        address: CellAddress,
        /// Formula text without a leading equals sign.
        formula: FormulaText,
    },
    /// Sets a dynamic-array formula and its optional declared spill range.
    SetCellDynamicFormula {
        /// Target sheet.
        sheet_id: SheetId,
        /// Anchor address.
        address: CellAddress,
        /// Formula text without a leading equals sign.
        formula: FormulaText,
        /// Optional producer-declared spill range.
        range: Option<CellRange>,
    },
    /// Removes one sparse cell.
    ClearCell {
        /// Target sheet.
        sheet_id: SheetId,
        /// Target address.
        address: CellAddress,
    },
    /// Replaces the number format of an existing sparse cell.
    SetCellNumberFormat {
        /// Target sheet.
        sheet_id: SheetId,
        /// Target address.
        address: CellAddress,
        /// New number format.
        number_format: NumberFormat,
    },
    /// Adds one visible empty sheet.
    AddSheet {
        /// New unique sheet name.
        name: SheetName,
    },
    /// Renames a sheet and rewrites matching formula and defined-name references.
    RenameSheet {
        /// Target sheet.
        sheet_id: SheetId,
        /// New unique sheet name.
        name: SheetName,
    },
    /// Deletes a sheet, the names scoped to it, and its tables, turning references to them in the
    /// remaining formulas and names into `#REF!` while retaining at least one visible sheet.
    DeleteSheet {
        /// Target sheet.
        sheet_id: SheetId,
    },
    /// Changes sheet visibility while retaining at least one visible sheet.
    SetSheetVisibility {
        /// Target sheet.
        sheet_id: SheetId,
        /// New visibility.
        visibility: SheetVisibility,
    },
    /// Adds or replaces a defined name.
    SetDefinedName {
        /// Complete validated name definition.
        defined_name: DefinedName,
    },
    /// Removes a defined name using case-insensitive identity.
    RemoveDefinedName {
        /// Workbook or sheet scope.
        scope: DefinedNameScope,
        /// Name to remove.
        name: Box<str>,
    },
    /// Changes the workbook date system.
    SetDateSystem {
        /// New date system.
        date_system: DateSystem,
    },
    /// Changes calculation metadata without initiating calculation.
    SetCalculationHints {
        /// New calculation hints.
        calculation_hints: CalculationHints,
    },
    /// Renames a table while retaining its stable identifier.
    RenameTable {
        /// Target table.
        table_id: TableId,
        /// New programmatic and display name.
        new_display_name: TableName,
    },
    /// Renames one table column while retaining its stable identifier.
    RenameTableColumn {
        /// Target table.
        table_id: TableId,
        /// Target column.
        column_id: TableColumnId,
        /// New column name.
        new_name: TableColumnName,
    },
    /// Changes only the inclusive data-body row range of one table.
    ResizeTableRows {
        /// Target table.
        table_id: TableId,
        /// First one-based data-body row.
        first_data_row: Row,
        /// Last one-based data-body row.
        last_data_row: Row,
    },
    /// Widens one table by columns, naming each new column from the text in its header cell.
    WidenTableColumns {
        /// Target table.
        table_id: TableId,
        /// New full table range, which keeps the current rows and contains the current range.
        range: CellRange,
    },
    /// Widens a sheet-level auto-filter by columns and aligns its hidden `_xlnm._FilterDatabase`
    /// name.
    WidenSheetAutoFilter {
        /// Target sheet.
        sheet_id: SheetId,
        /// New filter range, which keeps the current rows and contains the current range.
        range: CellRange,
    },
    /// Inserts `count` empty rows before `row`, moving that row and every later one down.
    ///
    /// References to the moved cells follow them, and a range the rows are inserted strictly
    /// inside grows. The inserted rows take the height and formats of the row above them.
    InsertRows {
        /// Target sheet.
        sheet_id: SheetId,
        /// First inserted row.
        row: Row,
        /// Number of inserted rows, at least one.
        count: u32,
    },
    /// Deletes rows `first..=last`, moving every later row up.
    ///
    /// A reference to deleted cells becomes `#REF!`, and a range that loses only some of its
    /// rows shrinks.
    DeleteRows {
        /// Target sheet.
        sheet_id: SheetId,
        /// First deleted row.
        first: Row,
        /// Last deleted row.
        last: Row,
    },
    /// Inserts `count` empty columns before `column`, moving that column and every later one
    /// right, as [`Self::InsertRows`] moves rows.
    InsertColumns {
        /// Target sheet.
        sheet_id: SheetId,
        /// First inserted column.
        column: Column,
        /// Number of inserted columns, at least one.
        count: u32,
    },
    /// Deletes columns `first..=last`, moving every later column left, as [`Self::DeleteRows`]
    /// deletes rows.
    DeleteColumns {
        /// Target sheet.
        sheet_id: SheetId,
        /// First deleted column.
        first: Column,
        /// Last deleted column.
        last: Column,
    },
    /// Moves rows `first..=last` so that they start at row `destination` afterwards, as cutting
    /// them and inserting the cut rows does.
    ///
    /// References to the moved rows follow them, and each end of a range follows its row; a range
    /// whose ends would cross shrinks as if the moved rows were deleted.
    MoveRows {
        /// Target sheet.
        sheet_id: SheetId,
        /// First moved row.
        first: Row,
        /// Last moved row.
        last: Row,
        /// Row the first moved row lands on.
        destination: Row,
    },
    /// Moves columns `first..=last` so that they start at column `destination` afterwards, as
    /// [`Self::MoveRows`] moves rows.
    MoveColumns {
        /// Target sheet.
        sheet_id: SheetId,
        /// First moved column.
        first: Column,
        /// Last moved column.
        last: Column,
        /// Column the first moved column lands on.
        destination: Column,
    },
    /// Replaces the font, fill, border, or alignment one cell displays with.
    ///
    /// This is a presentation change: it advances the presentation revision, not the semantic
    /// one, and never invalidates calculation. The cell need not hold a value.
    SetCellFormat {
        /// Target sheet.
        sheet_id: SheetId,
        /// Target address.
        address: CellAddress,
        /// Components to replace; the others keep their current value.
        edit: CellFormatEdit,
    },
    /// Sets or clears the width of an inclusive column span, a presentation change.
    SetColumnWidth {
        /// Target sheet.
        sheet_id: SheetId,
        /// First column.
        first: Column,
        /// Last column.
        last: Column,
        /// Width in characters from 0 through 255, or `None` for the sheet's default width.
        width: Option<f64>,
    },
    /// Sets or clears the height of one row, a presentation change.
    SetRowHeight {
        /// Target sheet.
        sheet_id: SheetId,
        /// Target row.
        row: Row,
        /// Height in points from 0 through 409, or `None` for the sheet's default height.
        height: Option<f64>,
    },
}

impl WorkbookChange {
    /// Constructs a literal-cell change.
    pub const fn set_cell_value(sheet_id: SheetId, address: CellAddress, value: CellValue) -> Self {
        Self::SetCellValue {
            sheet_id,
            address,
            value,
        }
    }

    /// Constructs a normal-formula change.
    pub const fn set_cell_formula(
        sheet_id: SheetId,
        address: CellAddress,
        formula: FormulaText,
    ) -> Self {
        Self::SetCellFormula {
            sheet_id,
            address,
            formula,
        }
    }

    /// Constructs a dynamic-array formula change.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::RangeStartAfterEnd`] when a declared range does not start at
    /// `address`.
    pub fn set_cell_dynamic_formula(
        sheet_id: SheetId,
        address: CellAddress,
        formula: FormulaText,
        range: Option<CellRange>,
    ) -> Result<Self, ValidationError> {
        if range.is_some_and(|candidate| candidate.start() != address) {
            return Err(ValidationError::RangeStartAfterEnd);
        }
        Ok(Self::SetCellDynamicFormula {
            sheet_id,
            address,
            formula,
            range,
        })
    }

    /// Constructs a sparse-cell removal.
    pub const fn clear_cell(sheet_id: SheetId, address: CellAddress) -> Self {
        Self::ClearCell { sheet_id, address }
    }

    /// Constructs a number-format change.
    pub const fn set_cell_number_format(
        sheet_id: SheetId,
        address: CellAddress,
        number_format: NumberFormat,
    ) -> Self {
        Self::SetCellNumberFormat {
            sheet_id,
            address,
            number_format,
        }
    }

    /// Constructs a visible-sheet addition.
    pub const fn add_sheet(name: SheetName) -> Self {
        Self::AddSheet { name }
    }

    /// Constructs a sheet rename.
    pub const fn rename_sheet(sheet_id: SheetId, name: SheetName) -> Self {
        Self::RenameSheet { sheet_id, name }
    }

    /// Constructs a sheet deletion.
    pub const fn delete_sheet(sheet_id: SheetId) -> Self {
        Self::DeleteSheet { sheet_id }
    }

    /// Constructs a sheet-visibility change.
    pub const fn set_sheet_visibility(sheet_id: SheetId, visibility: SheetVisibility) -> Self {
        Self::SetSheetVisibility {
            sheet_id,
            visibility,
        }
    }

    /// Constructs a defined-name addition or replacement.
    pub const fn set_defined_name(defined_name: DefinedName) -> Self {
        Self::SetDefinedName { defined_name }
    }

    /// Constructs a defined-name removal.
    pub fn remove_defined_name(scope: DefinedNameScope, name: impl Into<Box<str>>) -> Self {
        Self::RemoveDefinedName {
            scope,
            name: name.into(),
        }
    }

    /// Constructs a date-system change.
    pub const fn set_date_system(date_system: DateSystem) -> Self {
        Self::SetDateSystem { date_system }
    }

    /// Constructs a calculation-hints change.
    pub const fn set_calculation_hints(calculation_hints: CalculationHints) -> Self {
        Self::SetCalculationHints { calculation_hints }
    }

    /// Constructs a stable-ID table rename.
    pub const fn rename_table(table_id: TableId, new_display_name: TableName) -> Self {
        Self::RenameTable {
            table_id,
            new_display_name,
        }
    }

    /// Constructs a stable-ID table-column rename.
    pub const fn rename_table_column(
        table_id: TableId,
        column_id: TableColumnId,
        new_name: TableColumnName,
    ) -> Self {
        Self::RenameTableColumn {
            table_id,
            column_id,
            new_name,
        }
    }

    /// Constructs a stable-ID table data-row resize.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::TableDataRowsReversed`] when the first data row is after the
    /// last data row.
    pub fn resize_table_rows(
        table_id: TableId,
        first_data_row: Row,
        last_data_row: Row,
    ) -> Result<Self, ValidationError> {
        if first_data_row > last_data_row {
            return Err(ValidationError::TableDataRowsReversed {
                first_data_row: first_data_row.get(),
                last_data_row: last_data_row.get(),
            });
        }
        Ok(Self::ResizeTableRows {
            table_id,
            first_data_row,
            last_data_row,
        })
    }

    /// Constructs a stable-ID table column widening.
    pub const fn widen_table_columns(table_id: TableId, range: CellRange) -> Self {
        Self::WidenTableColumns { table_id, range }
    }

    /// Constructs a sheet-level auto-filter column widening.
    pub const fn widen_sheet_auto_filter(sheet_id: SheetId, range: CellRange) -> Self {
        Self::WidenSheetAutoFilter { sheet_id, range }
    }

    /// Constructs a row insertion.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::StructuralEditEmpty`] when `count` is zero and
    /// [`ValidationError::RowOutOfRange`] when the inserted rows would end past the last row.
    pub fn insert_rows(sheet_id: SheetId, row: Row, count: u32) -> Result<Self, ValidationError> {
        validate_insertion(row.get(), count, crate::EXCEL_MAX_ROWS, |value| {
            ValidationError::RowOutOfRange { value }
        })?;
        Ok(Self::InsertRows {
            sheet_id,
            row,
            count,
        })
    }

    /// Constructs a row deletion.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::RangeStartAfterEnd`] when `first` is after `last`.
    pub fn delete_rows(sheet_id: SheetId, first: Row, last: Row) -> Result<Self, ValidationError> {
        if first > last {
            return Err(ValidationError::RangeStartAfterEnd);
        }
        Ok(Self::DeleteRows {
            sheet_id,
            first,
            last,
        })
    }

    /// Constructs a column insertion.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::StructuralEditEmpty`] when `count` is zero and
    /// [`ValidationError::ColumnOutOfRange`] when the inserted columns would end past the last
    /// column.
    pub fn insert_columns(
        sheet_id: SheetId,
        column: Column,
        count: u32,
    ) -> Result<Self, ValidationError> {
        validate_insertion(column.get(), count, crate::EXCEL_MAX_COLUMNS, |value| {
            ValidationError::ColumnOutOfRange { value }
        })?;
        Ok(Self::InsertColumns {
            sheet_id,
            column,
            count,
        })
    }

    /// Constructs a column deletion.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::RangeStartAfterEnd`] when `first` is after `last`.
    pub fn delete_columns(
        sheet_id: SheetId,
        first: Column,
        last: Column,
    ) -> Result<Self, ValidationError> {
        if first > last {
            return Err(ValidationError::RangeStartAfterEnd);
        }
        Ok(Self::DeleteColumns {
            sheet_id,
            first,
            last,
        })
    }

    /// Constructs a row move.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::RangeStartAfterEnd`] when `first` is after `last` and
    /// [`ValidationError::RowOutOfRange`] when the moved rows would end past the last row.
    pub fn move_rows(
        sheet_id: SheetId,
        first: Row,
        last: Row,
        destination: Row,
    ) -> Result<Self, ValidationError> {
        validate_move(
            first.get(),
            last.get(),
            destination.get(),
            crate::EXCEL_MAX_ROWS,
            |value| ValidationError::RowOutOfRange { value },
        )?;
        Ok(Self::MoveRows {
            sheet_id,
            first,
            last,
            destination,
        })
    }

    /// Constructs a column move.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::RangeStartAfterEnd`] when `first` is after `last` and
    /// [`ValidationError::ColumnOutOfRange`] when the moved columns would end past the last
    /// column.
    pub fn move_columns(
        sheet_id: SheetId,
        first: Column,
        last: Column,
        destination: Column,
    ) -> Result<Self, ValidationError> {
        validate_move(
            first.get(),
            last.get(),
            destination.get(),
            crate::EXCEL_MAX_COLUMNS,
            |value| ValidationError::ColumnOutOfRange { value },
        )?;
        Ok(Self::MoveColumns {
            sheet_id,
            first,
            last,
            destination,
        })
    }

    /// Constructs a cell format change.
    pub const fn set_cell_format(
        sheet_id: SheetId,
        address: CellAddress,
        edit: CellFormatEdit,
    ) -> Self {
        Self::SetCellFormat {
            sheet_id,
            address,
            edit,
        }
    }

    /// Constructs a column width change; `None` restores the default width.
    pub const fn set_column_width(
        sheet_id: SheetId,
        first: Column,
        last: Column,
        width: Option<f64>,
    ) -> Self {
        Self::SetColumnWidth {
            sheet_id,
            first,
            last,
            width,
        }
    }

    /// Constructs a row height change; `None` restores the default height.
    pub const fn set_row_height(sheet_id: SheetId, row: Row, height: Option<f64>) -> Self {
        Self::SetRowHeight {
            sheet_id,
            row,
            height,
        }
    }
}

/// Rejects an insertion of no lines or of lines that would end past the last one.
pub(super) fn validate_insertion(
    at: u32,
    count: u32,
    maximum: u32,
    out_of_range: impl Fn(u32) -> ValidationError,
) -> Result<(), ValidationError> {
    if count == 0 {
        return Err(ValidationError::StructuralEditEmpty);
    }
    let end = at
        .checked_add(count - 1)
        .ok_or_else(|| out_of_range(u32::MAX))?;
    if end > maximum {
        return Err(out_of_range(end));
    }
    Ok(())
}

/// Rejects a move of reversed lines or of lines that would land past the last one.
pub(super) fn validate_move(
    first: u32,
    last: u32,
    destination: u32,
    maximum: u32,
    out_of_range: impl Fn(u32) -> ValidationError,
) -> Result<(), ValidationError> {
    if first > last {
        return Err(ValidationError::RangeStartAfterEnd);
    }
    let end = destination
        .checked_add(last - first)
        .ok_or_else(|| out_of_range(u32::MAX))?;
    if end > maximum {
        return Err(out_of_range(end));
    }
    Ok(())
}

/// An ordered collection of workbook changes committed atomically.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EditBatch {
    changes: Vec<WorkbookChange>,
}

impl EditBatch {
    /// Constructs a batch while preserving caller order.
    pub fn new(changes: impl IntoIterator<Item = WorkbookChange>) -> Self {
        Self {
            changes: changes.into_iter().collect(),
        }
    }

    /// Returns changes in their declared order.
    pub fn changes(&self) -> &[WorkbookChange] {
        &self.changes
    }

    /// Returns the number of operations in the batch.
    pub const fn len(&self) -> usize {
        self.changes.len()
    }

    /// Returns whether the batch contains no operations.
    pub const fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}
