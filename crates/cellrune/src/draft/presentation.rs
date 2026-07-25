use std::sync::Arc;

use super::WorkbookDraft;
use crate::presentation::validate_authoring_runs;
use crate::{
    CalculationCellId, CellAddress, CellContent, CellValue, NumberFormat, PhoneticAnnotation,
    PhoneticRun, PhoneticWriteOptions, Sheet, SheetId, ValidationError,
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
    /// # Errors
    ///
    /// Returns a [`ValidationError`] when the sheet is missing, a run is outside the base text,
    /// runs overlap, or either semantic or presentation revision is exhausted.
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
        validate_authoring_runs(&text, &runs)?;
        validate_authoring_properties(&options)?;
        let annotation = Arc::new(PhoneticAnnotation::new(
            runs,
            Some(
                options
                    .properties()
                    .cloned()
                    .unwrap_or_else(|| crate::PhoneticProperties::new(0)),
            ),
        ));
        let mut presentation = self.presentation.clone();
        let presentation_changed =
            presentation.set_cell_phonetics(sheet_id, address, annotation, options.visible())?;

        let mut sheets = self.workbook.sheets().to_vec();
        let sheet = sheet_mut(&mut sheets, sheet_id)?;
        let number_format = sheet
            .cell(address)
            .map_or_else(NumberFormat::default, |cell| cell.number_format().clone());
        let content = CellContent::Literal(CellValue::Text(text));
        let semantic_changed = sheet.cell(address).is_none_or(|cell| {
            cell.content() != &content || cell.number_format() != &number_format
        });
        if semantic_changed {
            sheet.upsert_cell(address, content, number_format);
            self.commit(sheets, self.workbook.defined_names().to_vec())?;
            self.mark_upsert(sheet_id, address, false);
        }
        if presentation_changed {
            self.presentation = presentation;
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
        validate_authoring_runs(text, &runs)?;
        validate_authoring_properties(&options)?;
        let annotation = Arc::new(PhoneticAnnotation::new(
            runs,
            Some(
                options
                    .properties()
                    .cloned()
                    .unwrap_or_else(|| crate::PhoneticProperties::new(0)),
            ),
        ));
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

fn phonetics_require_text_cell(sheet_id: SheetId, address: CellAddress) -> ValidationError {
    ValidationError::PhoneticsRequireTextCell {
        sheet_id: sheet_id.get(),
        row: address.row().get(),
        column: address.column().get(),
    }
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

fn sheet_mut(sheets: &mut [Sheet], sheet_id: SheetId) -> Result<&mut Sheet, ValidationError> {
    sheets
        .iter_mut()
        .find(|sheet| sheet.id() == sheet_id)
        .ok_or(ValidationError::UnknownSheetId {
            value: sheet_id.get(),
        })
}
