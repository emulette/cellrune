use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use cellrune_interop::{
    CalculationDeltaDto, CalculationOptionsDto, EditBatchDto, RecalculationModeDto,
    WorkbookChangeDto, WorkbookSession, WritableCellValueDto, WriteOptionsDto, WriteReportDto,
    function_catalog,
};
use serde_json::{Value, json};

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must follow the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "cellrune-mcp-stdio-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("test directory must be created");
        Self { path }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct McpProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: ChildStderr,
    next_id: u64,
    response_lines: Vec<String>,
}

impl McpProcess {
    fn start(root: &Path, extra_args: &[&str]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cellrune-mcp"));
        command
            .arg("--root")
            .arg(root)
            .arg("--log-level")
            .arg("info")
            .args(extra_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("MCP process must start");
        let stdin = child.stdin.take().expect("MCP stdin must be piped");
        let stdout = child.stdout.take().expect("MCP stdout must be piped");
        let stderr = child.stderr.take().expect("MCP stderr must be piped");
        Self {
            child,
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            stderr,
            next_id: 1,
            response_lines: Vec::new(),
        }
    }

    fn initialize(&mut self) -> Value {
        let response = self.request(
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {
                    "name": "cellrune-mcp-integration-test",
                    "version": "1"
                }
            }),
        );
        self.notify("notifications/initialized", None);
        response
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }));
        let mut line = String::new();
        let bytes = self
            .stdout
            .read_line(&mut line)
            .expect("MCP response must be readable");
        assert_ne!(bytes, 0, "MCP process closed before responding");
        let response: Value =
            serde_json::from_str(&line).expect("each stdout line must be valid JSON");
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], id);
        self.response_lines.push(line);
        response
    }

    fn notify(&mut self, method: &str, params: Option<Value>) {
        let mut message = json!({
            "jsonrpc": "2.0",
            "method": method
        });
        if let Some(params) = params {
            message["params"] = params;
        }
        self.send(&message);
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        self.request(
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments
            }),
        )["result"]
            .clone()
    }

    fn finish(mut self) -> (ExitStatus, String, Vec<String>) {
        drop(self.stdin.take());
        let mut remaining_stdout = String::new();
        self.stdout
            .read_to_string(&mut remaining_stdout)
            .expect("remaining MCP stdout must be readable");
        for line in remaining_stdout.lines() {
            serde_json::from_str::<Value>(line)
                .expect("every remaining stdout line must be valid JSON");
        }
        let status = self.child.wait().expect("MCP process must exit");
        let mut stderr = String::new();
        self.stderr
            .read_to_string(&mut stderr)
            .expect("MCP stderr must be readable");
        (status, stderr, self.response_lines)
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("MCP stdin must remain open");
        serde_json::to_writer(&mut *stdin, message).expect("MCP request must serialize");
        stdin
            .write_all(b"\n")
            .expect("MCP request delimiter must be written");
        stdin.flush().expect("MCP request must be flushed");
    }
}

