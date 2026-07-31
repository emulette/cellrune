use std::error::Error;
use std::fmt;

const MESSAGE_ROW_OUT_OF_RANGE: &str = "row is outside the supported Excel range";
const MESSAGE_COLUMN_OUT_OF_RANGE: &str = "column is outside the supported Excel range";
const MESSAGE_CELL_ADDRESS_INVALID: &str = "cell address is not valid A1 notation";
const MESSAGE_RANGE_REVERSED: &str = "range start must not be after range end";
const MESSAGE_SHEET_ID_ZERO: &str = "sheet ID must be greater than zero";
const MESSAGE_SHEET_NAME_EMPTY: &str = "sheet name must not be empty";
const MESSAGE_SHEET_NAME_TOO_LONG: &str = "sheet name exceeds 31 UTF-16 code units";
const MESSAGE_SHEET_NAME_INVALID_CHARACTER: &str = "sheet name contains an invalid character";
const MESSAGE_SHEET_NAME_APOSTROPHE_BOUNDARY: &str =
    "sheet name must not begin or end with an apostrophe";
const MESSAGE_DUPLICATE_SHEET_ID: &str = "workbook contains a duplicate sheet ID";
const MESSAGE_DUPLICATE_SHEET_NAME: &str =
    "workbook contains a duplicate case-insensitive sheet name";
const MESSAGE_DUPLICATE_CELL: &str = "sheet contains a duplicate cell address";
const MESSAGE_DEFINED_NAME_EMPTY: &str = "defined name must not be empty";
const MESSAGE_DEFINED_NAME_TOO_LONG: &str = "defined name exceeds 255 UTF-16 code units";
const MESSAGE_DEFINED_NAME_CONTROL: &str = "defined name contains a control character";
const MESSAGE_DEFINED_NAME_UNKNOWN_SHEET: &str = "defined name scope references an unknown sheet";
const MESSAGE_DUPLICATE_DEFINED_NAME: &str =
    "workbook contains a duplicate case-insensitive defined name in one scope";
const MESSAGE_NON_FINITE_NUMBER: &str = "cell number must be finite";
const MESSAGE_FORMULA_EMPTY: &str = "formula text must not be empty";
const MESSAGE_XLSX_FORMULA_EQUALS: &str =
    "XLSX formula text must not include a leading equals sign";
const MESSAGE_USER_FORMULA_EQUALS: &str = "user formula text must begin with an equals sign";
const MESSAGE_SOURCE_ID_EMPTY: &str = "source ID must not be empty";
const MESSAGE_DIAGNOSTIC_CODE_INVALID: &str =
    "diagnostic code must use lowercase dotted identifiers";
const MESSAGE_PROVIDER_NAME_EMPTY: &str = "provider name must not be empty";
const MESSAGE_PROVIDER_VERSION_EMPTY: &str = "provider version must not be empty";
const MESSAGE_DIAGNOSTIC_MESSAGE_EMPTY: &str = "diagnostic message must not be empty";
const MESSAGE_UNKNOWN_SHEET_ID: &str = "workbook does not contain the requested sheet ID";
const MESSAGE_CELL_NOT_FOUND: &str = "workbook does not contain the requested cell";
const MESSAGE_SHEET_ID_EXHAUSTED: &str = "workbook cannot allocate another sheet ID";
const MESSAGE_LAST_VISIBLE_SHEET: &str = "workbook must retain at least one visible sheet";
const MESSAGE_SEMANTIC_REVISION_EXHAUSTED: &str = "workbook semantic revision is exhausted";
const MESSAGE_PRESENTATION_REVISION_EXHAUSTED: &str = "workbook presentation revision is exhausted";
const MESSAGE_PHONETIC_RANGE_EMPTY: &str = "phonetic text range start must be less than its end";
const MESSAGE_PHONETIC_RANGE_OUT_OF_BOUNDS: &str = "phonetic text range exceeds the base text";
const MESSAGE_PHONETIC_RANGE_SPLITS_SURROGATE: &str =
    "phonetic text range must use UTF-16 character boundaries";
const MESSAGE_PHONETIC_RUNS_OUT_OF_ORDER: &str =
    "phonetic runs must be ordered and non-overlapping";
const MESSAGE_PHONETIC_TEXT_EMPTY: &str = "phonetic text must not be empty";
const MESSAGE_PHONETIC_TEXT_INVALID_CHARACTER: &str =
    "phonetic text contains a character forbidden by XML 1.0";
const MESSAGE_PHONETICS_REQUIRE_TEXT_CELL: &str =
    "phonetic annotations require a literal text cell";
const MESSAGE_PHONETIC_FONT_ID_UNSUPPORTED: &str =
    "phonetic authoring currently supports only the default font record";
const MESSAGE_ANNOTATED_TEXT_REPLACEMENT_REQUIRED: &str =
    "annotated text must be cleared or replaced atomically";
const MESSAGE_FROZEN_ROWS_OUT_OF_RANGE: &str =
    "frozen row count cannot produce a valid top-left cell";
const MESSAGE_FROZEN_COLUMNS_OUT_OF_RANGE: &str =
    "frozen column count cannot produce a valid top-left cell";
const MESSAGE_TABLE_ID_ZERO: &str = "table ID must be greater than zero";
const MESSAGE_TABLE_COLUMN_ID_ZERO: &str = "table column ID must be greater than zero";
const MESSAGE_TABLE_NAME_EMPTY: &str = "table name must not be empty";
const MESSAGE_TABLE_NAME_TOO_LONG: &str = "table name exceeds 255 UTF-16 code units";
const MESSAGE_TABLE_NAME_INVALID_CHARACTER: &str = "table name contains an invalid character";
const MESSAGE_TABLE_NAME_REFERENCE_CONFLICT: &str =
    "table name must not be an A1 or R1C1 reference";
const MESSAGE_DUPLICATE_TABLE_DISPLAY_NAME: &str =
    "workbook contains a duplicate case-insensitive table display name";
const MESSAGE_DUPLICATE_TABLE_ID: &str = "workbook contains a duplicate table ID";
const MESSAGE_DUPLICATE_TABLE_PROGRAMMATIC_NAME: &str =
    "worksheet contains a duplicate case-insensitive programmatic table name";
const MESSAGE_TABLE_DISPLAY_NAME_CONFLICTS_WITH_DEFINED_NAME: &str =
    "table display name conflicts with a workbook defined name";
const MESSAGE_TABLE_COLUMNS_EMPTY: &str = "table must declare at least one column";
const MESSAGE_TABLE_COLUMN_COUNT_MISMATCH: &str =
    "table column count does not match the table range width";
