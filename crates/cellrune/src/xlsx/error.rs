use std::error::Error;
use std::fmt;

use crate::SourceId;

const MESSAGE_ZERO_LIMIT: &str = "read limit must be greater than zero";

pub(crate) mod detail {
    pub(crate) const ZIP_ENTRY_NAME_NOT_UTF8: &str = "ZIP entry name is not UTF-8";
    pub(crate) const CONTENT_TYPE_PART_NOT_ABSOLUTE: &str =
        "Override PartName must be package-absolute";
    pub(crate) const UNEXPECTED_CLOSING_ELEMENT: &str = "unexpected closing element";
    pub(crate) const UNEXPECTED_ROOT_ELEMENT: &str = "unexpected root element";
    pub(crate) const DUPLICATE_DEFAULT_EXTENSION: &str = "duplicate default extension";
    pub(crate) const DUPLICATE_OVERRIDE_PART: &str = "duplicate override part";
    pub(crate) const UNKNOWN_TARGET_MODE: &str = "unknown TargetMode";
    pub(crate) const DUPLICATE_RELATIONSHIP_ID: &str = "duplicate relationship ID";
    pub(crate) const MISSING_ATTRIBUTE: &str = "missing attribute";
    pub(crate) const EMPTY_ATTRIBUTE: &str = "empty attribute";
    pub(crate) const EXTERNAL_WORKSHEET: &str = "worksheet relationship is external";
    pub(crate) const DUPLICATE_WORKSHEET_TARGET: &str = "duplicate worksheet relationship target";
    pub(crate) const EXTERNAL_SUPPORT_PART: &str =
        "required workbook support relationship is external";
    pub(crate) const DUPLICATE_SINGLETON_RELATIONSHIP: &str = "duplicate singleton relationship";
    pub(crate) const MISSING_CONTENT_TYPE: &str = "required part has no content type";
    pub(crate) const DUPLICATE_SHEET_RELATIONSHIP: &str =
        "multiple sheets reference the same worksheet relationship";
    pub(crate) const UNKNOWN_SHEET_RELATIONSHIP: &str =
        "sheet references an unknown worksheet relationship";
    pub(crate) const DUPLICATE_NUMBER_FORMAT: &str = "duplicate number format identifier";
    pub(crate) const SHARED_STRING_PART_REQUIRED: &str =
        "shared-string cell is present without a shared string part";
    pub(crate) const METADATA_PART_REQUIRED: &str =
        "cell metadata index is present without a metadata part";
    pub(crate) const DUPLICATE_SHARED_FORMULA_GROUP: &str = "duplicate shared formula group anchor";
    pub(crate) const UNKNOWN_SHARED_FORMULA_GROUP: &str = "unknown shared formula group";
    pub(crate) const SHARED_FORMULA_OUTSIDE_RANGE: &str =
        "shared formula follower is outside the anchor range";
    pub(crate) const SHARED_FORMULA_SHIFT_FAILED: &str = "shared formula reference shift failed";
}

