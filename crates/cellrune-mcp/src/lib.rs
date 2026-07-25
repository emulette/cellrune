//! Local stdio MCP server for bounded headless CellRune workbook workflows.
//!
//! The server exposes high-level workbook operations over the shared `cellrune-interop` session
//! contract. It deliberately contains no calculation graph, formula kernel, workbook writer, or
//! network transport of its own.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod config;
mod error;
mod model;
mod resources;
mod schema;
mod server;
mod session;
mod tools;

pub use config::{
    DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_MAX_SESSIONS, DEFAULT_MAX_WORKBOOK_BYTES,
    DEFAULT_SESSION_TTL_SECONDS, MAX_RESPONSE_BYTES, MAX_SESSION_TTL_SECONDS, MAX_SESSIONS,
    MAX_WORKBOOK_BYTES, ResolvedInput, ResolvedOutput, ServerConfig,
};
pub use error::{McpError, McpErrorDetails, McpErrorKind, McpErrorPayload};
pub use server::CellruneMcpServer;
