use crate::{
    CalculationHints, CellAddress, CellRange, CellValue, DateSystem, DefinedName, DefinedNameScope,
    FormulaText, NumberFormat, SheetId, SheetName, SheetVisibility, ValidationError,
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