pub(crate) mod compatibility {
    pub(crate) const INVALID_SAVED_RESULT_CODE: &str = "xlsx.saved_result.invalid";
    pub(crate) const UNSUPPORTED_SAVED_RESULT_CODE: &str = "xlsx.saved_result.unsupported_type";
    pub(crate) const EXTERNAL_LINK_CODE: &str = "xlsx.external_link";
    pub(crate) const EXTERNAL_LINK_MESSAGE: &str =
        "workbook contains an external link that is not opened automatically";
    pub(crate) const MACRO_CODE: &str = "xlsx.macro";
    pub(crate) const MACRO_MESSAGE: &str =
        "workbook contains a macro relationship that is never executed";
    pub(crate) const PHONETIC_OVERLAP_CODE: &str = "xlsx.phonetic.overlap";
    pub(crate) const PHONETIC_OVERLAP_MESSAGE: &str =
        "phonetic runs overlap or are not in ascending order; source order was preserved";
    pub(crate) const PRESERVED_PANE_CODE: &str = "xlsx.pane.preserved";
    pub(crate) const PRESERVED_PANE_MESSAGE: &str =
        "worksheet pane state is preserved but not exposed as a frozen pane";
    pub(crate) const MERGED_RANGE_INVALID_CODE: &str = "xlsx.merged_range.invalid";
    pub(crate) const MERGED_RANGE_INVALID_MESSAGE: &str =
        "merged range declaration is invalid and was dropped";
    pub(crate) const MERGED_RANGE_MISSING_REF: &str = "missing ref attribute";
    pub(crate) const MERGED_RANGE_SINGLE_CELL_CODE: &str = "xlsx.merged_range.single_cell";
    pub(crate) const MERGED_RANGE_SINGLE_CELL_MESSAGE: &str =
        "single-cell merged range carries no merge semantics and was dropped";
    pub(crate) const MERGED_RANGE_OVERLAP_CODE: &str = "xlsx.merged_range.overlap";
    pub(crate) const MERGED_RANGE_OVERLAP_MESSAGE: &str =
        "merged range overlaps an earlier merged range and was dropped";
    pub(crate) const TABLE_INVALID_CODE: &str = "xlsx.table.invalid";
    pub(crate) const TABLE_INVALID_MESSAGE: &str = "table definition is invalid and was dropped";
    pub(crate) const TABLE_NORMALIZED_CODE: &str = "xlsx.table.normalized";
    pub(crate) const TABLE_NORMALIZED_MESSAGE: &str =
        "table metadata was normalized from its semantic children";
    pub(crate) const TABLE_DUPLICATE_DISPLAY_NAME_CODE: &str = "xlsx.table.duplicate_display_name";
    pub(crate) const TABLE_DUPLICATE_DISPLAY_NAME_MESSAGE: &str =
        "table display name duplicates an earlier table and was dropped";
    pub(crate) const TABLE_DUPLICATE_ID_CODE: &str = "xlsx.table.duplicate_id";
    pub(crate) const TABLE_DUPLICATE_ID_MESSAGE: &str =
        "table ID duplicates an earlier table and was dropped";
    pub(crate) const TABLE_DUPLICATE_PROGRAMMATIC_NAME_CODE: &str =
        "xlsx.table.duplicate_programmatic_name";
    pub(crate) const TABLE_DUPLICATE_PROGRAMMATIC_NAME_MESSAGE: &str =
        "programmatic table name duplicates an earlier table on the worksheet and was dropped";
    pub(crate) const TABLE_DEFINED_NAME_CONFLICT_CODE: &str = "xlsx.table.display_name_conflict";
    pub(crate) const TABLE_DEFINED_NAME_CONFLICT_MESSAGE: &str =
        "table display name conflicts with a defined name and was dropped";
    pub(crate) const TABLE_OVERLAP_CODE: &str = "xlsx.table.overlap";
    pub(crate) const TABLE_OVERLAP_MESSAGE: &str =
        "table range overlaps another table and was dropped";
    pub(crate) const TABLE_UNRESOLVED_RELATIONSHIP: &str = "unresolved table relationship id";
    pub(crate) const TABLE_MISSING_RELATIONSHIP_ID: &str = "missing table relationship id";
}

/// Invalid caller-provided reader configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReadOptionsError {
    /// A resource limit was set to zero.
    ZeroLimit {
        /// Stable name of the limit that was set to zero.
        name: &'static str,
    },
}

impl fmt::Display for ReadOptionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit { name } => write!(formatter, "{MESSAGE_ZERO_LIMIT}: {name}"),
        }
    }
}

impl Error for ReadOptionsError {}