#[test]
fn stdio_workflow_matches_the_rust_interop_session() {
    let root = TestDirectory::new("workflow-root");
    let outside = TestDirectory::new("workflow-outside");
    let mcp_output = root.path.join("mcp-output.xlsx");
    let rust_output = root.path.join("rust-output.xlsx");
    let outside_input = outside.path.join("outside.xlsx");
    fs::write(&outside_input, b"outside root").expect("outside fixture must be written");

    let changes = vec![
        WorkbookChangeDto::SetValue {
            sheet: "Sheet1".to_owned(),
            address: "A1".to_owned(),
            value: WritableCellValueDto::Number { value: 2.0 },
        },
        WorkbookChangeDto::SetFormula {
            sheet: "Sheet1".to_owned(),
            address: "A2".to_owned(),
            formula: "=SUM(A1,4)".to_owned(),
            dynamic_range: None,
        },
    ];
    let mut direct = WorkbookSession::create();
    direct
        .apply_changes(
            0,
            EditBatchDto {
                changes: changes.clone(),
            },
        )
        .expect("direct edit must succeed");
    let direct_delta = direct
        .recalculate(RecalculationModeDto::Auto, CalculationOptionsDto::default())
        .expect("direct recalculation must succeed");
    let direct_write_report = direct
        .save_path(&rust_output, WriteOptionsDto::default())
        .expect("direct save must succeed");

    let mut mcp = McpProcess::start(&root.path, &[]);
    let initialization = mcp.initialize();
    assert_eq!(initialization["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        initialization["result"]["serverInfo"]["name"],
        "cellrune-mcp"
    );

    let listed = mcp.request("tools/list", json!({}));
    let tools = listed["result"]["tools"]
        .as_array()
        .expect("tool list must be an array");
    assert_eq!(tools.len(), 16);
    assert!(
        tools
            .iter()
            .all(|tool| tool["inputSchema"].is_object() && tool["outputSchema"].is_object())
    );
    assert!(
        tools
            .iter()
            .all(|tool| tool["name"].as_str().unwrap_or_default() != "SUM")
    );
    for name in [
        "workbook_preview_changes",
        "workbook_preview_changes_page",
        "workbook_commit_preview",
        "workbook_discard_preview",
    ] {
        assert!(
            tools.iter().any(|tool| tool["name"] == name),
            "{name} must be exposed through stdio"
        );
    }

    let created = successful_tool(mcp.call_tool("workbook_create", json!({})));
    let session_id = created["session_id"]
        .as_str()
        .expect("create must return a session ID")
        .to_owned();
    assert_eq!(created["summary"]["semantic_revision"], 0);
    let summary =
        successful_tool(mcp.call_tool("workbook_summary", json!({"session_id": session_id})));
    assert_eq!(summary["summary"], created["summary"]);

    let edit = successful_tool(mcp.call_tool(
        "workbook_apply_changes",
        json!({
            "session_id": session_id,
            "expected_revision": 0,
            "changes": changes
        }),
    ));
    assert_eq!(edit["result_revision"], 1);
    assert_eq!(edit["applied_change_count"], 2);

    let usage = successful_tool(
        mcp.call_tool("workbook_function_usage", json!({"session_id": session_id})),
    );
    assert_eq!(usage["entries"][0]["name"], "SUM");
    assert_eq!(usage["entries"][0]["supported"], true);

    let capabilities = successful_tool(mcp.call_tool(
        "workbook_scan_capabilities",
        json!({"session_id": session_id, "offset": 0, "limit": 100}),
    ));
    assert_eq!(capabilities["formula_count"], 1);
    assert_eq!(capabilities["supported_count"], 1);

    let recalculated = successful_tool(mcp.call_tool(
        "workbook_recalculate",
        json!({"session_id": session_id, "mode": "auto"}),
    ));
    let mcp_delta: CalculationDeltaDto =
        serde_json::from_value(recalculated.clone()).expect("delta must match the shared DTO");
    assert_eq!(mcp_delta, direct_delta);

    let range = successful_tool(mcp.call_tool(
        "workbook_read_range",
        json!({
            "session_id": session_id,
            "sheet": "Sheet1",
            "start": "A1",
            "end": "A2",
            "offset": 0,
            "limit": 100
        }),
    ));
    assert_eq!(
        range["cells"][1]["calculated"]["value"]["value"],
        json!(6.0)
    );

    let deltas = successful_tool(mcp.call_tool(
        "workbook_changes_since",
        json!({"session_id": session_id, "cursor": 0, "limit": 100}),
    ));
    assert_eq!(deltas["deltas"].as_array().map(Vec::len), Some(1));

    let saved = successful_tool(mcp.call_tool(
        "workbook_save_as",
        json!({
            "session_id": session_id,
            "path": mcp_output,
            "invalidate_unavailable": false,
            "replace_existing": false
        }),
    ));
    let mcp_write_report: WriteReportDto = serde_json::from_value(saved["report"].clone())
        .expect("write report must match the shared DTO");
    assert_eq!(mcp_write_report, direct_write_report);
    assert!(mcp_output.is_file());

    let overwrite_error = failed_tool(mcp.call_tool(
        "workbook_save_as",
        json!({
            "session_id": session_id,
            "path": mcp_output,
            "replace_existing": true
        }),
    ));
    assert_eq!(overwrite_error["code"], "mcp.output.overwrite_disallowed");

    let outside_error = failed_tool(mcp.call_tool("workbook_open", json!({"path": outside_input})));
    assert_eq!(outside_error["code"], "mcp.path.outside_root");

    let resources = mcp.request("resources/list", json!({}));
    let resource_entries = resources["result"]["resources"]
        .as_array()
        .expect("resources must be an array");
    assert!(
        resource_entries
            .iter()
            .any(|resource| resource["uri"] == "cellrune://support/functions")
    );
    let summary_uri = format!("cellrune://sessions/{session_id}/summary");
    assert!(
        resource_entries
            .iter()
            .any(|resource| resource["uri"] == summary_uri)
    );
    let catalog = mcp.request(
        "resources/read",
        json!({"uri": "cellrune://support/functions"}),
    );
    let catalog_text = catalog["result"]["contents"][0]["text"]
        .as_str()
        .expect("catalog resource must contain JSON text");
    let catalog_json: Value =
        serde_json::from_str(catalog_text).expect("catalog resource must be valid JSON");
    assert_eq!(
        catalog_json,
        serde_json::to_value(function_catalog()).expect("interop catalog must serialize")
    );
    let session_resource = mcp.request("resources/read", json!({"uri": summary_uri}));
    let session_resource_text = session_resource["result"]["contents"][0]["text"]
        .as_str()
        .expect("session resource must contain JSON text");
    let session_resource_json: Value =
        serde_json::from_str(session_resource_text).expect("session resource must be valid JSON");
    assert_eq!(session_resource_json["semantic_revision"], 1);
    let templates = mcp.request("resources/templates/list", json!({}));
    assert_eq!(
        templates["result"]["resourceTemplates"][0]["uriTemplate"],
        "cellrune://sessions/{session_id}/summary"
    );

    successful_tool(mcp.call_tool("workbook_close", json!({"session_id": session_id})));
    let reopened = successful_tool(mcp.call_tool("workbook_open", json!({"path": mcp_output})));
    let reopened_id = reopened["session_id"]
        .as_str()
        .expect("reopen must return a session ID")
        .to_owned();
    let reopened_capabilities = successful_tool(mcp.call_tool(
        "workbook_scan_capabilities",
        json!({"session_id": reopened_id}),
    ));
    assert_eq!(reopened_capabilities["supported_count"], 1);
    successful_tool(mcp.call_tool(
        "workbook_recalculate",
        json!({"session_id": reopened_id, "mode": "auto"}),
    ));
    let reopened_range = successful_tool(mcp.call_tool(
        "workbook_read_range",
        json!({
            "session_id": reopened_id,
            "sheet": "Sheet1",
            "start": "A2",
            "end": "A2"
        }),
    ));
    assert_eq!(
        reopened_range["cells"][0]["calculated"]["value"]["value"],
        json!(6.0)
    );

    let (status, stderr, response_lines) = mcp.finish();
    assert!(status.success());
    assert!(stderr.contains("CellRune MCP stdio server starting"));
    assert!(!response_lines.is_empty());
    for line in response_lines {
        serde_json::from_str::<Value>(&line)
            .expect("logging must never contaminate stdout protocol lines");
    }
}

#[test]
fn table_authoring_v2_is_available_as_a_separate_stdio_contract() {
    let root = TestDirectory::new("table-authoring-v2");
    let contract_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../binding-contract/table-authoring-v2.json");
    let contract: Value = serde_json::from_slice(
        &fs::read(&contract_path).expect("shared table contract must be readable"),
    )
    .expect("shared table contract must be valid JSON");
    let fixture_name = contract["fixture"]
        .as_str()
        .expect("table contract fixture name");
    let input = root.path.join(fixture_name);
    fs::copy(
        contract_path
            .parent()
            .expect("contract directory")
            .join(fixture_name),
        &input,
    )
    .expect("shared table fixture must be copied beneath the approved root");

    let mut mcp = McpProcess::start(&root.path, &[]);
    mcp.initialize();
    let opened = successful_tool(mcp.call_tool("workbook_open", json!({"path": input})));
    let session_id = opened["session_id"]
        .as_str()
        .expect("open must return a session ID")
        .to_owned();
    let revision = opened["summary"]["semantic_revision"]
        .as_u64()
        .expect("summary revision");
    let unknown_column = failed_tool(mcp.call_tool(
        "workbook_apply_changes_v2",
        json!({
            "session_id": session_id,
            "expected_revision": revision,
            "changes": [{
                "kind": "rename_table_column",
                "table_id": contract["table_id"],
                "column_id": 99,
                "new_name": "Missing"
            }]
        }),
    ));
    assert_eq!(unknown_column["code"], "validation.unknown_table_column_id");
    let unchanged =
        successful_tool(mcp.call_tool("workbook_summary", json!({"session_id": session_id})));
    assert_eq!(unchanged["summary"]["semantic_revision"], revision);

    let receipt = successful_tool(mcp.call_tool(
        "workbook_apply_changes_v2",
        json!({
            "session_id": session_id,
            "expected_revision": revision,
            "changes": [
                {
                    "kind": "rename_table",
                    "table_id": contract["table_id"],
                    "new_display_name": contract["new_display_name"]
                },
                {
                    "kind": "rename_table_column",
                    "table_id": contract["table_id"],
                    "column_id": contract["column_id"],
                    "new_name": contract["new_column_name"]
                },
                {
                    "kind": "resize_table_rows",
                    "table_id": contract["table_id"],
                    "first_data_row": contract["first_data_row"],
                    "last_data_row": contract["last_data_row"]
                }
            ]
        }),
    ));
    assert_eq!(receipt["schema_version"], contract["schema_version"]);
    assert_eq!(
        receipt["changed_table_ids"],
        json!([contract["table_id"].clone()])
    );
    let summary =
        successful_tool(mcp.call_tool("workbook_summary", json!({"session_id": session_id})));
    let tables = summary["summary"]["sheets"][0]["tables"]
        .as_array()
        .expect("tables");
    let table = tables
        .iter()
        .find(|table| table["id"] == contract["table_id"])
        .expect("edited table");
    assert_eq!(table["display_name"], contract["new_display_name"]);
    assert_eq!(table["range"], contract["expected_range"]);
    assert_eq!(table["columns"][1]["name"], contract["new_column_name"]);
    let formula = successful_tool(mcp.call_tool(
        "workbook_read_range",
        json!({
            "session_id": session_id,
            "sheet": "Data",
            "start": "E1",
            "end": "E1"
        }),
    ));
    assert_eq!(formula["cells"][0]["formula"], "=SUM(Orders[Gross Amount])");
    let empty = tables
        .iter()
        .find(|table| table["id"] == contract["empty_table_id"])
        .expect("empty table");
    assert_eq!(empty["display_name"], contract["empty_table_name"]);
    assert_eq!(empty["range"], contract["empty_table_range"]);

    successful_tool(mcp.call_tool(
        "workbook_recalculate",
        json!({"session_id": session_id, "mode": "full"}),
    ));
    let output = root.path.join("table-authoring-v2-output.xlsx");
    successful_tool(mcp.call_tool(
        "workbook_save_as",
        json!({
            "session_id": session_id,
            "path": output,
            "invalidate_unavailable": true,
            "replace_existing": false
        }),
    ));
    let reopened = successful_tool(mcp.call_tool("workbook_open", json!({"path": output})));
    let reopened_tables = reopened["summary"]["sheets"][0]["tables"]
        .as_array()
        .expect("reopened tables");
    let reopened_table = reopened_tables
        .iter()
        .find(|table| table["id"] == contract["table_id"])
        .expect("reopened edited table");
    assert_eq!(reopened_table["display_name"], contract["new_display_name"]);
    assert_eq!(reopened_table["range"], contract["expected_range"]);

    let (status, _, _) = mcp.finish();
    assert!(status.success());
}

#[test]
fn calculation_compatibility_modes_are_stdio_visible() {
    let root = TestDirectory::new("calculation-modes");
    let mut mcp = McpProcess::start(&root.path, &[]);
    mcp.initialize();
    let session_id = successful_tool(mcp.call_tool("workbook_create", json!({})))["session_id"]
        .as_str()
        .expect("create must return a session ID")
        .to_owned();
    successful_tool(mcp.call_tool(
        "workbook_apply_changes",
        json!({
            "session_id": session_id,
            "expected_revision": 0,
            "changes": [
                {
                    "kind": "set_formula",
                    "sheet": "Sheet1",
                    "address": "A1",
                    "formula": "=0.1+0.2-0.3",
                    "dynamic_range": null
                },
                {
                    "kind": "set_formula",
                    "sheet": "Sheet1",
                    "address": "A2",
                    "formula": "=IRR({-1,100000})",
                    "dynamic_range": null
                }
            ]
        }),
    ));

    successful_tool(mcp.call_tool(
        "workbook_recalculate",
        json!({"session_id": session_id, "mode": "full"}),
    ));
    let defaults = successful_tool(mcp.call_tool(
        "workbook_read_range",
        json!({
            "session_id": session_id,
            "sheet": "Sheet1",
            "start": "A1",
            "end": "A2"
        }),
    ));
    assert_eq!(
        defaults["cells"][0]["calculated"]["value"]["value"],
        json!(0.0)
    );
    assert_eq!(
        defaults["cells"][1]["calculated"]["value"]["value"],
        json!("#NUM!")
    );

    successful_tool(mcp.call_tool(
        "workbook_recalculate",
        json!({
            "session_id": session_id,
            "mode": "full",
            "arithmetic_semantics": "ieee_754",
            "financial_solver_semantics": "extended_search"
        }),
    ));
    let legacy = successful_tool(mcp.call_tool(
        "workbook_read_range",
        json!({
            "session_id": session_id,
            "sheet": "Sheet1",
            "start": "A1",
            "end": "A2"
        }),
    ));
    assert_ne!(
        legacy["cells"][0]["calculated"]["value"]["value"],
        json!(0.0)
    );
    let rate = legacy["cells"][1]["calculated"]["value"]["value"]
        .as_f64()
        .expect("extended search produces a number");
    assert!((rate - 99_999.0).abs() < 1e-5);

    let (status, _, _) = mcp.finish();
    assert!(status.success());
}

#[test]
fn response_limit_failures_do_not_commit_session_edit_calculation_or_file_state() {
    let root = TestDirectory::new("response-limit");
    let mut mcp = McpProcess::start(&root.path, &["--max-response-bytes", "1024"]);
    mcp.initialize();
    let created = successful_tool(mcp.call_tool("workbook_create", json!({})));
    let session_id = created["session_id"]
        .as_str()
        .expect("create must return a session ID")
        .to_owned();
    let changes = (0..40)
        .map(|index| {
            json!({
                "kind": "set_value",
                "sheet": "Sheet1",
                "address": format!("A{}", index + 1),
                "value": {
                    "kind": "number",
                    "value": index
                }
            })
        })
        .collect::<Vec<_>>();
    let edit_result = mcp.call_tool(
        "workbook_apply_changes",
        json!({
            "session_id": session_id,
            "expected_revision": 0,
            "changes": changes
        }),
    );
    assert_eq!(
        failed_tool(edit_result)["code"],
        "mcp.response.byte_limit_exceeded"
    );
    let summary =
        successful_tool(mcp.call_tool("workbook_summary", json!({"session_id": session_id})));
    assert_eq!(summary["summary"]["semantic_revision"], 0);
    assert_eq!(
        summary["summary"]["sheets"].as_array().map(Vec::len),
        Some(1)
    );

    successful_tool(mcp.call_tool(
        "workbook_apply_changes",
        json!({
            "session_id": session_id,
            "expected_revision": 0,
            "changes": [{
                "kind": "set_formula",
                "sheet": "Sheet1",
                "address": "A1",
                "formula": format!("=\"{}\"", "x".repeat(2_000))
            }]
        }),
    ));
    let calculation_error = failed_tool(mcp.call_tool(
        "workbook_recalculate",
        json!({"session_id": session_id, "mode": "full"}),
    ));
    assert_eq!(
        calculation_error["code"],
        "mcp.response.byte_limit_exceeded"
    );
    let history = successful_tool(mcp.call_tool(
        "workbook_changes_since",
        json!({"session_id": session_id, "cursor": 0, "limit": 100}),
    ));
    assert_eq!(history["deltas"].as_array().map(Vec::len), Some(0));

    let save_session = successful_tool(mcp.call_tool("workbook_create", json!({})))["session_id"]
        .as_str()
        .expect("create must return a session ID")
        .to_owned();
    for index in 0..40 {
        successful_tool(mcp.call_tool(
            "workbook_apply_changes",
            json!({
                "session_id": save_session,
                "expected_revision": index,
                "changes": [{
                    "kind": "add_sheet",
                    "name": format!("Sheet{}", index + 2)
                }]
            }),
        ));
    }
    successful_tool(mcp.call_tool(
        "workbook_recalculate",
        json!({"session_id": save_session, "mode": "full"}),
    ));
    let output = root.path.join("must-not-exist.xlsx");
    let save_error = failed_tool(mcp.call_tool(
        "workbook_save_as",
        json!({
            "session_id": save_session,
            "path": output,
            "replace_existing": false
        }),
    ));
    assert_eq!(save_error["code"], "mcp.response.byte_limit_exceeded");
    assert!(
        !output.exists(),
        "a rejected response must not create a file"
    );

    let input = root.path.join("large-summary.xlsx");
    let mut direct = WorkbookSession::create();
    for index in 0..40 {
        direct
            .add_sheet(&format!("DirectSheet{}", index + 2))
            .expect("direct sheet add must succeed");
    }
    direct
        .calculate(CalculationOptionsDto::default())
        .expect("direct calculation must succeed");
    direct
        .save_path(&input, WriteOptionsDto::default())
        .expect("direct fixture save must succeed");
    let resources_before = mcp.request("resources/list", json!({}))["result"]["resources"]
        .as_array()
        .map(Vec::len)
        .expect("resources must be an array");
    let open_error = failed_tool(mcp.call_tool("workbook_open", json!({"path": input})));
    assert_eq!(open_error["code"], "mcp.response.byte_limit_exceeded");
    let resources_after = mcp.request("resources/list", json!({}))["result"]["resources"]
        .as_array()
        .map(Vec::len)
        .expect("resources must be an array");
    assert_eq!(
        resources_after, resources_before,
        "a rejected open must not retain a workbook session"
    );

    let (status, _, _) = mcp.finish();
    assert!(status.success());
}

#[test]
fn resource_protocol_errors_and_byte_bounded_pagination_are_stdio_visible() {
    let root = TestDirectory::new("resource-pagination");
    let mut mcp = McpProcess::start(
        &root.path,
        &["--max-sessions", "32", "--max-response-bytes", "1024"],
    );
    mcp.initialize();
    let mut session_ids = Vec::new();
    for _ in 0..20 {
        let created = successful_tool(mcp.call_tool("workbook_create", json!({})));
        session_ids.push(
            created["session_id"]
                .as_str()
                .expect("create must return a session ID")
                .to_owned(),
        );
    }

    let mut cursor = None;
    let mut resource_uris = std::collections::BTreeSet::new();
    let mut page_count = 0;
    loop {
        let params = cursor
            .as_ref()
            .map_or_else(|| json!({}), |cursor| json!({"cursor": cursor}));
        let response = mcp.request("resources/list", params);
        let result = &response["result"];
        assert!(
            serde_json::to_vec(result)
                .expect("resource result must serialize")
                .len()
                <= 1_024,
            "each result must honor the configured serialized-byte limit"
        );
        let resources = result["resources"]
            .as_array()
            .expect("resources must be an array");
        assert!(!resources.is_empty(), "issued pages must make progress");
        for resource in resources {
            let uri = resource["uri"]
                .as_str()
                .expect("resource URI must be a string")
                .to_owned();
            assert!(
                resource_uris.insert(uri),
                "a resource must not repeat across cursor pages"
            );
        }
        page_count += 1;
        cursor = result["nextCursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }

    assert!(page_count > 1);
    assert_eq!(resource_uris.len(), session_ids.len() + 1);
    assert!(resource_uris.contains("cellrune://support/functions"));
    for session_id in &session_ids {
        assert!(resource_uris.contains(&format!("cellrune://sessions/{session_id}/summary")));
    }

    let malformed = mcp.request("resources/list", json!({"cursor": "not-a-cellrune-cursor"}));
    assert_eq!(malformed["error"]["code"], -32_602);
    assert_eq!(
        malformed["error"]["data"]["code"],
        "mcp.resource.cursor_invalid"
    );

    let missing = mcp.request(
        "resources/read",
        json!({"uri": "cellrune://sessions/workbook-ffffffffffffffff/summary"}),
    );
    assert_eq!(missing["error"]["code"], -32_002);
    assert_eq!(missing["error"]["data"]["code"], "mcp.session.not_found");

    let (status, _, _) = mcp.finish();
    assert!(status.success());
}

#[test]
fn retained_preview_tools_are_thin_stdio_lifecycle_adapters() {
    let root = TestDirectory::new("preview-workflow-root");
    let mut mcp = McpProcess::start(&root.path, &[]);
    mcp.initialize();

    let created = successful_tool(mcp.call_tool("workbook_create", json!({})));
    let session_id = created["session_id"]
        .as_str()
        .expect("create must return a session ID")
        .to_owned();
    successful_tool(mcp.call_tool(
        "workbook_apply_changes_v2",
        json!({
            "session_id": session_id,
            "expected_revision": 0,
            "changes": [
                {"kind": "set_value", "sheet": "Sheet1", "address": "A1", "value": {"kind": "number", "value": 1.0}},
                {"kind": "set_formula", "sheet": "Sheet1", "address": "A2", "formula": "=A1+1"}
            ]
        }),
    ));
    successful_tool(mcp.call_tool(
        "workbook_recalculate",
        json!({"session_id": session_id, "mode": "auto"}),
    ));

    let preview = successful_tool(mcp.call_tool(
        "workbook_preview_changes",
        json!({
            "session_id": session_id,
            "expected_revision": 1,
            "mode": "auto",
            "changes": [{"kind": "set_value", "sheet": "Sheet1", "address": "A1", "value": {"kind": "number", "value": 4.0}}]
        }),
    ));
    let preview_id = preview["preview_id"]
        .as_u64()
        .expect("preview must expose its opaque numeric ID");
    assert_eq!(preview["report"]["base_revision"], 1);
    assert_eq!(preview["report"]["result_revision"], 2);
    assert!(preview["report"]["calculation_options"]["limits"].is_object());

    let no_op = successful_tool(mcp.call_tool(
        "workbook_apply_changes_v2",
        json!({
            "session_id": session_id,
            "expected_revision": 1,
            "changes": [
                {"kind": "set_value", "sheet": "Sheet1", "address": "A1", "value": {"kind": "number", "value": 1.0}}
            ]
        }),
    ));
    assert_eq!(no_op["base_revision"], 1);
    assert_eq!(no_op["result_revision"], 1);

    successful_tool(mcp.call_tool("workbook_summary", json!({"session_id": session_id})));
    successful_tool(mcp.call_tool(
        "workbook_read_range",
        json!({
            "session_id": session_id,
            "sheet": "Sheet1",
            "start": "A1",
            "end": "A2",
            "offset": 0,
            "limit": 10
        }),
    ));
    successful_tool(mcp.call_tool("workbook_function_usage", json!({"session_id": session_id})));
    successful_tool(mcp.call_tool(
        "workbook_changes_since",
        json!({"session_id": session_id, "cursor": 0, "limit": 10}),
    ));
    let read_only_output = root.path.join("preview-read-only-save.xlsx");
    successful_tool(mcp.call_tool(
        "workbook_save_as",
        json!({
            "session_id": session_id,
            "path": read_only_output,
            "invalidate_unavailable": false,
            "replace_existing": false
        }),
    ));

    let first_page = successful_tool(mcp.call_tool(
        "workbook_preview_changes_page",
        json!({
            "session_id": session_id,
            "preview_id": preview_id,
            "section": "preview_results",
            "limit": 1
        }),
    ));
    assert_eq!(first_page["preview_id"], preview_id);
    assert_eq!(first_page["section"], "preview_results");
    assert!(
        first_page["items"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
    if let Some(cursor) = first_page["next_cursor"].as_object() {
        let wrong_section = failed_tool(mcp.call_tool(
            "workbook_preview_changes_page",
            json!({
                "session_id": session_id,
                "preview_id": preview_id,
                "section": "affected",
                "cursor": cursor,
                "limit": 1
            }),
        ));
        assert_eq!(wrong_section["code"], "session.transaction_cursor_invalid");

        let second_page = successful_tool(mcp.call_tool(
            "workbook_preview_changes_page",
            json!({
                "session_id": session_id,
                "preview_id": preview_id,
                "section": "preview_results",
                "cursor": cursor,
                "limit": 1
            }),
        ));
        assert_ne!(second_page["items"], first_page["items"]);
    }

    let receipt = successful_tool(mcp.call_tool(
        "workbook_commit_preview",
        json!({"session_id": session_id, "preview_id": preview_id}),
    ));
    assert_eq!(receipt["edit"]["result_revision"], 2);
    let consumed = failed_tool(mcp.call_tool(
        "workbook_commit_preview",
        json!({"session_id": session_id, "preview_id": preview_id}),
    ));
    assert_eq!(consumed["code"], "interop.preview.not_found");

    let disposable = successful_tool(mcp.call_tool(
        "workbook_preview_changes",
        json!({
            "session_id": session_id,
            "expected_revision": 2,
            "changes": [{"kind": "set_value", "sheet": "Sheet1", "address": "A1", "value": {"kind": "number", "value": 5.0}}]
        }),
    ));
    let disposable_id = disposable["preview_id"]
        .as_u64()
        .expect("replacement preview must expose an ID");
    let discarded = successful_tool(mcp.call_tool(
        "workbook_discard_preview",
        json!({"session_id": session_id, "preview_id": disposable_id}),
    ));
    assert_eq!(discarded["discarded"], true);
    let absent = failed_tool(mcp.call_tool(
        "workbook_preview_changes_page",
        json!({
            "session_id": session_id,
            "preview_id": disposable_id,
            "section": "affected"
        }),
    ));
    assert_eq!(absent["code"], "interop.preview.not_found");

    let (status, _, _) = mcp.finish();
    assert!(status.success());
}

fn successful_tool(result: Value) -> Value {
    assert_eq!(
        result["isError"], false,
        "tool unexpectedly failed: {result}"
    );
    result["structuredContent"].clone()
}

fn failed_tool(result: Value) -> Value {
    assert_eq!(
        result["isError"], true,
        "tool unexpectedly succeeded: {result}"
    );
    result["structuredContent"].clone()
}
