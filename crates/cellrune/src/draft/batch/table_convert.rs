use super::BatchExecutionError;
use super::formula_edit::{
    FormulaEditState, WorkbookFormulaEdit, WorkbookFormulaRename, rewrite_workbook_formulas,
};
use super::table_edit::{TableFormulaEdit, table_location};
use crate::calculation::table_conversion::TableArea;
use crate::{Sheet, SheetId, TableId};

/// Converts a table to a range: the table leaves its sheet, its header, data, and totals cells
/// keep their values and formulas, and every structured reference to it becomes the
/// sheet-qualified absolute reference to the cells it names, a this-row reference keeping its row
/// relative. Returns the table's sheet.
pub(super) fn convert_table_to_range(
    sheets: &mut [Sheet],
    defined_names: &mut [crate::DefinedName],
    edit: TableFormulaEdit<'_, '_>,
    table_id: TableId,
) -> Result<SheetId, BatchExecutionError> {
    let TableFormulaEdit {
        state,
        locations,
        formula_locations,
        budget,
    } = edit;
    let (sheet_index, table_index) = table_location(locations, table_id)?;
    let sheet = &sheets[sheet_index];
    let sheet_id = sheet.id();
    let sheet_name = sheet.name().clone();
    let table = sheet.tables()[table_index].clone();
    let columns = table
        .columns()
        .iter()
        .map(crate::TableColumn::name)
        .collect::<Vec<_>>();
    let area = TableArea {
        display_name: table.display_name().as_str(),
        sheet_name: sheet_name.as_str(),
        range: table.range(),
        header_rows: table.header_row_count(),
        totals_rows: if table.totals_row_shown() {
            table.totals_row_count()
        } else {
            0
        },
        columns: &columns,
    };
    rewrite_workbook_formulas(
        sheets,
        defined_names,
        WorkbookFormulaEdit {
            state: FormulaEditState {
                mutations: &mut *state.mutations,
                changed_cells: &mut *state.changed_cells,
                calculation_changed_cells: &mut *state.calculation_changed_cells,
                touched_sheets: &mut *state.touched_sheets,
            },
            changed_table_ids: &mut *state.changed_table_ids,
            table_formula_locations: formula_locations,
            budget,
        },
        WorkbookFormulaRename::TableToRange {
            table_id,
            target_sheet_index: sheet_index,
            area: &area,
        },
    )?;
    let sheet = &mut sheets[sheet_index];
    let tables = sheet
        .tables()
        .iter()
        .filter(|candidate| candidate.id() != table_id)
        .cloned()
        .collect();
    sheet.set_tables(tables);
    state.changed_table_ids.remove(&table_id);
    state.touched_sheets.insert(sheet_id);
    Ok(sheet_id)
}

#[cfg(test)]
#[path = "table_convert_tests.rs"]
mod tests;
