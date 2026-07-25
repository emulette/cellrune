use std::error::Error;
use std::fmt;

const MESSAGE_ZERO_LIMIT: &str = "write limit must be greater than zero";

const MAX_OUTPUT_ARCHIVE_BYTES: &str = "max_output_archive_bytes";
const MAX_ENTRIES: &str = "max_entries";
const MAX_ENTRY_UNCOMPRESSED_BYTES: &str = "max_entry_uncompressed_bytes";
const MAX_TOTAL_UNCOMPRESSED_BYTES: &str = "max_total_uncompressed_bytes";
const MAX_REWRITTEN_XML_BYTES: &str = "max_rewritten_xml_bytes";
const MAX_XML_DEPTH: &str = "max_xml_depth";
const MAX_EDITED_SHEETS: &str = "max_edited_sheets";
const MAX_EDITED_CELLS: &str = "max_edited_cells";
const MAX_MATERIALIZED_FORMULA_CELLS: &str = "max_materialized_formula_cells";
const MAX_MATERIALIZED_SPILL_CELLS: &str = "max_materialized_spill_cells";
const MAX_SHARED_STRINGS: &str = "max_shared_strings";
const MAX_SHARED_STRING_BYTES: &str = "max_shared_string_bytes";
const MAX_RELATIONSHIPS: &str = "max_relationships";
const MAX_CONTENT_TYPES: &str = "max_content_types";
const MAX_TEMPORARY_STORAGE_BYTES: &str = "max_temporary_storage_bytes";
const MAX_VERIFICATION_READ_BYTES: &str = "max_verification_read_bytes";
const MAX_PHONETIC_RUNS_PER_CELL: &str = "max_phonetic_runs_per_cell";
const MAX_TOTAL_PHONETIC_RUNS: &str = "max_total_phonetic_runs";
const MAX_ANNOTATED_CELLS: &str = "max_annotated_cells";
const MAX_PHONETIC_TEXT_BYTES: &str = "max_phonetic_text_bytes";
const MAX_TOTAL_PHONETIC_TEXT_BYTES: &str = "max_total_phonetic_text_bytes";

/// Invalid caller-provided writer configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WriteOptionsError {
    /// A resource limit was set to zero.
    ZeroLimit {
        /// Stable name of the limit that was set to zero.
        name: &'static str,
    },
}

impl fmt::Display for WriteOptionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit { name } => write!(formatter, "{MESSAGE_ZERO_LIMIT}: {name}"),
        }
    }
}

impl Error for WriteOptionsError {}

/// Resource budgets for XLSX package generation and verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteLimits {
    max_output_archive_bytes: u64,
    max_entries: u64,
    max_entry_uncompressed_bytes: u64,
    max_total_uncompressed_bytes: u64,
    max_rewritten_xml_bytes: u64,
    max_xml_depth: u64,
    max_edited_sheets: u64,
    max_edited_cells: u64,
    max_materialized_formula_cells: u64,
    max_materialized_spill_cells: u64,
    max_shared_strings: u64,
    max_shared_string_bytes: u64,
    max_relationships: u64,
    max_content_types: u64,
    max_temporary_storage_bytes: u64,
    max_verification_read_bytes: u64,
    max_phonetic_runs_per_cell: u64,
    max_total_phonetic_runs: u64,
    max_annotated_cells: u64,
    max_phonetic_text_bytes: u64,
    max_total_phonetic_text_bytes: u64,
}

impl WriteLimits {
    /// Returns the maximum completed ZIP archive size.
    pub const fn max_output_archive_bytes(self) -> u64 {
        self.max_output_archive_bytes
    }

    /// Returns the maximum output ZIP entry count.
    pub const fn max_entries(self) -> u64 {
        self.max_entries
    }

    /// Returns the maximum uncompressed size of one output entry.
    pub const fn max_entry_uncompressed_bytes(self) -> u64 {
        self.max_entry_uncompressed_bytes
    }

    /// Returns the maximum combined uncompressed output size.
    pub const fn max_total_uncompressed_bytes(self) -> u64 {
        self.max_total_uncompressed_bytes
    }

    /// Returns the maximum combined rewritten XML size.
    pub const fn max_rewritten_xml_bytes(self) -> u64 {
        self.max_rewritten_xml_bytes
    }

    /// Returns the maximum generated XML nesting depth.
    pub const fn max_xml_depth(self) -> u64 {
        self.max_xml_depth
    }

    /// Returns the maximum edited sheet count.
    pub const fn max_edited_sheets(self) -> u64 {
        self.max_edited_sheets
    }

