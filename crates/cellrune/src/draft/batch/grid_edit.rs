//! Structural edits of one sheet's grid, with every reference, structure, and format that
//! follows the moved cells.

use std::collections::{BTreeMap, BTreeSet};

use super::super::DraftCellMutationStore;
use super::formula_edit::{
    FormulaEditState, WorkbookFormulaEdit, WorkbookFormulaRename, rewrite_workbook_formulas,
};
use super::staged::mark_upsert;
use super::tab_order::TabOrder;
use super::{BatchExecutionError, TableMaterializationBudget, WorkbookChange};
use crate::calculation::formula_rewrite::{
    FormulaRewriteBudget, FormulaRewriteRequest, rewrite_formula,
};
use crate::calculation::grid_rewrite::{GridChange, move_anchored_formulas};
use crate::calculation::sheet_changes::SheetChanges;
use crate::grid::{GridAxis, GridEdit, GridHistory};
use crate::workbook::NonWorksheetTabs;
use crate::{
    CalculationCellId, CellAddress, CellContent, CellRange, CellValue, Column, DataValidation,
    DefinedName, DefinedNameScope, DocumentPresentation, FormulaMetadata, FormulaText, Sheet,
    SheetId, Table, TableColumn, TableId, TableType, ValidationError, case_insensitive_eq,
};

/// The staged draft state a structural edit changes.
pub(super) struct GridEditState<'a, 'cancel, 'materialize> {
    pub(super) sheets: &'a mut [Sheet],
    pub(super) defined_names: &'a mut [DefinedName],
    pub(super) presentation: &'a mut DocumentPresentation,
    pub(super) mutations: &'a mut DraftCellMutationStore,
    pub(super) presentation_cell_mutations: &'a mut BTreeSet<CalculationCellId>,
    pub(super) presentation_sheet_mutations: &'a mut BTreeSet<SheetId>,
    pub(super) changed_cells: &'a mut BTreeSet<CalculationCellId>,
    pub(super) calculation_changed_cells: &'a mut BTreeSet<CalculationCellId>,
    pub(super) touched_sheets: &'a mut BTreeSet<SheetId>,
    pub(super) changed_table_ids: &'a mut BTreeSet<TableId>,
    /// Present when the edited sheet is a source worksheet whose structural edits the writer
    /// replays on the preserved part.
    pub(super) history: Option<&'a mut GridHistory>,
    pub(super) budget: &'a mut FormulaRewriteBudget<'cancel>,
    /// Charged for the cell formats an insertion copies into the lines it adds.
    pub(super) materialization: &'a mut TableMaterializationBudget<'materialize>,
}

/// What a structural edit changed outside the edited sheet's cells.
pub(super) struct GridEditOutcome {
    /// Whether a defined name was rewritten.
    pub(super) names_changed: bool,
}

