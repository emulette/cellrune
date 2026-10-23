//! Stable codes the MCP server reports for its own failures; interop errors keep theirs.

/// A stable dotted error code owned by the MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum McpCode {
    /// At least one allowed workbook root is required.
    ConfigRootRequired,
    /// Allowed workbook root is not an accessible directory.
    ConfigRootInvalid,
    /// MCP server limit is outside the supported range.
    ConfigLimitInvalid,
    /// Workbook path must be absolute.
    PathAbsoluteRequired,
    /// Workbook path is outside every allowed root.
    PathOutsideRoot,
    /// Workbook path cannot be resolved safely.
    PathInvalid,
    /// Input workbook path is not a regular file.
    PathInputNotFile,
    /// Input workbook exceeds the configured byte limit.
    InputByteLimitExceeded,
    /// Server policy does not allow replacing workbooks.
    OutputOverwriteDisallowed,
    /// Workbook session does not exist.
    SessionNotFound,
    /// Workbook session expired.
    SessionExpired,
    /// Workbook session is busy.
    SessionBusy,
    /// All bounded workbook session slots are busy.
    SessionCacheFull,
    /// Workbook session identifier is exhausted.
    SessionIdExhausted,
    /// Workbook session cache is unavailable.
    SessionUnavailable,
    /// MCP response exceeds the configured byte limit.
    ResponseByteLimitExceeded,
    /// Recalculation was installed, but its delta response exceeds the configured byte limit.
    RecalculationResponseByteLimitExceeded,
    /// MCP response serialization failed.
    ResponseSerializationFailed,
    /// Blocking workbook operation failed.
    WorkerFailed,
    /// MCP resource does not exist.
    ResourceNotFound,
    /// MCP resource cursor is invalid.
    ResourceCursorInvalid,
    /// MCP workbook operation was cancelled.
    OperationCancelled,
}

impl McpCode {
    /// Returns the stable dotted code.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ConfigRootRequired => "mcp.config.root_required",
            Self::ConfigRootInvalid => "mcp.config.root_invalid",
            Self::ConfigLimitInvalid => "mcp.config.limit_invalid",
            Self::PathAbsoluteRequired => "mcp.path.absolute_required",
            Self::PathOutsideRoot => "mcp.path.outside_root",
            Self::PathInvalid => "mcp.path.invalid",
            Self::PathInputNotFile => "mcp.path.input_not_file",
            Self::InputByteLimitExceeded => "mcp.input.byte_limit_exceeded",
            Self::OutputOverwriteDisallowed => "mcp.output.overwrite_disallowed",
            Self::SessionNotFound => "mcp.session.not_found",
            Self::SessionExpired => "mcp.session.expired",
            Self::SessionBusy => "mcp.session.busy",
            Self::SessionCacheFull => "mcp.session.cache_full",
            Self::SessionIdExhausted => "mcp.session.id_exhausted",
            Self::SessionUnavailable => "mcp.session.unavailable",
            Self::ResponseByteLimitExceeded => "mcp.response.byte_limit_exceeded",
            Self::RecalculationResponseByteLimitExceeded => {
                "mcp.recalculation.response_byte_limit_exceeded"
            }
            Self::ResponseSerializationFailed => "mcp.response.serialization_failed",
            Self::WorkerFailed => "mcp.worker.failed",
            Self::ResourceNotFound => "mcp.resource.not_found",
            Self::ResourceCursorInvalid => "mcp.resource.cursor_invalid",
            Self::OperationCancelled => "mcp.operation.cancelled",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::McpCode;

    /// Lists the codes of every variant. Naming each variant in an exhaustive match makes this
    /// fail to compile when the enum gains one, so a new code cannot skip the registry.
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
    fn the_error_code_reference_lists_every_mcp_code() {
        let codes = codes!(McpCode::as_str;
            ConfigRootRequired,
            ConfigRootInvalid,
            ConfigLimitInvalid,
            PathAbsoluteRequired,
            PathOutsideRoot,
            PathInvalid,
            PathInputNotFile,
            InputByteLimitExceeded,
            OutputOverwriteDisallowed,
            SessionNotFound,
            SessionExpired,
            SessionBusy,
            SessionCacheFull,
            SessionIdExhausted,
            SessionUnavailable,
            ResponseByteLimitExceeded,
            RecalculationResponseByteLimitExceeded,
            ResponseSerializationFailed,
            WorkerFailed,
            ResourceNotFound,
            ResourceCursorInvalid,
            OperationCancelled,
        )
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
        let documented = documented("MCP server");
        let undocumented = codes.difference(&documented).collect::<Vec<_>>();
        let stale = documented.difference(&codes).collect::<Vec<_>>();
        assert!(
            undocumented.is_empty() && stale.is_empty(),
            "docs/ERROR_CODES.md must list exactly these codes; undocumented: {undocumented:?}, \
             not defined: {stale:?}"
        );
    }
}
