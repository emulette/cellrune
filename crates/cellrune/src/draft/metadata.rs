use std::collections::{BTreeMap, BTreeSet};

use super::{DraftCellMutation, WorkbookDraft, next_revision};
use crate::{
    CalculationCellId, CellAddress, DefinedName, Sheet, SheetId, ValidationError, WorkbookSnapshot,
    XlsxDocument,
};

impl WorkbookDraft {
    pub(crate) const fn source_document(&self) -> Option<&XlsxDocument> {
        self.source_document.as_ref()
    }

    pub(crate) const fn cell_mutations(&self) -> &BTreeMap<CalculationCellId, DraftCellMutation> {
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

    pub(crate) const fn workbook_changed(&self) -> bool {
        self.workbook_changed
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

    pub(super) fn mark_upsert(
        &mut self,
        sheet_id: SheetId,
        address: CellAddress,
        number_format_changed: bool,
    ) {
        let id = CalculationCellId::new(sheet_id, address);
        let changed = number_format_changed
            || matches!(
                self.cell_mutations.get(&id),
                Some(DraftCellMutation::Upsert {
                    number_format_changed: true
                })
            );
        self.cell_mutations.insert(
            id,
            DraftCellMutation::Upsert {
                number_format_changed: changed,
            },
        );
    }

    pub(super) fn commit(
        &mut self,
        sheets: Vec<Sheet>,
        defined_names: Vec<DefinedName>,
    ) -> Result<(), ValidationError> {
        let revision = next_revision(self.semantic_revision())?;
        self.workbook = WorkbookSnapshot::new_with_metadata(
            sheets,
            defined_names,
            self.workbook.diagnostics().to_vec(),
            self.workbook.date_system(),
            self.workbook.calculation_hints(),
            self.workbook.source(),
            self.workbook.provenance().clone(),
        )?
        .with_semantic_revision(revision);
        Ok(())
    }
}
