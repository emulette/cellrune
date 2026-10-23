//! The changes that undo a committed batch with structural edits.
//!
//! The engine keeps no undo stack: an app builds undo and redo from these changes. The inverse
//! starts with the structural edits that undo the batch's own, in reverse order, and then the
//! cell, name, table, and layout changes that bring the result back to the workbook the batch
//! started from. What changes cannot express is listed instead of silently lost.

use std::collections::{BTreeMap, BTreeSet};

use super::super::WorkbookDraft;
use super::{BatchExecutionError, EditBatch, TableMaterializationBudget, WorkbookChange};
use crate::calculation::formula_rewrite::{FormulaRewriteBudget, FormulaRewriteLimits};
use crate::grid::{GridAxis, GridEdit};
use crate::{
    CellAddress, CellContent, CellFormatEdit, CellValue, Column, Fill, FormulaMetadata, Row, Sheet,
    SheetId, TableId,
};

/// The changes that undo one committed batch, as far as workbook changes can express them.
///
/// Applied as one batch to the workbook the batch produced, [`Self::changes`] bring back its
/// cells, formulas, number and cell formats, defined names, sheet names and visibility, table
/// names and data rows, row heights, and column widths. [`Self::unrestored`] lists the content
/// they cannot bring back.
#[derive(Debug, Clone, PartialEq)]
pub struct EditInverse {
    changes: Vec<WorkbookChange>,
    unrestored: Vec<UnrestoredContent>,
}

// Inverse changes hold only validated values, whose numbers are finite, so equality is reflexive.
impl Eq for EditInverse {}

impl EditInverse {
    /// Returns the changes that undo the batch, in the order to apply them.
    pub fn changes(&self) -> &[WorkbookChange] {
        &self.changes
    }

    /// Returns the content the changes cannot bring back, in a stable order.
    pub fn unrestored(&self) -> &[UnrestoredContent] {
        &self.unrestored
    }
}

/// One kind of content an inverse cannot bring back, and the sheet or table it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnrestoredContent {
    kind: UnrestoredContentKind,
    sheet_id: Option<SheetId>,
    table_id: Option<TableId>,
}

impl UnrestoredContent {
    const fn sheet(kind: UnrestoredContentKind, sheet_id: SheetId) -> Self {
        Self {
            kind,
            sheet_id: Some(sheet_id),
            table_id: None,
        }
    }

    /// Returns what is not restored.
    pub const fn kind(&self) -> UnrestoredContentKind {
        self.kind
    }

    /// Returns the sheet the content was on, when it belongs to one.
    pub const fn sheet_id(&self) -> Option<SheetId> {
        self.sheet_id
    }

    /// Returns the table the content belongs to, when it does.
    pub const fn table_id(&self) -> Option<TableId> {
        self.table_id
    }
}

/// What an inverse cannot bring back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum UnrestoredContentKind {
    /// Merged ranges of the sheet.
    MergedRanges,
    /// Data validations of the sheet.
    DataValidations,
    /// Hyperlinks of the sheet.
    Hyperlinks,
    /// Comments of the sheet.
    Comments,
    /// Which rows of the sheet are hidden.
    HiddenRows,
    /// The sheet-level auto-filter.
    AutoFilter,
    /// The sheet's frozen pane.
    FrozenPane,
    /// Phonetic annotations and their visibility on the sheet.
    Phonetics,
    /// Cell formats no change expresses, such as gradient fills and row or column styles.
    CellFormats,
    /// Legacy array and data-table formulas of the sheet.
    FormulaContainers,
    /// A table's structure other than its names and data rows.
    Table,
    /// Content of the preserved package the workbook model does not carry, such as conditional
    /// formats, drawings, sparklines, and notes, on cells the batch deleted or moved.
    PreservedContent,
    /// A sheet the batch deleted.
    Sheet,
}

