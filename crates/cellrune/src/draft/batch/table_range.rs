use super::BatchExecutionError;
use super::formula_edit::{
    FormulaEditState, TableFormulaLocations, WorkbookFormulaEdit, WorkbookFormulaRename,
    rewrite_workbook_formulas,
};
use super::table_edit::{TableResizeEdit, resize_table_rows, table_location};
use super::widen_edit::widen_table_columns;
use crate::ValidationError;
use crate::{CellAddress, CellRange, Column, DefinedName, Row, Sheet, TableId, TableType};

/// Changes a table's range: its header row stays, its columns grow or shrink at either side, and
/// its data rows grow or shrink above its totals row.
///
/// A new column is named by the text in its header cell. A removed column's cells keep their
/// content, and a structured reference that names it, alone or as an end of a column range,
/// becomes `#REF!`, as Excel 16.114 saves it. Rows change as [`resize_table_rows`] changes them.
pub(super) fn set_table_range(
    sheets: &mut [Sheet],
    defined_names: &mut [DefinedName],
    edit: TableResizeEdit<'_, '_, '_>,
    formula_locations: &TableFormulaLocations,
    table_id: TableId,
    range: CellRange,
) -> Result<bool, BatchExecutionError> {
    let TableResizeEdit {
        state,
        locations,
        rewrite_budget,
        materialization_budget,
    } = edit;
    let (sheet_index, table_index) = table_location(locations, table_id)?;
    let table = sheets[sheet_index].tables()[table_index].clone();
    let current = table.range();
    if range == current {
        return Ok(false);
    }
    let first_data_row = range
        .start()
        .row()
        .get()
        .checked_add(table.header_row_count())
        .ok_or(ValidationError::TableRangeInvalid)?;
    let last_data_row = range
        .end()
        .row()
        .get()
        .checked_sub(table.totals_row_count())
        .ok_or(ValidationError::TableRangeInvalid)?;
    let first_column = range.start().column().max(current.start().column());
    let last_column = range.end().column().min(current.end().column());
    if range.start().row() != current.start().row()
        || first_column > last_column
        || first_data_row > last_data_row
    {
        return Err(ValidationError::TableRangeInvalid.into());
    }
    if table.table_type() != TableType::Worksheet
        || table.header_row_count() != 1
        || table.totals_row_count() > 1
    {
        return Err(ValidationError::UnsupportedTableAuthoringMetadata {
            table_id: table_id.get(),
        }
        .into());
    }
    let mut changed = false;

    if first_column != current.start().column() || last_column != current.end().column() {
        // Filter criteria and sort state name columns, which a removed column would leave stale.
        let filtered = table.auto_filter().is_some_and(|filter| {
            !filter.filter_columns().is_empty() || filter.sort_state().is_some()
        });
        if filtered || table.sort_state().is_some() {
            return Err(ValidationError::UnsupportedTableAuthoringMetadata {
                table_id: table_id.get(),
            }
            .into());
        }
        let offset = |column: Column| (column.get() - current.start().column().get()) as usize;
        let kept = table.columns()[offset(first_column)..=offset(last_column)].to_vec();
        let removed = table
            .columns()
            .iter()
            .filter(|column| !kept.contains(column))
            .map(crate::TableColumn::name)
            .collect::<Vec<_>>();
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
                budget: &mut *rewrite_budget,
            },
            WorkbookFormulaRename::TableColumnsRemoved {
                table_id,
                target_sheet_index: sheet_index,
                target_range: current,
                table_name: table.display_name().as_str(),
                removed: &removed,
            },
        )?;
        let narrowed = CellRange::from_ordered(
            CellAddress::new(current.start().row(), first_column),
            CellAddress::new(current.end().row(), last_column),
        );
        sheets[sheet_index].tables_mut()[table_index].narrow_columns(narrowed, kept);
        changed = true;
    }

    if range.start().column() != first_column || range.end().column() != last_column {
        let widened = CellRange::from_ordered(
            CellAddress::new(current.start().row(), range.start().column()),
            CellAddress::new(current.end().row(), range.end().column()),
        );
        changed |= widen_table_columns(sheets, locations, table_id, widened)?;
    }

    let first_data_row = Row::new(first_data_row)?;
    let last_data_row = Row::new(last_data_row)?;
    changed |= resize_table_rows(
        sheets,
        TableResizeEdit {
            state: super::table_edit::TableEditState {
                mutations: &mut *state.mutations,
                changed_cells: &mut *state.changed_cells,
                calculation_changed_cells: &mut *state.calculation_changed_cells,
                touched_sheets: &mut *state.touched_sheets,
                changed_table_ids: &mut *state.changed_table_ids,
                presentation: state.presentation,
                source: state.source,
            },
            locations,
            rewrite_budget,
            materialization_budget,
        },
        table_id,
        first_data_row,
        last_data_row,
    )?;
    if changed {
        state.changed_table_ids.insert(table_id);
        state.touched_sheets.insert(sheets[sheet_index].id());
    }
    Ok(changed)
}

#[cfg(test)]
#[path = "table_range_tests.rs"]
mod tests;
