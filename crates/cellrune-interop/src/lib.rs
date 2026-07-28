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

pub use dto::{
    ArithmeticSemanticsDto, CalculationDeltaCellDto, CalculationDeltaDto, CalculationDeltaPageDto,
    CalculationOptionsDto, CalculationReportDto, CalculationResultDto, CapabilityEntryDto,
    CapabilityPageDto, CellDto, CellReferenceDto, CellValueDto, EditBatchDto, EditReceiptDto,
    FinancialSolverSemanticsDto, FunctionCatalogEntryDto, FunctionCatalogReportDto,
    FunctionUsageEntryDto, FunctionUsageReportDto, RangePageDto, RangeRequestDto,
    RecalculationModeDto, SavedValueStateDto, SheetSummaryDto, TableColumnDto, TableSummaryDto,
    WorkbookChangeDto, WorkbookSummaryDto, WritableCellValueDto, WriteOptionsDto, WriteReportDto,
};
pub use error::{ErrorDetails, InteropError, InteropErrorKind};
pub use service::{
    CompletedRecalculation, DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, PreparedChanges,
    PreparedRecalculation, PreparedWorkbookSave, WorkbookSession, function_catalog,
};

/// Version of the serialized interop contract.
pub const INTEROP_SCHEMA_VERSION: u32 = 1;
