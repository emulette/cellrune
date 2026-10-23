use crate::calculation::sheet_changes::TabStep;
use crate::{
    CalculationCellId, CellAddress, DocumentPresentation, SheetId, TableId, ValidationError,
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
pub use batch::{EditBatch, EditReceipt, WorkbookChange};
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

    pub(crate) fn sheet_keys(&self, sheet_id: SheetId) -> impl Iterator<Item = &CalculationCellId> {
        self.chunks
            .range((sheet_id, 0)..=(sheet_id, u32::MAX))
            .flat_map(|(_, chunk)| chunk.keys())
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.len == 0
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
