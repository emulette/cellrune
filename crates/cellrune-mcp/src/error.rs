use std::error::Error;
use std::fmt;

use cellrune_interop::{ErrorDetails as InteropErrorDetails, InteropError, InteropErrorKind};
use rmcp::handler::server::tool::IntoCallToolResult;
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MESSAGE_ROOT_REQUIRED: &str = "at least one allowed workbook root is required";
const MESSAGE_ROOT_INVALID: &str = "allowed workbook root is not an accessible directory";
const MESSAGE_CONFIG_LIMIT: &str = "MCP server limit is outside the supported range";
const MESSAGE_PATH_ABSOLUTE: &str = "workbook path must be absolute";
const MESSAGE_PATH_OUTSIDE_ROOT: &str = "workbook path is outside every allowed root";
const MESSAGE_PATH_INVALID: &str = "workbook path cannot be resolved safely";
const MESSAGE_INPUT_NOT_FILE: &str = "input workbook path is not a regular file";
const MESSAGE_INPUT_TOO_LARGE: &str = "input workbook exceeds the configured byte limit";
const MESSAGE_OVERWRITE_DISALLOWED: &str = "server policy does not allow replacing workbooks";
const MESSAGE_SESSION_NOT_FOUND: &str = "workbook session does not exist";
const MESSAGE_SESSION_EXPIRED: &str = "workbook session expired";
const MESSAGE_SESSION_BUSY: &str = "workbook session is busy";
const MESSAGE_SESSION_CACHE_FULL: &str = "all bounded workbook session slots are busy";
const MESSAGE_SESSION_ID_EXHAUSTED: &str = "workbook session identifier is exhausted";
const MESSAGE_SESSION_STATE: &str = "workbook session cache is unavailable";
const MESSAGE_RESPONSE_TOO_LARGE: &str = "MCP response exceeds the configured byte limit";
const MESSAGE_SERIALIZATION: &str = "MCP response serialization failed";
const MESSAGE_WORKER: &str = "blocking workbook operation failed";
const MESSAGE_RESOURCE_NOT_FOUND: &str = "MCP resource does not exist";
const MESSAGE_RESOURCE_CURSOR_INVALID: &str = "MCP resource cursor is invalid";
const MESSAGE_CANCELLED: &str = "MCP workbook operation was cancelled";

const CODE_RESOURCE_NOT_FOUND: &str = "mcp.resource.not_found";
const CODE_RESOURCE_CURSOR_INVALID: &str = "mcp.resource.cursor_invalid";
const CODE_SESSION_NOT_FOUND: &str = "mcp.session.not_found";
const CODE_SESSION_EXPIRED: &str = "mcp.session.expired";

/// Broad stable MCP error category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpErrorKind {
    /// Transport-facing input is invalid.
    Input,
    /// A configured filesystem boundary rejected an operation.
    Path,
    /// A bounded workbook session could not serve the operation.
    Session,
    /// A server policy rejected the operation.
    Policy,
    /// A workbook interop operation failed.
    Interop,
    /// The local MCP service failed internally.
    Internal,
}

/// Structured context attached to a stable MCP error.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpErrorDetails {
    /// Lower-layer stable error code, when available.
    pub source_code: Option<String>,
    /// Lower-layer source identifier, when available.
    pub source_id: Option<String>,
    /// Operation-specific detail.
    pub detail: Option<String>,
    /// Configured maximum byte count, when relevant.
    pub maximum_bytes: Option<u64>,
    /// Observed byte count, when relevant.
    pub actual_bytes: Option<u64>,
}

/// Stable error payload returned from a CellRune MCP tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpErrorPayload {
    /// Interop schema version.
    pub schema_version: u32,
    /// Broad error category.
    pub kind: McpErrorKind,
    /// Stable dotted error code.
    pub code: String,
    /// Stable human-readable message.
    pub message: String,
    /// Optional structured error context.
    pub details: McpErrorDetails,
}

/// Error shared by MCP tools, resources, configuration, and the stdio runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpError {
    payload: Box<McpErrorPayload>,
}

