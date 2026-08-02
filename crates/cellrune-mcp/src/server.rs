use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{
    Implementation, ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
    ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
    ResourceContents, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, Json, RoleServer, ServerHandler};
use schemars::JsonSchema;
use serde::Serialize;

use crate::config::ServerConfig;
use crate::error::McpError;
use crate::resources::{
    FUNCTION_CATALOG_URI, list_resource_templates_page, list_resources_page,
    session_id_from_resource,
};
use crate::session::SessionCache;

const JSON_MIME_TYPE: &str = "application/json";
const SERVER_INSTRUCTIONS: &str = "Use high-level workbook session tools to open or create a \
workbook, apply typed edit batches (v2 for table authoring), recalculate existing formulas with CellRune, read bounded \
ranges, and save a verified copy. Tools are not defined per spreadsheet function. Paths must be \
absolute and remain inside an operator-approved root.";

/// Local stdio-only CellRune MCP server.
#[derive(Debug, Clone)]
pub struct CellruneMcpServer {
    pub(crate) tool_router: ToolRouter<Self>,
    pub(crate) sessions: SessionCache,
    pub(crate) config: ServerConfig,
}

impl CellruneMcpServer {
    /// Creates a server from a validated local policy.
    pub fn new(config: ServerConfig) -> Self {
        let sessions = SessionCache::new(config.max_sessions(), config.session_ttl());
        Self {
            tool_router: Self::tool_router(),
            sessions,
            config,
        }
    }

    pub(crate) fn bounded_json<T>(&self, value: T) -> Result<Json<T>, McpError>
    where
        T: Serialize + JsonSchema + 'static,
    {
        self.ensure_json_size(&value)?;
        Ok(Json(value))
    }

    pub(crate) fn ensure_json_size<T: Serialize>(&self, value: &T) -> Result<(), McpError> {
        let bytes = serde_json::to_vec(value)
            .map_err(|error| McpError::serialization(error.to_string()))?;
        if bytes.len() > self.config.max_response_bytes() {
            return Err(McpError::response_too_large(
                bytes.len() as u64,
                self.config.max_response_bytes() as u64,
            ));
        }
        Ok(())
    }

    async fn resource_text(&self, uri: &str) -> Result<String, McpError> {
        let text = if uri == FUNCTION_CATALOG_URI {
            serde_json::to_string(&cellrune_interop::function_catalog())
                .map_err(|error| McpError::serialization(error.to_string()))?
        } else if let Some(session_id) = session_id_from_resource(uri) {
            let handle = self.sessions.get(session_id)?;
            let summary = handle.workbook().lock().await.summary();
            self.sessions.touch(session_id)?;
            serde_json::to_string(&summary)
                .map_err(|error| McpError::serialization(error.to_string()))?
        } else {
            return Err(McpError::resource_not_found());
        };
        Ok(text)
    }
}

#[rmcp::tool_handler(router = self.tool_router)]
impl ServerHandler for CellruneMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_protocol_version(ProtocolVersion::V_2025_11_25)
        .with_server_info(
            Implementation::new("cellrune-mcp", env!("CARGO_PKG_VERSION"))
                .with_title("CellRune")
                .with_description(
                    "Local stdio MCP server for headless CellRune workbook workflows",
                ),
        )
        .with_instructions(SERVER_INSTRUCTIONS)
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let ids = self.sessions.ids().map_err(protocol_error)?;
        list_resources_page(
            ids,
            request.as_ref().and_then(|params| params.cursor.as_deref()),
            self.config.max_response_bytes(),
        )
        .map_err(protocol_error)
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        list_resource_templates_page(
            request.as_ref().and_then(|params| params.cursor.as_deref()),
            self.config.max_response_bytes(),
        )
        .map_err(protocol_error)
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let text = self
            .resource_text(&request.uri)
            .await
            .map_err(protocol_error)?;
        let result = ReadResourceResult::new(vec![
            ResourceContents::text(text, request.uri).with_mime_type(JSON_MIME_TYPE),
        ]);
        self.ensure_json_size(&result).map_err(protocol_error)?;
        Ok(result.into())
    }
}

