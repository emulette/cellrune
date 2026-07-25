use std::collections::BTreeSet;

use super::super::{
    DraftCellMutation, WorkbookDraft, annotated_text_replacement_required, case_insensitive_key,
    next_revision,
};
use super::formula_edit::{FormulaEditState, rename_sheet, upsert_formula};
use super::staged::{mark_upsert, sheet_by_id_mut};
use super::{EditBatch, EditReceipt, WorkbookChange};
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
        let base_revision = self.semantic_revision();
        if batch.is_empty() {
            return Ok(EditReceipt {
                base_revision,
                result_revision: base_revision,
                applied_change_count: 0,
                changed_cells: Vec::new(),
                calculation_changed_cells: Vec::new(),
                created_sheet_ids: Vec::new(),
                topology_changed: false,
                calculation_metadata_changed: false,
            });
        }

        let mut sheets = self.workbook.sheets().to_vec();
        let mut defined_names = self.workbook.defined_names().to_vec();
        let mut date_system = self.workbook.date_system();
        let mut calculation_hints = self.workbook.calculation_hints();
        let mut cell_mutations = self.cell_mutations.clone();
        let mut presentation = self.presentation.clone();
        let mut presentation_cell_mutations = self.presentation_cell_mutations.clone();
        let mut added_sheets = self.added_sheets.clone();
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
                                ));
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
                        return Err(annotated_text_replacement_required(*sheet_id, *address));
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
                        return Err(annotated_text_replacement_required(*sheet_id, *address));
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
                        });
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
                        &mut cell_mutations,
                        &mut changed_cells,
                        &mut calculation_changed_cells,
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
                            return Err(ValidationError::LastVisibleSheet);
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
        let workbook = WorkbookSnapshot::new_with_metadata(
            sheets,
            defined_names,
            self.workbook.diagnostics().to_vec(),
            date_system,
            calculation_hints,
            self.workbook.source(),
            self.workbook.provenance().clone(),
        )?
        .with_semantic_revision(result_revision);

        self.workbook = workbook;
        self.cell_mutations = cell_mutations;
        self.presentation = presentation;
        self.presentation_cell_mutations = presentation_cell_mutations;
        self.added_sheets = added_sheets;
        self.workbook_changed = workbook_changed;

        Ok(EditReceipt {
            base_revision,
            result_revision,
            applied_change_count: batch.len(),
            changed_cells: changed_cells.into_iter().collect(),
            calculation_changed_cells: calculation_changed_cells.into_iter().collect(),
            created_sheet_ids,
            topology_changed,
            calculation_metadata_changed,
        })
    }
}