const MESSAGE_TABLE_COLUMN_NAME_EMPTY: &str = "table column name must not be empty";
const MESSAGE_TABLE_COLUMN_NAME_TOO_LONG: &str = "table column name exceeds 255 UTF-16 code units";
const MESSAGE_DUPLICATE_TABLE_COLUMN_NAME: &str =
    "table contains a duplicate case-insensitive column name";
const MESSAGE_DUPLICATE_TABLE_COLUMN_ID: &str = "table contains a duplicate column identifier";
const MESSAGE_OVERLAPPING_TABLES: &str = "worksheet contains overlapping table ranges";
const MESSAGE_TABLE_ROW_COUNTS_EXCEED_RANGE: &str =
    "table header and totals rows exceed the table range height";
const MESSAGE_INVALID_TABLE_TOTALS_METADATA: &str =
    "table totals label, function, and formula metadata are inconsistent";
const MESSAGE_NUMBER_FORMAT_BUILTIN_ID: &str = "built-in number format ID must be less than 164";
const MESSAGE_NUMBER_FORMAT_CUSTOM_ID: &str = "custom number format ID must be at least 164";
const MESSAGE_NUMBER_FORMAT_CODE_EMPTY: &str = "custom number format code must not be empty";

/// Stable machine-readable code for a format-neutral validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ValidationErrorCode {
    /// A row index is outside the supported Excel range.
    RowOutOfRange,
    /// A column index is outside the supported Excel range.
    ColumnOutOfRange,
    /// A cell address is not valid A1 notation.
    CellAddressInvalid,
    /// A range start is after its end.
    RangeStartAfterEnd,
    /// A sheet identifier is zero.
    SheetIdZero,
    /// A sheet name is empty.
    SheetNameEmpty,
    /// A sheet name exceeds Excel's length limit.
    SheetNameTooLong,
    /// A sheet name contains a forbidden character.
    SheetNameInvalidCharacter,
    /// A sheet name begins or ends with an apostrophe.
    SheetNameApostropheBoundary,
    /// A workbook contains a duplicate sheet identifier.
    DuplicateSheetId,
    /// A workbook contains a duplicate case-insensitive sheet name.
    DuplicateSheetName,
    /// A sheet contains a duplicate cell address.
    DuplicateCell,
    /// A defined name is empty.
    DefinedNameEmpty,
    /// A defined name exceeds Excel's length limit.
    DefinedNameTooLong,
    /// A defined name contains a control character.
    DefinedNameControlCharacter,
    /// A sheet-scoped defined name references an unknown sheet.
    DefinedNameUnknownSheet,
    /// A workbook contains a duplicate defined name in one scope.
    DuplicateDefinedName,
    /// A numeric cell contains NaN or infinity.
    NonFiniteNumber,
    /// Formula text is empty.
    FormulaEmpty,
    /// Stored XLSX formula text includes a leading equals sign.
    XlsxFormulaHasLeadingEquals,
    /// User formula text omits a leading equals sign.
    UserFormulaMissingLeadingEquals,
    /// A source identifier is empty.
    SourceIdEmpty,
    /// A diagnostic code does not follow the stable code grammar.
    DiagnosticCodeInvalid,
    /// A provenance provider name is empty.
    ProviderNameEmpty,
    /// A provenance provider version is empty.
    ProviderVersionEmpty,
    /// A diagnostic message is empty.
    DiagnosticMessageEmpty,
    /// A draft operation references an unknown sheet.
    UnknownSheetId,
    /// A draft operation references a missing sparse cell.
    CellNotFound,
    /// No additional nonzero sheet identifier can be allocated.
    SheetIdExhausted,
    /// A workbook edit would hide its last visible sheet.
    LastVisibleSheet,
    /// The semantic revision counter is exhausted.
    SemanticRevisionExhausted,
    /// The presentation revision counter is exhausted.
    PresentationRevisionExhausted,
    /// A phonetic range is empty or reversed.
    PhoneticRangeEmpty,
    /// A phonetic range exceeds its base text.
    PhoneticRangeOutOfBounds,
    /// A phonetic range boundary splits a UTF-16 surrogate pair.
    PhoneticRangeSplitsSurrogate,
    /// Phonetic runs are unordered or overlapping.
    PhoneticRunsOutOfOrder,
    /// A phonetic run contains no text.
    PhoneticTextEmpty,
    /// Phonetic text contains a character forbidden by XML 1.0.
    PhoneticTextInvalidCharacter,
    /// Phonetic annotations target a non-text cell.
    PhoneticsRequireTextCell,
    /// A phonetic annotation references an unsupported font record.
    PhoneticFontIdUnsupported,
    /// A normal value edit would discard an existing annotation.
    AnnotatedTextReplacementRequired,
    /// A frozen row count cannot be represented within worksheet bounds.
    FrozenRowsOutOfRange,
    /// A frozen column count cannot be represented within worksheet bounds.
    FrozenColumnsOutOfRange,
    /// A built-in number format uses a custom-format identifier.
    BuiltInNumberFormatId,
    /// A custom number format uses a reserved built-in identifier.
    CustomNumberFormatId,
    /// A custom number format code is empty.
    NumberFormatCodeEmpty,
    /// A table identifier is zero.
    TableIdZero,
    /// A table column identifier is zero.
    TableColumnIdZero,
    /// A table name is empty.
    TableNameEmpty,
    /// A table name exceeds Excel's length limit.
    TableNameTooLong,
    /// A table name contains a forbidden character.
    TableNameInvalidCharacter,
    /// A table name is an A1 or R1C1 reference.
    TableNameReferenceConflict,
    /// A workbook contains a duplicate case-insensitive table display name.
    DuplicateTableDisplayName,
    /// A workbook contains a duplicate table identifier.
    DuplicateTableId,
    /// One worksheet contains a duplicate programmatic table name.
    DuplicateTableProgrammaticName,
    /// A table display name conflicts with a defined name.
    TableDisplayNameConflictsWithDefinedName,
    /// A table declares no columns.
    TableColumnsEmpty,
    /// A table's column count disagrees with its range width.
    TableColumnCountMismatch,
    /// A table column name is empty.
    TableColumnNameEmpty,
    /// A table column name exceeds Excel's length limit.
    TableColumnNameTooLong,
    /// A table contains a duplicate case-insensitive column name.
    DuplicateTableColumnName,
    /// A table contains a duplicate column identifier.
    DuplicateTableColumnId,
    /// Two tables on one worksheet have overlapping ranges.
    OverlappingTables,
    /// A table's header and totals rows exceed its range height.
    TableRowCountsExceedRange,
    /// A table column contains inconsistent totals metadata.
    InvalidTableTotalsMetadata,
}