fn protocol_error(error: McpError) -> ErrorData {
    let data = serde_json::to_value(error.payload()).ok();
    if error.is_missing_resource() {
        ErrorData::resource_not_found(error.to_string(), data)
    } else if error.is_invalid_resource_request() {
        ErrorData::invalid_params(error.to_string(), data)
    } else {
        ErrorData::internal_error(error.to_string(), data)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rmcp::ServerHandler;
    use serde_json::{Map, Value};

    use super::*;
    use crate::config::{
        DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_MAX_SESSIONS, DEFAULT_MAX_WORKBOOK_BYTES,
        DEFAULT_SESSION_TTL_SECONDS,
    };

    #[test]
    fn tool_catalog_is_high_level_and_schema_backed() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must follow the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "cellrune-mcp-catalog-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("test root must be created");
        let config = ServerConfig::new(
            vec![root.clone()],
            DEFAULT_MAX_SESSIONS,
            DEFAULT_SESSION_TTL_SECONDS,
            DEFAULT_MAX_RESPONSE_BYTES,
            DEFAULT_MAX_WORKBOOK_BYTES,
            false,
        )
        .expect("test configuration must be valid");
        let server = CellruneMcpServer::new(config);

        let tools = server.tool_router.list_all();
        let names = tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "workbook_apply_changes",
                "workbook_apply_changes_v2",
                "workbook_changes_since",
                "workbook_close",
                "workbook_create",
                "workbook_function_usage",
                "workbook_open",
                "workbook_read_range",
                "workbook_recalculate",
                "workbook_save_as",
                "workbook_scan_capabilities",
                "workbook_summary",
            ]
        );
        assert!(tools.iter().all(|tool| tool.output_schema.is_some()));
        for tool in &tools {
            for (schema_name, schema) in [
                ("input", Some(&tool.input_schema)),
                ("output", tool.output_schema.as_ref()),
            ] {
                if let Some(schema) = schema {
                    assert_schema_property_descriptions(&tool.name, schema_name, schema);
                }
            }
        }
        for (tool_name, expected_maximum) in [
            ("workbook_read_range", 10_000_u64),
            ("workbook_scan_capabilities", 10_000),
            ("workbook_changes_since", 100),
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool.name == tool_name)
                .expect("bounded tool must be present");
            let maximum = tool
                .input_schema
                .get("properties")
                .and_then(serde_json::Value::as_object)
                .and_then(|properties| properties.get("limit"))
                .and_then(|schema| schema.get("maximum"))
                .and_then(serde_json::Value::as_u64);
            assert_eq!(
                maximum,
                Some(expected_maximum),
                "{tool_name} must publish its hard limit in JSON Schema"
            );
        }
        let apply_changes = tools
            .iter()
            .find(|tool| tool.name == "workbook_apply_changes")
            .expect("apply-changes tool must be present");
        let changes = apply_changes
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get("changes"))
            .expect("apply-changes schema must expose its change list");
        assert_eq!(
            changes.get("minItems").and_then(Value::as_u64),
            Some(1),
            "apply-changes must advertise its non-empty batch invariant"
        );
        assert!(
            !schema_contains_const(
                &Value::Object(apply_changes.input_schema.as_ref().clone()),
                "unsupported"
            ),
            "the output-only unsupported value must not appear in an input schema"
        );
        let apply_changes_v2 = tools
            .iter()
            .find(|tool| tool.name == "workbook_apply_changes_v2")
            .expect("apply-changes-v2 tool must be present");
        let changes_v2 = apply_changes_v2
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get("changes"))
            .expect("apply-changes-v2 schema must expose its change list");
        assert_eq!(
            changes_v2.get("minItems").and_then(Value::as_u64),
            Some(1),
            "apply-changes-v2 must advertise its non-empty batch invariant"
        );
        let expected_annotations = [
            ("workbook_apply_changes", false, true, true, false),
            ("workbook_apply_changes_v2", false, true, true, false),
            ("workbook_changes_since", true, false, true, false),
            ("workbook_close", false, true, true, false),
            ("workbook_create", false, true, false, false),
            ("workbook_function_usage", true, false, true, false),
            ("workbook_open", false, true, false, false),
            ("workbook_read_range", true, false, true, false),
            ("workbook_recalculate", false, true, false, false),
            ("workbook_save_as", false, true, false, false),
            ("workbook_scan_capabilities", true, false, true, false),
            ("workbook_summary", true, false, true, false),
        ];
        for (name, read_only, destructive, idempotent, open_world) in expected_annotations {
            let annotations = tools
                .iter()
                .find(|tool| tool.name == name)
                .and_then(|tool| tool.annotations.as_ref())
                .expect("every tool must publish safety annotations");
            assert!(
                annotations
                    .title
                    .as_deref()
                    .is_some_and(|title| !title.trim().is_empty()),
                "{name} must publish a non-empty annotation title"
            );
            assert_eq!(annotations.read_only_hint, Some(read_only), "{name}");
            assert_eq!(annotations.destructive_hint, Some(destructive), "{name}");
            assert_eq!(annotations.idempotent_hint, Some(idempotent), "{name}");
            assert_eq!(annotations.open_world_hint, Some(open_world), "{name}");
        }
        assert_eq!(
            server.get_info().protocol_version,
            ProtocolVersion::V_2025_11_25
        );

        drop(server);
        fs::remove_dir_all(root).expect("test root must be removed");
    }

    fn assert_schema_property_descriptions(
        tool_name: &str,
        schema_name: &str,
        schema: &Map<String, Value>,
    ) {
        fn visit(tool_name: &str, schema_name: &str, path: &str, value: &Value) {
            match value {
                Value::Object(object) => {
                    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                        for (property_name, property_schema) in properties {
                            let property_path = format!("{path}.{property_name}");
                            if property_schema.get("const").is_none() {
                                let description = property_schema
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                assert!(
                                    !description.trim().is_empty(),
                                    "{tool_name} {schema_name} property {property_path} must have \
                                     a non-empty description"
                                );
                            }
                            visit(tool_name, schema_name, &property_path, property_schema);
                        }
                    }
                    for (key, child) in object {
                        if key != "properties" {
                            visit(tool_name, schema_name, &format!("{path}.{key}"), child);
                        }
                    }
                }
                Value::Array(items) => {
                    for (index, item) in items.iter().enumerate() {
                        visit(tool_name, schema_name, &format!("{path}[{index}]"), item);
                    }
                }
                _ => {}
            }
        }

        visit(tool_name, schema_name, "$", &Value::Object(schema.clone()));
    }

    fn schema_contains_const(value: &Value, expected: &str) -> bool {
        match value {
            Value::Object(object) => {
                object.get("const").and_then(Value::as_str) == Some(expected)
                    || object
                        .values()
                        .any(|value| schema_contains_const(value, expected))
            }
            Value::Array(items) => items
                .iter()
                .any(|value| schema_contains_const(value, expected)),
            _ => false,
        }
    }

    #[test]
    fn resource_protocol_errors_preserve_json_rpc_semantics() {
        let missing = protocol_error(McpError::session_not_found());
        assert_eq!(missing.code.0, -32_002);
        let expired = protocol_error(McpError::session_expired());
        assert_eq!(expired.code.0, -32_002);
        let malformed = protocol_error(McpError::resource_cursor_invalid());
        assert_eq!(malformed.code.0, -32_602);
    }
}