impl McpError {
    fn new(
        kind: McpErrorKind,
        code: &'static str,
        message: &'static str,
        details: McpErrorDetails,
    ) -> Self {
        Self {
            payload: Box::new(McpErrorPayload {
                schema_version: cellrune_interop::INTEROP_SCHEMA_VERSION,
                kind,
                code: code.to_owned(),
                message: message.to_owned(),
                details,
            }),
        }
    }

    pub(crate) fn root_required() -> Self {
        Self::new(
            McpErrorKind::Input,
            "mcp.config.root_required",
            MESSAGE_ROOT_REQUIRED,
            McpErrorDetails::default(),
        )
    }

    pub(crate) fn root_invalid(detail: String) -> Self {
        Self::new(
            McpErrorKind::Path,
            "mcp.config.root_invalid",
            MESSAGE_ROOT_INVALID,
            McpErrorDetails {
                detail: Some(detail),
                ..McpErrorDetails::default()
            },
        )
    }

    pub(crate) fn config_limit(detail: String) -> Self {
        Self::new(
            McpErrorKind::Input,
            "mcp.config.limit_invalid",
            MESSAGE_CONFIG_LIMIT,
            McpErrorDetails {
                detail: Some(detail),
                ..McpErrorDetails::default()
            },
        )
    }

    pub(crate) fn path_absolute() -> Self {
        Self::new(
            McpErrorKind::Path,
            "mcp.path.absolute_required",
            MESSAGE_PATH_ABSOLUTE,
            McpErrorDetails::default(),
        )
    }

    pub(crate) fn path_outside_root() -> Self {
        Self::new(
            McpErrorKind::Path,
            "mcp.path.outside_root",
            MESSAGE_PATH_OUTSIDE_ROOT,
            McpErrorDetails::default(),
        )
    }

    pub(crate) fn path_invalid(detail: String) -> Self {
        Self::new(
            McpErrorKind::Path,
            "mcp.path.invalid",
            MESSAGE_PATH_INVALID,
            McpErrorDetails {
                detail: Some(detail),
                ..McpErrorDetails::default()
            },
        )
    }

    pub(crate) fn input_not_file() -> Self {
        Self::new(
            McpErrorKind::Path,
            "mcp.path.input_not_file",
            MESSAGE_INPUT_NOT_FILE,
            McpErrorDetails::default(),
        )
    }

    pub(crate) fn input_too_large(actual: u64, maximum: u64) -> Self {
        Self::new(
            McpErrorKind::Policy,
            "mcp.input.byte_limit_exceeded",
            MESSAGE_INPUT_TOO_LARGE,
            McpErrorDetails {
                maximum_bytes: Some(maximum),
                actual_bytes: Some(actual),
                ..McpErrorDetails::default()
            },
        )
    }

    pub(crate) fn overwrite_disallowed() -> Self {
        Self::new(
            McpErrorKind::Policy,
            "mcp.output.overwrite_disallowed",
            MESSAGE_OVERWRITE_DISALLOWED,
            McpErrorDetails::default(),
        )
    }

    pub(crate) fn session_not_found() -> Self {
        Self::new(
            McpErrorKind::Session,
            CODE_SESSION_NOT_FOUND,
            MESSAGE_SESSION_NOT_FOUND,
            McpErrorDetails::default(),
        )
    }

    pub(crate) fn session_expired() -> Self {
        Self::new(
            McpErrorKind::Session,
            CODE_SESSION_EXPIRED,
            MESSAGE_SESSION_EXPIRED,
            McpErrorDetails::default(),
        )
    }

    pub(crate) fn session_busy() -> Self {
        Self::new(
            McpErrorKind::Session,
            "mcp.session.busy",
            MESSAGE_SESSION_BUSY,
            McpErrorDetails::default(),
        )
    }

    pub(crate) fn session_cache_full() -> Self {
        Self::new(
            McpErrorKind::Session,
            "mcp.session.cache_full",
            MESSAGE_SESSION_CACHE_FULL,
            McpErrorDetails::default(),
        )
    }

    pub(crate) fn session_id_exhausted() -> Self {
        Self::new(
            McpErrorKind::Session,
            "mcp.session.id_exhausted",
            MESSAGE_SESSION_ID_EXHAUSTED,
            McpErrorDetails::default(),
        )
    }