impl ValidationErrorCode {
    /// Returns the stable dotted identifier used across bindings.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RowOutOfRange => "validation.row_out_of_range",
            Self::ColumnOutOfRange => "validation.column_out_of_range",
            Self::CellAddressInvalid => "validation.cell_address_invalid",
            Self::RangeStartAfterEnd => "validation.range_start_after_end",
            Self::SheetIdZero => "validation.sheet_id_zero",
            Self::SheetNameEmpty => "validation.sheet_name_empty",
            Self::SheetNameTooLong => "validation.sheet_name_too_long",
            Self::SheetNameInvalidCharacter => "validation.sheet_name_invalid_character",
            Self::SheetNameApostropheBoundary => "validation.sheet_name_apostrophe_boundary",
            Self::DuplicateSheetId => "validation.duplicate_sheet_id",
            Self::DuplicateSheetName => "validation.duplicate_sheet_name",
            Self::DuplicateCell => "validation.duplicate_cell",
            Self::DefinedNameEmpty => "validation.defined_name_empty",
            Self::DefinedNameTooLong => "validation.defined_name_too_long",
            Self::DefinedNameControlCharacter => "validation.defined_name_control_character",
            Self::DefinedNameUnknownSheet => "validation.defined_name_unknown_sheet",
            Self::DuplicateDefinedName => "validation.duplicate_defined_name",
            Self::NonFiniteNumber => "validation.non_finite_number",
            Self::FormulaEmpty => "validation.formula_empty",
            Self::XlsxFormulaHasLeadingEquals => "validation.xlsx_formula_has_leading_equals",
            Self::UserFormulaMissingLeadingEquals => {
                "validation.user_formula_missing_leading_equals"
            }
            Self::SourceIdEmpty => "validation.source_id_empty",
            Self::DiagnosticCodeInvalid => "validation.diagnostic_code_invalid",
            Self::ProviderNameEmpty => "validation.provider_name_empty",
            Self::ProviderVersionEmpty => "validation.provider_version_empty",
            Self::DiagnosticMessageEmpty => "validation.diagnostic_message_empty",
            Self::UnknownSheetId => "validation.unknown_sheet_id",
            Self::CellNotFound => "validation.cell_not_found",
            Self::SheetIdExhausted => "validation.sheet_id_exhausted",
            Self::LastVisibleSheet => "validation.last_visible_sheet",
            Self::SemanticRevisionExhausted => "validation.semantic_revision_exhausted",
            Self::PresentationRevisionExhausted => "validation.presentation_revision_exhausted",
            Self::PhoneticRangeEmpty => "validation.phonetic_range_empty",
            Self::PhoneticRangeOutOfBounds => "validation.phonetic_range_out_of_bounds",
            Self::PhoneticRangeSplitsSurrogate => "validation.phonetic_range_splits_surrogate",
            Self::PhoneticRunsOutOfOrder => "validation.phonetic_runs_out_of_order",
            Self::PhoneticTextEmpty => "validation.phonetic_text_empty",
            Self::PhoneticTextInvalidCharacter => "validation.phonetic_text_invalid_character",
            Self::PhoneticsRequireTextCell => "validation.phonetics_require_text_cell",
            Self::PhoneticFontIdUnsupported => "validation.phonetic_font_id_unsupported",
            Self::AnnotatedTextReplacementRequired => {
                "validation.annotated_text_replacement_required"
            }
            Self::FrozenRowsOutOfRange => "validation.frozen_rows_out_of_range",
            Self::FrozenColumnsOutOfRange => "validation.frozen_columns_out_of_range",
            Self::BuiltInNumberFormatId => "validation.built_in_number_format_id",
            Self::CustomNumberFormatId => "validation.custom_number_format_id",
            Self::NumberFormatCodeEmpty => "validation.number_format_code_empty",
            Self::TableIdZero => "validation.table_id_zero",
            Self::TableColumnIdZero => "validation.table_column_id_zero",
            Self::TableNameEmpty => "validation.table_name_empty",
            Self::TableNameTooLong => "validation.table_name_too_long",
            Self::TableNameInvalidCharacter => "validation.table_name_invalid_character",
            Self::TableNameReferenceConflict => "validation.table_name_reference_conflict",
            Self::DuplicateTableDisplayName => "validation.duplicate_table_display_name",
            Self::DuplicateTableId => "validation.duplicate_table_id",
            Self::DuplicateTableProgrammaticName => "validation.duplicate_table_programmatic_name",
            Self::TableDisplayNameConflictsWithDefinedName => {
                "validation.table_display_name_conflicts_with_defined_name"
            }
            Self::TableColumnsEmpty => "validation.table_columns_empty",
            Self::TableColumnCountMismatch => "validation.table_column_count_mismatch",
            Self::TableColumnNameEmpty => "validation.table_column_name_empty",
            Self::TableColumnNameTooLong => "validation.table_column_name_too_long",
            Self::DuplicateTableColumnName => "validation.duplicate_table_column_name",
            Self::DuplicateTableColumnId => "validation.duplicate_table_column_id",
            Self::OverlappingTables => "validation.overlapping_tables",
            Self::TableRowCountsExceedRange => "validation.table_row_counts_exceed_range",
            Self::InvalidTableTotalsMetadata => "validation.invalid_table_totals_metadata",
        }
    }
}

