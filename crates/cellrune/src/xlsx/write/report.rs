use crate::{
    CalculationCellId, CalculationOptions, Diagnostic, InputHash, OutputHash, ProviderIdentity,
    SourceId, XlsxDocumentKind,
};

use super::{RecalculationWritePolicy, WriteOptions, XlsxWriteError};

/// Exact calculation and source identity recorded for a completed workbook write.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteProvenance {
    input_hash: Option<InputHash>,
    semantic_revision: u64,
    presentation_revision: u64,
    calculator: ProviderIdentity,
    calculation_options: CalculationOptions,
}

impl WriteProvenance {
    pub(super) fn new(
        input_hash: Option<InputHash>,
        semantic_revision: u64,
        presentation_revision: u64,
        calculator: ProviderIdentity,
        calculation_options: CalculationOptions,
    ) -> Self {
        Self {
            input_hash,
            semantic_revision,
            presentation_revision,
            calculator,
            calculation_options,
        }
    }

    /// Returns the SHA-256 identity of the exact input archive.
    pub const fn input_hash(&self) -> Option<InputHash> {
        self.input_hash
    }

    /// Returns the semantic workbook revision used by the calculation.
    pub const fn semantic_revision(&self) -> u64 {
        self.semantic_revision
    }

    /// Returns the presentation revision serialized by the writer.
    pub const fn presentation_revision(&self) -> u64 {
        self.presentation_revision
    }

    /// Returns the calculator identity and version.
    pub const fn calculator(&self) -> &ProviderIdentity {
        &self.calculator
    }

    /// Returns the deterministic calculation inputs and limits.
    pub const fn calculation_options(&self) -> CalculationOptions {
        self.calculation_options
    }
}

/// Structured outcome of materializing a calculation into a preserved workbook package.
#[derive(Debug, Clone)]
pub struct WriteReport {
    complete: bool,
    policy: RecalculationWritePolicy,
    materialized_count: usize,
    invalidated_cells: Vec<CalculationCellId>,
    changed_parts: Vec<SourceId>,
    removed_parts: Vec<SourceId>,
    diagnostics: Vec<Diagnostic>,
    output_hash: OutputHash,
    provenance: WriteProvenance,
}

pub(super) struct VerifiedOutputReceipt {
    changed_parts: Vec<SourceId>,
    removed_parts: Vec<SourceId>,
    diagnostics: Vec<Diagnostic>,
    output_hash: OutputHash,
}

impl VerifiedOutputReceipt {
    pub(super) fn new(
        changed_parts: Vec<SourceId>,
        removed_parts: Vec<SourceId>,
        diagnostics: Vec<Diagnostic>,
        output_bytes: &[u8],
    ) -> Self {
        Self {
            changed_parts,
            removed_parts,
            diagnostics,
            output_hash: OutputHash::for_bytes(output_bytes),
        }
    }
}

impl WriteReport {
    pub(super) fn new(
        policy: RecalculationWritePolicy,
        materialized_count: usize,
        invalidated_cells: Vec<CalculationCellId>,
        output: VerifiedOutputReceipt,
        provenance: WriteProvenance,
    ) -> Self {
        Self {
            complete: invalidated_cells.is_empty(),
            policy,
            materialized_count,
            invalidated_cells,
            changed_parts: output.changed_parts,
            removed_parts: output.removed_parts,
            diagnostics: output.diagnostics,
            output_hash: output.output_hash,
            provenance,
        }
    }

    /// Returns whether every required calculation result was materialized.
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    /// Returns the unavailable-result policy used for this write.
    pub const fn policy(&self) -> RecalculationWritePolicy {
        self.policy
    }

    /// Returns the number of typed direct-formula, legacy-array-region, and
    /// dynamic-spill-region cells written.
    pub const fn materialized_count(&self) -> usize {
        self.materialized_count
    }

    /// Returns cells whose stale saved results were removed.
    pub fn invalidated_cells(&self) -> &[CalculationCellId] {
        &self.invalidated_cells
    }

    /// Returns package parts rewritten by the plan.
    pub fn changed_parts(&self) -> &[SourceId] {
        &self.changed_parts
    }

    /// Returns stale package parts intentionally removed by the plan.
    pub fn removed_parts(&self) -> &[SourceId] {
        &self.removed_parts
    }

    /// Returns bounded write diagnostics.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Returns the SHA-256 identity of the exact verified output archive bytes.
    pub const fn output_hash(&self) -> OutputHash {
        self.output_hash
    }

    /// Returns exact source and calculation provenance.
    pub const fn provenance(&self) -> &WriteProvenance {
        &self.provenance
    }
}

/// Verified in-memory XLSX or XLSM output and its write report.
#[derive(Debug)]
pub struct RecalculatedWorkbook {
    bytes: Vec<u8>,
    report: WriteReport,
    kind: XlsxDocumentKind,
}

impl RecalculatedWorkbook {
    pub(crate) const fn new(bytes: Vec<u8>, report: WriteReport, kind: XlsxDocumentKind) -> Self {
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
    pub const fn report(&self) -> &WriteReport {
        &self.report
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
    /// The destination must be exactly one relative file name. Holding the directory open across
    /// validation and installation prevents a concurrent ambient-path replacement from
    /// redirecting the write.
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

    /// Consumes the output and returns both archive bytes and report.
    pub fn into_parts(self) -> (Vec<u8>, WriteReport) {
        (self.bytes, self.report)
    }
}
