use crate::{
    CalculationCellId, CellAddress, DocumentPresentation, SheetId, ValidationError,
    WorkbookSnapshot, XlsxDocument,
};
use std::collections::{BTreeMap, BTreeSet};

mod batch;
mod formula_rewrite;
mod metadata;
mod presentation;
mod single_edit;

pub use batch::{EditBatch, EditReceipt, WorkbookChange};

/// One explicit cell mutation retained for document-backed package patching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DraftCellMutation {
    Upsert { number_format_changed: bool },
    Remove,
}

/// An owned, mutable workbook editing session with monotonic semantic revisions.
#[derive(Debug, Clone)]
pub struct WorkbookDraft {
    workbook: WorkbookSnapshot,
    presentation: DocumentPresentation,
    source_document: Option<XlsxDocument>,
    cell_mutations: BTreeMap<CalculationCellId, DraftCellMutation>,
    presentation_cell_mutations: BTreeSet<CalculationCellId>,
    presentation_sheet_mutations: BTreeSet<SheetId>,
    added_sheets: BTreeSet<SheetId>,
    workbook_changed: bool,
}

impl WorkbookDraft {
    /// Creates a new XLSX workbook containing one visible sheet named `Sheet1`.
    pub fn new() -> Self {
        Self {
            workbook: WorkbookSnapshot::new_draft(),
            presentation: DocumentPresentation::default(),
            source_document: None,
            cell_mutations: BTreeMap::new(),
            presentation_cell_mutations: BTreeSet::new(),
            presentation_sheet_mutations: BTreeSet::new(),
            added_sheets: BTreeSet::new(),
            workbook_changed: true,
        }
    }

    /// Creates an owned editing session from a package-backed document.
    pub fn from_document(document: &XlsxDocument) -> Self {
        Self {
            workbook: document.workbook().clone(),
            presentation: document.presentation().clone(),
            source_document: Some(document.clone()),
            cell_mutations: BTreeMap::new(),
            presentation_cell_mutations: BTreeSet::new(),
            presentation_sheet_mutations: BTreeSet::new(),
            added_sheets: BTreeSet::new(),
            workbook_changed: false,
        }
    }

    /// Returns the current immutable semantic snapshot.
    pub const fn workbook(&self) -> &WorkbookSnapshot {
        &self.workbook
    }

    /// Returns immutable presentation metadata for the current draft.
    pub const fn presentation(&self) -> &DocumentPresentation {
        &self.presentation
    }

    /// Returns the monotonic semantic revision.
    pub const fn semantic_revision(&self) -> u64 {
        self.workbook.semantic_revision()
    }

    /// Returns the monotonic presentation-only revision.
    pub const fn presentation_revision(&self) -> u64 {
        self.presentation.revision()
    }

    /// Returns whether the draft preserves an existing XLSX or XLSM package.
    pub const fn is_document_backed(&self) -> bool {
        self.source_document.is_some()
    }

    /// Returns the preserved package kind for a document-backed draft.
    pub fn document_kind(&self) -> Option<crate::XlsxDocumentKind> {
        self.source_document.as_ref().map(XlsxDocument::kind)
    }

    #[cfg(test)]
    pub(crate) fn from_snapshot_for_test(workbook: WorkbookSnapshot) -> Self {
        Self {
            workbook,
            presentation: DocumentPresentation::default(),
            source_document: None,
            cell_mutations: BTreeMap::new(),
            presentation_cell_mutations: BTreeSet::new(),
            presentation_sheet_mutations: BTreeSet::new(),
            added_sheets: BTreeSet::new(),
            workbook_changed: true,
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

#[cfg(test)]
#[path = "draft_tests.rs"]
mod tests;