/// A violation of a format-neutral workbook invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValidationError {
    /// A row index is not in `1..=1_048_576`.
    RowOutOfRange {
        /// Rejected one-based row index.
        value: u32,
    },
    /// A column index is not in `1..=16_384`.
    ColumnOutOfRange {
        /// Rejected one-based column index.
        value: u32,
    },
    /// A cell address does not use an ASCII column label followed by a one-based row number.
    CellAddressInvalid,
    /// A range start is below or to the right of its end.
    RangeStartAfterEnd,
    /// A sheet ID is zero.
    SheetIdZero,
    /// A sheet name is empty.
    SheetNameEmpty,
    /// A sheet name is longer than Excel's 31 UTF-16 code-unit limit.
    SheetNameTooLong {
        /// Length of the rejected name in UTF-16 code units.
        utf16_len: usize,
    },
    /// A sheet name contains a forbidden character.
    SheetNameInvalidCharacter {
        /// Forbidden character found in the sheet name.
        character: char,
    },
    /// A sheet name begins or ends with an apostrophe.
    SheetNameApostropheBoundary,
    /// Two sheets use the same ID.
    DuplicateSheetId {
        /// Repeated nonzero sheet identifier.
        value: u32,
    },
    /// Two sheets use names that compare equal without case.
    DuplicateSheetName {
        /// Repeated sheet name as supplied by the caller.
        name: String,
    },
    /// A sparse sheet contains the same address more than once.
    DuplicateCell {
        /// One-based row of the repeated address.
        row: u32,
        /// One-based column of the repeated address.
        column: u32,
    },
    /// A defined name is empty.
    DefinedNameEmpty,
    /// A defined name exceeds Excel's 255 UTF-16 code-unit limit.
    DefinedNameTooLong {
        /// Length of the rejected name in UTF-16 code units.
        utf16_len: usize,
    },
    /// A defined name contains a control character.
    DefinedNameControlCharacter {
        /// Control character found in the defined name.
        character: char,
    },
    /// A sheet-scoped defined name references no workbook sheet.
    DefinedNameUnknownSheet {
        /// Missing sheet identifier referenced by the scope.
        sheet_id: u32,
    },
    /// Two defined names compare equal in the same scope.
    DuplicateDefinedName {
        /// Repeated defined name as supplied by the caller.
        name: String,
    },
    /// A numeric cell contains NaN or infinity.
    NonFiniteNumber,
    /// Formula text is empty or only whitespace.
    FormulaEmpty,
    /// Stored XLSX formula text incorrectly includes `=`.
    XlsxFormulaHasLeadingEquals,
    /// User-entered formula text is missing `=`.
    UserFormulaMissingLeadingEquals,
    /// A source identifier is empty.
    SourceIdEmpty,
    /// A diagnostic code does not follow the stable code grammar.
    DiagnosticCodeInvalid,
    /// A provenance provider name is empty.
    ProviderNameEmpty,
    /// A provenance provider version is empty.
    ProviderVersionEmpty,
    /// A diagnostic message is empty.
    DiagnosticMessageEmpty,
    /// A draft operation references no workbook sheet.
    UnknownSheetId {
        /// Missing sheet identifier.
        value: u32,
    },
    /// A draft operation requires an existing sparse cell.
    CellNotFound {
        /// Sheet containing the missing address.
        sheet_id: u32,
        /// One-based row of the missing cell.
        row: u32,
        /// One-based column of the missing cell.
        column: u32,
    },
    /// No larger nonzero sheet identifier can be allocated.
    SheetIdExhausted,
    /// A visibility edit would leave the workbook without a visible sheet.
    LastVisibleSheet,
    /// A draft cannot increment its monotonic semantic revision.
    SemanticRevisionExhausted,
    /// A draft cannot increment its monotonic presentation revision.
    PresentationRevisionExhausted,
    /// A phonetic range is empty or reversed.
    PhoneticRangeEmpty {
        /// Rejected zero-based UTF-16 start offset.
        start: u32,
        /// Rejected exclusive zero-based UTF-16 end offset.
        end: u32,
    },
    /// A phonetic range exceeds its base text.
    PhoneticRangeOutOfBounds {
        /// Rejected exclusive zero-based UTF-16 end offset.
        end: u32,
        /// UTF-16 code-unit length of the base text.
        base_utf16_len: u32,
    },
    /// A phonetic range boundary falls between a UTF-16 surrogate pair.
    PhoneticRangeSplitsSurrogate {
        /// Rejected zero-based UTF-16 offset.
        offset: u32,
    },
    /// Authoring runs are not strictly ordered and non-overlapping.
    PhoneticRunsOutOfOrder,
    /// A phonetic run contains no text.
    PhoneticTextEmpty,
    /// Phonetic text contains a character forbidden by XML 1.0.
    PhoneticTextInvalidCharacter {
        /// Rejected character.
        character: char,
    },
    /// A caller attempted to attach phonetics to a non-text cell.
    PhoneticsRequireTextCell {
        /// Sheet containing the rejected cell.
        sheet_id: u32,
        /// One-based row of the rejected cell.
        row: u32,
        /// One-based column of the rejected cell.
        column: u32,
    },
    /// Phonetic authoring referenced a font record outside the initial writer contract.
    PhoneticFontIdUnsupported {
        /// Rejected zero-based font record identifier.
        value: u32,
    },
    /// A normal value edit would silently discard an existing annotation.
    AnnotatedTextReplacementRequired {
        /// Sheet containing the annotated cell.
        sheet_id: u32,
        /// One-based row of the annotated cell.
        row: u32,
        /// One-based column of the annotated cell.
        column: u32,
    },
    /// A frozen row count cannot be represented within Excel worksheet bounds.
    FrozenRowsOutOfRange {
        /// Rejected frozen row count.
        value: u32,
    },
    /// A frozen column count cannot be represented within Excel worksheet bounds.
    FrozenColumnsOutOfRange {
        /// Rejected frozen column count.
        value: u32,
    },
    /// A built-in number format used a custom-format identifier.
    BuiltInNumberFormatId {
        /// Rejected number-format identifier.
        value: u32,
    },
    /// A custom number format used a reserved built-in identifier.
    CustomNumberFormatId {
        /// Rejected number-format identifier.
        value: u32,
    },
    /// A custom number format code is empty.
    NumberFormatCodeEmpty,
    /// An OOXML table identifier is zero.
    TableIdZero,
    /// An OOXML table column identifier is zero.
    TableColumnIdZero,
    /// A table name is empty.
    TableNameEmpty,
    /// A table name is longer than Excel's 255 UTF-16 code-unit limit.
    TableNameTooLong {
        /// Length of the rejected name in UTF-16 code units.
        utf16_len: usize,
    },
    /// A table name contains whitespace or a control character.
    TableNameInvalidCharacter {
        /// Forbidden character found in the table name.
        character: char,
    },
    /// A table name is parsed by Excel as an A1 or R1C1 reference.
    TableNameReferenceConflict,
    /// Two tables anywhere in the workbook use display names that compare equal without case.
    DuplicateTableDisplayName {
        /// Repeated table display name as supplied by the caller.
        name: String,
    },
    /// Two tables in the workbook use the same non-zero identifier.
    DuplicateTableId {
        /// Repeated table identifier.
        id: u32,
    },
    /// Two tables on one worksheet use programmatic names that compare equal without case.
    DuplicateTableProgrammaticName {
        /// Repeated programmatic name as supplied by the caller.
        name: String,
    },
    /// A table display name compares equal to a workbook defined name.
    TableDisplayNameConflictsWithDefinedName {
        /// Conflicting table display name as supplied by the caller.
        name: String,
    },
    /// A table declares no columns.
    TableColumnsEmpty,
    /// A table's declared column count disagrees with its range width.
    TableColumnCountMismatch {
        /// Number of declared columns.
        columns: usize,
        /// Width of the table range in columns.
        width: u32,
    },
    /// A table column name is empty.
    TableColumnNameEmpty,
    /// A table column name is longer than Excel's 255 UTF-16 code-unit limit.
    TableColumnNameTooLong {
        /// Length of the rejected name in UTF-16 code units.
        utf16_len: usize,
    },
    /// Two columns in one table use names that compare equal without case.
    DuplicateTableColumnName {
        /// Repeated column name as supplied by the caller.
        name: String,
    },
    /// Two columns in one table use the same identifier.
    DuplicateTableColumnId {
        /// Repeated column identifier.
        id: u32,
    },
    /// Two tables on one worksheet have overlapping ranges.
    OverlappingTables {
        /// Worksheet containing both tables.
        sheet_id: u32,
        /// First table in deterministic range order.
        first_table_id: u32,
        /// Overlapping table in deterministic range order.
        second_table_id: u32,
    },
    /// A table's header and totals rows do not fit inside its range.
    TableRowCountsExceedRange {
        /// Declared header row count.
        header_row_count: u32,
        /// Declared totals row count.
        totals_row_count: u32,
        /// Height of the table range in rows.
        height: u32,
    },
    /// A table column's totals label, function, and formula metadata are inconsistent.
    InvalidTableTotalsMetadata,
}