    pub(crate) fn session_state() -> Self {
        Self::new(
            McpErrorKind::Internal,
            "mcp.session.unavailable",
            MESSAGE_SESSION_STATE,
            McpErrorDetails::default(),
        )
    }

    pub(crate) fn response_too_large(actual: u64, maximum: u64) -> Self {
        Self::new(
            McpErrorKind::Policy,
            "mcp.response.byte_limit_exceeded",
            MESSAGE_RESPONSE_TOO_LARGE,
            McpErrorDetails {
                maximum_bytes: Some(maximum),
                actual_bytes: Some(actual),
                ..McpErrorDetails::default()
            },
        )
    }

    pub(crate) fn serialization(detail: String) -> Self {
        Self::new(
            McpErrorKind::Internal,
            "mcp.response.serialization_failed",
            MESSAGE_SERIALIZATION,
            McpErrorDetails {
                detail: Some(detail),
                ..McpErrorDetails::default()
            },
        )
    }

    pub(crate) fn worker(detail: String) -> Self {
        Self::new(
            McpErrorKind::Internal,
            "mcp.worker.failed",
            MESSAGE_WORKER,
            McpErrorDetails {
                detail: Some(detail),
                ..McpErrorDetails::default()
            },
        )
    }

    pub(crate) fn resource_not_found() -> Self {
        Self::new(
            McpErrorKind::Input,
            CODE_RESOURCE_NOT_FOUND,
            MESSAGE_RESOURCE_NOT_FOUND,
            McpErrorDetails::default(),
        )
    }

    pub(crate) fn resource_cursor_invalid() -> Self {
        Self::new(
            McpErrorKind::Input,
            CODE_RESOURCE_CURSOR_INVALID,
            MESSAGE_RESOURCE_CURSOR_INVALID,
            McpErrorDetails::default(),
        )
    }

    pub(crate) fn is_missing_resource(&self) -> bool {
        matches!(
            self.payload.code.as_str(),
            CODE_RESOURCE_NOT_FOUND | CODE_SESSION_NOT_FOUND | CODE_SESSION_EXPIRED
        )
    }

    pub(crate) fn is_invalid_resource_request(&self) -> bool {
        self.payload.code == CODE_RESOURCE_CURSOR_INVALID
    }

    pub(crate) fn cancelled() -> Self {
        Self::new(
            McpErrorKind::Session,
            "mcp.operation.cancelled",
            MESSAGE_CANCELLED,
            McpErrorDetails::default(),
        )
    }

    /// Returns the stable serialized payload.
    pub const fn payload(&self) -> &McpErrorPayload {
        &self.payload
    }
}

impl From<InteropError> for McpError {
    fn from(error: InteropError) -> Self {
        let kind = match error.kind() {
            InteropErrorKind::Input | InteropErrorKind::Validation => McpErrorKind::Interop,
            InteropErrorKind::Read | InteropErrorKind::Write => McpErrorKind::Interop,
            InteropErrorKind::State => McpErrorKind::Session,
        };
        let InteropErrorDetails {
            source_code,
            source_id,
            detail,
        } = error.details().clone();
        Self {
            payload: Box::new(McpErrorPayload {
                schema_version: cellrune_interop::INTEROP_SCHEMA_VERSION,
                kind,
                code: error.code().to_owned(),
                message: error.message().to_owned(),
                details: McpErrorDetails {
                    source_code,
                    source_id,
                    detail,
                    ..McpErrorDetails::default()
                },
            }),
        }
    }
}

impl fmt::Display for McpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.payload.code, self.payload.message)?;
        if let Some(detail) = &self.payload.details.detail {
            write!(formatter, ": {detail}")?;
        }
        Ok(())
    }
}

impl Error for McpError {}

impl IntoCallToolResult for McpError {
    fn into_call_tool_result(self) -> Result<CallToolResult, rmcp::ErrorData> {
        let value = serde_json::to_value(*self.payload).map_err(|error| {
            rmcp::ErrorData::internal_error(MESSAGE_SERIALIZATION, Some(error.to_string().into()))
        })?;
        Ok(CallToolResult::structured_error(value))
    }
}
