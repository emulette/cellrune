use super::ReadOptionsError;

const MAX_ARCHIVE_BYTES: &str = "max_archive_bytes";
const MAX_ENTRIES: &str = "max_entries";
const MAX_ENTRY_UNCOMPRESSED_BYTES: &str = "max_entry_uncompressed_bytes";
const MAX_TOTAL_UNCOMPRESSED_BYTES: &str = "max_total_uncompressed_bytes";
const MAX_COMPRESSION_RATIO: &str = "max_compression_ratio";
const MAX_XML_DEPTH: &str = "max_xml_depth";
const MAX_XML_ATTRIBUTES: &str = "max_xml_attributes";
const MAX_SHEETS: &str = "max_sheets";
const MAX_CELLS_PER_SHEET: &str = "max_cells_per_sheet";
const MAX_TOTAL_CELLS: &str = "max_total_cells";
const MAX_SHARED_STRINGS: &str = "max_shared_strings";
const MAX_SHARED_STRING_BYTES: &str = "max_shared_string_bytes";
const MAX_TOTAL_SHARED_STRING_BYTES: &str = "max_total_shared_string_bytes";
const MAX_DEFINED_NAMES: &str = "max_defined_names";
const MAX_FORMULA_BYTES: &str = "max_formula_bytes";
const MAX_TOTAL_FORMULA_BYTES: &str = "max_total_formula_bytes";
const MAX_MERGED_RANGES: &str = "max_merged_ranges";
const MAX_PHONETIC_RUNS_PER_ITEM: &str = "max_phonetic_runs_per_item";
const MAX_TOTAL_PHONETIC_RUNS: &str = "max_total_phonetic_runs";
const MAX_ANNOTATED_CELLS: &str = "max_annotated_cells";
const MAX_PHONETIC_TEXT_BYTES: &str = "max_phonetic_text_bytes";
const MAX_TOTAL_PHONETIC_TEXT_BYTES: &str = "max_total_phonetic_text_bytes";

/// Resource limits applied before workbook semantics are interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadLimits {
    max_archive_bytes: u64,
    max_entries: u64,
    max_entry_uncompressed_bytes: u64,
    max_total_uncompressed_bytes: u64,
    max_compression_ratio: u64,
    max_xml_depth: u64,
    max_xml_attributes: u64,
    max_sheets: u64,
    max_cells_per_sheet: u64,
    max_total_cells: u64,
    max_shared_strings: u64,
    max_shared_string_bytes: u64,
    max_total_shared_string_bytes: u64,
    max_defined_names: u64,
    max_formula_bytes: u64,
    max_total_formula_bytes: u64,
    max_merged_ranges: u64,
    max_phonetic_runs_per_item: u64,
    max_total_phonetic_runs: u64,
    max_annotated_cells: u64,
    max_phonetic_text_bytes: u64,
    max_total_phonetic_text_bytes: u64,
}

impl ReadLimits {
    /// Returns the maximum input archive size.
    pub const fn max_archive_bytes(self) -> u64 {
        self.max_archive_bytes
    }

    /// Returns the maximum ZIP entry count.
    pub const fn max_entries(self) -> u64 {
        self.max_entries
    }

    /// Returns the maximum uncompressed size of one entry.
    pub const fn max_entry_uncompressed_bytes(self) -> u64 {
        self.max_entry_uncompressed_bytes
    }

    /// Returns the maximum total uncompressed size.
    ///
    /// The limit is applied twice: once against the sizes the central directory
    /// declares, and again against the bytes entries actually produce as they are
    /// read.
    pub const fn max_total_uncompressed_bytes(self) -> u64 {
        self.max_total_uncompressed_bytes
    }

    /// Returns the maximum allowed uncompressed-to-compressed size ratio.
    ///
    /// The ratio is computed from central-directory metadata, which the input
    /// controls. Reading an entry rejects any package whose real output length
    /// disagrees with that metadata, so an entry cannot under-declare its size to
    /// stay below this ratio and still be read.
    pub const fn max_compression_ratio(self) -> u64 {
        self.max_compression_ratio
    }

    /// Returns the maximum nesting depth for required XML parts.
    pub const fn max_xml_depth(self) -> u64 {
        self.max_xml_depth
    }

    /// Returns the maximum attribute count on one XML element.
    pub const fn max_xml_attributes(self) -> u64 {
        self.max_xml_attributes
    }

