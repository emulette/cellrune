use super::super::{DraftCellMutation, DraftCellMutationStore};
use crate::{CalculationCellId, CellAddress, Sheet, SheetId, ValidationError};

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