impl ValidationError {
    /// Returns the stable machine-readable code for this validation failure.
    pub const fn code(&self) -> ValidationErrorCode {
        match self {
            Self::RowOutOfRange { .. } => ValidationErrorCode::RowOutOfRange,
            Self::ColumnOutOfRange { .. } => ValidationErrorCode::ColumnOutOfRange,
            Self::CellAddressInvalid => ValidationErrorCode::CellAddressInvalid,
            Self::RangeStartAfterEnd => ValidationErrorCode::RangeStartAfterEnd,
            Self::SheetIdZero => ValidationErrorCode::SheetIdZero,
            Self::SheetNameEmpty => ValidationErrorCode::SheetNameEmpty,
            Self::SheetNameTooLong { .. } => ValidationErrorCode::SheetNameTooLong,
            Self::SheetNameInvalidCharacter { .. } => {
                ValidationErrorCode::SheetNameInvalidCharacter
            }
            Self::SheetNameApostropheBoundary => ValidationErrorCode::SheetNameApostropheBoundary,
            Self::DuplicateSheetId { .. } => ValidationErrorCode::DuplicateSheetId,
            Self::DuplicateSheetName { .. } => ValidationErrorCode::DuplicateSheetName,
            Self::DuplicateCell { .. } => ValidationErrorCode::DuplicateCell,
            Self::DefinedNameEmpty => ValidationErrorCode::DefinedNameEmpty,
            Self::DefinedNameTooLong { .. } => ValidationErrorCode::DefinedNameTooLong,
            Self::DefinedNameControlCharacter { .. } => {
                ValidationErrorCode::DefinedNameControlCharacter
            }
            Self::DefinedNameUnknownSheet { .. } => ValidationErrorCode::DefinedNameUnknownSheet,
            Self::DuplicateDefinedName { .. } => ValidationErrorCode::DuplicateDefinedName,
            Self::NonFiniteNumber => ValidationErrorCode::NonFiniteNumber,
            Self::FormulaEmpty => ValidationErrorCode::FormulaEmpty,
            Self::XlsxFormulaHasLeadingEquals => ValidationErrorCode::XlsxFormulaHasLeadingEquals,
            Self::UserFormulaMissingLeadingEquals => {
                ValidationErrorCode::UserFormulaMissingLeadingEquals
            }
            Self::SourceIdEmpty => ValidationErrorCode::SourceIdEmpty,
            Self::DiagnosticCodeInvalid => ValidationErrorCode::DiagnosticCodeInvalid,
            Self::ProviderNameEmpty => ValidationErrorCode::ProviderNameEmpty,
            Self::ProviderVersionEmpty => ValidationErrorCode::ProviderVersionEmpty,
            Self::DiagnosticMessageEmpty => ValidationErrorCode::DiagnosticMessageEmpty,
            Self::UnknownSheetId { .. } => ValidationErrorCode::UnknownSheetId,
            Self::CellNotFound { .. } => ValidationErrorCode::CellNotFound,
            Self::SheetIdExhausted => ValidationErrorCode::SheetIdExhausted,
            Self::LastVisibleSheet => ValidationErrorCode::LastVisibleSheet,
            Self::SemanticRevisionExhausted => ValidationErrorCode::SemanticRevisionExhausted,
            Self::PresentationRevisionExhausted => {
                ValidationErrorCode::PresentationRevisionExhausted
            }
            Self::PhoneticRangeEmpty { .. } => ValidationErrorCode::PhoneticRangeEmpty,
            Self::PhoneticRangeOutOfBounds { .. } => ValidationErrorCode::PhoneticRangeOutOfBounds,
            Self::PhoneticRangeSplitsSurrogate { .. } => {
                ValidationErrorCode::PhoneticRangeSplitsSurrogate
            }
            Self::PhoneticRunsOutOfOrder => ValidationErrorCode::PhoneticRunsOutOfOrder,
            Self::PhoneticTextEmpty => ValidationErrorCode::PhoneticTextEmpty,
            Self::PhoneticTextInvalidCharacter { .. } => {
                ValidationErrorCode::PhoneticTextInvalidCharacter
            }
            Self::PhoneticsRequireTextCell { .. } => ValidationErrorCode::PhoneticsRequireTextCell,
            Self::PhoneticFontIdUnsupported { .. } => {
                ValidationErrorCode::PhoneticFontIdUnsupported
            }
            Self::AnnotatedTextReplacementRequired { .. } => {
                ValidationErrorCode::AnnotatedTextReplacementRequired
            }
            Self::FrozenRowsOutOfRange { .. } => ValidationErrorCode::FrozenRowsOutOfRange,
            Self::FrozenColumnsOutOfRange { .. } => ValidationErrorCode::FrozenColumnsOutOfRange,
            Self::BuiltInNumberFormatId { .. } => ValidationErrorCode::BuiltInNumberFormatId,
            Self::CustomNumberFormatId { .. } => ValidationErrorCode::CustomNumberFormatId,
            Self::NumberFormatCodeEmpty => ValidationErrorCode::NumberFormatCodeEmpty,
            Self::TableIdZero => ValidationErrorCode::TableIdZero,
            Self::TableColumnIdZero => ValidationErrorCode::TableColumnIdZero,
            Self::TableNameEmpty => ValidationErrorCode::TableNameEmpty,
            Self::TableNameTooLong { .. } => ValidationErrorCode::TableNameTooLong,
            Self::TableNameInvalidCharacter { .. } => {
                ValidationErrorCode::TableNameInvalidCharacter
            }
            Self::TableNameReferenceConflict => ValidationErrorCode::TableNameReferenceConflict,
            Self::DuplicateTableDisplayName { .. } => {
                ValidationErrorCode::DuplicateTableDisplayName
            }
            Self::DuplicateTableId { .. } => ValidationErrorCode::DuplicateTableId,
            Self::DuplicateTableProgrammaticName { .. } => {
                ValidationErrorCode::DuplicateTableProgrammaticName
            }
            Self::TableDisplayNameConflictsWithDefinedName { .. } => {
                ValidationErrorCode::TableDisplayNameConflictsWithDefinedName
            }
            Self::TableColumnsEmpty => ValidationErrorCode::TableColumnsEmpty,
            Self::TableColumnCountMismatch { .. } => ValidationErrorCode::TableColumnCountMismatch,
            Self::TableColumnNameEmpty => ValidationErrorCode::TableColumnNameEmpty,
            Self::TableColumnNameTooLong { .. } => ValidationErrorCode::TableColumnNameTooLong,
            Self::DuplicateTableColumnName { .. } => ValidationErrorCode::DuplicateTableColumnName,
            Self::DuplicateTableColumnId { .. } => ValidationErrorCode::DuplicateTableColumnId,
            Self::OverlappingTables { .. } => ValidationErrorCode::OverlappingTables,
            Self::TableRowCountsExceedRange { .. } => {
                ValidationErrorCode::TableRowCountsExceedRange
            }
            Self::InvalidTableTotalsMetadata => ValidationErrorCode::InvalidTableTotalsMetadata,
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RowOutOfRange { value } => {
                write!(formatter, "{MESSAGE_ROW_OUT_OF_RANGE}: {value}")
            }
            Self::ColumnOutOfRange { value } => {
                write!(formatter, "{MESSAGE_COLUMN_OUT_OF_RANGE}: {value}")
            }
            Self::CellAddressInvalid => formatter.write_str(MESSAGE_CELL_ADDRESS_INVALID),
            Self::RangeStartAfterEnd => formatter.write_str(MESSAGE_RANGE_REVERSED),
            Self::SheetIdZero => formatter.write_str(MESSAGE_SHEET_ID_ZERO),
            Self::SheetNameEmpty => formatter.write_str(MESSAGE_SHEET_NAME_EMPTY),
            Self::SheetNameTooLong { utf16_len } => {
                write!(formatter, "{MESSAGE_SHEET_NAME_TOO_LONG}: {utf16_len}")
            }
            Self::SheetNameInvalidCharacter { character } => write!(
                formatter,
                "{MESSAGE_SHEET_NAME_INVALID_CHARACTER}: {character:?}"
            ),
            Self::SheetNameApostropheBoundary => {
                formatter.write_str(MESSAGE_SHEET_NAME_APOSTROPHE_BOUNDARY)
            }
            Self::DuplicateSheetId { value } => {
                write!(formatter, "{MESSAGE_DUPLICATE_SHEET_ID}: {value}")
            }
            Self::DuplicateSheetName { name } => {
                write!(formatter, "{MESSAGE_DUPLICATE_SHEET_NAME}: {name}")
            }
            Self::DuplicateCell { row, column } => {
                write!(
                    formatter,
                    "{MESSAGE_DUPLICATE_CELL}: row {row}, column {column}"
                )
            }
            Self::DefinedNameEmpty => formatter.write_str(MESSAGE_DEFINED_NAME_EMPTY),
            Self::DefinedNameTooLong { utf16_len } => {
                write!(formatter, "{MESSAGE_DEFINED_NAME_TOO_LONG}: {utf16_len}")
            }
            Self::DefinedNameControlCharacter { character } => {
                write!(formatter, "{MESSAGE_DEFINED_NAME_CONTROL}: {character:?}")
            }
            Self::DefinedNameUnknownSheet { sheet_id } => {
                write!(
                    formatter,
                    "{MESSAGE_DEFINED_NAME_UNKNOWN_SHEET}: {sheet_id}"
                )
            }
            Self::DuplicateDefinedName { name } => {
                write!(formatter, "{MESSAGE_DUPLICATE_DEFINED_NAME}: {name}")
            }
            Self::NonFiniteNumber => formatter.write_str(MESSAGE_NON_FINITE_NUMBER),
            Self::FormulaEmpty => formatter.write_str(MESSAGE_FORMULA_EMPTY),
            Self::XlsxFormulaHasLeadingEquals => formatter.write_str(MESSAGE_XLSX_FORMULA_EQUALS),
            Self::UserFormulaMissingLeadingEquals => {
                formatter.write_str(MESSAGE_USER_FORMULA_EQUALS)
            }
            Self::SourceIdEmpty => formatter.write_str(MESSAGE_SOURCE_ID_EMPTY),
            Self::DiagnosticCodeInvalid => formatter.write_str(MESSAGE_DIAGNOSTIC_CODE_INVALID),
            Self::ProviderNameEmpty => formatter.write_str(MESSAGE_PROVIDER_NAME_EMPTY),
            Self::ProviderVersionEmpty => formatter.write_str(MESSAGE_PROVIDER_VERSION_EMPTY),
            Self::DiagnosticMessageEmpty => formatter.write_str(MESSAGE_DIAGNOSTIC_MESSAGE_EMPTY),
            Self::UnknownSheetId { value } => {
                write!(formatter, "{MESSAGE_UNKNOWN_SHEET_ID}: {value}")
            }
            Self::CellNotFound {
                sheet_id,
                row,
                column,
            } => write!(
                formatter,
                "{MESSAGE_CELL_NOT_FOUND}: sheet {sheet_id}, row {row}, column {column}"
            ),
            Self::SheetIdExhausted => formatter.write_str(MESSAGE_SHEET_ID_EXHAUSTED),
            Self::LastVisibleSheet => formatter.write_str(MESSAGE_LAST_VISIBLE_SHEET),
            Self::SemanticRevisionExhausted => {
                formatter.write_str(MESSAGE_SEMANTIC_REVISION_EXHAUSTED)
            }
            Self::PresentationRevisionExhausted => {
                formatter.write_str(MESSAGE_PRESENTATION_REVISION_EXHAUSTED)
            }
            Self::PhoneticRangeEmpty { start, end } => {
                write!(formatter, "{MESSAGE_PHONETIC_RANGE_EMPTY}: {start}..{end}")
            }
            Self::PhoneticRangeOutOfBounds {
                end,
                base_utf16_len,
            } => write!(
                formatter,
                "{MESSAGE_PHONETIC_RANGE_OUT_OF_BOUNDS}: {end} > {base_utf16_len}"
            ),
            Self::PhoneticRangeSplitsSurrogate { offset } => {
                write!(
                    formatter,
                    "{MESSAGE_PHONETIC_RANGE_SPLITS_SURROGATE}: {offset}"
                )
            }
            Self::PhoneticRunsOutOfOrder => formatter.write_str(MESSAGE_PHONETIC_RUNS_OUT_OF_ORDER),
            Self::PhoneticTextEmpty => formatter.write_str(MESSAGE_PHONETIC_TEXT_EMPTY),
            Self::PhoneticTextInvalidCharacter { character } => write!(
                formatter,
                "{MESSAGE_PHONETIC_TEXT_INVALID_CHARACTER}: {character:?}"
            ),
            Self::PhoneticsRequireTextCell {
                sheet_id,
                row,
                column,
            } => write!(
                formatter,
                "{MESSAGE_PHONETICS_REQUIRE_TEXT_CELL}: sheet {sheet_id}, row {row}, column {column}"
            ),
            Self::PhoneticFontIdUnsupported { value } => {
                write!(formatter, "{MESSAGE_PHONETIC_FONT_ID_UNSUPPORTED}: {value}")
            }
            Self::AnnotatedTextReplacementRequired {
                sheet_id,
                row,
                column,
            } => write!(
                formatter,
                "{MESSAGE_ANNOTATED_TEXT_REPLACEMENT_REQUIRED}: sheet {sheet_id}, row {row}, column {column}"
            ),
            Self::FrozenRowsOutOfRange { value } => {
                write!(formatter, "{MESSAGE_FROZEN_ROWS_OUT_OF_RANGE}: {value}")
            }
            Self::FrozenColumnsOutOfRange { value } => {
                write!(formatter, "{MESSAGE_FROZEN_COLUMNS_OUT_OF_RANGE}: {value}")
            }
            Self::BuiltInNumberFormatId { value } => {
                write!(formatter, "{MESSAGE_NUMBER_FORMAT_BUILTIN_ID}: {value}")
            }
            Self::CustomNumberFormatId { value } => {
                write!(formatter, "{MESSAGE_NUMBER_FORMAT_CUSTOM_ID}: {value}")
            }
            Self::NumberFormatCodeEmpty => formatter.write_str(MESSAGE_NUMBER_FORMAT_CODE_EMPTY),
            Self::TableIdZero => formatter.write_str(MESSAGE_TABLE_ID_ZERO),
            Self::TableColumnIdZero => formatter.write_str(MESSAGE_TABLE_COLUMN_ID_ZERO),
            Self::TableNameEmpty => formatter.write_str(MESSAGE_TABLE_NAME_EMPTY),
            Self::TableNameTooLong { utf16_len } => {
                write!(formatter, "{MESSAGE_TABLE_NAME_TOO_LONG}: {utf16_len}")
            }
            Self::TableNameInvalidCharacter { character } => write!(
                formatter,
                "{MESSAGE_TABLE_NAME_INVALID_CHARACTER}: {character:?}"
            ),
            Self::TableNameReferenceConflict => {
                formatter.write_str(MESSAGE_TABLE_NAME_REFERENCE_CONFLICT)
            }
            Self::DuplicateTableDisplayName { name } => {
                write!(formatter, "{MESSAGE_DUPLICATE_TABLE_DISPLAY_NAME}: {name}")
            }
            Self::DuplicateTableId { id } => {
                write!(formatter, "{MESSAGE_DUPLICATE_TABLE_ID}: {id}")
            }
            Self::DuplicateTableProgrammaticName { name } => {
                write!(
                    formatter,
                    "{MESSAGE_DUPLICATE_TABLE_PROGRAMMATIC_NAME}: {name}"
                )
            }
            Self::TableDisplayNameConflictsWithDefinedName { name } => {
                write!(
                    formatter,
                    "{MESSAGE_TABLE_DISPLAY_NAME_CONFLICTS_WITH_DEFINED_NAME}: {name}"
                )
            }
            Self::TableColumnsEmpty => formatter.write_str(MESSAGE_TABLE_COLUMNS_EMPTY),
            Self::TableColumnCountMismatch { columns, width } => write!(
                formatter,
                "{MESSAGE_TABLE_COLUMN_COUNT_MISMATCH}: {columns} columns, width {width}"
            ),
            Self::TableColumnNameEmpty => formatter.write_str(MESSAGE_TABLE_COLUMN_NAME_EMPTY),
            Self::TableColumnNameTooLong { utf16_len } => {
                write!(
                    formatter,
                    "{MESSAGE_TABLE_COLUMN_NAME_TOO_LONG}: {utf16_len}"
                )
            }
            Self::DuplicateTableColumnName { name } => {
                write!(formatter, "{MESSAGE_DUPLICATE_TABLE_COLUMN_NAME}: {name}")
            }
            Self::DuplicateTableColumnId { id } => {
                write!(formatter, "{MESSAGE_DUPLICATE_TABLE_COLUMN_ID}: {id}")
            }
            Self::OverlappingTables {
                sheet_id,
                first_table_id,
                second_table_id,
            } => write!(
                formatter,
                "{MESSAGE_OVERLAPPING_TABLES}: sheet {sheet_id}, tables {first_table_id} and {second_table_id}"
            ),
            Self::TableRowCountsExceedRange {
                header_row_count,
                totals_row_count,
                height,
            } => write!(
                formatter,
                "{MESSAGE_TABLE_ROW_COUNTS_EXCEED_RANGE}: header {header_row_count}, totals {totals_row_count}, height {height}"
            ),
            Self::InvalidTableTotalsMetadata => {
                formatter.write_str(MESSAGE_INVALID_TABLE_TOTALS_METADATA)
            }
        }
    }
}

