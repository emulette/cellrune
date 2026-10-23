use super::BatchExecutionError;
use super::formula_edit::{
    WorkbookFormulaEdit, WorkbookFormulaRename, rewrite_workbook_formulas, table_formula_locations,
};
use crate::calculation::formula_rewrite::SheetChanges;
use crate::workbook::NonWorksheetTabs;
use crate::{
    DefinedName, DefinedNameScope, Sheet, SheetId, SheetName, SheetVisibility, ValidationError,
};

/// Removes one sheet with the names scoped to it, moves the later tabs that are not worksheets
/// forward, and rewrites references to the sheet and its tables in the remaining formulas, names,
/// and table formulas. Returns the removed sheet.
pub(super) fn delete_sheet(
    sheets: &mut Vec<Sheet>,
    defined_names: &mut Vec<DefinedName>,
    non_worksheet_tabs: &mut NonWorksheetTabs,
    edit: WorkbookFormulaEdit<'_, '_>,
    sheet_id: SheetId,
) -> Result<Sheet, BatchExecutionError> {
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
    let worksheet_names = sheets
        .iter()
        .map(|sheet| sheet.name().clone())
        .collect::<Vec<_>>();
    let tab_names = tabs.tab_names(worksheet_names.iter().map(SheetName::as_str));

    let removed = sheets.remove(index);
    defined_names.retain(|name| name.scope() != DefinedNameScope::Sheet(sheet_id));
    *non_worksheet_tabs = tabs.without_worksheet_at(tabs.worksheet_position(index));

    let deleted_tables = removed
        .tables()
        .iter()
        .map(|table| table.display_name().as_str())
        .collect::<Vec<_>>();
    let WorkbookFormulaEdit {
        state,
        changed_table_ids,
        budget,
        ..
    } = edit;
    let locations = table_formula_locations(sheets, budget)?;
    rewrite_workbook_formulas(
        sheets,
        defined_names,
        WorkbookFormulaEdit {
            state,
            changed_table_ids,
            table_formula_locations: &locations,
            budget,
        },
        WorkbookFormulaRename::SheetDeletion(SheetChanges {
            renames: &[],
            deleted: &[removed.name().as_str()],
            tabs: &tab_names,
            deleted_tables: &deleted_tables,
        }),
    )?;
    Ok(removed)
}

#[cfg(test)]
#[path = "sheet_delete_tests.rs"]
mod tests;
