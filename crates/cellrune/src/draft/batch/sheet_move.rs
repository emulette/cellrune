use std::sync::Arc;

use super::BatchExecutionError;
use super::formula_edit::{
    WorkbookFormulaEdit, WorkbookFormulaRename, rewrite_sheet_metadata_formulas,
    rewrite_workbook_formulas, table_formula_locations,
};
use super::tab_order::TabOrder;
use crate::calculation::formula_rewrite::{FormulaRewriteRequest, SheetChanges};
use crate::calculation::sheet_changes::TabStep;
use crate::workbook::{NonWorksheetTab, NonWorksheetTabs};
use crate::{DefinedName, Sheet, SheetId, ValidationError};

/// Moves the tab `sheet_id`, a worksheet or not, to zero-based `position` of the tab order and
/// rewrites the 3-D references the move reverses. Returns the move as a tab-order step, or `None`
/// when the tab is already at `position`.
///
/// Sheet IDs, scoped names, and every other reference keep their text: a 3-D reference's tabs
/// follow tab order, so it gains or loses the tabs moved across its ends.
pub(super) fn move_sheet(
    sheets: &mut Vec<Sheet>,
    defined_names: &mut [DefinedName],
    non_worksheet_tabs: &mut NonWorksheetTabs,
    edit: WorkbookFormulaEdit<'_, '_>,
    sheet_id: SheetId,
    position: usize,
) -> Result<Option<TabStep>, BatchExecutionError> {
    let order = TabOrder::of(sheets, non_worksheet_tabs);
    let from = order
        .entries
        .iter()
        .position(|tab| tab.id == sheet_id)
        .ok_or(ValidationError::UnknownSheetId {
            value: sheet_id.get(),
        })?;
    let tab_count = order.entries.len();
    if position >= tab_count {
        return Err(ValidationError::SheetPositionOutOfRange {
            position,
            tab_count,
        }
        .into());
    }
    if position == from {
        return Ok(None);
    }
    let step = TabStep::Move {
        tabs: Arc::clone(&order.entries),
        from,
        to: position,
    };
    let mut moved = order.entries.to_vec();
    let tab = moved.remove(from);
    moved.insert(position, tab);

    // Worksheets take their new order, and the other tabs their new positions.
    let mut taken = std::mem::take(sheets)
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    let mut tabs = Vec::new();
    for (index, entry) in moved.iter().enumerate() {
        if entry.worksheet {
            let slot = taken
                .iter_mut()
                .find(|sheet| sheet.as_ref().is_some_and(|sheet| sheet.id() == entry.id))
                .and_then(Option::take)
                .ok_or(ValidationError::UnknownSheetId {
                    value: entry.id.get(),
                })?;
            sheets.push(slot);
        } else {
            let tab =
                non_worksheet_tabs
                    .by_id(entry.id)
                    .ok_or(ValidationError::UnknownSheetId {
                        value: entry.id.get(),
                    })?;
            tabs.push(NonWorksheetTab::new(
                index,
                tab.id(),
                tab.name().clone(),
                tab.visibility(),
            ));
        }
    }
    *non_worksheet_tabs = NonWorksheetTabs::new(tabs);

    let before = order.names();
    let after = order.unchanged_names();
    let steps = [step];
    let changes = SheetChanges {
        before: &before,
        after: &after,
        steps: &steps,
        retarget: None,
        deleted_tables: &[],
    };
    let WorkbookFormulaEdit {
        state,
        changed_table_ids,
        budget,
        ..
    } = edit;
    let locations = table_formula_locations(sheets, budget)?;
    rewrite_sheet_metadata_formulas(
        sheets,
        &FormulaRewriteRequest::Sheets(changes),
        false,
        &mut *budget,
    )?;
    rewrite_workbook_formulas(
        sheets,
        defined_names,
        WorkbookFormulaEdit {
            state,
            changed_table_ids,
            table_formula_locations: &locations,
            budget,
        },
        WorkbookFormulaRename::SheetChanges(changes),
    )?;
    let [step] = steps;
    Ok(Some(step))
}

#[cfg(test)]
#[path = "sheet_move_tests.rs"]
mod tests;
