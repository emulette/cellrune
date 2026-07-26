use std::error::Error;
use std::path::PathBuf;

use cellrune_mcp::{
    CellruneMcpServer, DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_MAX_SESSIONS,
    DEFAULT_MAX_WORKBOOK_BYTES, DEFAULT_SESSION_TTL_SECONDS, ServerConfig,
};
use clap::{Parser, ValueEnum};
use rmcp::{ServiceExt, transport::stdio};
use tracing::level_filters::LevelFilter;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<LogLevel> for LevelFilter {
    fn from(value: LogLevel) -> Self {
        match value {
            LogLevel::Off => Self::OFF,
            LogLevel::Error => Self::ERROR,
            LogLevel::Warn => Self::WARN,
            LogLevel::Info => Self::INFO,
            LogLevel::Debug => Self::DEBUG,
            LogLevel::Trace => Self::TRACE,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "cellrune-mcp",
    version,
    about = "Local stdio MCP server for headless CellRune workbook workflows"
)]
struct Cli {
    /// Directory boundary canonicalized at startup for workbook input and Save As; may be repeated.
    #[arg(long = "root", required = true)]
    roots: Vec<PathBuf>,

    /// Maximum resident workbook sessions.
    #[arg(long, default_value_t = DEFAULT_MAX_SESSIONS)]
    max_sessions: usize,

    /// Idle seconds before a workbook session expires.
    #[arg(long, default_value_t = DEFAULT_SESSION_TTL_SECONDS)]
    session_ttl_seconds: u64,

    /// Maximum bytes in one structured tool or resource payload.
    #[arg(long, default_value_t = DEFAULT_MAX_RESPONSE_BYTES)]
    max_response_bytes: usize,

    /// Maximum bytes accepted for one input workbook.
    #[arg(long, default_value_t = DEFAULT_MAX_WORKBOOK_BYTES)]
    max_workbook_bytes: u64,

    /// Permit a tool request with replace_existing=true to overwrite a destination.
    #[arg(long, default_value_t = false)]
    allow_overwrite: bool,

    /// Diagnostic verbosity written only to stderr.
    #[arg(long, value_enum, default_value_t = LogLevel::Warn)]
    log_level: LogLevel,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::from(cli.log_level))
        .with_writer(std::io::stderr)
        .try_init()?;

    let config = ServerConfig::new(
        cli.roots,
        cli.max_sessions,
        cli.session_ttl_seconds,
        cli.max_response_bytes,
        cli.max_workbook_bytes,
        cli.allow_overwrite,
    )?;
    tracing::info!(
        allowed_root_count = config.roots().len(),
        max_sessions = config.max_sessions(),
        "CellRune MCP stdio server starting"
    );
    let service = CellruneMcpServer::new(config).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