/// Stable machine-readable failure codes for XLSX file errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum XlsxErrorCode {
    /// The input could not be read or sought.
    Io,
    /// ZIP structures are malformed.
    InvalidZip,
    /// Input archive bytes exceed the configured limit.
    ArchiveTooLarge,
    /// ZIP entry count exceeds the configured limit.
    TooManyEntries,
    /// One uncompressed entry exceeds the configured limit.
    EntryTooLarge,
    /// Total uncompressed bytes exceed the configured limit.
    TotalUncompressedTooLarge,
    /// An entry produced a different byte count than its central directory declared.
    DeclaredSizeMismatch,
    /// An entry exceeds the configured compression ratio.
    CompressionRatioExceeded,
    /// Compressed byte ranges overlap.
    OverlappingEntries,
    /// An encrypted entry cannot be safely read without credentials.
    EncryptedEntry,
    /// An entry uses a compression method outside the XLSX reader subset.
    UnsupportedCompression,
    /// A package part name is malformed or escapes the package root.
    InvalidPartName,
    /// Two ZIP entries normalize to the same package part.
    DuplicatePart,
    /// A required package part is missing.
    MissingPart,
    /// Required XML is malformed.
    InvalidXml,
    /// Required XML exceeds the configured nesting depth.
    XmlDepthExceeded,
    /// One XML element exceeds the configured attribute count.
    XmlAttributesExceeded,
    /// DTD or another forbidden XML construct is present.
    ForbiddenXmlConstruct,
    /// `[Content_Types].xml` is incomplete or inconsistent.
    InvalidContentTypes,
    /// A Relationship part is incomplete or inconsistent.
    InvalidRelationships,
    /// No internal Office Document relationship identifies the workbook.
    MissingWorkbookRelationship,
    /// More than one internal Office Document relationship identifies a workbook.
    DuplicateWorkbookRelationship,
    /// The Office Document relationship points outside the package.
    ExternalWorkbookRelationship,
    /// A Relationship target cannot be normalized safely.
    InvalidRelationshipTarget,
    /// The workbook has no internal worksheet relationship.
    MissingWorksheetRelationship,
    /// A required part has a content type outside the supported XLSX subset.
    UnsupportedContentType,
    /// Workbook metadata is malformed or inconsistent with relationships.
    InvalidWorkbook,
    /// Workbook sheet count exceeds the configured limit.
    TooManySheets,
    /// Workbook sheet metadata references no matching worksheet relationship.
    MissingSheetRelationship,
    /// Shared-string XML or its declared counts are invalid.
    InvalidSharedStrings,
    /// Shared-string count exceeds the configured limit.
    TooManySharedStrings,
    /// One decoded shared string exceeds the configured limit.
    SharedStringTooLarge,
    /// Combined decoded shared strings exceed the configured limit.
    TotalSharedStringsTooLarge,
    /// Styles XML is malformed or inconsistent.
    InvalidStyles,
    /// A cell references a missing cell-format record.
    InvalidStyleIndex,
    /// Worksheet XML structure is malformed.
    InvalidWorksheet,
    /// A worksheet cell reference is malformed or outside Excel bounds.
    InvalidCellReference,
    /// One worksheet exceeds the configured cell-element limit.
    TooManyCellsInSheet,
    /// The workbook exceeds the configured total cell-element limit.
    TooManyCells,
    /// A literal cell type is outside the supported XLSX reader subset.
    UnsupportedCellType,
    /// A literal cell value does not match its declared type.
    InvalidCellValue,
    /// Workbook defined-name metadata is malformed.
    InvalidDefinedName,
    /// Workbook defined-name count exceeds the configured limit.
    TooManyDefinedNames,
    /// Formula text exceeds the configured decoded byte limit.
    FormulaTooLarge,
    /// Materialized formula text exceeds the configured workbook-wide byte limit.
    TotalFormulaBytesTooLarge,
    /// Formula container metadata is malformed or inconsistent.
    InvalidFormulaMetadata,
    /// Cell metadata or a referenced metadata index is malformed.
    InvalidCellMetadata,
    /// Phonetic run, property, or visibility metadata is malformed.
    InvalidPhoneticMetadata,
    /// A phonetic run count exceeds the configured limit.
    TooManyPhoneticRuns,
    /// One decoded phonetic string exceeds the configured limit.
    PhoneticTextTooLarge,
    /// Combined decoded phonetic strings exceed the configured limit.
    TotalPhoneticTextTooLarge,
    /// Annotated cell references exceed the configured limit.
    TooManyAnnotatedCells,
    /// Frozen-pane or sheet-view metadata is malformed.
    InvalidFrozenPane,
    /// Merged-range declarations exceed the configured limit.
    TooManyMergedRanges,
    /// Referenced table parts exceed the configured limit.
    TooManyTables,
    /// One table definition exceeds the configured column-count limit.
    TooManyTableColumns,
    /// A table, display, or column name exceeds the configured byte limit.
    TableNameTooLarge,
    /// One table definition exceeds the configured filter-item count limit.
    TooManyTableFilterItems,
    /// Filter and sort attribute text in one table exceeds the configured byte limit.
    TableFilterTextTooLarge,
}