    /// Returns the maximum workbook sheet count.
    pub const fn max_sheets(self) -> u64 {
        self.max_sheets
    }

    /// Returns the maximum cell-element count in one worksheet.
    pub const fn max_cells_per_sheet(self) -> u64 {
        self.max_cells_per_sheet
    }

    /// Returns the maximum cell-element count across all worksheets.
    pub const fn max_total_cells(self) -> u64 {
        self.max_total_cells
    }

    /// Returns the maximum unique shared-string count.
    pub const fn max_shared_strings(self) -> u64 {
        self.max_shared_strings
    }

    /// Returns the maximum UTF-8 byte length of one shared string.
    pub const fn max_shared_string_bytes(self) -> u64 {
        self.max_shared_string_bytes
    }

    /// Returns the maximum combined UTF-8 byte length of shared strings.
    pub const fn max_total_shared_string_bytes(self) -> u64 {
        self.max_total_shared_string_bytes
    }

    /// Returns the maximum workbook defined-name count.
    pub const fn max_defined_names(self) -> u64 {
        self.max_defined_names
    }

    /// Returns the maximum decoded UTF-8 byte length of one formula.
    pub const fn max_formula_bytes(self) -> u64 {
        self.max_formula_bytes
    }

    /// Returns the maximum combined UTF-8 byte length of materialized formulas.
    pub const fn max_total_formula_bytes(self) -> u64 {
        self.max_total_formula_bytes
    }

    /// Returns the maximum merged-range declaration count across all worksheets.
    pub const fn max_merged_ranges(self) -> u64 {
        self.max_merged_ranges
    }

    /// Returns the maximum phonetic run count in one string item.
    pub const fn max_phonetic_runs_per_item(self) -> u64 {
        self.max_phonetic_runs_per_item
    }

    /// Returns the maximum combined phonetic run count across unique string items.
    pub const fn max_total_phonetic_runs(self) -> u64 {
        self.max_total_phonetic_runs
    }

    /// Returns the maximum number of cells that may reference annotations.
    pub const fn max_annotated_cells(self) -> u64 {
        self.max_annotated_cells
    }

    /// Returns the maximum UTF-8 byte length of one phonetic run.
    pub const fn max_phonetic_text_bytes(self) -> u64 {
        self.max_phonetic_text_bytes
    }

    /// Returns the maximum combined UTF-8 byte length of unique phonetic string items.
    pub const fn max_total_phonetic_text_bytes(self) -> u64 {
        self.max_total_phonetic_text_bytes
    }

