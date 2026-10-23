use super::BatchExecutionError;
use super::formula_edit::{
    WorkbookFormulaEdit, WorkbookFormulaRename, rewrite_sheet_metadata_formulas,
    rewrite_workbook_formulas, table_formula_locations,
};
use super::tab_order::TabOrder;
use crate::calculation::formula_rewrite::{FormulaRewriteRequest, SheetChanges};
use crate::calculation::sheet_changes::TabStep;
use crate::workbook::NonWorksheetTabs;
use crate::{DefinedName, DefinedNameScope, Sheet, SheetId, SheetVisibility, ValidationError};
use std::sync::Arc;

/// Removes one sheet with the names scoped to it, moves the later tabs that are not worksheets
/// forward, and rewrites references to the sheet and its tables in the remaining formulas, names,
/// table formulas, and data validation formulas. Returns the removed sheet, which takes its data
/// validations, hyperlinks, and comments with it, and the deletion as a tab-order step.
pub(super) fn delete_sheet(
    sheets: &mut Vec<Sheet>,
    defined_names: &mut Vec<DefinedName>,
    non_worksheet_tabs: &mut NonWorksheetTabs,
    edit: WorkbookFormulaEdit<'_, '_>,
    sheet_id: SheetId,
) -> Result<(Sheet, TabStep), BatchExecutionError> {
    let index = sheets
        .iter()
        .position(|sheet| sheet.id() == sheet_id)
        .ok_or(ValidationError::UnknownSheetId {
            value: sheet_id.get(),
        })?;
    if !sheets
        .iter()
        .any(|sheet| sheet.id() != sheet_id && sheet.visibility() == SheetVisibility::Visible)
    {
        return Err(ValidationError::LastVisibleSheet.into());
    }
    let tabs = non_worksheet_tabs.clone();
    let worksheet_index = index;
    let order = TabOrder::of(sheets, &tabs);
    let before = order.names();
    let deleted = sheet_id;
    let step = TabStep::Delete {
        tabs: Arc::clone(&order.entries),
        deleted,
    };

    let removed = sheets.remove(worksheet_index);
    defined_names.retain(|name| name.scope() != DefinedNameScope::Sheet(sheet_id));
    *non_worksheet_tabs = tabs.without_worksheet_at(tabs.worksheet_position(worksheet_index));

    let deleted_tables = removed
        .tables()
        .iter()
        .map(|table| table.display_name().as_str())
        .collect::<Vec<_>>();
    let after = order
        .unchanged_names()
        .into_iter()
        .filter(|(id, _)| *id != deleted)
        .collect::<Vec<_>>();
    let WorkbookFormulaEdit {
        state,
        changed_table_ids,
        budget,
        ..
    } = edit;
    let locations = table_formula_locations(sheets, budget)?;
    let steps = [step];
    let changes = SheetChanges {
        before: &before,
        after: &after,
        steps: &steps,
        retarget: None,
        deleted_tables: &deleted_tables,
    };
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
    Ok((removed, step))
}

#[cfg(test)]
#[path = "sheet_delete_tests.rs"]
mod tests;
