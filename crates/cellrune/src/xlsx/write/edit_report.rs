//! The outcome of writing a document draft's edits without calculating.

use super::{WriteOptions, XlsxWriteError};
use crate::{CalculationCellId, InputHash, OutputHash, SourceId, XlsxDocumentKind};

/// Structured outcome of writing a document draft's edits without calculating.
#[derive(Debug, Clone)]
pub struct EditWriteReport {
    pub(super) affected_formulas: Vec<CalculationCellId>,
    pub(super) requests_host_recalculation: bool,
    pub(super) changed_parts: Vec<SourceId>,
    pub(super) removed_parts: Vec<SourceId>,
    pub(super) output_hash: OutputHash,
    pub(super) input_hash: InputHash,
    pub(super) semantic_revision: u64,
    pub(super) presentation_revision: u64,
}

impl EditWriteReport {
    /// Returns the formulas the edits can affect, in cell order. Their saved results were removed
    /// so the host recalculates them; every other formula keeps its saved result.
    pub fn affected_formulas(&self) -> &[CalculationCellId] {
        &self.affected_formulas
    }

    /// Returns whether the output asks the host to recalculate every formula when it opens the
    /// file, which happens exactly when a formula is affected.
    pub const fn requests_host_recalculation(&self) -> bool {
        self.requests_host_recalculation
    }

    /// Returns package parts rewritten by the write.
    pub fn changed_parts(&self) -> &[SourceId] {
        &self.changed_parts
    }

    /// Returns stale package parts intentionally removed, such as a `calcChain.xml` that no longer
    /// matches the formula layout.
    pub fn removed_parts(&self) -> &[SourceId] {
        &self.removed_parts
    }

    /// Returns the SHA-256 identity of the exact verified output archive bytes.
    pub const fn output_hash(&self) -> OutputHash {
        self.output_hash
    }

    /// Returns the SHA-256 identity of the source archive the draft was opened from.
    pub const fn input_hash(&self) -> InputHash {
        self.input_hash
    }

    /// Returns the semantic workbook revision written.
    pub const fn semantic_revision(&self) -> u64 {
        self.semantic_revision
    }

    /// Returns the presentation revision written.
    pub const fn presentation_revision(&self) -> u64 {
        self.presentation_revision
    }
}

/// Verified in-memory XLSX or XLSM output of a draft's edits and its report.
#[derive(Debug)]
pub struct EditedWorkbook {
    bytes: Vec<u8>,
    report: EditWriteReport,
    kind: XlsxDocumentKind,
}

impl EditedWorkbook {
    pub(super) const fn new(
        bytes: Vec<u8>,
        report: EditWriteReport,
        kind: XlsxDocumentKind,
    ) -> Self {
        Self {
            bytes,
            report,
            kind,
        }
    }

    /// Returns the verified output archive bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the output and returns its archive bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Returns the structured write outcome.
    pub const fn report(&self) -> &EditWriteReport {
        &self.report
    }

    /// Consumes the output and returns both archive bytes and report.
    pub fn into_parts(self) -> (Vec<u8>, EditWriteReport) {
        (self.bytes, self.report)
    }

    /// Atomically saves this verified package to a destination of the matching workbook kind.
    ///
    /// # Errors
    ///
    /// Returns an [`XlsxWriteError`] when the destination kind is wrong, already exists without
    /// replacement permission, or the atomic Save As operation fails.
    pub fn save_path(
        &self,
        path: impl AsRef<std::path::Path>,
        options: WriteOptions,
    ) -> Result<(), XlsxWriteError> {
        super::path_output::write_bytes_to_path(&self.bytes, self.kind, path.as_ref(), options)
    }

    /// Atomically saves this verified package beneath an already-open directory capability.
    ///
    /// The destination must be exactly one relative file name, as for
    /// [`RecalculatedWorkbook::save_in_directory`](super::RecalculatedWorkbook::save_in_directory).
    ///
    /// # Errors
    ///
    /// Returns an [`XlsxWriteError`] when the destination is not one file name, has the wrong
    /// workbook extension, already exists without replacement permission, or the atomic Save As
    /// operation fails.
    #[cfg(feature = "capability-fs")]
    pub fn save_in_directory(
        &self,
        directory: &cap_std::fs::Dir,
        destination: impl AsRef<std::path::Path>,
        options: WriteOptions,
    ) -> Result<(), XlsxWriteError> {
        super::path_output::write_bytes_to_directory(
            &self.bytes,
            self.kind,
            directory,
            destination.as_ref(),
            options,
        )
    }
}
