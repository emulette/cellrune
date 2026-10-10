use super::super::{DraftCellMutation, DraftCellMutationStore};
use crate::{CalculationCellId, CellAddress, Sheet, SheetId, ValidationError, WorkbookSnapshot};

pub(super) fn sheet_by_id_mut(
    sheets: &mut [Sheet],
    sheet_id: SheetId,
) -> Result<&mut Sheet, ValidationError> {
    sheets
        .iter_mut()
        .find(|sheet| sheet.id() == sheet_id)
        .ok_or(ValidationError::UnknownSheetId {
            value: sheet_id.get(),
        })
}

/// Records a staged cell removal against the source document and reports whether the source holds
/// the cell.
///
/// A cell the draft added and then removed leaves no mutation, because the source document has no
/// such cell for the writer to remove.
pub(super) fn mark_removed(
    mutations: &mut DraftCellMutationStore,
    source: Option<&WorkbookSnapshot>,
    sheet_id: SheetId,
    address: CellAddress,
) -> bool {
    let id = CalculationCellId::new(sheet_id, address);
    let in_source = source
        .and_then(|workbook| workbook.sheet_by_id(sheet_id))
        .is_some_and(|sheet| sheet.cell(address).is_some());
    if in_source {
        mutations.insert(id, DraftCellMutation::Remove);
    } else {
        mutations.remove(&id);
    }
    in_source
}

pub(super) fn mark_upsert(
    mutations: &mut DraftCellMutationStore,
    sheet_id: SheetId,
    address: CellAddress,
    number_format_changed: bool,
) {
    let id = CalculationCellId::new(sheet_id, address);
    let changed = number_format_changed
        || matches!(
            mutations.get(&id),
            Some(DraftCellMutation::Upsert {
                number_format_changed: true
            })
        );
    mutations.insert(
        id,
        DraftCellMutation::Upsert {
            number_format_changed: changed,
        },
    );
}
