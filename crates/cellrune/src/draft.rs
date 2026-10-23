use crate::calculation::sheet_changes::TabStep;
use crate::grid::GridHistory;
use crate::{
    CalculationCellId, Cell, CellAddress, DocumentPresentation, SheetId, TableId, ValidationError,
    WorkbookSnapshot, XlsxDocument,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod batch;
pub(crate) use batch::DefinedNameRename;
mod format;
mod metadata;
mod presentation;
mod single_edit;

pub(crate) use batch::{
    BatchExecutionError, TableMaterializationBudget, TableMaterializationError,
    rewrite_limit_detail,
};
pub use batch::{
    EditBatch, EditInverse, EditReceipt, ImportedSheet, UnrestoredContent, UnrestoredContentKind,
    WorkbookChange,
};
pub(crate) use format::effective_cell_format;

/// One explicit cell mutation retained for document-backed package patching.
///
/// `replaces_source_cell` marks a cell written after the draft removed the source cell at the
/// same address: it is a new cell and takes no style from the removed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DraftCellMutation {
    Upsert {
        number_format_changed: bool,
        replaces_source_cell: bool,
    },
    Remove,
}

const DRAFT_MUTATION_ROW_CHUNK_SIZE: u32 = 256;

#[derive(Debug, Clone, Default)]
pub(crate) struct DraftCellMutationStore {
    chunks: BTreeMap<(SheetId, u32), Arc<BTreeMap<CalculationCellId, DraftCellMutation>>>,
    len: usize,
}

impl DraftCellMutationStore {
    fn chunk(id: CalculationCellId) -> (SheetId, u32) {
        (
            id.sheet_id(),
            (id.address().row().get() - 1) / DRAFT_MUTATION_ROW_CHUNK_SIZE,
        )
    }

    pub(crate) fn get(&self, id: &CalculationCellId) -> Option<&DraftCellMutation> {
        self.chunks
            .get(&Self::chunk(*id))
            .and_then(|chunk| chunk.get(id))
    }

    pub(crate) fn insert(
        &mut self,
        id: CalculationCellId,
        mutation: DraftCellMutation,
    ) -> Option<DraftCellMutation> {
        let chunk = self
            .chunks
            .entry(Self::chunk(id))
            .or_insert_with(|| Arc::new(BTreeMap::new()));
        let previous = Arc::make_mut(chunk).insert(id, mutation);
        if previous.is_none() {
            self.len += 1;
        }
        previous
    }

