//! Checks that `docs/ERROR_CODES.md` lists exactly the stable codes the core enums define.

use std::collections::BTreeSet;

use crate::{
    CalculationIssueCode, CsvErrorCode, DefinedNameAnalysisErrorKind, DisplayFormatError,
    SessionErrorCode, TargetCalculationErrorCode, ValidationErrorCode, XlsxErrorCode,
    XlsxWriteErrorCode,
};

/// Lists the codes of every variant of an enum. Naming each variant in an exhaustive match makes
/// this fail to compile when the enum gains one, so a new code cannot skip the registry.
macro_rules! codes {
    ($enum:ident :: $method:ident; $($variant:ident),+ $(,)?) => {{
        fn exhaustive(code: $enum) {
            match code {
                $($enum::$variant)|+ => {}
            }
        }
        let _: fn($enum) = exhaustive;
        vec![$($enum::$variant.$method()),+]
    }};
}

fn registry() -> Vec<Vec<&'static str>> {
    vec![
        codes!(XlsxErrorCode::as_str;
            Io,
            InvalidZip,
            ArchiveTooLarge,
            TooManyEntries,
            EntryTooLarge,
            TotalUncompressedTooLarge,
            DeclaredSizeMismatch,
            CompressionRatioExceeded,
            OverlappingEntries,
            EncryptedEntry,
            UnsupportedCompression,
            InvalidPartName,
            DuplicatePart,
            MissingPart,
            InvalidXml,
            XmlDepthExceeded,
            XmlAttributesExceeded,
            ForbiddenXmlConstruct,
            InvalidContentTypes,
            InvalidRelationships,
            MissingWorkbookRelationship,
            DuplicateWorkbookRelationship,
            ExternalWorkbookRelationship,
            InvalidRelationshipTarget,
            MissingWorksheetRelationship,
            UnsupportedContentType,
            InvalidWorkbook,
            TooManySheets,
            MissingSheetRelationship,
            InvalidSharedStrings,
            TooManySharedStrings,
            SharedStringTooLarge,
            TotalSharedStringsTooLarge,
            TotalCellTextTooLarge,
            InvalidStyles,
            InvalidStyleIndex,
            InvalidWorksheet,
            InvalidCellReference,
            TooManyCellsInSheet,
            TooManyCells,
            UnsupportedCellType,
            InvalidCellValue,
            InvalidDefinedName,
            TooManyDefinedNames,
            FormulaTooLarge,
            TotalFormulaBytesTooLarge,
            InvalidFormulaMetadata,
            InvalidCellMetadata,
            InvalidPhoneticMetadata,
            TooManyPhoneticRuns,
            PhoneticTextTooLarge,
            TotalPhoneticTextTooLarge,
            TooManyAnnotatedCells,
            InvalidFrozenPane,
            TooManyMergedRanges,
            TooManyTables,
            TooManyTableColumns,
            TableNameTooLarge,
            TooManyTableFilterItems,
            TableFilterTextTooLarge,
        ),
        codes!(XlsxWriteErrorCode::as_str;
            Io,
            InvalidPackagePlan,
            SourceIdentityMismatch,
            StaleSemanticRevision,
            IncompleteCalculation,
            UnsupportedResultMaterialization,
            OutputKindMismatch,
            UnsupportedPreservation,
            ConflictingPartOperation,
            DanglingRelationship,
            InvalidGeneratedXml,
            DestinationExists,
            AtomicReplaceFailed,
            ResourceLimitExceeded,
            OutputVerificationFailed,
        ),
        codes!(ValidationErrorCode::as_str;
            RowOutOfRange,
            ColumnOutOfRange,
            CellAddressInvalid,
            RangeStartAfterEnd,
            SheetIdZero,
            SheetNameEmpty,
            SheetNameTooLong,
            SheetNameInvalidCharacter,
            SheetNameApostropheBoundary,
            DuplicateSheetId,
            DuplicateSheetName,
            DuplicateCell,
            DefinedNameEmpty,
            DefinedNameTooLong,
            DefinedNameControlCharacter,
            DefinedNameUnknownSheet,
            DuplicateDefinedName,
            NonFiniteNumber,
            FormulaEmpty,
            XlsxFormulaHasLeadingEquals,
            UserFormulaMissingLeadingEquals,
            SourceIdEmpty,
            DiagnosticCodeInvalid,
            ProviderNameEmpty,
            ProviderVersionEmpty,
            DiagnosticMessageEmpty,
            UnknownSheetId,
            CellNotFound,
            SheetIdExhausted,
            LastVisibleSheet,
            SemanticRevisionExhausted,
            PresentationRevisionExhausted,
            PhoneticRangeEmpty,
            PhoneticRangeOutOfBounds,
            PhoneticRangeSplitsSurrogate,
            PhoneticRunsOutOfOrder,
            PhoneticTextEmpty,
            PhoneticTextInvalidCharacter,
            PhoneticsRequireTextCell,
            PhoneticFontIdUnsupported,
            AnnotatedTextReplacementRequired,
            ArrayFormulaPartEdit,
            FrozenRowsOutOfRange,
            FrozenColumnsOutOfRange,
            FontNameInvalid,
            FormatValueOutOfRange,
            BuiltInNumberFormatId,
            CustomNumberFormatId,
            NumberFormatCodeEmpty,
            TableIdZero,
            TableColumnIdZero,
            TableNameEmpty,
            TableNameTooLong,
            TableNameInvalidCharacter,
            TableNameReferenceConflict,
            DuplicateTableDisplayName,
            DuplicateTableId,
            DuplicateTableProgrammaticName,
            TableDisplayNameConflictsWithDefinedName,
            TableColumnsEmpty,
            TableColumnCountMismatch,
            TableColumnNameEmpty,
            TableHeaderRequiresText,
            TableColumnNameTooLong,
            TableColumnNameInvalidCharacter,
            TableColumnNameSpaceBoundary,
            DuplicateTableColumnName,
            DuplicateTableColumnId,
            OverlappingTables,
            TableRowCountsExceedRange,
            InvalidTableTotalsMetadata,
            UnknownTableId,
            UnknownTableColumnId,
            TableDataRowsReversed,
            TableResizeHeaderUnderflow,
            TableMaterializationCollision,
            UnsupportedTableAuthoringMetadata,
            TableTotalsCellOccupied,
            SheetAutoFilterNotFound,
            SheetAutoFilterOverlapsTable,
            ColumnWideningRangeInvalid,
            ColumnWideningOverlap,
            FormulaRewriteParseFailed,
            TableRangeOverlap,
            TableRangeInvalid,
            DefinedNameRenameShadowed,
            UnknownDefinedName,
            SheetPositionOutOfRange,
            StructuralEditEmpty,
            ContentShiftedOffSheet,
            MergedRangeSplit,
            TableHeaderShift,
            TableDataRowsDeleted,
            TableSplit,
            CopyDestinationShape,
            CopyUnsupportedFormula,
        ),
        codes!(SessionErrorCode::as_str;
            RevisionMismatch,
            CalculationUninitialized,
            StateRevisionMismatch,
            IncrementalUnsafe,
            StaleResult,
            InvalidLimits,
            EmptyBatch,
            BatchLimitExceeded,
            EvaluationLimitExceeded,
            DeltaLimitExceeded,
            CursorExpired,
            PageLimitExceeded,
            Cancelled,
            RewriteLimitExceeded,
            TableMaterializationLimitExceeded,
            TransactionConsumed,
            TransactionCursorInvalid,
            TransactionDetailLimitExceeded,
            TransactionResourceLimitExceeded,
        ),
        codes!(TargetCalculationErrorCode::as_str;
            EmptyTargets,
            SheetNotFound,
            InvalidLimits,
            TargetLimitExceeded,
            EvaluationLimitExceeded,
            Cancelled,
            StaleResult,
        ),
        codes!(CsvErrorCode::as_str;
            InvalidDelimiter,
            InvalidUtf8,
            UnterminatedQuote,
            InvalidQuote,
            TooManyRows,
            TooManyColumns,
            TooManyCells,
            FieldTooLong,
            InvalidValue,
            UnknownSheet,
            StaleCalculation,
            UnavailableResult,
        ),
        codes!(DefinedNameAnalysisErrorKind::as_str;
            UnknownCurrentSheet,
            ResourceLimit,
            Cancelled,
        ),
        codes!(DisplayFormatError::code;
            Unsupported,
        ),
        codes!(CalculationIssueCode::as_str;
            MissingFormulaText,
            ParseError,
            UnsupportedFunction,
            UnsupportedName,
            UnsupportedSheetRange,
            UnsupportedStructuredReference,
            UnsupportedExpression,
            ResourceLimitExceeded,
            VolatileInputMissing,
            CircularReference,
            BlockedByUpstream,
        ),
    ]
}

/// Returns the codes in the first column of the tables under the `## {section}` heading.
fn documented(section: &str) -> BTreeSet<String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/ERROR_CODES.md");
    let document = std::fs::read_to_string(path).expect("docs/ERROR_CODES.md is readable");
    let heading = format!("## {section}");
    document
        .lines()
        .skip_while(|line| *line != heading)
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|line| line.split_once('`').map(|(code, _)| code.to_owned()))
        .collect()
}

#[test]
fn the_error_code_reference_lists_every_core_code() {
    let mut codes = BTreeSet::new();
    for enum_codes in registry() {
        for code in enum_codes {
            assert!(codes.insert(code.to_owned()), "{code} is defined twice");
        }
    }
    let documented = documented("Rust core");
    let undocumented = codes.difference(&documented).collect::<Vec<_>>();
    let stale = documented.difference(&codes).collect::<Vec<_>>();
    assert!(
        undocumented.is_empty() && stale.is_empty(),
        "docs/ERROR_CODES.md must list exactly the core codes; undocumented: {undocumented:?}, \
         not defined: {stale:?}"
    );
}
