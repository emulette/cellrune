//! Copying and filling ranges: each copied formula's relative references move by the distance
//! it is copied.

use std::collections::BTreeSet;

use super::super::format::effective_cell_format;
use super::super::{DraftCellMutationStore, SourceCells, ensure_not_array_formula_part};
use super::BatchExecutionError;
use super::staged::{mark_removed, mark_upsert, sheet_by_id_mut};
use super::table_edit::table_header_column;
use crate::calculation::formula_rewrite::{
    FormulaRewriteBudget, FormulaRewriteRequest, rewrite_formula,
};
use crate::calculation::grid_rewrite::{CopyOffset, contains_spill_reference};
use crate::{
    CalculationCellId, Cell, CellAddress, CellContent, CellFormat, CellRange, DocumentPresentation,
    FormulaCell, FormulaMetadata, FormulaText, SavedResult, Sheet, SheetId, ValidationError,
};

/// The staged draft state a copy changes.
pub(super) struct CopyEditState<'a, 'cancel> {
    pub(super) sheets: &'a mut [Sheet],
    pub(super) presentation: &'a mut DocumentPresentation,
    pub(super) mutations: &'a mut DraftCellMutationStore,
    pub(super) presentation_cell_mutations: &'a mut BTreeSet<CalculationCellId>,
    pub(super) changed_cells: &'a mut BTreeSet<CalculationCellId>,
    pub(super) calculation_changed_cells: &'a mut BTreeSet<CalculationCellId>,
    pub(super) touched_sheets: &'a mut BTreeSet<SheetId>,
    pub(super) source: SourceCells<'a>,
    pub(super) budget: &'a mut FormulaRewriteBudget<'cancel>,
}

/// What one source position holds: its cell, if any, and the format it displays.
struct Copied {
    offset: (u32, u32),
    cell: Option<Cell>,
    format: CellFormat,
}

/// Copies the cells of `source` on `sheet_id` into `destination` on `destination_sheet_id`,
/// repeating them over a destination that is a whole multiple of their size. Values, formulas,
/// number formats, and cell formats are copied, and an empty source position clears its
/// destination. Returns whether a cell gained, lost, or replaced a formula.
pub(super) fn copy_range(
    state: CopyEditState<'_, '_>,
    sheet_id: SheetId,
    source: CellRange,
    destination_sheet_id: SheetId,
    destination: CellRange,
) -> Result<bool, BatchExecutionError> {
    let CopyEditState {
        sheets,
        presentation,
        mutations,
        presentation_cell_mutations,
        changed_cells,
        calculation_changed_cells,
        touched_sheets,
        source: source_cells,
        budget,
    } = state;
    super::change::validate_copy_shape(source, destination)?;
    let sheet = sheet_by_id_mut(sheets, sheet_id)?;
    let mut copied = Vec::new();
    for row in source.start().row().get()..=source.end().row().get() {
        for column in source.start().column().get()..=source.end().column().get() {
            budget.check_cancelled()?;
            let address = CellAddress::from_indices(row, column)?;
            let cell = sheet.cell(address).cloned();
            if let Some(CellContent::Formula(formula)) = cell.as_ref().map(Cell::content) {
                let unsupported = matches!(
                    formula.metadata(),
                    FormulaMetadata::Array { .. } | FormulaMetadata::DataTable { .. }
                ) || formula
                    .text()
                    .is_none_or(|text| contains_spill_reference(text.as_str()));
                if unsupported {
                    return Err(ValidationError::CopyUnsupportedFormula {
                        sheet_id: sheet_id.get(),
                        row,
                        column,
                    }
                    .into());
                }
            }
            copied.push(Copied {
                offset: (
                    row - source.start().row().get(),
                    column - source.start().column().get(),
                ),
                cell,
                format: effective_cell_format(
                    presentation,
                    sheet,
                    source_cells,
                    mutations,
                    address,
                ),
            });
        }
    }

    let mut formulas_changed = false;
    for tile_row in 0..destination.height() / source.height() {
        for tile_column in 0..destination.width() / source.width() {
            let top = destination.start().row().get() + tile_row * source.height();
            let left = destination.start().column().get() + tile_column * source.width();
            let offset = CopyOffset {
                rows: i64::from(top) - i64::from(source.start().row().get()),
                columns: i64::from(left) - i64::from(source.start().column().get()),
            };
            for item in &copied {
                budget.check_cancelled()?;
                let target = CellAddress::from_indices(top + item.offset.0, left + item.offset.1)?;
                formulas_changed |= paste(
                    PasteState {
                        sheet: sheet_by_id_mut(sheets, destination_sheet_id)?,
                        presentation,
                        mutations,
                        presentation_cell_mutations,
                        source: source_cells,
                        budget,
                    },
                    item,
                    target,
                    offset,
                )?;
                let id = CalculationCellId::new(destination_sheet_id, target);
                changed_cells.insert(id);
                calculation_changed_cells.insert(id);
            }
        }
    }
    touched_sheets.insert(destination_sheet_id);
    Ok(formulas_changed)
}