    pub(crate) fn remove(&mut self, id: &CalculationCellId) -> Option<DraftCellMutation> {
        let key = Self::chunk(*id);
        let chunk = self.chunks.get_mut(&key)?;
        let removed = Arc::make_mut(chunk).remove(id)?;
        if chunk.is_empty() {
            self.chunks.remove(&key);
        }
        self.len -= 1;
        Some(removed)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&CalculationCellId, &DraftCellMutation)> {
        self.chunks.values().flat_map(|chunk| chunk.iter())
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &CalculationCellId> {
        self.iter().map(|(id, _)| id)
    }

    /// Drops every mutation recorded for one sheet.
    pub(crate) fn remove_sheet(&mut self, sheet_id: SheetId) {
        let chunks = self
            .chunks
            .range((sheet_id, 0)..=(sheet_id, u32::MAX))
            .map(|(key, chunk)| (*key, chunk.len()))
            .collect::<Vec<_>>();
        for (key, len) in chunks {
            self.chunks.remove(&key);
            self.len -= len;
        }
    }

    /// Moves every mutation recorded for one sheet to the address `map` gives it, dropping the
    /// ones whose cell `map` removes.
    pub(crate) fn remap_sheet(
        &mut self,
        sheet_id: SheetId,
        map: impl Fn(CellAddress) -> Option<CellAddress>,
    ) {
        let entries = self
            .chunks
            .range((sheet_id, 0)..=(sheet_id, u32::MAX))
            .flat_map(|(_, chunk)| chunk.iter().map(|(id, mutation)| (*id, *mutation)))
            .collect::<Vec<_>>();
        self.remove_sheet(sheet_id);
        for (id, mutation) in entries {
            if let Some(address) = map(id.address()) {
                self.insert(CalculationCellId::new(sheet_id, address), mutation);
            }
        }
    }

    pub(crate) fn sheet_keys(&self, sheet_id: SheetId) -> impl Iterator<Item = &CalculationCellId> {
        self.chunks
            .range((sheet_id, 0)..=(sheet_id, u32::MAX))
            .flat_map(|(_, chunk)| chunk.keys())
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// The source document's cells where the draft's structural edits moved them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceCells<'a> {
    workbook: Option<&'a WorkbookSnapshot>,
    histories: &'a BTreeMap<SheetId, GridHistory>,
}

impl<'a> SourceCells<'a> {
    pub(crate) const fn new(
        workbook: Option<&'a WorkbookSnapshot>,
        histories: &'a BTreeMap<SheetId, GridHistory>,
    ) -> Self {
        Self {
            workbook,
            histories,
        }
    }

    /// Returns the container metadata of the source formula that now sits at `address`, moved as
    /// the structural edits moved its cell.
    pub(crate) fn formula_metadata(
        &self,
        sheet_id: SheetId,
        address: CellAddress,
    ) -> Option<crate::FormulaMetadata> {
        let crate::CellContent::Formula(formula) = self.cell(sheet_id, address)?.content() else {
            return None;
        };
        let mut metadata = formula.metadata().clone();
        if let Some(history) = self.histories.get(&sheet_id) {
            let mut current = history.to_source(address)?;
            for edit in history.edits() {
                current = edit.map_address(current)?;
                metadata = crate::workbook::grid_metadata(&metadata, *edit, current);
            }
        }
        Some(metadata)
    }

    /// Returns the source document's workbook, `None` for a draft without one.
    pub(crate) const fn workbook(&self) -> Option<&'a WorkbookSnapshot> {
        self.workbook
    }

    /// Returns the source cell that now sits at `address`.
    pub(crate) fn cell(&self, sheet_id: SheetId, address: CellAddress) -> Option<&'a Cell> {
        self.cell_through(sheet_id, sheet_id, address)
    }

    /// Returns the cell of source sheet `origin` that now sits at `address` of `sheet_id`, which is
    /// `origin` itself or a copy of it, moved by `sheet_id`'s structural edits.
    pub(crate) fn cell_through(
        &self,
        origin: SheetId,
        sheet_id: SheetId,
        address: CellAddress,
    ) -> Option<&'a Cell> {
        let sheet = self.workbook?.sheet_by_id(origin)?;
        match self.histories.get(&sheet_id) {
            Some(history) => sheet.cell(history.to_source(address)?),
            None => sheet.cell(address),
        }
    }

    /// Returns where `sheet_id`'s structural edits moved a cell of its source part at `address`,
    /// or `None` when they deleted it.
    pub(crate) fn current_address(
        &self,
        sheet_id: SheetId,
        address: CellAddress,
    ) -> Option<CellAddress> {
        let Some(history) = self.histories.get(&sheet_id) else {
            return Some(address);
        };
        history
            .edits()
            .iter()
            .try_fold(address, |address, edit| edit.map_address(address))
    }
}