    /// Returns the maximum edited cell count.
    pub const fn max_edited_cells(self) -> u64 {
        self.max_edited_cells
    }

    /// Returns the maximum direct formula result count.
    pub const fn max_materialized_formula_cells(self) -> u64 {
        self.max_materialized_formula_cells
    }

    /// Returns the maximum spill-follower result count.
    pub const fn max_materialized_spill_cells(self) -> u64 {
        self.max_materialized_spill_cells
    }

    /// Returns the maximum generated shared-string count.
    pub const fn max_shared_strings(self) -> u64 {
        self.max_shared_strings
    }

    /// Returns the maximum generated shared-string UTF-8 byte count.
    pub const fn max_shared_string_bytes(self) -> u64 {
        self.max_shared_string_bytes
    }

    /// Returns the maximum generated relationship count.
    pub const fn max_relationships(self) -> u64 {
        self.max_relationships
    }

    /// Returns the maximum generated content-type declaration count.
    pub const fn max_content_types(self) -> u64 {
        self.max_content_types
    }

    /// Returns the maximum temporary-storage byte count.
    pub const fn max_temporary_storage_bytes(self) -> u64 {
        self.max_temporary_storage_bytes
    }

    /// Returns the maximum bytes consumed while reopening output.
    pub const fn max_verification_read_bytes(self) -> u64 {
        self.max_verification_read_bytes
    }

    /// Returns the maximum phonetic run count in one authored cell.
    pub const fn max_phonetic_runs_per_cell(self) -> u64 {
        self.max_phonetic_runs_per_cell
    }

    /// Returns the maximum combined authored phonetic run count.
    pub const fn max_total_phonetic_runs(self) -> u64 {
        self.max_total_phonetic_runs
    }

    /// Returns the maximum number of annotated cells in generated output.
    pub const fn max_annotated_cells(self) -> u64 {
        self.max_annotated_cells
    }

    /// Returns the maximum UTF-8 byte length of one authored phonetic run.
    pub const fn max_phonetic_text_bytes(self) -> u64 {
        self.max_phonetic_text_bytes
    }

    /// Returns the maximum combined UTF-8 bytes of authored phonetic text.
    pub const fn max_total_phonetic_text_bytes(self) -> u64 {
        self.max_total_phonetic_text_bytes
    }