struct PasteState<'a, 'b, 'cancel> {
    sheet: &'a mut Sheet,
    presentation: &'a mut DocumentPresentation,
    mutations: &'a mut DraftCellMutationStore,
    presentation_cell_mutations: &'a mut BTreeSet<CalculationCellId>,
    source: SourceCells<'b>,
    budget: &'a mut FormulaRewriteBudget<'cancel>,
}

/// Writes one copied position at `target`, refusing a target inside a legacy array formula or
/// on a table's header row. Returns whether the target gained, lost, or replaced a formula.
fn paste(
    state: PasteState<'_, '_, '_>,
    item: &Copied,
    target: CellAddress,
    offset: CopyOffset,
) -> Result<bool, BatchExecutionError> {
    let PasteState {
        sheet,
        presentation,
        mutations,
        presentation_cell_mutations,
        source,
        budget,
    } = state;
    let sheet_id = sheet.id();
    ensure_not_array_formula_part(sheet, target)?;
    if let Some((table_id, _)) = table_header_column(sheet, target) {
        return Err(ValidationError::TableHeaderShift {
            table_id: table_id.get(),
        }
        .into());
    }
    for follower in sheet.detach_shared_formula_followers(target) {
        mark_upsert(mutations, sheet_id, follower, false);
    }
    if presentation.clear_cell_phonetics(sheet_id, target)? {
        presentation_cell_mutations.insert(CalculationCellId::new(sheet_id, target));
    }
    let previous_was_formula = sheet
        .cell(target)
        .is_some_and(|cell| matches!(cell.content(), CellContent::Formula(_)));
    let format_before = effective_cell_format(presentation, sheet, source, mutations, target);
    let mut wrote_formula = false;
    match item.cell.as_ref() {
        None => {
            if sheet.remove_cell_deferred(target)
                && !mark_removed(mutations, source, sheet_id, target)
            {
                presentation_cell_mutations.remove(&CalculationCellId::new(sheet_id, target));
            }
        }
        Some(cell) => {
            let content = match cell.content() {
                CellContent::Literal(value) => CellContent::Literal(value.clone()),
                CellContent::Formula(formula) => {
                    wrote_formula = true;
                    let text = formula.text().expect("copied formulas hold text");
                    let owner = format!("cell:sheet_id={},address={target}", sheet_id.get());
                    let text = match rewrite_formula(
                        text.as_str(),
                        &FormulaRewriteRequest::Copy(offset),
                        budget,
                    )
                    .map_err(|error| error.with_owner(owner))?
                    {
                        Some(rewritten) => FormulaText::from_xlsx(rewritten)?,
                        None => text.clone(),
                    };
                    let metadata = match formula.metadata() {
                        FormulaMetadata::DynamicArray {
                            always_calculate, ..
                        } => FormulaMetadata::DynamicArray {
                            range: None,
                            always_calculate: *always_calculate,
                        },
                        _ => FormulaMetadata::Normal,
                    };
                    CellContent::Formula(FormulaCell::from_xlsx_parts(
                        formula.dialect(),
                        Some(text),
                        SavedResult::Missing,
                        metadata,
                        formula.recalculate_always(),
                    ))
                }
            };
            let number_format_changed = sheet
                .cell(target)
                .is_none_or(|existing| existing.number_format() != cell.number_format());
            sheet.upsert_cell_deferred(target, content, cell.number_format().clone());
            mark_upsert(mutations, sheet_id, target, number_format_changed);
        }
    }
    if format_before != item.format {
        presentation.set_cell_format(sheet_id, target, item.format.clone())?;
    }
    Ok(previous_was_formula || wrote_formula)
}
