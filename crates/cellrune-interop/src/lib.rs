//! Typed, versioned services shared by CellRune language bindings and local MCP.
//!
//! The crate intentionally owns no spreadsheet semantics. It validates transport-facing inputs,
//! converts them into `cellrune` domain types, and converts domain results back into owned DTOs.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod convert;
mod csv;
mod dependencies;
mod dto;
mod error;
mod format_convert;
mod format_dto;
mod json_object;
mod phonetic;
mod service;
mod sheet_metadata;
mod targeted;
mod usage_count;
mod values;
pub use service::{CompletedTargetRequest, PreparedTargetRequest};
pub use targeted::{
    CalculationTargetDto, TargetCalculationCellDto, TargetCalculationLimitsDto,
    TargetCalculationRequestDto, TargetCalculationResultDto, TargetCalculationScopeDto,
};

pub use cellrune::CancellationToken;
pub use csv::{CsvReadOptionsDto, CsvWriteRequestDto};
pub use dependencies::{
    CellDependencyRequestDto, DependentDto, DependentPageDto, PrecedentDto, PrecedentPageDto,
};
pub use dto::{
    ArithmeticSemanticsDto, CalculationDeltaCellDto, CalculationDeltaDto, CalculationDeltaPageDto,
    CalculationIssueDto, CalculationLimitOptionsDto, CalculationLimitsDto, CalculationOptionsDto,
    CalculationOptionsReportDto, CalculationReportDto, CalculationResultDto, CapabilityEntryDto,
    CapabilityPageDto, CellDto, CellReferenceDto, CellValueDto, DateSystemDto,
    DefinedNameDynamicKindDto, DefinedNameExternalTargetKindDto, DefinedNameInspectionDto,
    DefinedNameInspectionRequestDto, DefinedNameInspectionResultDto, DefinedNameInvalidReasonDto,
    DefinedNameReferenceAreaDto, DefinedNameSheetSpanDto, DefinedNameUnsupportedReasonDto,
    DisplayFormatRequestDto, DisplayFormatResultDto, EditBatchDto, EditBatchV2Dto, EditReceiptDto,
    EditReceiptV2Dto, EditWriteOptionsDto, EditWriteReportDto, FinancialSolverSemanticsDto,
    FunctionCatalogEntryDto, FunctionCatalogPageDto, FunctionCatalogPageRequestDto,
    FunctionCatalogReportDto, FunctionUsageEntryDto, FunctionUsageReportDto, LocaleDto,
    LocaleProfileDto, MaterializedResultOriginDto, NumberFormatDto, PreviewChangesDto,
    PreviewCursorDto, ProviderIdentityDto, RangePageDto, RangeRequestDto, ReadLimitOptionsDto,
    RecalculationModeDto, SavedValueStateDto, SheetSummaryDto, TableChangeV2Dto, TableColumnDto,
    TableSummaryDto, TransactionDetailCountsDto, TransactionDetailItemDto,
    TransactionDetailSectionDto, TransactionImpactCoverageDto, TransactionImpactPageDto,
    WorkbookChangeDto, WorkbookChangeV2Dto, WorkbookFingerprintDto, WorkbookSummaryDto,
    WorkbookTransactionReceiptDto, WorkbookTransactionReportDto, WritableCellValueDto,
    WriteOptionsDto, WriteReportDto,
};
pub use error::{ErrorDetails, InteropError, InteropErrorKind, LimitDiagnosticsDto};
pub use format_dto::{
    AlignmentDto, BorderDto, BorderEdgeDto, CellFormatDto, CellFormatEntryDto, CellFormatPageDto,
    ColorDto, ColumnWidthDto, FillDto, FontDto, FormatChangeV2Dto, PatternFillDto, RowHeightDto,
};
pub use json_object::{deserialize_object, deserialize_objects, deserialize_optional_object};
pub use phonetic::{
    FrozenPaneDto, PhoneticCellDto, PhoneticPageDto, PhoneticPropertiesDto, PhoneticRunDto,
};
pub use service::{
    CompletedPreview, CompletedRecalculation, DEFAULT_PAGE_SIZE, DEFAULT_PREVIEW_PAGE_SIZE,
    MAX_PAGE_SIZE, MAX_PREVIEW_PAGE_SIZE, MAX_RANGE_VALUE_CELLS, PreparedChanges,
    PreparedChangesV2, PreparedEditsSave, PreparedPreview, PreparedRecalculation,
    PreparedWorkbookSave, WorkbookSession, format_display_text, function_catalog,
    function_catalog_page,
};
pub use sheet_metadata::{
    CommentDto, CommentPageDto, DataValidationDto, DataValidationPageDto, HyperlinkDto,
    HyperlinkPageDto, SheetPageRequestDto,
};
pub use values::{
    RangeCellValueDto, RangeReadOptionsDto, RangeValueDto, RangeValuesDto, RangeValuesRequestDto,
    RangeWriteRequestDto, UnavailableValueDto,
};

/// Version of the serialized interop contract.
pub const INTEROP_SCHEMA_VERSION: u32 = 1;

/// Version of function usage reports with decimal-string JSON call counts.
pub const INTEROP_FUNCTION_USAGE_SCHEMA_VERSION: u32 = 2;

/// Version of the parallel table-authoring edit contract.
pub const INTEROP_EDIT_SCHEMA_V2: u32 = 2;
