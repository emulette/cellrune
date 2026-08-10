use std::collections::BTreeSet;

use super::super::{
    DraftCellMutation, WorkbookDraft, annotated_text_replacement_required, case_insensitive_key,
    next_revision,
};
use super::formula_edit::{
    FormulaEditState, WorkbookFormulaEdit, rename_sheet, table_formula_locations, upsert_formula,
};
use super::staged::{mark_upsert, sheet_by_id_mut};
use super::table_edit::{
    TableEditState, TableFormulaEdit, TableResizeEdit, rename_table, rename_table_column,
    resize_table_rows, table_locations,
};
use super::{
    BatchExecutionError, EditBatch, EditReceipt, TableMaterializationBudget, WorkbookChange,
};
use crate::calculation::formula_rewrite::{
    FormulaRewriteBudget, FormulaRewriteError, FormulaRewriteLimits,
};
use crate::workbook::{WorkbookBuildError, WorkbookSnapshotInput};
use crate::{
    CalculationCellId, CellContent, CellValue, FormulaCell, FormulaDialect, FormulaMetadata,
    NumberFormat, SavedResult, Sheet, SheetId, SheetVisibility, ValidationError, WorkbookSnapshot,
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
                calculation_metadata_changed: false,
            });
        }

        rewrite_budget.check_cancelled()?;
        let mut sheets = clone_sheets(self.workbook.sheets(), rewrite_budget)?;
        let table_locations = table_locations(&sheets)?;
        let table_formula_locations = table_formula_locations(&sheets, rewrite_budget)?;
        let mut defined_names = clone_slice(self.workbook.defined_names(), rewrite_budget)?;
        let mut date_system = self.workbook.date_system();
        let mut calculation_hints = self.workbook.calculation_hints();
        let mut cell_mutations = self.cell_mutations.clone();
        let mut presentation = self
            .presentation
            .clone_cancellable(&|| rewrite_budget.check_cancelled().is_err())
            .map_err(|()| FormulaRewriteError::Cancelled)?;
        let mut presentation_cell_mutations =
            clone_set(&self.presentation_cell_mutations, rewrite_budget)?;
        let mut added_sheets = clone_set(&self.added_sheets, rewrite_budget)?;
        let previous_changed_table_ids = clone_set(&self.changed_table_ids, rewrite_budget)?;
        let mut changed_table_ids = BTreeSet::new();
        let mut workbook_changed = self.workbook_changed;
        let mut changed_cells = BTreeSet::new();
        let mut calculation_changed_cells = BTreeSet::new();
        let mut touched_sheets = BTreeSet::new();
        let mut created_sheet_ids = Vec::new();
        let mut topology_changed = false;
        let mut calculation_metadata_changed = false;
        let mut semantic_changed = false;

        for change in batch.changes() {
            match change {
                WorkbookChange::SetCellValue {
                    sheet_id,
                    address,
                    value,
                } => {
                    let sheet = sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    let previous = sheet.cell(*address).cloned();
                    let previous_was_formula = previous
                        .as_ref()
                        .is_some_and(|cell| matches!(cell.content(), CellContent::Formula(_)));
                    if presentation.has_cell_annotation(*sheet_id, *address) {
                        if *value == CellValue::Blank {
                            if presentation.clear_cell_phonetics(*sheet_id, *address)? {
                                presentation_cell_mutations
                                    .insert(CalculationCellId::new(*sheet_id, *address));
                            }
                        } else {
                            let replacement = CellContent::Literal(value.clone());
                            if previous
                                .as_ref()
                                .is_none_or(|cell| cell.content() != &replacement)
                            {
                                return Err(annotated_text_replacement_required(
                                    *sheet_id, *address,
                                )
                                .into());
                            }
                        }
                    }
                    if *value == CellValue::Blank {
                        if sheet.remove_cell_deferred(*address) {
                            cell_mutations.insert(
                                CalculationCellId::new(*sheet_id, *address),
                                DraftCellMutation::Remove,
                            );
                            changed_cells.insert(CalculationCellId::new(*sheet_id, *address));
                            calculation_changed_cells
                                .insert(CalculationCellId::new(*sheet_id, *address));
                            touched_sheets.insert(*sheet_id);
                            semantic_changed = true;
                        }
                    } else {
                        let number_format = previous
                            .as_ref()
                            .map_or_else(NumberFormat::default, |cell| {
                                cell.number_format().clone()
                            });
                        let content = CellContent::Literal(value.clone());
                        if previous
                            .as_ref()
                            .is_none_or(|cell| cell.content() != &content)
                        {
                            sheet.upsert_cell_deferred(*address, content, number_format);
                            mark_upsert(&mut cell_mutations, *sheet_id, *address, false);
                            changed_cells.insert(CalculationCellId::new(*sheet_id, *address));
                            calculation_changed_cells
                                .insert(CalculationCellId::new(*sheet_id, *address));
                            touched_sheets.insert(*sheet_id);
                            semantic_changed = true;
                        }
                    }
                    topology_changed |= previous_was_formula;
                }
                WorkbookChange::SetCellFormula {
                    sheet_id,
                    address,
                    formula,
                } => {
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
                        topology_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::SetCellDynamicFormula {
                    sheet_id,
                    address,
                    formula,
                    range,
                } => {
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
                        topology_changed = true;
                        semantic_changed = true;
                    }
                }
                WorkbookChange::ClearCell { sheet_id, address } => {
                    let sheet = sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    if presentation.clear_cell_phonetics(*sheet_id, *address)? {
                        presentation_cell_mutations
                            .insert(CalculationCellId::new(*sheet_id, *address));
                    }
                    let previous_was_formula = sheet
                        .cell(*address)
                        .is_some_and(|cell| matches!(cell.content(), CellContent::Formula(_)));
                    if sheet.remove_cell_deferred(*address) {
                        cell_mutations.insert(
                            CalculationCellId::new(*sheet_id, *address),
                            DraftCellMutation::Remove,
                        );
                        changed_cells.insert(CalculationCellId::new(*sheet_id, *address));
                        calculation_changed_cells
                            .insert(CalculationCellId::new(*sheet_id, *address));
                        touched_sheets.insert(*sheet_id);
                        semantic_changed = true;
                    }
                    topology_changed |= previous_was_formula;
                }
                WorkbookChange::SetCellNumberFormat {
                    sheet_id,
                    address,
                    number_format,
                } => {
                    let sheet = sheet_by_id_mut(&mut sheets, *sheet_id)?;
                    let cell =
                        sheet
                            .cell(*address)
                            .cloned()
                            .ok_or(ValidationError::CellNotFound {
                                sheet_id: sheet_id.get(),
                                row: address.row().get(),
                                column: address.column().get(),
                            })?;
                    if cell.number_format() != number_format {
                        sheet.upsert_cell_deferred(
                            *address,
                            cell.content().clone(),
                            number_format.clone(),
                        );
                        mark_upsert(&mut cell_mutations, *sheet_id, *address, true);
                        changed_cells.insert(CalculationCellId::new(*sheet_id, *address));
                        touched_sheets.insert(*sheet_id);
                        semantic_changed = true;
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
                    let maximum = sheets
                        .iter()
                        .map(|sheet| sheet.id().get())
                        .max()
                        .unwrap_or(0);
                    let next = maximum
                        .checked_add(1)
                        .ok_or(ValidationError::SheetIdExhausted)?;
                    let sheet_id =
                        SheetId::new(next).map_err(|_| ValidationError::SheetIdExhausted)?;
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
                        workbook_changed = true;
                        topology_changed = true;
                        semantic_changed = true;
                    }
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
            }
        }

        for sheet_id in touched_sheets {
            sheet_by_id_mut(&mut sheets, sheet_id)?.finish_deferred_cell_edits();
        }
        let result_revision = if semantic_changed {
            next_revision(base_revision)?
        } else {
            base_revision
        };
        let diagnostics = clone_slice(self.workbook.diagnostics(), rewrite_budget)?;
        let workbook = WorkbookSnapshot::new_with_metadata_cancellable(
            WorkbookSnapshotInput {
                sheets,
                defined_names,
                diagnostics,
                date_system,
                calculation_hints,
                source: self.workbook.source(),
                provenance: self.workbook.provenance().clone(),
            },
            &|| rewrite_budget.check_cancelled().is_err(),
        )
        .map_err(|error| match error {
            WorkbookBuildError::Validation(error) => BatchExecutionError::Validation(error),
            WorkbookBuildError::Cancelled => {
                BatchExecutionError::Rewrite(FormulaRewriteError::Cancelled)
            }
        })?
        .with_semantic_revision(result_revision);

        self.workbook = std::sync::Arc::new(workbook);
        self.cell_mutations = cell_mutations;
        self.presentation = presentation;
        self.presentation_cell_mutations = presentation_cell_mutations;
        self.added_sheets = added_sheets;
        self.changed_table_ids = previous_changed_table_ids
            .union(&changed_table_ids)
            .copied()
            .collect();
        self.workbook_changed = workbook_changed;

        Ok(EditReceipt {
            base_revision,
            result_revision,
            applied_change_count: batch.len(),
            changed_cells: changed_cells.into_iter().collect(),
            calculation_changed_cells: calculation_changed_cells.into_iter().collect(),
            created_sheet_ids,
            changed_table_ids: changed_table_ids.into_iter().collect(),
            topology_changed,
            calculation_metadata_changed,
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