    /// Replaces the input archive byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_archive_bytes(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_archive_bytes = nonzero(MAX_ARCHIVE_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the ZIP entry count limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_entries(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_entries = nonzero(MAX_ENTRIES, value)?;
        Ok(self)
    }

    /// Replaces the per-entry uncompressed byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_entry_uncompressed_bytes(
        mut self,
        value: u64,
    ) -> Result<Self, ReadOptionsError> {
        self.max_entry_uncompressed_bytes = nonzero(MAX_ENTRY_UNCOMPRESSED_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the total uncompressed byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_total_uncompressed_bytes(
        mut self,
        value: u64,
    ) -> Result<Self, ReadOptionsError> {
        self.max_total_uncompressed_bytes = nonzero(MAX_TOTAL_UNCOMPRESSED_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the compression ratio limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_compression_ratio(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_compression_ratio = nonzero(MAX_COMPRESSION_RATIO, value)?;
        Ok(self)
    }

    /// Replaces the required XML depth limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_xml_depth(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_xml_depth = nonzero(MAX_XML_DEPTH, value)?;
        Ok(self)
    }

    /// Replaces the per-element XML attribute limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_xml_attributes(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_xml_attributes = nonzero(MAX_XML_ATTRIBUTES, value)?;
        Ok(self)
    }

    /// Replaces the workbook sheet-count limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_sheets(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_sheets = nonzero(MAX_SHEETS, value)?;
        Ok(self)
    }

    /// Replaces the per-worksheet cell-element limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_cells_per_sheet(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_cells_per_sheet = nonzero(MAX_CELLS_PER_SHEET, value)?;
        Ok(self)
    }

    /// Replaces the workbook-wide cell-element limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_total_cells(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_total_cells = nonzero(MAX_TOTAL_CELLS, value)?;
        Ok(self)
    }

    /// Replaces the shared-string count limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_shared_strings(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_shared_strings = nonzero(MAX_SHARED_STRINGS, value)?;
        Ok(self)
    }

    /// Replaces the per-shared-string UTF-8 byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_shared_string_bytes(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_shared_string_bytes = nonzero(MAX_SHARED_STRING_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the total shared-string UTF-8 byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_total_shared_string_bytes(
        mut self,
        value: u64,
    ) -> Result<Self, ReadOptionsError> {
        self.max_total_shared_string_bytes = nonzero(MAX_TOTAL_SHARED_STRING_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the workbook defined-name count limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_defined_names(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_defined_names = nonzero(MAX_DEFINED_NAMES, value)?;
        Ok(self)
    }

    /// Replaces the per-formula decoded UTF-8 byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_formula_bytes(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_formula_bytes = nonzero(MAX_FORMULA_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the combined materialized-formula UTF-8 byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_total_formula_bytes(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_total_formula_bytes = nonzero(MAX_TOTAL_FORMULA_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the workbook-wide merged-range declaration limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_merged_ranges(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_merged_ranges = nonzero(MAX_MERGED_RANGES, value)?;
        Ok(self)
    }

    /// Replaces the per-item phonetic run-count limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_phonetic_runs_per_item(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_phonetic_runs_per_item = nonzero(MAX_PHONETIC_RUNS_PER_ITEM, value)?;
        Ok(self)
    }

    /// Replaces the workbook-wide unique phonetic run-count limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_total_phonetic_runs(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_total_phonetic_runs = nonzero(MAX_TOTAL_PHONETIC_RUNS, value)?;
        Ok(self)
    }

    /// Replaces the annotated-cell reference limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_annotated_cells(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_annotated_cells = nonzero(MAX_ANNOTATED_CELLS, value)?;
        Ok(self)
    }

    /// Replaces the per-run phonetic UTF-8 byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_phonetic_text_bytes(mut self, value: u64) -> Result<Self, ReadOptionsError> {
        self.max_phonetic_text_bytes = nonzero(MAX_PHONETIC_TEXT_BYTES, value)?;
        Ok(self)
    }

    /// Replaces the total unique phonetic UTF-8 byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`ReadOptionsError::ZeroLimit`] when `value` is zero.
    pub fn with_max_total_phonetic_text_bytes(
        mut self,
        value: u64,
    ) -> Result<Self, ReadOptionsError> {
        self.max_total_phonetic_text_bytes = nonzero(MAX_TOTAL_PHONETIC_TEXT_BYTES, value)?;
        Ok(self)
    }
}

impl Default for ReadLimits {
    fn default() -> Self {
        Self {
            max_archive_bytes: 256 * 1024 * 1024,
            max_entries: 10_000,
            max_entry_uncompressed_bytes: 64 * 1024 * 1024,
            max_total_uncompressed_bytes: 512 * 1024 * 1024,
            max_compression_ratio: 200,
            max_xml_depth: 128,
            max_xml_attributes: 256,
            max_sheets: 1_024,
            max_cells_per_sheet: 2_000_000,
            max_total_cells: 5_000_000,
            max_shared_strings: 2_000_000,
            max_shared_string_bytes: 1024 * 1024,
            max_total_shared_string_bytes: 256 * 1024 * 1024,
            max_defined_names: 100_000,
            max_formula_bytes: 1024 * 1024,
            max_total_formula_bytes: 256 * 1024 * 1024,
            max_merged_ranges: 100_000,
            max_phonetic_runs_per_item: 32_768,
            max_total_phonetic_runs: 2_000_000,
            max_annotated_cells: 2_000_000,
            max_phonetic_text_bytes: 1024 * 1024,
            max_total_phonetic_text_bytes: 256 * 1024 * 1024,
        }
    }
}

/// Read behavior and resource budgets for XLSX input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReadOptions {
    limits: ReadLimits,
}

impl ReadOptions {
    /// Constructs options from validated limits.
    pub const fn new(limits: ReadLimits) -> Self {
        Self { limits }
    }

    /// Returns the configured resource limits.
    pub const fn limits(self) -> ReadLimits {
        self.limits
    }
}

fn nonzero(name: &'static str, value: u64) -> Result<u64, ReadOptionsError> {
    if value == 0 {
        return Err(ReadOptionsError::ZeroLimit { name });
    }
    Ok(value)
}
