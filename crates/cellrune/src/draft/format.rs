//! The format each cell of a draft displays with.

use super::{DraftCellMutation, DraftCellMutationStore, SourceCells, WorkbookDraft};
use crate::{
    CalculationCellId, CellAddress, CellFormat, DocumentPresentation, Sheet, SheetId,
    ValidationError,
};

impl WorkbookDraft {
    /// Returns the font, fill, border, and alignment the cell at `address` displays with.
    ///
    /// An edited cell keeps its edited format. Otherwise a source cell that holds a value keeps its
    /// own style, and an empty position or a cell the draft added takes the style a new cell
    /// receives there: the blank source cell's, else its row's, else its column's. A workbook
    /// created by [`WorkbookDraft::new`], or opened from a package without a styles part, displays
    /// Calibri 11 by default, the default style the writers generate for it.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::UnknownSheetId`] when the sheet is not in this draft.
    pub fn cell_format(
        &self,
        sheet_id: SheetId,
        address: CellAddress,
    ) -> Result<CellFormat, ValidationError> {
        let sheet = self
            .workbook
            .sheet_by_id(sheet_id)
            .ok_or(ValidationError::UnknownSheetId {
                value: sheet_id.get(),
            })?;
        Ok(effective_cell_format(
            &self.presentation,
            sheet,
            self.source_cells(),
            &self.cell_mutations,
            address,
        ))
    }
}

/// Resolves the format a cell displays with in a draft's staged or current state.
pub(crate) fn effective_cell_format(
    presentation: &DocumentPresentation,
    sheet: &Sheet,
    source: SourceCells<'_>,
    mutations: &DraftCellMutationStore,
    address: CellAddress,
) -> CellFormat {
    if let Some(format) = presentation.edited_cell_format(sheet.id(), address) {
        return format.clone();
    }
    presentation.style_format(source_style_index(sheet, source, mutations, address))
}

/// Returns the source `cellXfs` index a cell displays: a kept valued source cell's own style,
/// else the style a new cell receives at its position, else the default style. The document
/// writer styles written cells the same way.
pub(crate) fn source_style_index(
    sheet: &Sheet,
    source: SourceCells<'_>,
    mutations: &DraftCellMutationStore,
    address: CellAddress,
) -> usize {
    let keeps_source_cell = match mutations.get(&CalculationCellId::new(sheet.id(), address)) {
        None => true,
        Some(DraftCellMutation::Upsert {
            replaces_source_cell,
            ..
        }) => !replaces_source_cell,
        Some(DraftCellMutation::Remove) => false,
    };
    let source_cell = source.cell(sheet.id(), address);
    if keeps_source_cell && source_cell.is_some() && sheet.cell(address).is_some() {
        return sheet.new_cell_styles().valued_cell_style(address);
    }
    sheet
        .new_cell_styles()
        .style(address)
        .map_or(0, |(index, _)| index)
}