    /// Replaces the completed ZIP archive size limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_output_archive_bytes(mut self, value: u64) -> Result<Self, WriteOptionsError> {
        self.max_output_archive_bytes = nonzero(MAX_OUTPUT_ARCHIVE_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the output ZIP entry count limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_entries(mut self, value: u64) -> Result<Self, WriteOptionsError> {
        self.max_entries = nonzero(MAX_ENTRIES, value)?;
        Ok(self)
    }

    /// Replaces the per-entry uncompressed byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_entry_uncompressed_bytes(
        mut self,
        value: u64,
    ) -> Result<Self, WriteOptionsError> {
        self.max_entry_uncompressed_bytes = nonzero(MAX_ENTRY_UNCOMPRESSED_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the total uncompressed output byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_total_uncompressed_bytes(
        mut self,
        value: u64,
    ) -> Result<Self, WriteOptionsError> {
        self.max_total_uncompressed_bytes = nonzero(MAX_TOTAL_UNCOMPRESSED_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the rewritten XML byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_rewritten_xml_bytes(mut self, value: u64) -> Result<Self, WriteOptionsError> {
        self.max_rewritten_xml_bytes = nonzero(MAX_REWRITTEN_XML_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the generated XML nesting-depth limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_xml_depth(mut self, value: u64) -> Result<Self, WriteOptionsError> {
        self.max_xml_depth = nonzero(MAX_XML_DEPTH, value)?;
        Ok(self)
    }

    /// Replaces the edited sheet count limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_edited_sheets(mut self, value: u64) -> Result<Self, WriteOptionsError> {
        self.max_edited_sheets = nonzero(MAX_EDITED_SHEETS, value)?;
        Ok(self)
    }

    /// Replaces the edited cell count limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_edited_cells(mut self, value: u64) -> Result<Self, WriteOptionsError> {
        self.max_edited_cells = nonzero(MAX_EDITED_CELLS, value)?;
        Ok(self)
    }

    /// Replaces the direct formula result count limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_materialized_formula_cells(
        mut self,
        value: u64,
    ) -> Result<Self, WriteOptionsError> {
        self.max_materialized_formula_cells = nonzero(MAX_MATERIALIZED_FORMULA_CELLS, value)?;
        Ok(self)
    }

    /// Replaces the spill-follower result count limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_materialized_spill_cells(
        mut self,
        value: u64,
    ) -> Result<Self, WriteOptionsError> {
        self.max_materialized_spill_cells = nonzero(MAX_MATERIALIZED_SPILL_CELLS, value)?;
        Ok(self)
    }

    /// Replaces the generated shared-string count limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_shared_strings(mut self, value: u64) -> Result<Self, WriteOptionsError> {
        self.max_shared_strings = nonzero(MAX_SHARED_STRINGS, value)?;
        Ok(self)
    }

    /// Replaces the generated shared-string UTF-8 byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_shared_string_bytes(mut self, value: u64) -> Result<Self, WriteOptionsError> {
        self.max_shared_string_bytes = nonzero(MAX_SHARED_STRING_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the generated relationship count limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_relationships(mut self, value: u64) -> Result<Self, WriteOptionsError> {
        self.max_relationships = nonzero(MAX_RELATIONSHIPS, value)?;
        Ok(self)
    }

    /// Replaces the generated content-type declaration count limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_content_types(mut self, value: u64) -> Result<Self, WriteOptionsError> {
        self.max_content_types = nonzero(MAX_CONTENT_TYPES, value)?;
        Ok(self)
    }

    /// Replaces the temporary-storage byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_temporary_storage_bytes(
        mut self,
        value: u64,
    ) -> Result<Self, WriteOptionsError> {
        self.max_temporary_storage_bytes = nonzero(MAX_TEMPORARY_STORAGE_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the output verification read limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_verification_read_bytes(
        mut self,
        value: u64,
    ) -> Result<Self, WriteOptionsError> {
        self.max_verification_read_bytes = nonzero(MAX_VERIFICATION_READ_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the per-cell phonetic run-count limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_phonetic_runs_per_cell(
        mut self,
        value: u64,
    ) -> Result<Self, WriteOptionsError> {
        self.max_phonetic_runs_per_cell = nonzero(MAX_PHONETIC_RUNS_PER_CELL, value)?;
        Ok(self)
    }

    /// Replaces the total authored phonetic run-count limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_total_phonetic_runs(mut self, value: u64) -> Result<Self, WriteOptionsError> {
        self.max_total_phonetic_runs = nonzero(MAX_TOTAL_PHONETIC_RUNS, value)?;
        Ok(self)
    }

    /// Replaces the generated annotated-cell limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_annotated_cells(mut self, value: u64) -> Result<Self, WriteOptionsError> {
        self.max_annotated_cells = nonzero(MAX_ANNOTATED_CELLS, value)?;
        Ok(self)
    }

    /// Replaces the per-run phonetic UTF-8 byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_phonetic_text_bytes(mut self, value: u64) -> Result<Self, WriteOptionsError> {
        self.max_phonetic_text_bytes = nonzero(MAX_PHONETIC_TEXT_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the combined authored phonetic UTF-8 byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`WriteOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_total_phonetic_text_bytes(
        mut self,
        value: u64,
    ) -> Result<Self, WriteOptionsError> {
        self.max_total_phonetic_text_bytes = nonzero(MAX_TOTAL_PHONETIC_TEXT_BYTES, value)?;
        Ok(self)
    }
}

impl Default for WriteLimits {
    fn default() -> Self {
        Self {
            max_output_archive_bytes: 512 * 1024 * 1024,
            max_entries: 10_000,
            max_entry_uncompressed_bytes: 64 * 1024 * 1024,
            max_total_uncompressed_bytes: 512 * 1024 * 1024,
            max_rewritten_xml_bytes: 256 * 1024 * 1024,
            max_xml_depth: 128,
            max_edited_sheets: 1_024,
            max_edited_cells: 5_000_000,
            max_materialized_formula_cells: 5_000_000,
            max_materialized_spill_cells: 5_000_000,
            max_shared_strings: 2_000_000,
            max_shared_string_bytes: 256 * 1024 * 1024,
            max_relationships: 100_000,
            max_content_types: 100_000,
            max_temporary_storage_bytes: 1024 * 1024 * 1024,
            max_verification_read_bytes: 512 * 1024 * 1024,
            max_phonetic_runs_per_cell: 32_768,
            max_total_phonetic_runs: 2_000_000,
            max_annotated_cells: 2_000_000,
            max_phonetic_text_bytes: 1024 * 1024,
            max_total_phonetic_text_bytes: 256 * 1024 * 1024,
        }
    }
}

/// XLSX output behavior and resource budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WriteOptions {
    limits: WriteLimits,
    replace_existing: bool,
}

impl WriteOptions {
    /// Constructs options from validated limits.
    pub const fn new(limits: WriteLimits) -> Self {
        Self {
            limits,
            replace_existing: false,
        }
    }

    /// Explicitly enables or disables destination replacement.
    ///
    /// When replacement is disabled, which is the default, the output is
    /// installed without ever overwriting an existing destination. The install
    /// prefers a hard link and falls back to an exclusive `create_new`
    /// reservation followed by a rename on filesystems that do not support
    /// links, such as exFAT, FAT32, and many SMB and FUSE mounts. Both paths
    /// reserve the destination in a single atomic step, so an existing file is
    /// reported as [`XlsxWriteErrorCode::DestinationExists`] rather than
    /// replaced.
    ///
    /// [`XlsxWriteErrorCode::DestinationExists`]: crate::XlsxWriteErrorCode::DestinationExists
    pub const fn with_replace_existing(mut self, replace_existing: bool) -> Self {
        self.replace_existing = replace_existing;
        self
    }

    /// Returns the configured write resource limits.
    pub const fn limits(self) -> WriteLimits {
        self.limits
    }

    /// Returns whether an existing destination may be replaced.
    pub const fn replace_existing(self) -> bool {
        self.replace_existing
    }
}

/// Policy for formulas that do not have a current materialized calculation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum RecalculationWritePolicy {
    /// Reject the complete write before producing an output artifact.
    #[default]
    RequireComplete,
    /// Remove stale caches for unavailable formulas and request host recalculation on load.
    InvalidateUnavailable,
}

/// Options for materializing a calculation into an existing XLSX or XLSM package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecalculationWriteOptions {
    write_options: WriteOptions,
    policy: RecalculationWritePolicy,
}

impl RecalculationWriteOptions {
    /// Constructs recalculation options from general package-write options.
    pub const fn new(write_options: WriteOptions) -> Self {
        Self {
            write_options,
            policy: RecalculationWritePolicy::RequireComplete,
        }
    }

    /// Replaces the unavailable-result policy.
    pub const fn with_policy(mut self, policy: RecalculationWritePolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Returns the general package-write options.
    pub const fn write_options(self) -> WriteOptions {
        self.write_options
    }

    /// Returns the unavailable-result policy.
    pub const fn policy(self) -> RecalculationWritePolicy {
        self.policy
    }
}

fn nonzero(name: &'static str, value: u64) -> Result<u64, WriteOptionsError> {
    if value == 0 {
        return Err(WriteOptionsError::ZeroLimit { name });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{WriteLimits, WriteOptions, WriteOptionsError};

    #[test]
    fn write_limits_reject_zero_at_every_public_boundary() {
        let setters = [
            WriteLimits::with_max_output_archive_bytes,
            WriteLimits::with_max_entries,
            WriteLimits::with_max_entry_uncompressed_bytes,
            WriteLimits::with_max_total_uncompressed_bytes,
            WriteLimits::with_max_rewritten_xml_bytes,
            WriteLimits::with_max_xml_depth,
            WriteLimits::with_max_edited_sheets,
            WriteLimits::with_max_edited_cells,
            WriteLimits::with_max_materialized_formula_cells,
            WriteLimits::with_max_materialized_spill_cells,
            WriteLimits::with_max_shared_strings,
            WriteLimits::with_max_shared_string_bytes,
            WriteLimits::with_max_relationships,
            WriteLimits::with_max_content_types,
            WriteLimits::with_max_temporary_storage_bytes,
            WriteLimits::with_max_verification_read_bytes,
            WriteLimits::with_max_phonetic_runs_per_cell,
            WriteLimits::with_max_total_phonetic_runs,
            WriteLimits::with_max_annotated_cells,
            WriteLimits::with_max_phonetic_text_bytes,
            WriteLimits::with_max_total_phonetic_text_bytes,
        ];
        for setter in setters {
            assert!(matches!(
                setter(WriteLimits::default(), 0),
                Err(WriteOptionsError::ZeroLimit { .. })
            ));
        }
    }

    #[test]
    fn replacement_remains_opt_in() {
        assert!(!WriteOptions::default().replace_existing());
        assert!(
            WriteOptions::default()
                .with_replace_existing(true)
                .replace_existing()
        );
    }
}