impl XlsxErrorCode {
    /// Returns a stable dotted identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Io => "xlsx.io",
            Self::InvalidZip => "xlsx.invalid_zip",
            Self::ArchiveTooLarge => "xlsx.archive_too_large",
            Self::TooManyEntries => "xlsx.too_many_entries",
            Self::EntryTooLarge => "xlsx.entry_too_large",
            Self::TotalUncompressedTooLarge => "xlsx.total_uncompressed_too_large",
            Self::DeclaredSizeMismatch => "xlsx.declared_size_mismatch",
            Self::CompressionRatioExceeded => "xlsx.compression_ratio_exceeded",
            Self::OverlappingEntries => "xlsx.overlapping_entries",
            Self::EncryptedEntry => "xlsx.encrypted_entry",
            Self::UnsupportedCompression => "xlsx.unsupported_compression",
            Self::InvalidPartName => "xlsx.invalid_part_name",
            Self::DuplicatePart => "xlsx.duplicate_part",
            Self::MissingPart => "xlsx.missing_part",
            Self::InvalidXml => "xlsx.invalid_xml",
            Self::XmlDepthExceeded => "xlsx.xml_depth_exceeded",
            Self::XmlAttributesExceeded => "xlsx.xml_attributes_exceeded",
            Self::ForbiddenXmlConstruct => "xlsx.forbidden_xml_construct",
            Self::InvalidContentTypes => "xlsx.invalid_content_types",
            Self::InvalidRelationships => "xlsx.invalid_relationships",
            Self::MissingWorkbookRelationship => "xlsx.missing_workbook_relationship",
            Self::DuplicateWorkbookRelationship => "xlsx.duplicate_workbook_relationship",
            Self::ExternalWorkbookRelationship => "xlsx.external_workbook_relationship",
            Self::InvalidRelationshipTarget => "xlsx.invalid_relationship_target",
            Self::MissingWorksheetRelationship => "xlsx.missing_worksheet_relationship",
            Self::UnsupportedContentType => "xlsx.unsupported_content_type",
            Self::InvalidWorkbook => "xlsx.invalid_workbook",
            Self::TooManySheets => "xlsx.too_many_sheets",
            Self::MissingSheetRelationship => "xlsx.missing_sheet_relationship",
            Self::InvalidSharedStrings => "xlsx.invalid_shared_strings",
            Self::TooManySharedStrings => "xlsx.too_many_shared_strings",
            Self::SharedStringTooLarge => "xlsx.shared_string_too_large",
            Self::TotalSharedStringsTooLarge => "xlsx.total_shared_strings_too_large",
            Self::InvalidStyles => "xlsx.invalid_styles",
            Self::InvalidStyleIndex => "xlsx.invalid_style_index",
            Self::InvalidWorksheet => "xlsx.invalid_worksheet",
            Self::InvalidCellReference => "xlsx.invalid_cell_reference",
            Self::TooManyCellsInSheet => "xlsx.too_many_cells_in_sheet",
            Self::TooManyCells => "xlsx.too_many_cells",
            Self::UnsupportedCellType => "xlsx.unsupported_cell_type",
            Self::InvalidCellValue => "xlsx.invalid_cell_value",
            Self::InvalidDefinedName => "xlsx.invalid_defined_name",
            Self::TooManyDefinedNames => "xlsx.too_many_defined_names",
            Self::FormulaTooLarge => "xlsx.formula_too_large",
            Self::TotalFormulaBytesTooLarge => "xlsx.total_formula_bytes_too_large",
            Self::InvalidFormulaMetadata => "xlsx.invalid_formula_metadata",
            Self::InvalidCellMetadata => "xlsx.invalid_cell_metadata",
            Self::InvalidPhoneticMetadata => "xlsx.invalid_phonetic_metadata",
            Self::TooManyPhoneticRuns => "xlsx.too_many_phonetic_runs",
            Self::PhoneticTextTooLarge => "xlsx.phonetic_text_too_large",
            Self::TotalPhoneticTextTooLarge => "xlsx.total_phonetic_text_too_large",
            Self::TooManyAnnotatedCells => "xlsx.too_many_annotated_cells",
            Self::InvalidFrozenPane => "xlsx.invalid_frozen_pane",
            Self::TooManyMergedRanges => "xlsx.too_many_merged_ranges",
            Self::TooManyTables => "xlsx.too_many_tables",
            Self::TooManyTableColumns => "xlsx.too_many_table_columns",
            Self::TableNameTooLarge => "xlsx.table_name_too_large",
            Self::TooManyTableFilterItems => "xlsx.too_many_table_filter_items",
            Self::TableFilterTextTooLarge => "xlsx.table_filter_text_too_large",
        }
    }

    const fn message(self) -> &'static str {
        match self {
            Self::Io => "failed to read XLSX input",
            Self::InvalidZip => "invalid ZIP archive",
            Self::ArchiveTooLarge => "archive byte length exceeds the configured limit",
            Self::TooManyEntries => "ZIP entry count exceeds the configured limit",
            Self::EntryTooLarge => "uncompressed ZIP entry exceeds the configured limit",
            Self::TotalUncompressedTooLarge => {
                "total uncompressed ZIP size exceeds the configured limit"
            }
            Self::DeclaredSizeMismatch => {
                "ZIP entry size disagrees with the declared central-directory size"
            }
            Self::CompressionRatioExceeded => {
                "ZIP entry compression ratio exceeds the configured limit"
            }
            Self::OverlappingEntries => "ZIP entries have overlapping compressed data",
            Self::EncryptedEntry => "encrypted ZIP entries are not supported",
            Self::UnsupportedCompression => "ZIP compression method is not supported",
            Self::InvalidPartName => "invalid package part name",
            Self::DuplicatePart => "duplicate normalized package part",
            Self::MissingPart => "required package part is missing",
            Self::InvalidXml => "invalid XML in required package part",
            Self::XmlDepthExceeded => "XML nesting depth exceeds the configured limit",
            Self::XmlAttributesExceeded => "XML attribute count exceeds the configured limit",
            Self::ForbiddenXmlConstruct => "forbidden XML construct in required package part",
            Self::InvalidContentTypes => "invalid package content type declarations",
            Self::InvalidRelationships => "invalid package relationship declarations",
            Self::MissingWorkbookRelationship => "workbook relationship is missing",
            Self::DuplicateWorkbookRelationship => "multiple workbook relationships are present",
            Self::ExternalWorkbookRelationship => "workbook relationship is external",
            Self::InvalidRelationshipTarget => "relationship target escapes the package",
            Self::MissingWorksheetRelationship => "worksheet relationship is missing",
            Self::UnsupportedContentType => "package part content type is not supported",
            Self::InvalidWorkbook => "invalid workbook metadata",
            Self::TooManySheets => "workbook sheet count exceeds the configured limit",
            Self::MissingSheetRelationship => {
                "workbook sheet relationship does not resolve to a worksheet"
            }
            Self::InvalidSharedStrings => "invalid shared string table",
            Self::TooManySharedStrings => "shared string count exceeds the configured limit",
            Self::SharedStringTooLarge => "decoded shared string exceeds the configured limit",
            Self::TotalSharedStringsTooLarge => {
                "combined decoded shared strings exceed the configured limit"
            }
            Self::InvalidStyles => "invalid workbook styles",
            Self::InvalidStyleIndex => "cell style index is out of range",
            Self::InvalidWorksheet => "invalid worksheet metadata",
            Self::InvalidCellReference => "invalid worksheet cell reference",
            Self::TooManyCellsInSheet => "worksheet cell count exceeds the configured limit",
            Self::TooManyCells => "workbook cell count exceeds the configured limit",
            Self::UnsupportedCellType => "worksheet cell type is not supported",
            Self::InvalidCellValue => "worksheet cell value is invalid",
            Self::InvalidDefinedName => "workbook defined name is invalid",
            Self::TooManyDefinedNames => "workbook defined name count exceeds the configured limit",
            Self::FormulaTooLarge => "decoded formula text exceeds the configured limit",
            Self::TotalFormulaBytesTooLarge => {
                "combined materialized formula text exceeds the configured limit"
            }
            Self::InvalidFormulaMetadata => "formula metadata is invalid",
            Self::InvalidCellMetadata => "cell metadata is invalid",
            Self::InvalidPhoneticMetadata => "phonetic metadata is invalid",
            Self::TooManyPhoneticRuns => "phonetic run count exceeds the configured limit",
            Self::PhoneticTextTooLarge => "decoded phonetic text exceeds the configured limit",
            Self::TotalPhoneticTextTooLarge => {
                "combined decoded phonetic text exceeds the configured limit"
            }
            Self::TooManyAnnotatedCells => "annotated cell count exceeds the configured limit",
            Self::InvalidFrozenPane => "frozen pane metadata is invalid",
            Self::TooManyMergedRanges => "workbook merged-range count exceeds the configured limit",
            Self::TooManyTables => "workbook table count exceeds the configured limit",
            Self::TooManyTableColumns => "table column count exceeds the configured limit",
            Self::TableNameTooLarge => "table name exceeds the configured byte limit",
            Self::TooManyTableFilterItems => "table filter item count exceeds the configured limit",
            Self::TableFilterTextTooLarge => {
                "table filter and sort text exceeds the configured byte limit"
            }
        }
    }
}

