use cellrune_interop::{EditBatchDto, WorkbookSummaryDto, WriteReportDto};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Empty input for creating a canonical workbook session.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateWorkbookArgs {}

/// Input for opening an existing workbook beneath an approved root.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct OpenWorkbookArgs {
    /// Absolute path to an existing XLSX or XLSM file beneath an approved root.
    pub(crate) path: String,
}

/// Input identifying one resident workbook session.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionArgs {
    /// Opaque session identifier returned by `workbook_create` or `workbook_open`.
    pub(crate) session_id: String,
}

/// Input for one bounded row-major range page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadRangeArgs {
    /// Opaque resident workbook session identifier.
    pub(crate) session_id: String,
    /// Case-insensitive sheet name.
    pub(crate) sheet: String,
    /// Inclusive unqualified A1 start address.
    pub(crate) start: String,
    /// Inclusive unqualified A1 end address.
    pub(crate) end: String,
    /// Zero-based row-major cell offset; omitted or zero starts at the first cell.
    #[serde(default)]
    pub(crate) offset: u64,
    /// Page size; omitted or zero selects the server default, with a hard maximum of 10,000.
    #[serde(default)]
    #[schemars(range(max = 10_000))]
    pub(crate) limit: u32,
}

/// Input for a bounded static formula-capability page.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CapabilityArgs {
    /// Opaque resident workbook session identifier.
    pub(crate) session_id: String,
    /// Optional finite Excel serial returned by `TODAY()`.
    pub(crate) today_serial: Option<f64>,
    /// Optional finite Excel serial returned by `NOW()`.
    pub(crate) now_serial: Option<f64>,
    /// Zero-based formula-entry offset; omitted or zero starts at the first formula.
    #[serde(default)]
    pub(crate) offset: u64,
    /// Page size; omitted or zero selects the server default, with a hard maximum of 10,000.
    #[serde(default)]
    #[schemars(range(max = 10_000))]
    pub(crate) limit: u32,
}

/// Input for one optimistic, atomic workbook edit batch.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyChangesArgs {
    /// Opaque resident workbook session identifier.
    pub(crate) session_id: String,
    /// Current semantic revision that must still match when the batch commits.
    pub(crate) expected_revision: u64,
    /// Ordered typed changes committed together or not at all.
    #[serde(flatten)]
    pub(crate) batch: EditBatchDto,
}

/// Input for one full or incremental workbook recalculation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecalculateArgs {
    /// Opaque resident workbook session identifier.
    pub(crate) session_id: String,
    /// `auto`, `incremental`, or `full`; omitted selects `auto`.
    #[serde(default)]
    pub(crate) mode: cellrune_interop::RecalculationModeDto,
    /// Optional finite Excel serial returned by `TODAY()`.
    pub(crate) today_serial: Option<f64>,
    /// Optional finite Excel serial returned by `NOW()`.
    pub(crate) now_serial: Option<f64>,
}

/// Input for a page of installed recalculation deltas.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChangesSinceArgs {
    /// Opaque resident workbook session identifier.
    pub(crate) session_id: String,
    /// Exclusive installed-delta cursor; omitted or zero starts at the earliest retained delta.
    #[serde(default)]
    pub(crate) cursor: u64,
    /// Delta count; omitted or zero selects the session default, with a hard maximum of 100.
    #[serde(default)]
    #[schemars(range(max = 100))]
    pub(crate) limit: u32,
}

/// Input for a verified, atomic workbook Save As operation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SaveWorkbookArgs {
    /// Opaque resident workbook session identifier.
    pub(crate) session_id: String,
    /// Absolute destination path with an existing parent beneath an approved root.
    pub(crate) path: String,
    /// Invalidate stale caches for unavailable formulas instead of rejecting an incomplete save.
    #[serde(default)]
    pub(crate) invalidate_unavailable: bool,
    /// Replace an existing file; also requires the server's `--allow-overwrite` opt-in.
    #[serde(default)]
    pub(crate) replace_existing: bool,
}

/// Result of creating or opening one resident workbook session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionStarted {
    /// Interop schema version.
    pub(crate) schema_version: u32,
    /// New opaque resident workbook session identifier.
    pub(crate) session_id: String,
    /// Initial workbook and sheet metadata.
    pub(crate) summary: WorkbookSummaryDto,
}

impl SessionStarted {
    pub(crate) fn new(session_id: String, summary: WorkbookSummaryDto) -> Self {
        Self {
            schema_version: cellrune_interop::INTEROP_SCHEMA_VERSION,
            session_id,
            summary,
        }
    }
}

/// Result of reading metadata for one resident workbook session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionSummary {
    /// Interop schema version.
    pub(crate) schema_version: u32,
    /// Opaque resident workbook session identifier.
    pub(crate) session_id: String,
    /// Current workbook and sheet metadata.
    pub(crate) summary: WorkbookSummaryDto,
}

impl SessionSummary {
    pub(crate) fn new(session_id: String, summary: WorkbookSummaryDto) -> Self {
        Self {
            schema_version: cellrune_interop::INTEROP_SCHEMA_VERSION,
            session_id,
            summary,
        }
    }
}

/// Result of closing one resident workbook session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionClosed {
    /// Interop schema version.
    pub(crate) schema_version: u32,
    /// Opaque identifier of the removed workbook session.
    pub(crate) session_id: String,
    /// Always true for a successful close.
    pub(crate) closed: bool,
}

impl SessionClosed {
    pub(crate) fn new(session_id: String) -> Self {
        Self {
            schema_version: cellrune_interop::INTEROP_SCHEMA_VERSION,
            session_id,
            closed: true,
        }
    }
}

/// Result of a verified, atomic workbook Save As operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkbookSaved {
    /// Interop schema version.
    pub(crate) schema_version: u32,
    /// Opaque resident workbook session identifier.
    pub(crate) session_id: String,
    /// Normalized destination path beneath the approved root.
    pub(crate) destination: String,
    /// Exact package materialization report.
    pub(crate) report: WriteReportDto,
}

impl WorkbookSaved {
    pub(crate) fn new(session_id: String, destination: String, report: WriteReportDto) -> Self {
        Self {
            schema_version: cellrune_interop::INTEROP_SCHEMA_VERSION,
            session_id,
            destination,
            report,
        }
    }
}