/// Applies one structural edit to sheet `sheet_id`.
///
/// Every formula of the workbook, defined name, table formula, and data validation formula is
/// rewritten first, so a refused rewrite leaves nothing half done in the staged state; then the
/// edited sheet's cells and structures move, and the draft's per-cell bookkeeping moves with
/// them.
pub(super) fn apply_grid_edit(
    state: GridEditState<'_, '_, '_>,
    sheet_id: SheetId,
    edit: GridEdit,
) -> Result<GridEditOutcome, BatchExecutionError> {
    let GridEditState {
        sheets,
        defined_names,
        presentation,
        mutations,
        presentation_cell_mutations,
        presentation_sheet_mutations,
        changed_cells,
        calculation_changed_cells,
        touched_sheets,
        changed_table_ids,
        history,
        budget,
        materialization,
    } = state;
    let index = sheets
        .iter()
        .position(|sheet| sheet.id() == sheet_id)
        .ok_or(ValidationError::UnknownSheetId {
            value: sheet_id.get(),
        })?;
    validate_sheet(&sheets[index], edit)?;
    materialization.charge_cells(presentation.inherited_format_cells(sheet_id, edit))?;
    let name = sheets[index].name().as_str().to_owned();
    let change = |host_is_sheet| {
        FormulaRewriteRequest::Grid(GridChange {
            sheet: &name,
            host_is_sheet,
            edit,
        })
    };

    let mut rewritten = BTreeMap::new();
    let mut changed_elsewhere = BTreeMap::<usize, BTreeSet<CellAddress>>::new();
    for (sheet_index, sheet) in sheets.iter_mut().enumerate() {
        let request = change(sheet_index == index);
        let mut previous = None;
        while let Some(cell) = sheet.next_formula_cell_after(previous) {
            budget.check_cancelled()?;
            let address = cell.address();
            previous = Some(address);
            let CellContent::Formula(formula) = cell.content() else {
                continue;
            };
            let Some(text) = formula.text() else {
                continue;
            };
            // A formula the paste overwrites goes with its cell, whatever it reads.
            if sheet_index == index && edit.overwrites(CellRange::from_ordered(address, address)) {
                continue;
            }
            let owner = format!("cell:sheet_id={},address={address}", sheet.id().get());
            let Some(text) = rewrite_formula(text.as_str(), &request, budget)
                .map_err(|error| error.with_owner(owner))?
            else {
                continue;
            };
            let text = FormulaText::from_xlsx(text)?;
            if sheet_index == index {
                rewritten.insert(address, text);
            } else {
                sheet.upsert_cell_deferred(
                    address,
                    CellContent::Formula(formula.clone().with_text(text)),
                    cell.number_format().clone(),
                );
                changed_elsewhere
                    .entry(sheet_index)
                    .or_default()
                    .insert(address);
            }
        }
    }

    let mut names_changed = false;
    for defined_name in defined_names.iter_mut() {
        budget.check_cancelled()?;
        let owner = match defined_name.scope() {
            DefinedNameScope::Workbook => {
                format!("defined_name:scope=workbook,name={}", defined_name.name())
            }
            DefinedNameScope::Sheet(scope) => format!(
                "defined_name:scope=sheet:{},name={}",
                scope.get(),
                defined_name.name()
            ),
        };
        let Some(text) = rewrite_formula(defined_name.formula().as_str(), &change(false), budget)
            .map_err(|error| error.with_owner(owner))?
        else {
            continue;
        };
        *defined_name = DefinedName::new(
            defined_name.name(),
            defined_name.scope(),
            FormulaText::from_xlsx(text)?,
            defined_name.hidden(),
        )?;
        names_changed = true;
    }

    for (sheet_index, sheet) in sheets.iter_mut().enumerate() {
        let request = change(sheet_index == index);
        let mut rewrites = Vec::new();
        for (table_index, table) in sheet.tables().iter().enumerate() {
            for (column_index, column) in table.columns().iter().enumerate() {
                budget.check_cancelled()?;
                let owner = |kind: &str| {
                    format!(
                        "table_formula:table_id={},column_id={},kind={kind}",
                        table.id().get(),
                        column.column_id().get()
                    )
                };
                let rewrite = |formula: Option<&crate::TableFormula>,
                               kind: &str,
                               budget: &mut FormulaRewriteBudget<'_>|
                 -> Result<Option<FormulaText>, BatchExecutionError> {
                    let Some(formula) = formula else {
                        return Ok(None);
                    };
                    let rewritten = rewrite_formula(formula.text().as_str(), &request, budget)
                        .map_err(|error| error.with_owner(owner(kind)))?;
                    Ok(rewritten.map(FormulaText::from_xlsx).transpose()?)
                };
                let calculated = rewrite(column.calculated_column_formula(), "calculated", budget)?;
                let totals = rewrite(column.totals_row_formula(), "totals", budget)?;
                if calculated.is_some() || totals.is_some() {
                    rewrites.push((table_index, column_index, calculated, totals));
                }
            }
        }
        for (table_index, column_index, calculated, totals) in rewrites {
            let table = &mut sheet.tables_mut()[table_index];
            changed_table_ids.insert(table.id());
            table.columns_mut()[column_index].rewrite_formulas(calculated, totals);
        }
        if sheet_index != index {
            let validations = sheet
                .data_validations()
                .iter()
                .enumerate()
                .map(|(validation_index, validation)| {
                    rewritten_validation(validation, &request, sheet.id(), validation_index, budget)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if validations.iter().any(Option::is_some) {
                let validations = validations
                    .into_iter()
                    .zip(sheet.data_validations())
                    .map(|(rewritten, original)| rewritten.unwrap_or_else(|| original.clone()))
                    .collect();
                sheet.set_data_validations(validations);
            }
        }
    }

    let sheet = &mut sheets[index];
    let validations = sheet
        .data_validations()
        .iter()
        .enumerate()
        .map(|(validation_index, validation)| {
            moved_validation(validation, edit, &name, sheet_id, validation_index, budget)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let tables = moved_tables(sheet, edit)?;
    let mut changed = sheet.apply_grid_edit(edit, &rewritten);
    sheet.set_data_validations(validations.into_iter().flatten().collect());
    let mut headers = Vec::new();
    let previous_tables = sheet.tables().to_vec();
    let mut moved = Vec::with_capacity(tables.len());
    for (table, previous, header_cells) in tables
        .into_iter()
        .zip(previous_tables)
        .map(|((table, header_cells), previous)| (table, previous, header_cells))
    {
        if !table.semantic_eq(&previous) {
            changed_table_ids.insert(table.id());
        }
        headers.extend(header_cells);
        moved.push(table);
    }
    sheet.set_tables(moved);
    for (address, name) in headers {
        let number_format = sheet.new_cell_number_format(address);
        sheet.upsert_cell_deferred(
            address,
            CellContent::Literal(CellValue::Text(name)),
            number_format,
        );
        changed.insert(address);
    }
    let detached = sheet.normalize_shared_formula_groups(&changed);
    changed.extend(detached);
    for (sheet_index, addresses) in &mut changed_elsewhere {
        let detached = sheets[*sheet_index].normalize_shared_formula_groups(addresses);
        addresses.extend(detached);
    }

    let remap = |cells: &mut BTreeSet<CalculationCellId>| {
        let moved = cells
            .iter()
            .filter(|cell| cell.sheet_id() == sheet_id)
            .copied()
            .collect::<Vec<_>>();
        for cell in moved {
            cells.remove(&cell);
            if let Some(address) = edit.map_address(cell.address()) {
                cells.insert(CalculationCellId::new(sheet_id, address));
            }
        }
    };
    remap(changed_cells);
    remap(calculation_changed_cells);
    remap(presentation_cell_mutations);
    mutations.remap_sheet(sheet_id, |address| edit.map_address(address));
    for (sheet, addresses) in std::iter::once((sheet_id, &changed)).chain(
        changed_elsewhere
            .iter()
            .map(|(sheet_index, addresses)| (sheets[*sheet_index].id(), addresses)),
    ) {
        for address in addresses {
            mark_upsert(mutations, sheet, *address, false);
            changed_cells.insert(CalculationCellId::new(sheet, *address));
            calculation_changed_cells.insert(CalculationCellId::new(sheet, *address));
        }
        touched_sheets.insert(sheet);
    }
    if presentation.apply_grid_edit(sheet_id, edit) {
        presentation_sheet_mutations.insert(sheet_id);
    }
    if let Some(history) = history {
        history.push(edit);
    }
    Ok(GridEditOutcome { names_changed })
}

/// Refuses an edit Excel does not make, or makes in a way not yet measured: shifting nonblank
/// cells off the sheet, changing part of a legacy array formula or data table, cutting part of a
/// spill range, moving part of a merged range, and deleting or moving a table's header row, every data row, its totals row,
/// or some of its columns.
fn validate_sheet(sheet: &Sheet, edit: GridEdit) -> Result<(), ValidationError> {
    let sheet_id = sheet.id().get();
    if let GridEdit::Insert { axis, count, .. } = edit {
        let last_kept = axis.max() - count;
        let occupied = sheet
            .used_range()
            .into_iter()
            .chain(sheet.merged_ranges().iter().copied())
            .chain(sheet.tables().iter().map(Table::range))
            .any(|range| axis.of(range.end()) > last_kept);
        if occupied {
            return Err(ValidationError::ContentShiftedOffSheet { sheet_id });
        }
    }
    for address in sheet.formula_addresses() {
        let Some(CellContent::Formula(formula)) = sheet.cell(*address).map(crate::Cell::content)
        else {
            continue;
        };
        let range = match formula.metadata() {
            FormulaMetadata::Array { range, .. } | FormulaMetadata::DataTable { range, .. } => {
                *range
            }
            // Excel refuses to cut part of a spill range; a line edit keeps the anchor's spill.
            FormulaMetadata::DynamicArray {
                range: Some(range), ..
            } if matches!(edit, GridEdit::MoveRange { .. }) => *range,
            _ => continue,
        };
        if edit.splits(range) {
            return Err(ValidationError::ArrayFormulaPartEdit {
                sheet_id,
                row: address.row().get(),
                column: address.column().get(),
            });
        }
    }
    if matches!(edit, GridEdit::Move { .. } | GridEdit::MoveRange { .. })
        && let Some(merged) = sheet
            .merged_ranges()
            .iter()
            .find(|range| edit.splits(**range))
    {
        return Err(ValidationError::MergedRangeSplit {
            sheet_id,
            row: merged.start().row().get(),
            column: merged.start().column().get(),
        });
    }
    for table in sheet.tables() {
        validate_table(table, edit)?;
    }
    Ok(())
}

fn validate_table(table: &Table, edit: GridEdit) -> Result<(), ValidationError> {
    let table_id = table.id().get();
    let range = table.range();
    let lines = |axis: GridAxis| (axis.of(range.start()), axis.of(range.end()));
    match edit {
        GridEdit::Insert {
            axis: GridAxis::Columns,
            at,
            ..
        } => {
            let (first, last) = lines(GridAxis::Columns);
            if first < at
                && at <= last
                && (table.table_type() != TableType::Worksheet || table.header_row_count() > 1)
            {
                return Err(ValidationError::UnsupportedTableAuthoringMetadata { table_id });
            }
        }
        GridEdit::Insert { .. } => {}
        GridEdit::Delete {
            axis: GridAxis::Rows,
            first,
            last,
        } => {
            let (top, bottom) = lines(GridAxis::Rows);
            let deleted = |start: u32, end: u32| start <= last && first <= end;
            if table.header_row_count() > 0 && deleted(top, top + table.header_row_count() - 1) {
                return Err(ValidationError::TableHeaderShift { table_id });
            }
            if table.totals_row_count() > 0
                && deleted(bottom + 1 - table.totals_row_count(), bottom)
            {
                return Err(ValidationError::TableSplit { table_id });
            }
            if let Some(data) = table.data_range()
                && first <= data.start().row().get()
                && data.end().row().get() <= last
            {
                return Err(ValidationError::TableDataRowsDeleted { table_id });
            }
        }
        GridEdit::Delete {
            axis: GridAxis::Columns,
            first,
            last,
        } => {
            // Excel 16.114 deletes the table columns the deletion reaches, and the table with its
            // last column (excel-measure K). Filter criteria and sort state name columns, which a
            // removed column would leave stale.
            let (left, right) = lines(GridAxis::Columns);
            let filtered = table.auto_filter().is_some_and(|filter| {
                !filter.filter_columns().is_empty() || filter.sort_state().is_some()
            });
            if first <= right
                && left <= last
                && (table.table_type() != TableType::Worksheet
                    || table.header_row_count() > 1
                    || filtered
                    || table.sort_state().is_some())
            {
                return Err(ValidationError::UnsupportedTableAuthoringMetadata { table_id });
            }
        }
        // Excel 16.114 reorders rows within a table's data and keeps its range.
        GridEdit::Move { .. } if table.reorders_data(edit) => {}
        GridEdit::Move { .. } | GridEdit::MoveRange { .. } => {
            if edit.splits(range) || edit.overwrites(range) {
                return Err(ValidationError::TableSplit { table_id });
            }
        }
    }
    Ok(())
}

/// Rewrites the structured references to the table columns a column deletion of sheet `sheet_id`
/// removes: a reference naming one, alone or as an end of a column range, becomes `#REF!`, and
/// every reference to a table the deletion removes whole does, as Excel 16.114 saves them
/// (excel-measure K). Runs before the grid edit moves the cells.
pub(super) fn remove_deleted_table_columns(
    sheets: &mut [Sheet],
    defined_names: &mut [DefinedName],
    non_worksheet_tabs: &NonWorksheetTabs,
    edit_state: WorkbookFormulaEdit<'_, '_>,
    sheet_id: SheetId,
    edit: GridEdit,
) -> Result<(), BatchExecutionError> {
    let GridEdit::Delete {
        axis: GridAxis::Columns,
        first,
        last,
    } = edit
    else {
        return Ok(());
    };
    let Some(sheet_index) = sheets.iter().position(|sheet| sheet.id() == sheet_id) else {
        return Ok(());
    };
    let affected = sheets[sheet_index]
        .tables()
        .iter()
        .filter_map(|table| {
            let start = table.range().start().column().get();
            let removed = table
                .columns()
                .iter()
                .zip(start..)
                .filter(|(_, column)| (first..=last).contains(column))
                .map(|(column, _)| column.name().to_owned())
                .collect::<Vec<_>>();
            (!removed.is_empty()).then(|| {
                let whole = removed.len() == table.columns().len();
                (
                    table.id(),
                    table.range(),
                    table.display_name().as_str().to_owned(),
                    removed,
                    whole,
                )
            })
        })
        .collect::<Vec<_>>();
    let WorkbookFormulaEdit {
        state,
        changed_table_ids,
        table_formula_locations,
        budget,
    } = edit_state;
    let FormulaEditState {
        mutations,
        changed_cells,
        calculation_changed_cells,
        touched_sheets,
    } = state;
    let tab_names = TabOrder::of(sheets, non_worksheet_tabs)
        .names()
        .into_iter()
        .map(|(name, id)| (name.to_owned(), id))
        .collect::<Vec<_>>();
    for (table_id, range, name, removed, whole) in affected {
        let removed = removed.iter().map(String::as_str).collect::<Vec<_>>();
        let before = tab_names
            .iter()
            .map(|(name, id)| (name.as_str(), *id))
            .collect::<Vec<_>>();
        let after = tab_names
            .iter()
            .map(|(name, id)| (*id, name.as_str()))
            .collect::<Vec<_>>();
        let deleted_tables = [name.as_str()];
        let rename = if whole {
            WorkbookFormulaRename::SheetChanges(SheetChanges {
                before: &before,
                after: &after,
                steps: &[],
                retarget: None,
                deleted_tables: &deleted_tables,
            })
        } else {
            WorkbookFormulaRename::TableColumnsRemoved {
                table_id,
                target_sheet_index: sheet_index,
                target_range: range,
                table_name: &name,
                removed: &removed,
            }
        };
        rewrite_workbook_formulas(
            sheets,
            defined_names,
            WorkbookFormulaEdit {
                state: FormulaEditState {
                    mutations: &mut *mutations,
                    changed_cells: &mut *changed_cells,
                    calculation_changed_cells: &mut *calculation_changed_cells,
                    touched_sheets: &mut *touched_sheets,
                },
                changed_table_ids: &mut *changed_table_ids,
                table_formula_locations,
                budget: &mut *budget,
            },
            rename,
        )?;
    }
    Ok(())
}

/// The header cells, and the names they hold, of the columns an insertion adds to a table.
type HeaderCells = Vec<(CellAddress, String)>;

/// Returns each table of `sheet` after the edit, with the header cells of the columns an
/// insertion inside it adds.
fn moved_tables(
    sheet: &Sheet,
    edit: GridEdit,
) -> Result<Vec<(Table, HeaderCells)>, ValidationError> {
    let mut moved = Vec::with_capacity(sheet.tables().len());
    for table in sheet.tables() {
        let (inserted, headers) = inserted_table_columns(table, edit)?;
        match table.grid_mapped(edit, inserted) {
            Some(mapped) => moved.push((mapped, headers)),
            // A column deletion reaching every column deletes the table.
            None if matches!(
                edit,
                GridEdit::Delete {
                    axis: GridAxis::Columns,
                    ..
                }
            ) => {}
            None => {
                return Err(ValidationError::TableSplit {
                    table_id: table.id().get(),
                });
            }
        }
    }
    Ok(moved)
}

/// Returns the columns an insertion strictly inside a table adds, named `Column1`, `Column2`,
/// and so on with the smallest numbers no column of the table uses, and their header cells.
fn inserted_table_columns(
    table: &Table,
    edit: GridEdit,
) -> Result<(Vec<TableColumn>, HeaderCells), ValidationError> {
    let GridEdit::Insert {
        axis: GridAxis::Columns,
        at,
        count,
    } = edit
    else {
        return Ok((Vec::new(), Vec::new()));
    };
    let range = table.range();
    if !(range.start().column().get() < at && at <= range.end().column().get()) {
        return Ok((Vec::new(), Vec::new()));
    }
    let unsupported = || ValidationError::UnsupportedTableAuthoringMetadata {
        table_id: table.id().get(),
    };
    let mut names = table
        .columns()
        .iter()
        .map(|column| column.name().to_owned())
        .collect::<Vec<_>>();
    let mut last_id = table
        .columns()
        .iter()
        .map(TableColumn::id)
        .max()
        .unwrap_or(0);
    let mut number = 1_u32;
    let mut columns = Vec::new();
    let mut headers = Vec::new();
    for offset in 0..count {
        let name = loop {
            let candidate = format!("Column{number}");
            number = number.checked_add(1).ok_or_else(unsupported)?;
            if !names
                .iter()
                .any(|existing| case_insensitive_eq(existing, &candidate))
            {
                break candidate;
            }
        };
        last_id = last_id.checked_add(1).ok_or_else(unsupported)?;
        columns.push(TableColumn::new(last_id, name.as_str(), None)?);
        if table.header_row_count() == 1 {
            headers.push((
                CellAddress::new(range.start().row(), Column::new(at + offset)?),
                name.clone(),
            ));
        }
        names.push(name);
    }
    Ok((columns, headers))
}

/// Rewrites the formulas of a data validation on another sheet, which name the edited sheet only
/// when qualified. Returns `None` when neither formula changes.
fn rewritten_validation(
    validation: &DataValidation,
    request: &FormulaRewriteRequest<'_>,
    sheet_id: SheetId,
    validation_index: usize,
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<Option<DataValidation>, BatchExecutionError> {
    let mut rewritten = validation.clone();
    let mut changed = false;
    for (slot, formula) in rewritten.formulas_mut().into_iter().enumerate() {
        let Some(text) = formula.as_deref() else {
            continue;
        };
        let owner = validation_owner(sheet_id, validation_index, slot);
        if let Some(text) =
            rewrite_formula(text, request, budget).map_err(|error| error.with_owner(owner))?
        {
            *formula = Some(text.into_boxed_str());
            changed = true;
        }
    }
    Ok(changed.then_some(rewritten))
}

/// Moves a data validation of the edited sheet: its ranges keep the cells they validate, cells
/// inserted right after a range join it, a validation whose cells are all deleted goes, and the
/// part of its cells a range move cuts away becomes a validation of its own.
fn moved_validation(
    validation: &DataValidation,
    edit: GridEdit,
    sheet_name: &str,
    sheet_id: SheetId,
    validation_index: usize,
    budget: &mut FormulaRewriteBudget<'_>,
) -> Result<Vec<DataValidation>, BatchExecutionError> {
    let formulas = [validation.formula1(), validation.formula2()];
    let present = formulas.iter().flatten().copied().collect::<Vec<_>>();
    let rules = move_anchored_formulas(
        validation.ranges(),
        &present,
        GridChange {
            sheet: sheet_name,
            host_is_sheet: true,
            edit,
        },
        budget,
    )
    .map_err(|error| error.with_owner(validation_owner(sheet_id, validation_index, 0)))?;
    Ok(rules
        .into_iter()
        .map(|rule| {
            let mut moved = validation.clone();
            moved.set_ranges(rule.ranges);
            let mut rewritten = rule.formulas.into_iter();
            for formula in moved.formulas_mut() {
                if formula.is_some()
                    && let Some(Some(text)) = rewritten.next()
                {
                    *formula = Some(text.into_boxed_str());
                }
            }
            moved
        })
        .collect())
}

fn validation_owner(sheet_id: SheetId, validation_index: usize, slot: usize) -> String {
    format!(
        "data_validation:sheet_id={},index={validation_index},formula={}",
        sheet_id.get(),
        slot + 1
    )
}

/// Returns the sheet and grid edit of a structural change, or `None` for any other change.
pub(super) fn grid_change(
    change: &WorkbookChange,
) -> Result<Option<(SheetId, GridEdit)>, ValidationError> {
    let insertion = |axis: GridAxis, at: u32, count: u32| {
        super::change::validate_insertion(at, count, axis.max(), |value| match axis {
            GridAxis::Rows => ValidationError::RowOutOfRange { value },
            GridAxis::Columns => ValidationError::ColumnOutOfRange { value },
        })?;
        Ok(GridEdit::Insert { axis, at, count })
    };
    let movement = |axis: GridAxis, first: u32, last: u32, destination: u32| {
        super::change::validate_move(first, last, destination, axis.max(), |value| match axis {
            GridAxis::Rows => ValidationError::RowOutOfRange { value },
            GridAxis::Columns => ValidationError::ColumnOutOfRange { value },
        })?;
        Ok(GridEdit::Move {
            axis,
            first,
            last,
            destination,
        })
    };
    let deletion = |axis: GridAxis, first: u32, last: u32| {
        if first > last {
            return Err(ValidationError::RangeStartAfterEnd);
        }
        Ok(GridEdit::Delete { axis, first, last })
    };
    Ok(Some(match change {
        WorkbookChange::InsertRows {
            sheet_id,
            row,
            count,
        } => (*sheet_id, insertion(GridAxis::Rows, row.get(), *count)?),
        WorkbookChange::DeleteRows {
            sheet_id,
            first,
            last,
        } => (
            *sheet_id,
            deletion(GridAxis::Rows, first.get(), last.get())?,
        ),
        WorkbookChange::InsertColumns {
            sheet_id,
            column,
            count,
        } => (
            *sheet_id,
            insertion(GridAxis::Columns, column.get(), *count)?,
        ),
        WorkbookChange::DeleteColumns {
            sheet_id,
            first,
            last,
        } => (
            *sheet_id,
            deletion(GridAxis::Columns, first.get(), last.get())?,
        ),
        WorkbookChange::MoveRows {
            sheet_id,
            first,
            last,
            destination,
        } => (
            *sheet_id,
            movement(GridAxis::Rows, first.get(), last.get(), destination.get())?,
        ),
        WorkbookChange::MoveColumns {
            sheet_id,
            first,
            last,
            destination,
        } => (
            *sheet_id,
            movement(
                GridAxis::Columns,
                first.get(),
                last.get(),
                destination.get(),
            )?,
        ),
        WorkbookChange::MoveRange {
            sheet_id,
            source,
            destination,
        } => {
            super::change::validate_range_move(*source, *destination)?;
            (
                *sheet_id,
                GridEdit::MoveRange {
                    source: *source,
                    destination: *destination,
                },
            )
        }
        _ => return Ok(None),
    }))
}

#[cfg(test)]
#[path = "grid_edit_tests.rs"]
mod tests;