/// A source-linked XLSX read failure with a stable error code.
#[derive(Debug)]
pub struct XlsxReadError {
    code: XlsxErrorCode,
    detail: Option<Box<str>>,
    source_id: Option<SourceId>,
    cause: Option<Box<dyn Error + Send + Sync>>,
}

impl XlsxReadError {
    pub(crate) const fn new(code: XlsxErrorCode) -> Self {
        Self {
            code,
            detail: None,
            source_id: None,
            cause: None,
        }
    }

    pub(crate) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into().into_boxed_str());
        self
    }

    pub(crate) fn at_source(mut self, source_id: SourceId) -> Self {
        self.source_id = Some(source_id);
        self
    }

    pub(crate) fn with_cause(mut self, cause: impl Error + Send + Sync + 'static) -> Self {
        self.cause = Some(Box::new(cause));
        self
    }

    /// Returns the stable error code.
    pub const fn code(&self) -> XlsxErrorCode {
        self.code
    }

    /// Returns the package source identifier, when known.
    pub const fn source_id(&self) -> Option<&SourceId> {
        self.source_id.as_ref()
    }

    /// Returns source-specific context that supplements the stable error code.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

impl fmt::Display for XlsxReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code.message())?;
        if let Some(detail) = &self.detail {
            write!(formatter, ": {detail}")?;
        }
        Ok(())
    }
}

impl Error for XlsxReadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.cause
            .as_deref()
            .map(|cause| cause as &(dyn Error + 'static))
    }
}