impl UnrestoredContentKind {
    /// Returns the stable snake-case identifier used across bindings.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MergedRanges => "merged_ranges",
            Self::DataValidations => "data_validations",
            Self::Hyperlinks => "hyperlinks",
            Self::Comments => "comments",
            Self::HiddenRows => "hidden_rows",
            Self::AutoFilter => "auto_filter",
            Self::FrozenPane => "frozen_pane",
            Self::Phonetics => "phonetics",
            Self::CellFormats => "cell_formats",
            Self::FormulaContainers => "formula_containers",
            Self::Table => "table",
            Self::PreservedContent => "preserved_content",
            Self::Sheet => "sheet",
        }
    }
}

/// The parts of a batch its inverse is built from.
#[derive(Debug, Default)]
pub(super) struct InverseSeed {
    /// The structural edits of the batch, in order.
    pub(super) edits: Vec<(SheetId, GridEdit)>,
    /// Source sheets whose preserved package content a deletion or range move removed or moved.
    pub(super) preserved_sheets: BTreeSet<SheetId>,
}

/// Builds the inverse of the batch that turned `before` into `after`. Returns `None` when the
/// structural edits that undo the batch's own cannot be applied to its result, as when a later
/// change filled the rows a deletion freed.
pub(super) fn edit_inverse(
    before: &WorkbookDraft,
    after: &WorkbookDraft,
    seed: InverseSeed,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<EditInverse>, BatchExecutionError> {
    let mut restored = after.clone();
    let mut changes = seed
        .edits
        .iter()
        .rev()
        .filter_map(|(sheet_id, edit)| undoing_change(*sheet_id, *edit))
        .collect::<Vec<_>>();
    let mut unrestored = BTreeSet::new();
    if !apply(&mut restored, changes.clone(), cancelled)? {
        return Ok(None);
    }

    // Sheet and table renames rewrite formulas and header cells, and a table resize may write
    // cells, so they apply before the cells are compared.
    let mut renames = Vec::new();
    for sheet in before.workbook().sheets() {
        let Some(current) = restored.workbook().sheet_by_id(sheet.id()) else {
            unrestored.insert(UnrestoredContent::sheet(
                UnrestoredContentKind::Sheet,
                sheet.id(),
            ));
            continue;
        };
        if current.name() != sheet.name() {
            renames.push(WorkbookChange::rename_sheet(
                sheet.id(),
                sheet.name().clone(),
            ));
        }
    }
    renames.extend(table_changes(before, &restored));
    if !apply(&mut restored, renames.clone(), cancelled)? {
        return Ok(None);
    }
    changes.extend(renames);

    let mut cells = Vec::new();
    for sheet in before.workbook().sheets() {
        if let Some(current) = restored.workbook().sheet_by_id(sheet.id()) {
            cell_changes(sheet, current, &mut cells, &mut unrestored);
        }
    }
    if !apply(&mut restored, cells.clone(), cancelled)? {
        return Ok(None);
    }
    changes.extend(cells);

    for sheet in before.workbook().sheets() {
        let Some(current) = restored.workbook().sheet_by_id(sheet.id()) else {
            continue;
        };
        format_changes(
            before,
            &restored,
            sheet,
            current,
            &mut changes,
            &mut unrestored,
        );
        structure_differences(before, &restored, sheet, current, &mut unrestored);
        if current.visibility() != sheet.visibility() {
            changes.push(WorkbookChange::set_sheet_visibility(
                sheet.id(),
                sheet.visibility(),
            ));
        }
    }
    name_changes(before, &restored, &mut changes);
    if before.workbook().date_system() != restored.workbook().date_system() {
        changes.push(WorkbookChange::set_date_system(
            before.workbook().date_system(),
        ));
    }
    if before.workbook().calculation_hints() != restored.workbook().calculation_hints() {
        changes.push(WorkbookChange::set_calculation_hints(
            before.workbook().calculation_hints(),
        ));
    }
    for table in restored.workbook().sheets().iter().flat_map(Sheet::tables) {
        if before
            .workbook()
            .table_by_id(table.id())
            .is_none_or(|original| !original.semantic_eq(table))
        {
            unrestored.insert(UnrestoredContent {
                kind: UnrestoredContentKind::Table,
                sheet_id: None,
                table_id: Some(table.id()),
            });
        }
    }
    for sheet in restored.workbook().sheets() {
        if before.workbook().sheet_by_id(sheet.id()).is_none() {
            changes.push(WorkbookChange::delete_sheet(sheet.id()));
        }
    }
    for sheet_id in seed.preserved_sheets {
        unrestored.insert(UnrestoredContent::sheet(
            UnrestoredContentKind::PreservedContent,
            sheet_id,
        ));
    }
    Ok(Some(EditInverse {
        changes,
        unrestored: unrestored.into_iter().collect(),
    }))
}

/// Applies `changes` to `draft` without building their own inverse. Returns `false` when the
/// workbook refuses them.
fn apply(
    draft: &mut WorkbookDraft,
    changes: Vec<WorkbookChange>,
    cancelled: &dyn Fn() -> bool,
) -> Result<bool, BatchExecutionError> {
    if changes.is_empty() {
        return Ok(true);
    }
    let mut rewrite = FormulaRewriteBudget::new(FormulaRewriteLimits::UNBOUNDED, cancelled);
    let mut materialization = TableMaterializationBudget::new(usize::MAX, cancelled);
    match draft.apply_batch(
        EditBatch::new(changes),
        &mut rewrite,
        &mut materialization,
        false,
    ) {
        Ok(_) => Ok(true),
        Err(BatchExecutionError::Validation(_) | BatchExecutionError::Rewrite(_))
            if !cancelled() =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// Returns the change that undoes one structural edit. A range move has none: moving the cells
/// back would paste over the cells left behind, so the cell changes restore both places.
fn undoing_change(sheet_id: SheetId, edit: GridEdit) -> Option<WorkbookChange> {
    let row = |line: u32| Row::new(line).expect("a validated edit stays on the sheet");
    let column = |line: u32| Column::new(line).expect("a validated edit stays on the sheet");
    Some(match edit.inverse() {
        GridEdit::Insert {
            axis: GridAxis::Rows,
            at,
            count,
        } => WorkbookChange::InsertRows {
            sheet_id,
            row: row(at),
            count,
        },
        GridEdit::Insert {
            axis: GridAxis::Columns,
            at,
            count,
        } => WorkbookChange::InsertColumns {
            sheet_id,
            column: column(at),
            count,
        },
        GridEdit::Delete {
            axis: GridAxis::Rows,
            first,
            last,
        } => WorkbookChange::DeleteRows {
            sheet_id,
            first: row(first),
            last: row(last),
        },
        GridEdit::Delete {
            axis: GridAxis::Columns,
            first,
            last,
        } => WorkbookChange::DeleteColumns {
            sheet_id,
            first: column(first),
            last: column(last),
        },
        GridEdit::Move {
            axis: GridAxis::Rows,
            first,
            last,
            destination,
        } => WorkbookChange::MoveRows {
            sheet_id,
            first: row(first),
            last: row(last),
            destination: row(destination),
        },
        GridEdit::Move {
            axis: GridAxis::Columns,
            first,
            last,
            destination,
        } => WorkbookChange::MoveColumns {
            sheet_id,
            first: column(first),
            last: column(last),
            destination: column(destination),
        },
        GridEdit::MoveRange { .. } => return None,
    })
}

/// Returns the renames and data-row resizes that bring each table back.
fn table_changes(before: &WorkbookDraft, restored: &WorkbookDraft) -> Vec<WorkbookChange> {
    let mut changes = Vec::new();
    for table in before.workbook().sheets().iter().flat_map(Sheet::tables) {
        let Some(current) = restored.workbook().table_by_id(table.id()) else {
            continue;
        };
        if current.semantic_eq(table) {
            continue;
        }
        if current.display_name() != table.display_name() {
            changes.push(WorkbookChange::rename_table(
                table.id(),
                table.display_name().clone(),
            ));
        }
        for column in table.columns() {
            let renamed = current
                .columns()
                .iter()
                .find(|candidate| candidate.column_id() == column.column_id())
                .is_some_and(|candidate| candidate.name() != column.name());
            if renamed && let Ok(name) = crate::TableColumnName::new(column.name()) {
                changes.push(WorkbookChange::rename_table_column(
                    table.id(),
                    column.column_id(),
                    name,
                ));
            }
        }
        if let (Some(data), Some(current_data)) = (table.data_range(), current.data_range())
            && data != current_data
            && let Ok(resize) =
                WorkbookChange::resize_table_rows(table.id(), data.start().row(), data.end().row())
        {
            changes.push(resize);
        }
    }
    changes
}

/// Appends the cell changes that bring `current`'s cells back to `sheet`'s.
fn cell_changes(
    sheet: &Sheet,
    current: &Sheet,
    changes: &mut Vec<WorkbookChange>,
    unrestored: &mut BTreeSet<UnrestoredContent>,
) {
    let sheet_id = sheet.id();
    let addresses = sheet
        .cells()
        .map(crate::Cell::address)
        .chain(current.cells().map(crate::Cell::address))
        .collect::<BTreeSet<_>>();
    for address in addresses {
        let (original, now) = (sheet.cell(address), current.cell(address));
        if same_cell(original, now) {
            continue;
        }
        let Some(original) = original else {
            changes.push(WorkbookChange::clear_cell(sheet_id, address));
            continue;
        };
        let content_change = match original.content() {
            CellContent::Literal(CellValue::Blank) => {
                Some(WorkbookChange::clear_cell(sheet_id, address))
            }
            CellContent::Literal(value) => Some(WorkbookChange::set_cell_value(
                sheet_id,
                address,
                value.clone(),
            )),
            CellContent::Formula(formula) => match (formula.metadata(), formula.text()) {
                (FormulaMetadata::Normal | FormulaMetadata::Shared { .. }, Some(text)) => Some(
                    WorkbookChange::set_cell_formula(sheet_id, address, text.clone()),
                ),
                (FormulaMetadata::DynamicArray { range, .. }, Some(text)) => {
                    WorkbookChange::set_cell_dynamic_formula(
                        sheet_id,
                        address,
                        text.clone(),
                        *range,
                    )
                    .ok()
                }
                _ => None,
            },
        };
        let Some(content_change) = content_change else {
            unrestored.insert(UnrestoredContent::sheet(
                UnrestoredContentKind::FormulaContainers,
                sheet_id,
            ));
            continue;
        };
        let content_differs =
            now.is_none_or(|now| !same_content(original.content(), now.content()));
        if content_differs {
            changes.push(content_change);
        }
        let number_format = now.map_or_else(
            || current.new_cell_number_format(address),
            |now| now.number_format().clone(),
        );
        if !matches!(original.content(), CellContent::Literal(CellValue::Blank))
            && *original.number_format() != number_format
        {
            changes.push(WorkbookChange::set_cell_number_format(
                sheet_id,
                address,
                original.number_format().clone(),
            ));
        }
    }
}

fn same_cell(original: Option<&crate::Cell>, now: Option<&crate::Cell>) -> bool {
    match (original, now) {
        (None, None) => true,
        (Some(original), Some(now)) => {
            same_content(original.content(), now.content())
                && original.number_format() == now.number_format()
        }
        _ => false,
    }
}

/// Compares cell contents as calculation reads them: a shared formula equals an ordinary one
/// with the same text.
fn same_content(original: &CellContent, now: &CellContent) -> bool {
    match (original, now) {
        (CellContent::Formula(original), CellContent::Formula(now)) => {
            let ordinary = |metadata: &FormulaMetadata| {
                matches!(
                    metadata,
                    FormulaMetadata::Normal | FormulaMetadata::Shared { .. }
                )
            };
            original.text() == now.text()
                && (original.metadata() == now.metadata()
                    || (ordinary(original.metadata()) && ordinary(now.metadata())))
        }
        _ => original == now,
    }
}

/// Appends the cell format, row height, and column width changes that bring `current` back to
/// `sheet`.
fn format_changes(
    before: &WorkbookDraft,
    restored: &WorkbookDraft,
    sheet: &Sheet,
    current: &Sheet,
    changes: &mut Vec<WorkbookChange>,
    unrestored: &mut BTreeSet<UnrestoredContent>,
) {
    let sheet_id = sheet.id();
    let positions = sheet
        .cells()
        .map(crate::Cell::address)
        .chain(current.cells().map(crate::Cell::address))
        .chain(sheet.new_cell_styles().styled_positions())
        .chain(current.new_cell_styles().styled_positions())
        .chain(
            before
                .presentation()
                .edited_cell_formats(sheet_id)
                .map(|(address, _)| address),
        )
        .chain(
            restored
                .presentation()
                .edited_cell_formats(sheet_id)
                .map(|(address, _)| address),
        )
        .collect::<BTreeSet<_>>();
    for address in positions {
        let (Ok(original), Ok(now)) = (
            before.cell_format(sheet_id, address),
            restored.cell_format(sheet_id, address),
        ) else {
            continue;
        };
        if original == now {
            continue;
        }
        let Fill::Pattern(fill) = original.fill() else {
            unrestored.insert(UnrestoredContent::sheet(
                UnrestoredContentKind::CellFormats,
                sheet_id,
            ));
            continue;
        };
        changes.push(WorkbookChange::set_cell_format(
            sheet_id,
            address,
            CellFormatEdit::new()
                .with_font(original.font().clone())
                .with_fill(fill)
                .with_border(original.border())
                .with_alignment(original.alignment()),
        ));
    }
    if !sheet
        .new_cell_styles()
        .same_line_styles(current.new_cell_styles())
    {
        unrestored.insert(UnrestoredContent::sheet(
            UnrestoredContentKind::CellFormats,
            sheet_id,
        ));
    }

    let heights = |draft: &WorkbookDraft| {
        draft
            .presentation()
            .row_heights(sheet_id)
            .collect::<BTreeMap<_, _>>()
    };
    let (original_heights, current_heights) = (heights(before), heights(restored));
    for row in original_heights
        .keys()
        .chain(current_heights.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let height = original_heights.get(&row).copied();
        if height != current_heights.get(&row).copied() {
            changes.push(WorkbookChange::set_row_height(sheet_id, row, height));
        }
    }
    let widths = |draft: &WorkbookDraft| {
        draft
            .presentation()
            .column_widths(sheet_id)
            .iter()
            .flat_map(|span| {
                (span.first().get()..=span.last().get()).map(move |column| (column, span.width()))
            })
            .collect::<BTreeMap<_, _>>()
    };
    let (original_widths, current_widths) = (widths(before), widths(restored));
    let mut runs = Vec::<(u32, u32, Option<f64>)>::new();
    for column in original_widths
        .keys()
        .chain(current_widths.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let width = original_widths.get(&column).copied();
        if width == current_widths.get(&column).copied() {
            continue;
        }
        match runs.last_mut() {
            Some((_, last, previous)) if *last + 1 == column && *previous == width => {
                *last = column;
            }
            _ => runs.push((column, column, width)),
        }
    }
    for (first, last, width) in runs {
        if let (Ok(first), Ok(last)) = (Column::new(first), Column::new(last)) {
            changes.push(WorkbookChange::set_column_width(
                sheet_id, first, last, width,
            ));
        }
    }
}

/// Records the sheet structures no change expresses that differ between `sheet` and `current`.
fn structure_differences(
    before: &WorkbookDraft,
    restored: &WorkbookDraft,
    sheet: &Sheet,
    current: &Sheet,
    unrestored: &mut BTreeSet<UnrestoredContent>,
) {
    let sheet_id = sheet.id();
    for (differs, kind) in [
        (
            sheet.merged_ranges() != current.merged_ranges(),
            UnrestoredContentKind::MergedRanges,
        ),
        (
            sheet.data_validations() != current.data_validations(),
            UnrestoredContentKind::DataValidations,
        ),
        (
            sheet.hyperlinks() != current.hyperlinks(),
            UnrestoredContentKind::Hyperlinks,
        ),
        (
            sheet.comments() != current.comments(),
            UnrestoredContentKind::Comments,
        ),
        (
            sheet.hidden_rows() != current.hidden_rows(),
            UnrestoredContentKind::HiddenRows,
        ),
        (
            sheet.auto_filter() != current.auto_filter(),
            UnrestoredContentKind::AutoFilter,
        ),
        (
            before.presentation().frozen_pane(sheet_id)
                != restored.presentation().frozen_pane(sheet_id),
            UnrestoredContentKind::FrozenPane,
        ),
        (
            phonetics(before, sheet_id) != phonetics(restored, sheet_id),
            UnrestoredContentKind::Phonetics,
        ),
    ] {
        if differs {
            unrestored.insert(UnrestoredContent::sheet(kind, sheet_id));
        }
    }
}

type SheetPhonetics<'a> = Option<(
    &'a BTreeMap<CellAddress, crate::CellPresentation>,
    &'a BTreeMap<Row, bool>,
    &'a [crate::ColumnPhoneticVisibility],
)>;

fn phonetics(draft: &WorkbookDraft, sheet_id: SheetId) -> SheetPhonetics<'_> {
    draft.presentation().sheet(sheet_id).map(|sheet| {
        (
            &sheet.cell_phonetics,
            &sheet.row_phonetic_visibility,
            sheet.column_phonetic_visibility.as_slice(),
        )
    })
}

/// Appends the defined-name changes that bring the names back.
fn name_changes(
    before: &WorkbookDraft,
    restored: &WorkbookDraft,
    changes: &mut Vec<WorkbookChange>,
) {
    let key = |name: &crate::DefinedName| (name.scope(), name.lookup_key().to_owned());
    let current = restored
        .workbook()
        .defined_names()
        .iter()
        .map(|name| (key(name), name))
        .collect::<BTreeMap<_, _>>();
    let original = before
        .workbook()
        .defined_names()
        .iter()
        .map(|name| (key(name), name))
        .collect::<BTreeMap<_, _>>();
    for (name_key, name) in &original {
        if current.get(name_key) != Some(name) {
            changes.push(WorkbookChange::set_defined_name((*name).clone()));
        }
    }
    for (name_key, name) in &current {
        if !original.contains_key(name_key) {
            changes.push(WorkbookChange::remove_defined_name(
                name.scope(),
                name.name(),
            ));
        }
    }
}

/// Reports whether a structural edit removes or moves preserved package content of its sheet so
/// that its inverse cannot bring it back: a deletion, a range move, or a line move, which shrinks
/// a preserved range whose ends it crosses.
pub(super) const fn moves_preserved_content(edit: GridEdit) -> bool {
    matches!(
        edit,
        GridEdit::Delete { .. } | GridEdit::Move { .. } | GridEdit::MoveRange { .. }
    )
}

/// Reports whether a change makes the batch's receipt carry an inverse.
pub(super) const fn has_inverse(change: &WorkbookChange) -> bool {
    matches!(
        change,
        WorkbookChange::InsertRows { .. }
            | WorkbookChange::DeleteRows { .. }
            | WorkbookChange::InsertColumns { .. }
            | WorkbookChange::DeleteColumns { .. }
            | WorkbookChange::MoveRows { .. }
            | WorkbookChange::MoveColumns { .. }
            | WorkbookChange::MoveRange { .. }
            | WorkbookChange::CopyRange { .. }
    )
}

#[cfg(test)]
#[path = "inverse_tests.rs"]
mod tests;