/// An owned, mutable workbook editing session with monotonic semantic revisions.
#[derive(Debug, Clone)]
pub struct WorkbookDraft {
    workbook: Arc<WorkbookSnapshot>,
    presentation: DocumentPresentation,
    source_document: Option<Arc<XlsxDocument>>,
    cell_mutations: DraftCellMutationStore,
    presentation_cell_mutations: BTreeSet<CalculationCellId>,
    presentation_sheet_mutations: BTreeSet<SheetId>,
    added_sheets: BTreeSet<SheetId>,
    changed_table_ids: BTreeSet<TableId>,
    workbook_changed: bool,
    /// The sheet deletions and moves in the order they were made, which the writer replays on the
    /// 3-D references of the parts it preserves.
    tab_steps: Vec<TabStep>,
    /// The defined-name renames in the order they were made, which the writer replays on the
    /// formulas of the parts it preserves.
    name_renames: Vec<DefinedNameRename>,
    /// Each copy of a sheet the source package holds, with the source sheet whose part the writer
    /// duplicates for it; copies of other sheets are added sheets.
    copied_sheets: BTreeMap<SheetId, SheetId>,
    /// Each table of such a copy that duplicates a table the source package holds, with that
    /// table, whose part the writer duplicates for it.
    copied_tables: BTreeMap<TableId, TableId>,
    /// The structural edits applied to each source worksheet, which the writer replays on the
    /// preserved part.
    grid_histories: BTreeMap<SheetId, GridHistory>,
}

impl WorkbookDraft {
    /// Creates a new XLSX workbook containing one visible sheet named `Sheet1`.
    pub fn new() -> Self {
        Self {
            workbook: Arc::new(WorkbookSnapshot::new_draft()),
            presentation: DocumentPresentation::default(),
            source_document: None,
            cell_mutations: DraftCellMutationStore::default(),
            presentation_cell_mutations: BTreeSet::new(),
            presentation_sheet_mutations: BTreeSet::new(),
            added_sheets: BTreeSet::new(),
            changed_table_ids: BTreeSet::new(),
            workbook_changed: true,
            tab_steps: Vec::new(),
            name_renames: Vec::new(),
            copied_sheets: BTreeMap::new(),
            copied_tables: BTreeMap::new(),
            grid_histories: BTreeMap::new(),
        }
    }

    /// Creates an owned editing session from a package-backed document.
    pub fn from_document(document: &XlsxDocument) -> Self {
        Self {
            workbook: Arc::new(document.workbook().clone()),
            presentation: document.presentation().clone(),
            source_document: Some(Arc::new(document.clone())),
            cell_mutations: DraftCellMutationStore::default(),
            presentation_cell_mutations: BTreeSet::new(),
            presentation_sheet_mutations: BTreeSet::new(),
            added_sheets: BTreeSet::new(),
            changed_table_ids: BTreeSet::new(),
            workbook_changed: false,
            tab_steps: Vec::new(),
            name_renames: Vec::new(),
            copied_sheets: BTreeMap::new(),
            copied_tables: BTreeMap::new(),
            grid_histories: BTreeMap::new(),
        }
    }

    /// Returns the current immutable semantic snapshot.
    pub fn workbook(&self) -> &WorkbookSnapshot {
        self.workbook.as_ref()
    }

    pub(crate) fn shared_workbook(&self) -> Arc<WorkbookSnapshot> {
        Arc::clone(&self.workbook)
    }

    /// Returns immutable presentation metadata for the current draft.
    pub const fn presentation(&self) -> &DocumentPresentation {
        &self.presentation
    }

    /// Returns the monotonic semantic revision.
    pub fn semantic_revision(&self) -> u64 {
        self.workbook.semantic_revision()
    }

    /// Returns the monotonic presentation-only revision.
    pub const fn presentation_revision(&self) -> u64 {
        self.presentation.revision()
    }

    pub(crate) fn tab_steps(&self) -> &[TabStep] {
        &self.tab_steps
    }

    pub(crate) fn name_renames(&self) -> &[DefinedNameRename] {
        &self.name_renames
    }

    pub(crate) const fn copied_sheets(&self) -> &BTreeMap<SheetId, SheetId> {
        &self.copied_sheets
    }

    pub(crate) const fn copied_tables(&self) -> &BTreeMap<TableId, TableId> {
        &self.copied_tables
    }

