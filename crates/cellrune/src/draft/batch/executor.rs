use std::collections::BTreeSet;

use super::super::format::effective_cell_format;
use super::super::presentation::{authored_annotation, phonetics_require_text_cell};
use super::super::{
    SourceCells, WorkbookDraft, annotated_text_replacement_required, case_insensitive_key,
    ensure_not_array_formula_part, next_revision,
};
use super::auto_filter_edit::set_sheet_auto_filter;
use super::copy_edit::{CopyEditState, copy_range};
use super::formula_edit::{
    FormulaEditState, WorkbookFormulaEdit, rename_sheet, table_formula_locations, upsert_formula,
};
use super::grid_edit::{GridEditState, apply_grid_edit, grid_change, remove_deleted_table_columns};
use super::inverse::{
    InverseSeed, TableShape, UndoStep, edit_inverse, has_inverse, moves_preserved_content,
};
use super::name_rename::rename_defined_name;
use super::sheet_copy::{SheetCopy, copy_sheet};
use super::sheet_delete::delete_sheet;
use super::sheet_move::move_sheet;
use super::staged::{mark_removed, mark_upsert, sheet_by_id_mut};
use super::tab_order::TabOrder;
use super::table_convert::convert_table_to_range;
use super::table_create::create_table;
use super::table_edit::{
    TableEditState, TableFormulaEdit, TableResizeEdit, rename_table, rename_table_column,
    resize_table_rows, table_header_column, table_header_edit_error, table_locations,
};
use super::table_range::set_table_range;
use super::widen_edit::{align_filter_database_name, widen_sheet_auto_filter, widen_table_columns};
use super::{
    BatchExecutionError, EditBatch, EditReceipt, TableMaterializationBudget, WorkbookChange,
};
use crate::calculation::formula_rewrite::{
    FormulaRewriteBudget, FormulaRewriteError, FormulaRewriteLimits,
};
use crate::workbook::{NonWorksheetTabs, WorkbookBuildError, WorkbookSnapshotInput};
use crate::{
    CalculationCellId, CellContent, CellRange, CellValue, FormulaCell, FormulaDialect,
    FormulaMetadata, SavedResult, Sheet, SheetAutoFilter, SheetId, SheetVisibility, Table,
    TableColumnName, TableId, ValidationError, WorkbookSnapshot,
};

impl WorkbookDraft {
    /// Applies an ordered batch atomically and advances the semantic revision at most once.
    ///
    /// All mutations are performed on one staged workbook clone. If any operation or final
    /// workbook invariant fails, `self` remains unchanged.
    ///
    /// # Errors
    ///
    /// Returns a [`ValidationError`] for an invalid sheet, cell, name, range, visibility state,
    /// workbook invariant, or exhausted semantic revision.
    pub fn apply_changes(&mut self, batch: EditBatch) -> Result<EditReceipt, ValidationError> {
        let never_cancelled = || false;
        let mut rewrite_budget =
            FormulaRewriteBudget::new(FormulaRewriteLimits::UNBOUNDED, &never_cancelled);
        let mut materialization_budget =
            TableMaterializationBudget::new(usize::MAX, &never_cancelled);
        self.apply_changes_controlled(batch, &mut rewrite_budget, &mut materialization_budget)
            .map_err(BatchExecutionError::into_validation)
    }

    pub(crate) fn apply_changes_controlled(
        &mut self,
        batch: EditBatch,
        rewrite_budget: &mut FormulaRewriteBudget<'_>,
        materialization_budget: &mut TableMaterializationBudget<'_>,
    ) -> Result<EditReceipt, BatchExecutionError> {
        self.apply_batch(batch, rewrite_budget, materialization_budget, true)
    }