impl Error for ValidationError {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::ValidationErrorCode;

    #[test]
    fn validation_error_code_strings_are_complete_stable_and_unique() {
        let cases = [
            (
                ValidationErrorCode::RowOutOfRange,
                "validation.row_out_of_range",
            ),
            (
                ValidationErrorCode::ColumnOutOfRange,
                "validation.column_out_of_range",
            ),
            (
                ValidationErrorCode::CellAddressInvalid,
                "validation.cell_address_invalid",
            ),
            (
                ValidationErrorCode::RangeStartAfterEnd,
                "validation.range_start_after_end",
            ),
            (ValidationErrorCode::SheetIdZero, "validation.sheet_id_zero"),
            (
                ValidationErrorCode::SheetNameEmpty,
                "validation.sheet_name_empty",
            ),
            (
                ValidationErrorCode::SheetNameTooLong,
                "validation.sheet_name_too_long",
            ),
            (
                ValidationErrorCode::SheetNameInvalidCharacter,
                "validation.sheet_name_invalid_character",
            ),
            (
                ValidationErrorCode::SheetNameApostropheBoundary,
                "validation.sheet_name_apostrophe_boundary",
            ),
            (
                ValidationErrorCode::DuplicateSheetId,
                "validation.duplicate_sheet_id",
            ),
            (
                ValidationErrorCode::DuplicateSheetName,
                "validation.duplicate_sheet_name",
            ),
            (
                ValidationErrorCode::DuplicateCell,
                "validation.duplicate_cell",
            ),
            (
                ValidationErrorCode::DefinedNameEmpty,
                "validation.defined_name_empty",
            ),
            (
                ValidationErrorCode::DefinedNameTooLong,
                "validation.defined_name_too_long",
            ),
            (
                ValidationErrorCode::DefinedNameControlCharacter,
                "validation.defined_name_control_character",
            ),
            (
                ValidationErrorCode::DefinedNameUnknownSheet,
                "validation.defined_name_unknown_sheet",
            ),
            (
                ValidationErrorCode::DuplicateDefinedName,
                "validation.duplicate_defined_name",
            ),
            (
                ValidationErrorCode::NonFiniteNumber,
                "validation.non_finite_number",
            ),
            (
                ValidationErrorCode::FormulaEmpty,
                "validation.formula_empty",
            ),
            (
                ValidationErrorCode::XlsxFormulaHasLeadingEquals,
                "validation.xlsx_formula_has_leading_equals",
            ),
            (
                ValidationErrorCode::UserFormulaMissingLeadingEquals,
                "validation.user_formula_missing_leading_equals",
            ),
            (
                ValidationErrorCode::SourceIdEmpty,
                "validation.source_id_empty",
            ),
            (
                ValidationErrorCode::DiagnosticCodeInvalid,
                "validation.diagnostic_code_invalid",
            ),
            (
                ValidationErrorCode::ProviderNameEmpty,
                "validation.provider_name_empty",
            ),
            (
                ValidationErrorCode::ProviderVersionEmpty,
                "validation.provider_version_empty",
            ),
            (
                ValidationErrorCode::DiagnosticMessageEmpty,
                "validation.diagnostic_message_empty",
            ),
            (
                ValidationErrorCode::UnknownSheetId,
                "validation.unknown_sheet_id",
            ),
            (
                ValidationErrorCode::CellNotFound,
                "validation.cell_not_found",
            ),
            (
                ValidationErrorCode::SheetIdExhausted,
                "validation.sheet_id_exhausted",
            ),
            (
                ValidationErrorCode::LastVisibleSheet,
                "validation.last_visible_sheet",
            ),
            (
                ValidationErrorCode::SemanticRevisionExhausted,
                "validation.semantic_revision_exhausted",
            ),
            (
                ValidationErrorCode::PresentationRevisionExhausted,
                "validation.presentation_revision_exhausted",
            ),
            (
                ValidationErrorCode::PhoneticRangeEmpty,
                "validation.phonetic_range_empty",
            ),
            (
                ValidationErrorCode::PhoneticRangeOutOfBounds,
                "validation.phonetic_range_out_of_bounds",
            ),
            (
                ValidationErrorCode::PhoneticRangeSplitsSurrogate,
                "validation.phonetic_range_splits_surrogate",
            ),
            (
                ValidationErrorCode::PhoneticRunsOutOfOrder,
                "validation.phonetic_runs_out_of_order",
            ),
            (
                ValidationErrorCode::PhoneticTextEmpty,
                "validation.phonetic_text_empty",
            ),
            (
                ValidationErrorCode::PhoneticTextInvalidCharacter,
                "validation.phonetic_text_invalid_character",
            ),
            (
                ValidationErrorCode::PhoneticsRequireTextCell,
                "validation.phonetics_require_text_cell",
            ),
            (
                ValidationErrorCode::PhoneticFontIdUnsupported,
                "validation.phonetic_font_id_unsupported",
            ),
            (
                ValidationErrorCode::AnnotatedTextReplacementRequired,
                "validation.annotated_text_replacement_required",
            ),
            (
                ValidationErrorCode::FrozenRowsOutOfRange,
                "validation.frozen_rows_out_of_range",
            ),
            (
                ValidationErrorCode::FrozenColumnsOutOfRange,
                "validation.frozen_columns_out_of_range",
            ),
            (
                ValidationErrorCode::BuiltInNumberFormatId,
                "validation.built_in_number_format_id",
            ),
            (
                ValidationErrorCode::CustomNumberFormatId,
                "validation.custom_number_format_id",
            ),
            (
                ValidationErrorCode::NumberFormatCodeEmpty,
                "validation.number_format_code_empty",
            ),
            (ValidationErrorCode::TableIdZero, "validation.table_id_zero"),
            (
                ValidationErrorCode::TableColumnIdZero,
                "validation.table_column_id_zero",
            ),
            (
                ValidationErrorCode::TableNameEmpty,
                "validation.table_name_empty",
            ),
            (
                ValidationErrorCode::TableNameTooLong,
                "validation.table_name_too_long",
            ),
            (
                ValidationErrorCode::TableNameInvalidCharacter,
                "validation.table_name_invalid_character",
            ),
            (
                ValidationErrorCode::TableNameReferenceConflict,
                "validation.table_name_reference_conflict",
            ),
            (
                ValidationErrorCode::DuplicateTableDisplayName,
                "validation.duplicate_table_display_name",
            ),
            (
                ValidationErrorCode::DuplicateTableId,
                "validation.duplicate_table_id",
            ),
            (
                ValidationErrorCode::DuplicateTableProgrammaticName,
                "validation.duplicate_table_programmatic_name",
            ),
            (
                ValidationErrorCode::TableDisplayNameConflictsWithDefinedName,
                "validation.table_display_name_conflicts_with_defined_name",
            ),
            (
                ValidationErrorCode::TableColumnsEmpty,
                "validation.table_columns_empty",
            ),
            (
                ValidationErrorCode::TableColumnCountMismatch,
                "validation.table_column_count_mismatch",
            ),
            (
                ValidationErrorCode::TableColumnNameEmpty,
                "validation.table_column_name_empty",
            ),
            (
                ValidationErrorCode::TableColumnNameTooLong,
                "validation.table_column_name_too_long",
            ),
            (
                ValidationErrorCode::DuplicateTableColumnName,
                "validation.duplicate_table_column_name",
            ),
            (
                ValidationErrorCode::DuplicateTableColumnId,
                "validation.duplicate_table_column_id",
            ),
            (
                ValidationErrorCode::OverlappingTables,
                "validation.overlapping_tables",
            ),
            (
                ValidationErrorCode::TableRowCountsExceedRange,
                "validation.table_row_counts_exceed_range",
            ),
            (
                ValidationErrorCode::InvalidTableTotalsMetadata,
                "validation.invalid_table_totals_metadata",
            ),
        ];
        let mut seen_codes = BTreeSet::new();

        for (code, expected_text) in &cases {
            assert_eq!(code.as_str(), *expected_text);
            assert!(seen_codes.insert(*expected_text));
        }

        assert_eq!(seen_codes.len(), cases.len());
    }
}