    /// Returns whether the draft preserves an existing XLSX or XLSM package.
    pub const fn is_document_backed(&self) -> bool {
        self.source_document.is_some()
    }

    /// Returns the preserved package kind for a document-backed draft.
    pub fn document_kind(&self) -> Option<crate::XlsxDocumentKind> {
        self.source_document.as_deref().map(XlsxDocument::kind)
    }

    pub(crate) fn clone_cancellable(&self, cancelled: &impl Fn() -> bool) -> Result<Self, ()> {
        let workbook = Arc::clone(&self.workbook);
        let presentation = self.presentation.clone_cancellable(cancelled)?;
        if cancelled() {
            return Err(());
        }
        let source_document = self.source_document.clone();
        Ok(Self {
            workbook,
            presentation,
            source_document,
            cell_mutations: self.cell_mutations.clone(),
            presentation_cell_mutations: clone_set_cancellable(
                &self.presentation_cell_mutations,
                cancelled,
            )?,
            presentation_sheet_mutations: clone_set_cancellable(
                &self.presentation_sheet_mutations,
                cancelled,
            )?,
            added_sheets: clone_set_cancellable(&self.added_sheets, cancelled)?,
            changed_table_ids: clone_set_cancellable(&self.changed_table_ids, cancelled)?,
            workbook_changed: self.workbook_changed,
            tab_steps: self.tab_steps.clone(),
            name_renames: self.name_renames.clone(),
            copied_sheets: self.copied_sheets.clone(),
            copied_tables: self.copied_tables.clone(),
            grid_histories: self.grid_histories.clone(),
        })
    }

    #[cfg(test)]
    pub(crate) fn from_snapshot_for_test(workbook: WorkbookSnapshot) -> Self {
        Self {
            workbook: Arc::new(workbook),
            presentation: DocumentPresentation::default(),
            source_document: None,
            cell_mutations: DraftCellMutationStore::default(),
            presentation_cell_mutations: BTreeSet::new(),
            presentation_sheet_mutations: BTreeSet::new(),
            added_sheets: BTreeSet::new(),
            changed_table_ids: BTreeSet::new(),
            workbook_changed: true,
            tab_steps: Vec::new(),
            name_renames: Vec::new(),
            copied_sheets: BTreeMap::new(),
            copied_tables: BTreeMap::new(),
            grid_histories: BTreeMap::new(),
        }
    }
}

fn annotated_text_replacement_required(sheet_id: SheetId, address: CellAddress) -> ValidationError {
    ValidationError::AnnotatedTextReplacementRequired {
        sheet_id: sheet_id.get(),
        row: address.row().get(),
        column: address.column().get(),
    }
}

/// Rejects a cell edit inside a legacy array formula range away from its anchor, which Excel
/// edits only as a whole.
fn ensure_not_array_formula_part(
    sheet: &crate::Sheet,
    address: CellAddress,
) -> Result<(), ValidationError> {
    if sheet.legacy_array_anchor_covering(address).is_some() {
        return Err(ValidationError::ArrayFormulaPartEdit {
            sheet_id: sheet.id().get(),
            row: address.row().get(),
            column: address.column().get(),
        });
    }
    Ok(())
}

impl Default for WorkbookDraft {
    fn default() -> Self {
        Self::new()
    }
}

fn next_revision(revision: u64) -> Result<u64, ValidationError> {
    revision
        .checked_add(1)
        .ok_or(ValidationError::SemanticRevisionExhausted)
}

fn case_insensitive_key(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

fn clone_set_cancellable<T>(
    source: &BTreeSet<T>,
    cancelled: &impl Fn() -> bool,
) -> Result<BTreeSet<T>, ()>
where
    T: Clone + Ord,
{
    let mut cloned = BTreeSet::new();
    for value in source {
        if cancelled() {
            return Err(());
        }
        cloned.insert(value.clone());
    }
    Ok(cloned)
}

#[cfg(test)]
#[path = "draft_tests.rs"]
mod tests;