    /// Applies a batch; `with_inverse` builds the inverse a structural batch's receipt carries.
    pub(super) fn apply_batch(
        &mut self,
        batch: EditBatch,
        rewrite_budget: &mut FormulaRewriteBudget<'_>,
        materialization_budget: &mut TableMaterializationBudget<'_>,
        with_inverse: bool,
    ) -> Result<EditReceipt, BatchExecutionError> {
        let base_revision = self.semantic_revision();
        if batch.is_empty() {
            return Ok(EditReceipt {
                base_revision,
                result_revision: base_revision,
                applied_change_count: 0,
                changed_cells: Vec::new(),
                calculation_changed_cells: Vec::new(),
                created_sheet_ids: Vec::new(),
                changed_table_ids: Vec::new(),
                topology_changed: false,
                cell_formulas_changed_topology_only: false,
                calculation_metadata_changed: false,
                number_formats_changed: false,
                inverse: None,
            });
        }
        let with_inverse = with_inverse && batch.changes().iter().any(has_inverse);
        let mut inverse_seed = InverseSeed::default();

        rewrite_budget.check_cancelled()?;
        let mut sheets = clone_sheets(self.workbook.sheets(), rewrite_budget)?;
        let mut non_worksheet_tabs = self.workbook.non_worksheet_tabs().clone();
        let mut table_locations = table_locations(&sheets)?;
        let mut table_formula_locations = table_formula_locations(&sheets, rewrite_budget)?;
        let mut defined_names = clone_slice(self.workbook.defined_names(), rewrite_budget)?;
        let mut date_system = self.workbook.date_system();
        let mut calculation_hints = self.workbook.calculation_hints();
        let source_workbook = self
            .source_document
            .as_deref()
            .map(crate::XlsxDocument::workbook);
        let mut grid_histories = self.grid_histories.clone();
        let mut cell_mutations = self.cell_mutations.clone();
        let mut presentation = self
            .presentation
            .clone_cancellable(&|| rewrite_budget.check_cancelled().is_err())
            .map_err(|()| FormulaRewriteError::Cancelled)?;
        let mut presentation_cell_mutations =
            clone_set(&self.presentation_cell_mutations, rewrite_budget)?;
        let mut presentation_sheet_mutations =
            clone_set(&self.presentation_sheet_mutations, rewrite_budget)?;
        let mut added_sheets = clone_set(&self.added_sheets, rewrite_budget)?;
        let mut previous_changed_table_ids = clone_set(&self.changed_table_ids, rewrite_budget)?;
        let mut changed_table_ids = BTreeSet::new();
        let mut workbook_changed = self.workbook_changed;
        let mut tab_steps = clone_slice(&self.tab_steps, rewrite_budget)?;
        let mut name_renames = clone_slice(&self.name_renames, rewrite_budget)?;
        let mut copied_sheets = self.copied_sheets.clone();
        let mut copied_tables = self.copied_tables.clone();
        let mut changed_cells = BTreeSet::new();
        let mut calculation_changed_cells = BTreeSet::new();
        let mut touched_sheets = BTreeSet::new();
        let mut created_sheet_ids = Vec::new();
        let mut topology_changed = false;
        // Topology changes from cells gaining, losing, or replacing a formula, which a session can
        // apply locally; every other change sets `topology_changed`.
        let mut cell_formulas_changed = false;
        let mut calculation_metadata_changed = false;
        let mut number_formats_changed = false;
        let mut semantic_changed = false;
        // Every ID the workbook or this batch has used stays taken, including those of sheets
        // deleted since, so an added sheet takes the one after the highest.
        let mut highest_sheet_id = sheets
            .iter()
            .map(Sheet::id)
            .chain(non_worksheet_tabs.iter().map(|tab| tab.id()))
            .chain(self.workbook.highest_sheet_id())
            .map(SheetId::get)
            .max()
            .unwrap_or(0);

        for change in batch.changes() {
            match change {
                WorkbookChange::SetCellValue {
                    sheet_id,
                    address,
                    value,
                } => {
                    // A header cell holds its column's name, so writing one renames the column.
                    if let Some((table_id, column_id)) =
                        table_header_column(sheet_by_id_mut(&mut sheets, *sheet_id)?, *address)
                    {
                        let CellValue::Text(text) = value else {
                            return Err(table_header_edit_error(
                                table_id,
                                *address,
                                *value == CellValue::Blank,
                            )
                            .into());
                        };
                        let new_name = TableColumnName::new(text.as_str())?;
                        if rename_table_column(
                            &mut sheets,
                            &mut defined_names,
                            TableFormulaEdit {
                                state: TableEditState {
                                    mutations: &mut cell_mutations,
                                    changed_cells: &mut changed_cells,
                                    calculation_changed_cells: &mut calculation_changed_cells,
                                    touched_sheets: &mut touched_sheets,
                                    changed_table_ids: &mut changed_table_ids,
                                    presentation: &presentation,
                                    source: SourceCells::new(source_workbook, &grid_histories),
                                },
                                locations: &table_locations,
                                formula_locations: &table_formula_locations,
                                budget: rewrite_budget,
                            },
                            table_id,
                            column_id,
                            &new_name,
                        )? {
                            workbook_changed = true;
                            topology_changed = true;
                            semantic_changed = true;
                            continue;
                        }
                    }
                    let sheet = sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    ensure_not_array_formula_part(sheet, *address)?;
                    for follower in sheet.detach_shared_formula_followers(*address) {
                        mark_upsert(&mut cell_mutations, *sheet_id, follower, false);
                    }
                    let previous_was_formula = sheet
                        .cell(*address)
                        .is_some_and(|cell| matches!(cell.content(), CellContent::Formula(_)));
                    let content_unchanged = sheet.cell(*address).is_some_and(|cell| {
                        matches!(cell.content(), CellContent::Literal(previous) if previous == value)
                    });
                    if presentation.has_cell_annotation(*sheet_id, *address) {
                        if *value == CellValue::Blank {
                            if presentation.clear_cell_phonetics(*sheet_id, *address)? {
                                presentation_cell_mutations
                                    .insert(CalculationCellId::new(*sheet_id, *address));
                            }
                        } else if !content_unchanged {
                            return Err(
                                annotated_text_replacement_required(*sheet_id, *address).into()
                            );
                        }
                    }
                    if *value == CellValue::Blank {
                        if sheet.remove_cell_deferred(*address) {
                            if !mark_removed(
                                &mut cell_mutations,
                                SourceCells::new(source_workbook, &grid_histories),
                                *sheet_id,
                                *address,
                            ) {
                                presentation_cell_mutations
                                    .remove(&CalculationCellId::new(*sheet_id, *address));
                            }
                            changed_cells.insert(CalculationCellId::new(*sheet_id, *address));
                            calculation_changed_cells
                                .insert(CalculationCellId::new(*sheet_id, *address));
                            touched_sheets.insert(*sheet_id);
                            semantic_changed = true;
                        }
                    } else if !content_unchanged {
                        let content = CellContent::Literal(value.clone());
                        let replacement = match sheet.cell(*address) {
                            Some(cell) => cell.with_replaced_content(content),
                            None => crate::Cell::with_number_format(
                                *address,
                                content,
                                sheet.new_cell_number_format(*address),
                            ),
                        };
                        sheet.upsert_cell_instance_deferred(replacement);
                        mark_upsert(&mut cell_mutations, *sheet_id, *address, false);
                        changed_cells.insert(CalculationCellId::new(*sheet_id, *address));
                        calculation_changed_cells
                            .insert(CalculationCellId::new(*sheet_id, *address));
                        touched_sheets.insert(*sheet_id);
                        semantic_changed = true;
                    }
                    cell_formulas_changed |= previous_was_formula;
                }
                WorkbookChange::SetCellFormula {
                    sheet_id,
                    address,
                    formula,
                } => {
                    let sheet = sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    ensure_not_array_formula_part(sheet, *address)?;
                    if let Some((table_id, _)) = table_header_column(sheet, *address) {
                        return Err(table_header_edit_error(table_id, *address, false).into());
                    }
                    for follower in sheet.detach_shared_formula_followers(*address) {
                        mark_upsert(&mut cell_mutations, *sheet_id, follower, false);
                    }
                    if presentation.has_cell_annotation(*sheet_id, *address) {
                        return Err(annotated_text_replacement_required(*sheet_id, *address).into());
                    }
                    let formula = FormulaCell::new(
                        FormulaDialect::ExcelA1,
                        formula.clone(),
                        SavedResult::Missing,
                        FormulaMetadata::Normal,
                    );
                    if upsert_formula(
                        &mut sheets,
                        FormulaEditState {
                            mutations: &mut cell_mutations,
                            changed_cells: &mut changed_cells,
                            calculation_changed_cells: &mut calculation_changed_cells,
                            touched_sheets: &mut touched_sheets,
                        },
                        *sheet_id,
                        *address,
                        formula,
                    )? {
                        cell_formulas_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::SetCellDynamicFormula {
                    sheet_id,
                    address,
                    formula,
                    range,
                } => {
                    let sheet = sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    ensure_not_array_formula_part(sheet, *address)?;
                    if let Some((table_id, _)) = table_header_column(sheet, *address) {
                        return Err(table_header_edit_error(table_id, *address, false).into());
                    }
                    for follower in sheet.detach_shared_formula_followers(*address) {
                        mark_upsert(&mut cell_mutations, *sheet_id, follower, false);
                    }
                    if presentation.has_cell_annotation(*sheet_id, *address) {
                        return Err(annotated_text_replacement_required(*sheet_id, *address).into());
                    }
                    let formula = FormulaCell::new(
                        FormulaDialect::ExcelA1,
                        formula.clone(),
                        SavedResult::Missing,
                        FormulaMetadata::DynamicArray {
                            range: *range,
                            always_calculate: false,
                        },
                    );
                    if upsert_formula(
                        &mut sheets,
                        FormulaEditState {
                            mutations: &mut cell_mutations,
                            changed_cells: &mut changed_cells,
                            calculation_changed_cells: &mut calculation_changed_cells,
                            touched_sheets: &mut touched_sheets,
                        },
                        *sheet_id,
                        *address,
                        formula,
                    )? {
                        cell_formulas_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::ClearCell { sheet_id, address } => {
                    let sheet = sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    ensure_not_array_formula_part(sheet, *address)?;
                    if let Some((table_id, _)) = table_header_column(sheet, *address) {
                        return Err(table_header_edit_error(table_id, *address, true).into());
                    }
                    for follower in sheet.detach_shared_formula_followers(*address) {
                        mark_upsert(&mut cell_mutations, *sheet_id, follower, false);
                    }
                    if presentation.clear_cell_phonetics(*sheet_id, *address)? {
                        presentation_cell_mutations
                            .insert(CalculationCellId::new(*sheet_id, *address));
                    }
                    let previous_was_formula = sheet
                        .cell(*address)
                        .is_some_and(|cell| matches!(cell.content(), CellContent::Formula(_)));
                    if sheet.remove_cell_deferred(*address) {
                        if !mark_removed(
                            &mut cell_mutations,
                            SourceCells::new(source_workbook, &grid_histories),
                            *sheet_id,
                            *address,
                        ) {
                            presentation_cell_mutations
                                .remove(&CalculationCellId::new(*sheet_id, *address));
                        }
                        changed_cells.insert(CalculationCellId::new(*sheet_id, *address));
                        calculation_changed_cells
                            .insert(CalculationCellId::new(*sheet_id, *address));
                        touched_sheets.insert(*sheet_id);
                        semantic_changed = true;
                    }
                    cell_formulas_changed |= previous_was_formula;
                }
                WorkbookChange::SetCellNumberFormat {
                    sheet_id,
                    address,
                    number_format,
                } => {
                    let sheet = sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    let replacement = {
                        let cell = sheet.cell(*address).ok_or(ValidationError::CellNotFound {
                            sheet_id: sheet_id.get(),
                            row: address.row().get(),
                            column: address.column().get(),
                        })?;
                        (cell.number_format() != number_format)
                            .then(|| cell.with_replaced_number_format(number_format.clone()))
                    };
                    if let Some(replacement) = replacement {
                        sheet.upsert_cell_instance_deferred(replacement);
                        mark_upsert(&mut cell_mutations, *sheet_id, *address, true);
                        changed_cells.insert(CalculationCellId::new(*sheet_id, *address));
                        touched_sheets.insert(*sheet_id);
                        semantic_changed = true;
                        number_formats_changed = true;
                    }
                }
                WorkbookChange::AddSheet { name } => {
                    if sheets
                        .iter()
                        .any(|sheet| sheet.name().lookup_key() == name.lookup_key())
                    {
                        return Err(ValidationError::DuplicateSheetName {
                            name: name.as_str().to_owned(),
                        }
                        .into());
                    }
                    let next = highest_sheet_id
                        .checked_add(1)
                        .ok_or(ValidationError::SheetIdExhausted)?;
                    let sheet_id =
                        SheetId::new(next).map_err(|_| ValidationError::SheetIdExhausted)?;
                    highest_sheet_id = next;
                    sheets.push(Sheet::new(sheet_id, name.clone(), SheetVisibility::Visible));
                    added_sheets.insert(sheet_id);
                    created_sheet_ids.push(sheet_id);
                    workbook_changed = true;
                    topology_changed = true;
                    semantic_changed = true;
                }
                WorkbookChange::RenameSheet { sheet_id, name } => {
                    if rename_sheet(
                        &mut sheets,
                        &mut defined_names,
                        WorkbookFormulaEdit {
                            state: FormulaEditState {
                                mutations: &mut cell_mutations,
                                changed_cells: &mut changed_cells,
                                calculation_changed_cells: &mut calculation_changed_cells,
                                touched_sheets: &mut touched_sheets,
                            },
                            changed_table_ids: &mut changed_table_ids,
                            table_formula_locations: &table_formula_locations,
                            budget: rewrite_budget,
                        },
                        *sheet_id,
                        name,
                    )? {
                        touched_sheets.insert(*sheet_id);
                        workbook_changed = true;
                        topology_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::DeleteSheet { sheet_id } => {
                    let undo = sheets
                        .iter()
                        .find(|sheet| sheet.id() == *sheet_id)
                        .map(|sheet| UndoStep::DeleteSheet {
                            sheet_id: *sheet_id,
                            name: sheet.name().clone(),
                            position: tab_position(&sheets, &non_worksheet_tabs, *sheet_id),
                            tables: sheet
                                .tables()
                                .iter()
                                .filter_map(|table| TableShape::of(*sheet_id, table))
                                .collect(),
                            source: source_workbook
                                .is_some_and(|source| source.sheet_by_id(*sheet_id).is_some()),
                        });
                    let (removed, step) = delete_sheet(
                        &mut sheets,
                        &mut defined_names,
                        &mut non_worksheet_tabs,
                        WorkbookFormulaEdit {
                            state: FormulaEditState {
                                mutations: &mut cell_mutations,
                                changed_cells: &mut changed_cells,
                                calculation_changed_cells: &mut calculation_changed_cells,
                                touched_sheets: &mut touched_sheets,
                            },
                            changed_table_ids: &mut changed_table_ids,
                            table_formula_locations: &table_formula_locations,
                            budget: rewrite_budget,
                        },
                        *sheet_id,
                    )?;
                    tab_steps.push(step);
                    inverse_seed.steps.extend(undo);
                    // Nothing the draft recorded for the sheet outlives it: its cells, tables, and
                    // presentation are gone, and the writer drops its package parts.
                    let elsewhere = |cell: &CalculationCellId| cell.sheet_id() != *sheet_id;
                    cell_mutations.remove_sheet(*sheet_id);
                    changed_cells.retain(elsewhere);
                    calculation_changed_cells.retain(elsewhere);
                    presentation_cell_mutations.retain(elsewhere);
                    presentation.remove_sheet(*sheet_id);
                    presentation_sheet_mutations.remove(sheet_id);
                    touched_sheets.remove(sheet_id);
                    added_sheets.remove(sheet_id);
                    copied_sheets.remove(sheet_id);
                    grid_histories.remove(sheet_id);
                    for table in removed.tables() {
                        changed_table_ids.remove(&table.id());
                        previous_changed_table_ids.remove(&table.id());
                    }
                    table_locations = super::table_edit::table_locations(&sheets)?;
                    table_formula_locations =
                        super::formula_edit::table_formula_locations(&sheets, rewrite_budget)?;
                    workbook_changed = true;
                    topology_changed = true;
                    semantic_changed = true;
                }
                WorkbookChange::CopySheet { sheet_id, position } => {
                    let next = highest_sheet_id
                        .checked_add(1)
                        .ok_or(ValidationError::SheetIdExhausted)?;
                    let copy_id =
                        SheetId::new(next).map_err(|_| ValidationError::SheetIdExhausted)?;
                    let tables = copy_sheet(
                        &mut sheets,
                        &mut defined_names,
                        &mut non_worksheet_tabs,
                        source_workbook,
                        rewrite_budget,
                        SheetCopy {
                            sheet_id: *sheet_id,
                            position: *position,
                            copy_id,
                        },
                    )?;
                    highest_sheet_id = next;
                    presentation.copy_sheet(*sheet_id, copy_id);
                    // The writer duplicates the source part of a sheet the document holds, which a
                    // copy of a copy reaches through its own source.
                    let origin = if source_workbook
                        .is_some_and(|source| source.sheet_by_id(*sheet_id).is_some())
                    {
                        Some(*sheet_id)
                    } else {
                        copied_sheets.get(sheet_id).copied()
                    };
                    match origin {
                        Some(origin) => {
                            copied_sheets.insert(copy_id, origin);
                            // The copy duplicates its origin's part as the package holds it, so
                            // it replays the structural edits its source had before the copy.
                            if let Some(history) = grid_histories.get(sheet_id).cloned() {
                                grid_histories.insert(copy_id, history);
                            }
                            // A copied table duplicates the source table its own source does.
                            for (source_table, copied) in &tables {
                                let duplicated = if source_workbook.is_some_and(|source| {
                                    source.table_by_id(*source_table).is_some()
                                }) {
                                    Some(*source_table)
                                } else {
                                    copied_tables.get(source_table).copied()
                                };
                                if let Some(duplicated) = duplicated {
                                    copied_tables.insert(*copied, duplicated);
                                }
                            }
                        }
                        None => {
                            added_sheets.insert(copy_id);
                        }
                    }
                    created_sheet_ids.push(copy_id);
                    changed_table_ids.extend(tables.into_iter().map(|(_, copied)| copied));
                    table_locations = super::table_edit::table_locations(&sheets)?;
                    table_formula_locations =
                        super::formula_edit::table_formula_locations(&sheets, rewrite_budget)?;
                    workbook_changed = true;
                    topology_changed = true;
                    semantic_changed = true;
                }
                WorkbookChange::ImportSheet { sheet, position } => {
                    let next = highest_sheet_id
                        .checked_add(1)
                        .ok_or(ValidationError::SheetIdExhausted)?;
                    let sheet_id =
                        SheetId::new(next).map_err(|_| ValidationError::SheetIdExhausted)?;
                    let mut number_formats =
                        super::sheet_import::used_number_formats(self.source_document(), &sheets)?;
                    let tables = super::sheet_import::import_sheet(
                        &mut sheets,
                        &mut defined_names,
                        &mut non_worksheet_tabs,
                        date_system,
                        rewrite_budget,
                        super::sheet_import::SheetImport {
                            imported: sheet,
                            position: *position,
                            sheet_id,
                            number_formats: &mut number_formats,
                        },
                    )?;
                    highest_sheet_id = next;
                    super::sheet_import::import_presentation(&mut presentation, sheet, sheet_id)?;
                    added_sheets.insert(sheet_id);
                    created_sheet_ids.push(sheet_id);
                    changed_table_ids.extend(tables);
                    table_locations = super::table_edit::table_locations(&sheets)?;
                    table_formula_locations =
                        super::formula_edit::table_formula_locations(&sheets, rewrite_budget)?;
                    workbook_changed = true;
                    topology_changed = true;
                    semantic_changed = true;
                }
                WorkbookChange::MoveSheet { sheet_id, position } => {
                    let from = tab_position(&sheets, &non_worksheet_tabs, *sheet_id);
                    let Some(step) = move_sheet(
                        &mut sheets,
                        &mut defined_names,
                        &mut non_worksheet_tabs,
                        WorkbookFormulaEdit {
                            state: FormulaEditState {
                                mutations: &mut cell_mutations,
                                changed_cells: &mut changed_cells,
                                calculation_changed_cells: &mut calculation_changed_cells,
                                touched_sheets: &mut touched_sheets,
                            },
                            changed_table_ids: &mut changed_table_ids,
                            table_formula_locations: &table_formula_locations,
                            budget: rewrite_budget,
                        },
                        *sheet_id,
                        *position,
                    )?
                    else {
                        continue;
                    };
                    tab_steps.push(step);
                    inverse_seed.steps.push(UndoStep::MoveSheet {
                        sheet_id: *sheet_id,
                        position: from,
                    });
                    // Table locations name sheets by their index, which the move changed.
                    table_locations = super::table_edit::table_locations(&sheets)?;
                    table_formula_locations =
                        super::formula_edit::table_formula_locations(&sheets, rewrite_budget)?;
                    workbook_changed = true;
                    topology_changed = true;
                    semantic_changed = true;
                }
                WorkbookChange::SetSheetVisibility {
                    sheet_id,
                    visibility,
                } => {
                    let current = sheet_by_id_mut(&mut sheets, *sheet_id)?.visibility();
                    if current != *visibility {
                        if current == SheetVisibility::Visible
                            && *visibility != SheetVisibility::Visible
                            && sheets
                                .iter()
                                .filter(|sheet| sheet.visibility() == SheetVisibility::Visible)
                                .count()
                                == 1
                        {
                            return Err(ValidationError::LastVisibleSheet.into());
                        }
                        sheet_by_id_mut(&mut sheets, *sheet_id)?.set_visibility(*visibility);
                        touched_sheets.insert(*sheet_id);
                        workbook_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::SetDefinedName { defined_name } => {
                    if let Some(existing) = defined_names.iter_mut().find(|candidate| {
                        candidate.scope() == defined_name.scope()
                            && candidate.lookup_key() == defined_name.lookup_key()
                    }) {
                        if existing != defined_name {
                            *existing = defined_name.clone();
                            workbook_changed = true;
                            topology_changed = true;
                            semantic_changed = true;
                        }
                    } else {
                        defined_names.push(defined_name.clone());
                        workbook_changed = true;
                        topology_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::RemoveDefinedName { scope, name } => {
                    let lookup = case_insensitive_key(name);
                    let previous_len = defined_names.len();
                    defined_names.retain(|candidate| {
                        candidate.scope() != *scope || candidate.lookup_key() != lookup.as_str()
                    });
                    if defined_names.len() != previous_len {
                        workbook_changed = true;
                        topology_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::RenameDefinedName {
                    scope,
                    name,
                    new_name,
                } => {
                    if let Some(rename) = rename_defined_name(
                        &mut sheets,
                        &mut defined_names,
                        &non_worksheet_tabs,
                        WorkbookFormulaEdit {
                            state: FormulaEditState {
                                mutations: &mut cell_mutations,
                                changed_cells: &mut changed_cells,
                                calculation_changed_cells: &mut calculation_changed_cells,
                                touched_sheets: &mut touched_sheets,
                            },
                            changed_table_ids: &mut changed_table_ids,
                            table_formula_locations: &table_formula_locations,
                            budget: rewrite_budget,
                        },
                        *scope,
                        name,
                        new_name,
                    )? {
                        name_renames.push(rename);
                        inverse_seed.steps.push(UndoStep::RenameDefinedName {
                            scope: *scope,
                            name: name.clone(),
                            new_name: new_name.clone(),
                        });
                        workbook_changed = true;
                        topology_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::SetDateSystem { date_system: next } => {
                    if date_system != *next {
                        date_system = *next;
                        workbook_changed = true;
                        calculation_metadata_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::SetCalculationHints {
                    calculation_hints: next,
                } => {
                    if calculation_hints != *next {
                        calculation_hints = *next;
                        workbook_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::RenameTable {
                    table_id,
                    new_display_name,
                } => {
                    if rename_table(
                        &mut sheets,
                        &mut defined_names,
                        TableFormulaEdit {
                            state: TableEditState {
                                mutations: &mut cell_mutations,
                                changed_cells: &mut changed_cells,
                                calculation_changed_cells: &mut calculation_changed_cells,
                                touched_sheets: &mut touched_sheets,
                                changed_table_ids: &mut changed_table_ids,
                                presentation: &presentation,
                                source: SourceCells::new(source_workbook, &grid_histories),
                            },
                            locations: &table_locations,
                            formula_locations: &table_formula_locations,
                            budget: rewrite_budget,
                        },
                        *table_id,
                        new_display_name,
                    )? {
                        workbook_changed = true;
                        topology_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::RenameTableColumn {
                    table_id,
                    column_id,
                    new_name,
                } => {
                    if rename_table_column(
                        &mut sheets,
                        &mut defined_names,
                        TableFormulaEdit {
                            state: TableEditState {
                                mutations: &mut cell_mutations,
                                changed_cells: &mut changed_cells,
                                calculation_changed_cells: &mut calculation_changed_cells,
                                touched_sheets: &mut touched_sheets,
                                changed_table_ids: &mut changed_table_ids,
                                presentation: &presentation,
                                source: SourceCells::new(source_workbook, &grid_histories),
                            },
                            locations: &table_locations,
                            formula_locations: &table_formula_locations,
                            budget: rewrite_budget,
                        },
                        *table_id,
                        *column_id,
                        new_name,
                    )? {
                        workbook_changed = true;
                        topology_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::ResizeTableRows {
                    table_id,
                    first_data_row,
                    last_data_row,
                } => {
                    if resize_table_rows(
                        &mut sheets,
                        TableResizeEdit {
                            state: TableEditState {
                                mutations: &mut cell_mutations,
                                changed_cells: &mut changed_cells,
                                calculation_changed_cells: &mut calculation_changed_cells,
                                touched_sheets: &mut touched_sheets,
                                changed_table_ids: &mut changed_table_ids,
                                presentation: &presentation,
                                source: SourceCells::new(source_workbook, &grid_histories),
                            },
                            locations: &table_locations,
                            rewrite_budget,
                            materialization_budget,
                        },
                        *table_id,
                        *first_data_row,
                        *last_data_row,
                    )? {
                        topology_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::CreateTable {
                    sheet_id,
                    range,
                    name,
                } => {
                    let table_id = create_table(
                        &mut sheets,
                        &defined_names,
                        TableEditState {
                            mutations: &mut cell_mutations,
                            changed_cells: &mut changed_cells,
                            calculation_changed_cells: &mut calculation_changed_cells,
                            touched_sheets: &mut touched_sheets,
                            changed_table_ids: &mut changed_table_ids,
                            presentation: &presentation,
                            source: SourceCells::new(source_workbook, &grid_histories),
                        },
                        *sheet_id,
                        *range,
                        name.as_ref(),
                    )?;
                    inverse_seed.steps.push(UndoStep::CreateTable { table_id });
                    table_locations = super::table_edit::table_locations(&sheets)?;
                    table_formula_locations =
                        super::formula_edit::table_formula_locations(&sheets, rewrite_budget)?;
                    workbook_changed = true;
                    topology_changed = true;
                    semantic_changed = true;
                }
                WorkbookChange::ConvertTableToRange { table_id } => {
                    let undo = sheets.iter().find_map(|sheet| {
                        sheet
                            .tables()
                            .iter()
                            .find(|table| table.id() == *table_id)
                            .and_then(|table| TableShape::of(sheet.id(), table))
                    });
                    convert_table_to_range(
                        &mut sheets,
                        &mut defined_names,
                        TableFormulaEdit {
                            state: TableEditState {
                                mutations: &mut cell_mutations,
                                changed_cells: &mut changed_cells,
                                calculation_changed_cells: &mut calculation_changed_cells,
                                touched_sheets: &mut touched_sheets,
                                changed_table_ids: &mut changed_table_ids,
                                presentation: &presentation,
                                source: SourceCells::new(source_workbook, &grid_histories),
                            },
                            locations: &table_locations,
                            formula_locations: &table_formula_locations,
                            budget: rewrite_budget,
                        },
                        *table_id,
                    )?;
                    inverse_seed.steps.extend(undo.map(UndoStep::ConvertTable));
                    previous_changed_table_ids.remove(table_id);
                    table_locations = super::table_edit::table_locations(&sheets)?;
                    table_formula_locations =
                        super::formula_edit::table_formula_locations(&sheets, rewrite_budget)?;
                    workbook_changed = true;
                    topology_changed = true;
                    semantic_changed = true;
                }
                WorkbookChange::SetTableRange { table_id, range } => {
                    let previous = table_range(&sheets, *table_id);
                    if set_table_range(
                        &mut sheets,
                        &mut defined_names,
                        TableResizeEdit {
                            state: TableEditState {
                                mutations: &mut cell_mutations,
                                changed_cells: &mut changed_cells,
                                calculation_changed_cells: &mut calculation_changed_cells,
                                touched_sheets: &mut touched_sheets,
                                changed_table_ids: &mut changed_table_ids,
                                presentation: &presentation,
                                source: SourceCells::new(source_workbook, &grid_histories),
                            },
                            locations: &table_locations,
                            rewrite_budget,
                            materialization_budget,
                        },
                        &table_formula_locations,
                        *table_id,
                        *range,
                    )? {
                        inverse_seed
                            .steps
                            .extend(previous.map(|range| UndoStep::TableRange {
                                table_id: *table_id,
                                range,
                            }));
                        table_formula_locations =
                            super::formula_edit::table_formula_locations(&sheets, rewrite_budget)?;
                        workbook_changed = true;
                        topology_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::WidenTableColumns { table_id, range } => {
                    let previous = table_range(&sheets, *table_id);
                    if widen_table_columns(&mut sheets, &table_locations, *table_id, *range)? {
                        inverse_seed
                            .steps
                            .extend(previous.map(|range| UndoStep::TableRange {
                                table_id: *table_id,
                                range,
                            }));
                        changed_table_ids.insert(*table_id);
                        topology_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::WidenSheetAutoFilter { sheet_id, range } => {
                    let sheet = sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    let previous = sheet.auto_filter().map(SheetAutoFilter::range);
                    if widen_sheet_auto_filter(sheet, *range)? {
                        inverse_seed.steps.push(UndoStep::AutoFilter {
                            sheet_id: *sheet_id,
                            range: previous,
                        });
                        touched_sheets.insert(*sheet_id);
                        semantic_changed = true;
                        if align_filter_database_name(sheet, &mut defined_names)? {
                            workbook_changed = true;
                            topology_changed = true;
                        }
                    }
                }
                WorkbookChange::SetSheetAutoFilter { sheet_id, range } => {
                    let sheet = sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    let previous = sheet.auto_filter().map(SheetAutoFilter::range);
                    let edit = set_sheet_auto_filter(sheet, &mut defined_names, *range)?;
                    if edit.filter_changed {
                        inverse_seed.steps.push(UndoStep::AutoFilter {
                            sheet_id: *sheet_id,
                            range: previous,
                        });
                        touched_sheets.insert(*sheet_id);
                        semantic_changed = true;
                    }
                    if edit.names_changed {
                        workbook_changed = true;
                        topology_changed = true;
                    }
                }
                WorkbookChange::InsertRows { .. }
                | WorkbookChange::DeleteRows { .. }
                | WorkbookChange::InsertColumns { .. }
                | WorkbookChange::DeleteColumns { .. }
                | WorkbookChange::MoveRows { .. }
                | WorkbookChange::MoveColumns { .. }
                | WorkbookChange::MoveRange { .. } => {
                    let (sheet_id, edit) =
                        grid_change(change)?.expect("a structural change has a grid edit");
                    // A copy of a source sheet records its edits too, which its saved part
                    // cannot take.
                    let records_history = (source_workbook
                        .is_some_and(|workbook| workbook.sheet_by_id(sheet_id).is_some())
                        || copied_sheets.contains_key(&sheet_id))
                        && !added_sheets.contains(&sheet_id);
                    inverse_seed.steps.push(UndoStep::Grid { sheet_id, edit });
                    if records_history && moves_preserved_content(edit) {
                        inverse_seed.preserved_sheets.insert(sheet_id);
                    }
                    remove_deleted_table_columns(
                        &mut sheets,
                        &mut defined_names,
                        &non_worksheet_tabs,
                        WorkbookFormulaEdit {
                            state: FormulaEditState {
                                mutations: &mut cell_mutations,
                                changed_cells: &mut changed_cells,
                                calculation_changed_cells: &mut calculation_changed_cells,
                                touched_sheets: &mut touched_sheets,
                            },
                            changed_table_ids: &mut changed_table_ids,
                            table_formula_locations: &table_formula_locations,
                            budget: rewrite_budget,
                        },
                        sheet_id,
                        edit,
                    )?;
                    let outcome = apply_grid_edit(
                        GridEditState {
                            sheets: &mut sheets,
                            defined_names: &mut defined_names,
                            presentation: &mut presentation,
                            mutations: &mut cell_mutations,
                            presentation_cell_mutations: &mut presentation_cell_mutations,
                            presentation_sheet_mutations: &mut presentation_sheet_mutations,
                            changed_cells: &mut changed_cells,
                            calculation_changed_cells: &mut calculation_changed_cells,
                            touched_sheets: &mut touched_sheets,
                            changed_table_ids: &mut changed_table_ids,
                            history: records_history
                                .then(|| grid_histories.entry(sheet_id).or_default()),
                            budget: rewrite_budget,
                            materialization: materialization_budget,
                        },
                        sheet_id,
                        edit,
                    )?;
                    table_locations = super::table_edit::table_locations(&sheets)?;
                    table_formula_locations =
                        super::formula_edit::table_formula_locations(&sheets, rewrite_budget)?;
                    workbook_changed |= outcome.names_changed;
                    topology_changed = true;
                    semantic_changed = true;
                }
                WorkbookChange::CopyRange {
                    sheet_id,
                    source,
                    destination_sheet_id,
                    destination,
                } => {
                    cell_formulas_changed |= copy_range(
                        CopyEditState {
                            sheets: &mut sheets,
                            presentation: &mut presentation,
                            mutations: &mut cell_mutations,
                            presentation_cell_mutations: &mut presentation_cell_mutations,
                            changed_cells: &mut changed_cells,
                            calculation_changed_cells: &mut calculation_changed_cells,
                            touched_sheets: &mut touched_sheets,
                            source: SourceCells::new(source_workbook, &grid_histories),
                            budget: rewrite_budget,
                        },
                        *sheet_id,
                        *source,
                        *destination_sheet_id,
                        *destination,
                    )?;
                    semantic_changed = true;
                }
                WorkbookChange::SetCellFormat {
                    sheet_id,
                    address,
                    edit,
                } => {
                    let sheet = sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    let current = effective_cell_format(
                        &presentation,
                        sheet,
                        SourceCells::new(source_workbook, &grid_histories),
                        &cell_mutations,
                        *address,
                    );
                    let edited = edit.apply(&current);
                    if edited != current {
                        presentation.set_cell_format(*sheet_id, *address, edited)?;
                    }
                }
                WorkbookChange::SetColumnWidth {
                    sheet_id,
                    first,
                    last,
                    width,
                } => {
                    sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    presentation.set_column_width(*sheet_id, *first, *last, *width)?;
                }
                WorkbookChange::SetRowHeight {
                    sheet_id,
                    row,
                    height,
                } => {
                    sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    presentation.set_row_height(*sheet_id, *row, *height)?;
                }
                WorkbookChange::SetFrozenPane { sheet_id, pane } => {
                    sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    let pane = (!pane.is_clear()).then_some(*pane);
                    if presentation.set_frozen_pane(*sheet_id, pane)? {
                        presentation_sheet_mutations.insert(*sheet_id);
                    }
                }
                WorkbookChange::SetPhonetics {
                    sheet_id,
                    address,
                    runs,
                    options,
                } => {
                    let sheet = sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    let Some(CellContent::Literal(CellValue::Text(text))) =
                        sheet.cell(*address).map(crate::Cell::content)
                    else {
                        return Err(phonetics_require_text_cell(*sheet_id, *address).into());
                    };
                    let annotation = authored_annotation(text, runs.clone(), options)?;
                    if presentation.set_cell_phonetics(
                        *sheet_id,
                        *address,
                        annotation,
                        options.visible(),
                    )? {
                        presentation_cell_mutations
                            .insert(CalculationCellId::new(*sheet_id, *address));
                    }
                }
                WorkbookChange::ClearPhonetics { sheet_id, address } => {
                    sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    if presentation.clear_cell_phonetics(*sheet_id, *address)? {
                        presentation_cell_mutations
                            .insert(CalculationCellId::new(*sheet_id, *address));
                    }
                }
            }
        }

        for sheet in &sheets {
            if sheet
                .tables()
                .iter()
                .any(|table| changed_table_ids.contains(&table.id()))
            {
                touched_sheets.insert(sheet.id());
            }
        }
        for sheet_id in &touched_sheets {
            sheet_by_id_mut(&mut sheets, *sheet_id)?.finish_deferred_cell_edits();
        }
        let result_revision = if semantic_changed {
            next_revision(base_revision)?
        } else {
            base_revision
        };
        let diagnostics = clone_slice(self.workbook.diagnostics(), rewrite_budget)?;
        let workbook = WorkbookSnapshot::new_with_metadata_cancellable_from_previous(
            WorkbookSnapshotInput {
                sheets,
                non_worksheet_tabs,
                defined_names,
                diagnostics,
                date_system,
                calculation_hints,
                source: self.workbook.source(),
                provenance: self.workbook.provenance().clone(),
            },
            self.workbook.as_ref(),
            &touched_sheets,
            &|| rewrite_budget.check_cancelled().is_err(),
        )
        .map_err(|error| match error {
            WorkbookBuildError::Validation(error) => BatchExecutionError::Validation(error),
            WorkbookBuildError::Cancelled => {
                BatchExecutionError::Rewrite(FormulaRewriteError::Cancelled)
            }
        })?
        .with_highest_sheet_id(highest_sheet_id)
        .with_semantic_revision(result_revision);

        let next = Self {
            workbook: std::sync::Arc::new(workbook),
            presentation,
            source_document: self.source_document.clone(),
            cell_mutations,
            presentation_cell_mutations,
            presentation_sheet_mutations,
            added_sheets,
            changed_table_ids: previous_changed_table_ids
                .union(&changed_table_ids)
                .copied()
                .collect(),
            workbook_changed,
            tab_steps,
            name_renames,
            copied_sheets,
            copied_tables,
            grid_histories,
        };
        // The inverse is built before the batch is installed, so a cancelled build leaves the
        // draft unchanged.
        let inverse = if with_inverse {
            edit_inverse(self, &next, inverse_seed, rewrite_budget.cancellation())?.map(Box::new)
        } else {
            None
        };
        *self = next;

        Ok(EditReceipt {
            base_revision,
            result_revision,
            applied_change_count: batch.len(),
            changed_cells: changed_cells.into_iter().collect(),
            calculation_changed_cells: calculation_changed_cells.into_iter().collect(),
            created_sheet_ids,
            changed_table_ids: changed_table_ids.into_iter().collect(),
            topology_changed: topology_changed || cell_formulas_changed,
            cell_formulas_changed_topology_only: !topology_changed,
            calculation_metadata_changed,
            number_formats_changed,
            inverse,
        })
    }
}

fn clone_sheets(
    source: &[Sheet],
    budget: &FormulaRewriteBudget<'_>,
) -> Result<Vec<Sheet>, FormulaRewriteError> {
    let mut cloned = Vec::with_capacity(source.len());
    for sheet in source {
        budget.check_cancelled()?;
        cloned.push(
            sheet
                .clone_cancellable(&|| budget.check_cancelled().is_err())
                .map_err(|()| FormulaRewriteError::Cancelled)?,
        );
    }
    Ok(cloned)
}

fn clone_slice<T: Clone>(
    source: &[T],
    budget: &FormulaRewriteBudget<'_>,
) -> Result<Vec<T>, FormulaRewriteError> {
    let mut cloned = Vec::with_capacity(source.len());
    for value in source {
        budget.check_cancelled()?;
        cloned.push(value.clone());
    }
    Ok(cloned)
}

fn clone_set<T>(
    source: &BTreeSet<T>,
    budget: &FormulaRewriteBudget<'_>,
) -> Result<BTreeSet<T>, FormulaRewriteError>
where
    T: Clone + Ord,
{
    let mut cloned = BTreeSet::new();
    for value in source {
        budget.check_cancelled()?;
        cloned.insert(value.clone());
    }
    Ok(cloned)
}

/// Returns the zero-based tab position of a worksheet among every tab.
fn tab_position(
    sheets: &[Sheet],
    non_worksheet_tabs: &NonWorksheetTabs,
    sheet_id: SheetId,
) -> usize {
    TabOrder::of(sheets, non_worksheet_tabs)
        .entries
        .iter()
        .position(|tab| tab.id == sheet_id)
        .unwrap_or(0)
}

/// Returns the range of a table, when a sheet holds it.
fn table_range(sheets: &[Sheet], table_id: TableId) -> Option<CellRange> {
    sheets
        .iter()
        .flat_map(Sheet::tables)
        .find(|table| table.id() == table_id)
        .map(Table::range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CellAddress, NumberFormat, NumberFormatKind};

    #[test]
    fn number_format_edit_shares_existing_large_cell_content() {
        let mut draft = WorkbookDraft::new();
        let sheet = SheetId::new(1).expect("default sheet");
        let address = CellAddress::from_a1("A1").expect("cell address");
        draft
            .apply_changes(EditBatch::new([WorkbookChange::set_cell_value(
                sheet,
                address,
                CellValue::Text("x".repeat(8_192)),
            )]))
            .expect("large text cell");
        let before = draft.workbook().clone();

        draft
            .apply_changes(EditBatch::new([WorkbookChange::set_cell_number_format(
                sheet,
                address,
                NumberFormat::custom(164, "0.000", NumberFormatKind::Number)
                    .expect("custom format"),
            )]))
            .expect("metadata-only edit");

        let before_cell = before
            .sheet_by_id(sheet)
            .expect("sheet")
            .cell(address)
            .expect("cell");
        let after_cell = draft
            .workbook()
            .sheet_by_id(sheet)
            .expect("sheet")
            .cell(address)
            .expect("cell");
        assert!(before_cell.shares_content_with(after_cell));
        assert!(!before_cell.shares_number_format_with(after_cell));
    }
}
