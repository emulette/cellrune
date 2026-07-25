use super::{EditBatch, WorkbookChange, WorkbookDraft};
use crate::{
    CalculationHints, CellAddress, CellValue, DateSystem, DefinedName, DefinedNameScope,
    FormulaText, NumberFormat, SheetId, SheetName, SheetVisibility, ValidationError,
};

impl WorkbookDraft {
    /// Sets a literal value, retaining an existing cell's number format.
    ///
    /// `CellValue::Blank` clears the sparse cell.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::UnknownSheetId`] for a missing sheet or
    /// [`ValidationError::SemanticRevisionExhausted`] if the revision cannot advance.
    pub fn set_cell_value(
        &mut self,
        sheet_id: SheetId,
        address: CellAddress,
        value: CellValue,
    ) -> Result<(), ValidationError> {
        self.apply_changes(EditBatch::new([WorkbookChange::set_cell_value(
            sheet_id, address, value,
        )]))?;
        Ok(())
    }

    /// Sets a normal Excel A1 formula with no trusted saved result.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::UnknownSheetId`] for a missing sheet or
    /// [`ValidationError::SemanticRevisionExhausted`] if the revision cannot advance.
    pub fn set_cell_formula(
        &mut self,
        sheet_id: SheetId,
        address: CellAddress,
        formula: FormulaText,
    ) -> Result<(), ValidationError> {
        self.apply_changes(EditBatch::new([WorkbookChange::set_cell_formula(
            sheet_id, address, formula,
        )]))?;
        Ok(())
    }

    /// Sets a dynamic-array formula with an optional expected spill range.
    ///
    /// A declared range must start at `address`. Calculation resolves the actual result shape and
    /// fails with `#SPILL!` when the declared range is stale or an undeclared target is occupied.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::RangeStartAfterEnd`] when the declared range has another
    /// anchor, [`ValidationError::UnknownSheetId`] for a missing sheet, or
    /// [`ValidationError::SemanticRevisionExhausted`] if the revision cannot advance.
    pub fn set_cell_dynamic_formula(
        &mut self,
        sheet_id: SheetId,
        address: CellAddress,
        formula: FormulaText,
        range: Option<crate::CellRange>,
    ) -> Result<(), ValidationError> {
        let change = WorkbookChange::set_cell_dynamic_formula(sheet_id, address, formula, range)?;
        self.apply_changes(EditBatch::new([change]))?;
        Ok(())
    }

    /// Replaces the number format of an existing sparse cell.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::UnknownSheetId`] or [`ValidationError::CellNotFound`] when the
    /// target does not exist, or [`ValidationError::SemanticRevisionExhausted`] if the revision
    /// cannot advance.
    pub fn set_cell_number_format(
        &mut self,
        sheet_id: SheetId,
        address: CellAddress,
        number_format: NumberFormat,
    ) -> Result<(), ValidationError> {
        self.apply_changes(EditBatch::new([WorkbookChange::set_cell_number_format(
            sheet_id,
            address,
            number_format,
        )]))?;
        Ok(())
    }

    /// Removes a sparse cell and returns whether one existed.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::UnknownSheetId`] for a missing sheet or
    /// [`ValidationError::SemanticRevisionExhausted`] if the revision cannot advance.
    pub fn clear_cell(
        &mut self,
        sheet_id: SheetId,
        address: CellAddress,
    ) -> Result<bool, ValidationError> {
        let receipt = self.apply_changes(EditBatch::new([WorkbookChange::clear_cell(
            sheet_id, address,
        )]))?;
        Ok(!receipt.changed_cells().is_empty())
    }

    /// Adds a visible empty sheet and returns its stable ID.
    ///
    /// # Errors
    ///
    /// Returns a sheet-name validation error, [`ValidationError::DuplicateSheetName`],
    /// [`ValidationError::SheetIdExhausted`], or
    /// [`ValidationError::SemanticRevisionExhausted`].
    pub fn add_sheet(&mut self, name: SheetName) -> Result<SheetId, ValidationError> {
        let receipt = self.apply_changes(EditBatch::new([WorkbookChange::add_sheet(name)]))?;
        receipt
            .created_sheet_ids()
            .first()
            .copied()
            .ok_or(ValidationError::SheetIdExhausted)
    }

    /// Renames a sheet and rewrites matching stored sheet references in formulas and names.
    ///
    /// # Errors
    ///
    /// Returns a sheet-name validation error, [`ValidationError::UnknownSheetId`],
    /// [`ValidationError::DuplicateSheetName`], or
    /// [`ValidationError::SemanticRevisionExhausted`].
    pub fn rename_sheet(
        &mut self,
        sheet_id: SheetId,
        name: SheetName,
    ) -> Result<(), ValidationError> {
        self.apply_changes(EditBatch::new([WorkbookChange::rename_sheet(
            sheet_id, name,
        )]))?;
        Ok(())
    }

    /// Changes sheet visibility while retaining at least one visible sheet.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::UnknownSheetId`], [`ValidationError::LastVisibleSheet`], or
    /// [`ValidationError::SemanticRevisionExhausted`].
    pub fn set_sheet_visibility(
        &mut self,
        sheet_id: SheetId,
        visibility: SheetVisibility,
    ) -> Result<(), ValidationError> {
        self.apply_changes(EditBatch::new([WorkbookChange::set_sheet_visibility(
            sheet_id, visibility,
        )]))?;
        Ok(())
    }

    /// Adds or replaces a defined name in the same scope using case-insensitive identity.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::DefinedNameUnknownSheet`] for an invalid scope or
    /// [`ValidationError::SemanticRevisionExhausted`] if the revision cannot advance.
    pub fn set_defined_name(&mut self, defined_name: DefinedName) -> Result<(), ValidationError> {
        self.apply_changes(EditBatch::new([WorkbookChange::set_defined_name(
            defined_name,
        )]))?;
        Ok(())
    }

    /// Removes a defined name and returns whether one existed.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::SemanticRevisionExhausted`] if the revision cannot advance.
    pub fn remove_defined_name(
        &mut self,
        scope: DefinedNameScope,
        name: &str,
    ) -> Result<bool, ValidationError> {
        let receipt = self.apply_changes(EditBatch::new([WorkbookChange::remove_defined_name(
            scope, name,
        )]))?;
        Ok(receipt.topology_changed())
    }

    /// Changes the workbook date system.
    ///
    /// This changes how numeric date serials are interpreted; existing numeric values are not
    /// converted.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::SemanticRevisionExhausted`] if the revision cannot advance.
    pub fn set_date_system(&mut self, date_system: DateSystem) -> Result<(), ValidationError> {
        self.apply_changes(EditBatch::new([WorkbookChange::set_date_system(
            date_system,
        )]))?;
        Ok(())
    }

    /// Changes supported workbook calculation metadata without initiating calculation.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::SemanticRevisionExhausted`] if the revision cannot advance.
    pub fn set_calculation_hints(
        &mut self,
        calculation_hints: CalculationHints,
    ) -> Result<(), ValidationError> {
        self.apply_changes(EditBatch::new([WorkbookChange::set_calculation_hints(
            calculation_hints,
        )]))?;
        Ok(())
    }
}
