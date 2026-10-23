use std::sync::Arc;

use super::WorkbookDraft;
use crate::presentation::validate_authoring_runs;
use crate::{
    CalculationCellId, CellAddress, CellContent, CellValue, PhoneticAnnotation, PhoneticRun,
    PhoneticWriteOptions, SheetId, ValidationError,
};

impl WorkbookDraft {
    /// Sets the frozen rows and columns for one worksheet.
    ///
    /// Passing [`crate::FrozenPane::is_clear`] clears the pane. This changes only the presentation
    /// revision and never invalidates calculation.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::UnknownSheetId`] when the sheet is not in this draft or
    /// [`ValidationError::PresentationRevisionExhausted`] when the presentation revision cannot
    /// advance.
    pub fn set_frozen_pane(
        &mut self,
        sheet_id: SheetId,
        pane: crate::FrozenPane,
    ) -> Result<(), ValidationError> {
        self.require_sheet(sheet_id)?;
        let pane = (!pane.is_clear()).then_some(pane);
        if self.presentation.set_frozen_pane(sheet_id, pane)? {
            self.presentation_sheet_mutations.insert(sheet_id);
        }
        Ok(())
    }

    /// Clears the frozen pane from one worksheet.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::UnknownSheetId`] when the sheet is not in this draft or
    /// [`ValidationError::PresentationRevisionExhausted`] when the presentation revision cannot
    /// advance.
    pub fn clear_frozen_pane(&mut self, sheet_id: SheetId) -> Result<(), ValidationError> {
        self.require_sheet(sheet_id)?;
        if self.presentation.set_frozen_pane(sheet_id, None)? {
            self.presentation_sheet_mutations.insert(sheet_id);
        }
        Ok(())
    }

    /// Atomically replaces a literal text cell and its phonetic annotation.
    ///
    /// The text is written as [`Self::set_cell_value`] writes it, so a table header cell renames
    /// its column and rewrites the structured references to it.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when the sheet is missing, a run is outside the base text,
    /// runs overlap, the text cannot be written as a cell value, or either semantic or
    /// presentation revision is exhausted.
    pub fn set_annotated_text(
        &mut self,
        sheet_id: SheetId,
        address: CellAddress,
        text: impl Into<String>,
        runs: Vec<PhoneticRun>,
        options: PhoneticWriteOptions,
    ) -> Result<(), ValidationError> {
        self.require_sheet(sheet_id)?;
        let text = text.into();
        let annotation = authored_annotation(&text, runs, &options)?;
        let mut presentation = self.presentation.clone();
        let presentation_changed =
            presentation.set_cell_phonetics(sheet_id, address, annotation, options.visible())?;

        // A value write refuses to replace annotated text, so it runs without the cell's old
        // annotation, which `presentation` replaces once the write succeeds.
        let mut released = self.presentation.clone();
        released.clear_cell_phonetics(sheet_id, address)?;
        let previous = std::mem::replace(&mut self.presentation, released);
        if let Err(error) = self.set_cell_value(sheet_id, address, CellValue::Text(text)) {
            self.presentation = previous;
            return Err(error);
        }
        self.presentation = presentation;
        if presentation_changed {
            self.presentation_cell_mutations
                .insert(CalculationCellId::new(sheet_id, address));
        }
        Ok(())
    }

    /// Replaces phonetic runs on an existing literal text cell.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::PhoneticsRequireTextCell`] for a missing or non-text cell, or a
    /// range/revision validation error.
    pub fn set_phonetics(
        &mut self,
        sheet_id: SheetId,
        address: CellAddress,
        runs: Vec<PhoneticRun>,
        options: PhoneticWriteOptions,
    ) -> Result<(), ValidationError> {
        self.require_sheet(sheet_id)?;
        let text = self
            .workbook
            .sheet_by_id(sheet_id)
            .and_then(|sheet| sheet.cell(address))
            .and_then(|cell| match cell.content() {
                CellContent::Literal(CellValue::Text(text)) => Some(text.as_str()),
                _ => None,
            })
            .ok_or_else(|| phonetics_require_text_cell(sheet_id, address))?;
        let annotation = authored_annotation(text, runs, &options)?;
        let mut presentation = self.presentation.clone();
        if presentation.set_cell_phonetics(sheet_id, address, annotation, options.visible())? {
            self.presentation = presentation;
            self.presentation_cell_mutations
                .insert(CalculationCellId::new(sheet_id, address));
        }
        Ok(())
    }

    /// Removes one cell's phonetic runs, properties, and explicit visibility.
    ///
    /// # Errors
    ///
    /// Returns an unknown-sheet or presentation-revision validation error.
    pub fn clear_phonetics(
        &mut self,
        sheet_id: SheetId,
        address: CellAddress,
    ) -> Result<bool, ValidationError> {
        self.require_sheet(sheet_id)?;
        let mut presentation = self.presentation.clone();
        if !presentation.clear_cell_phonetics(sheet_id, address)? {
            return Ok(false);
        }
        self.presentation = presentation;
        self.presentation_cell_mutations
            .insert(CalculationCellId::new(sheet_id, address));
        Ok(true)
    }
}

pub(super) fn phonetics_require_text_cell(
    sheet_id: SheetId,
    address: CellAddress,
) -> ValidationError {
    ValidationError::PhoneticsRequireTextCell {
        sheet_id: sheet_id.get(),
        row: address.row().get(),
        column: address.column().get(),
    }
}

/// Validates authored runs over `text` and builds the annotation they and `options` describe.
pub(super) fn authored_annotation(
    text: &str,
    runs: Vec<PhoneticRun>,
    options: &PhoneticWriteOptions,
) -> Result<Arc<PhoneticAnnotation>, ValidationError> {
    validate_authoring_runs(text, &runs)?;
    validate_authoring_properties(options)?;
    Ok(Arc::new(PhoneticAnnotation::new(
        runs,
        Some(
            options
                .properties()
                .cloned()
                .unwrap_or_else(|| crate::PhoneticProperties::new(0)),
        ),
    )))
}

fn validate_authoring_properties(options: &PhoneticWriteOptions) -> Result<(), ValidationError> {
    if let Some(properties) = options.properties()
        && properties.font_id() != 0
    {
        return Err(ValidationError::PhoneticFontIdUnsupported {
            value: properties.font_id(),
        });
    }
    Ok(())
}
