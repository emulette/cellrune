use std::collections::{BTreeMap, BTreeSet};

use super::{DraftCellMutationStore, SourceCells, WorkbookDraft};
use crate::grid::GridHistory;
use crate::{CalculationCellId, SheetId, TableId, ValidationError, XlsxDocument};

impl WorkbookDraft {
    pub(crate) fn source_document(&self) -> Option<&XlsxDocument> {
        self.source_document.as_deref()
    }

    pub(crate) const fn cell_mutations(&self) -> &DraftCellMutationStore {
        &self.cell_mutations
    }

    pub(crate) const fn presentation_cell_mutations(&self) -> &BTreeSet<CalculationCellId> {
        &self.presentation_cell_mutations
    }

    pub(crate) const fn presentation_sheet_mutations(&self) -> &BTreeSet<SheetId> {
        &self.presentation_sheet_mutations
    }

    pub(crate) const fn added_sheets(&self) -> &BTreeSet<SheetId> {
        &self.added_sheets
    }

    pub(crate) const fn changed_table_ids(&self) -> &BTreeSet<TableId> {
        &self.changed_table_ids
    }

    pub(crate) const fn workbook_changed(&self) -> bool {
        self.workbook_changed
    }

    pub(crate) const fn grid_histories(&self) -> &BTreeMap<SheetId, GridHistory> {
        &self.grid_histories
    }

    /// Returns the source document's cells where the draft's structural edits moved them.
    pub(crate) fn source_cells(&self) -> SourceCells<'_> {
        SourceCells::new(
            self.source_document.as_deref().map(XlsxDocument::workbook),
            &self.grid_histories,
        )
    }

    pub(super) fn require_sheet(&self, sheet_id: SheetId) -> Result<(), ValidationError> {
        if self.workbook.sheet_by_id(sheet_id).is_none() {
            Err(ValidationError::UnknownSheetId {
                value: sheet_id.get(),
            })
        } else {
            Ok(())
        }
    }
}
