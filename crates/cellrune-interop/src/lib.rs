//! Typed, versioned services shared by CellRune language bindings and local MCP.
//!
//! The crate intentionally owns no spreadsheet semantics. It validates transport-facing inputs,
//! converts them into `cellrune` domain types, and converts domain results back into owned DTOs.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod convert;
mod dto;
mod error;
mod service;

pub use cellrune::CancellationToken;
pub use dto::{
    ArithmeticSemanticsDto, CalculationDeltaCellDto, CalculationDeltaDto, CalculationDeltaPageDto,
    CalculationIssueDto, CalculationLimitsDto, CalculationOptionsDto, CalculationOptionsReportDto,
    CalculationReportDto, CalculationResultDto, CapabilityEntryDto, CapabilityPageDto, CellDto,
    CellReferenceDto, CellValueDto, DefinedNameDynamicKindDto, DefinedNameExternalTargetKindDto,
    DefinedNameInspectionDto, DefinedNameInspectionRequestDto, DefinedNameInspectionResultDto,
    DefinedNameInvalidReasonDto, DefinedNameReferenceAreaDto, DefinedNameSheetSpanDto,
    DefinedNameUnsupportedReasonDto, EditBatchDto, EditBatchV2Dto, EditReceiptDto,
    EditReceiptV2Dto, FinancialSolverSemanticsDto, FunctionCatalogEntryDto,
    FunctionCatalogReportDto, FunctionUsageEntryDto, FunctionUsageReportDto,
    MaterializedResultOriginDto, PreviewChangesDto, PreviewCursorDto, ProviderIdentityDto,
    RangePageDto, RangeRequestDto, RecalculationModeDto, SavedValueStateDto, SheetSummaryDto,
    TableChangeV2Dto, TableColumnDto, TableSummaryDto, TransactionDetailCountsDto,
    TransactionDetailItemDto, TransactionDetailSectionDto, TransactionImpactCoverageDto,
    TransactionImpactPageDto, WorkbookChangeDto, WorkbookChangeV2Dto, WorkbookFingerprintDto,
    WorkbookSummaryDto, WorkbookTransactionReceiptDto, WorkbookTransactionReportDto,
    WritableCellValueDto, WriteOptionsDto, WriteReportDto,
};
pub use error::{ErrorDetails, InteropError, InteropErrorKind};
pub use service::{
    CompletedPreview, CompletedRecalculation, DEFAULT_PAGE_SIZE, DEFAULT_PREVIEW_PAGE_SIZE,
    MAX_PAGE_SIZE, MAX_PREVIEW_PAGE_SIZE, PreparedChanges, PreparedChangesV2, PreparedPreview,
    PreparedRecalculation, PreparedWorkbookSave, WorkbookSession, function_catalog,
};

/// Version of the serialized interop contract.
pub const INTEROP_SCHEMA_VERSION: u32 = 1;

/// Version of the parallel table-authoring edit contract.
pub const INTEROP_EDIT_SCHEMA_V2: u32 = 2;
