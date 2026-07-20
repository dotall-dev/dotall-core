use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use dotall_mcp::params::{
    ApplyParams, EditParams, FileParams, HistoryParams, OperationParam, ReadParams, RevertParams,
};
use dotall_mcp::response::ToolResponse;
use dotall_mcp::server::{DotallServer, FlushOnClose};
use rmcp::handler::server::wrapper::Parameters;
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;

struct StdioClient {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    stdout: BufReader<std::process::ChildStdout>,
}

impl StdioClient {
    fn spawn(workspace: &std::path::Path, arguments: &[&str]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dotall-mcp"));
        command
            .args(arguments)
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("spawn dotall-mcp");
        let stdin = child.stdin.take().expect("child stdin");
        let stdout = BufReader::new(child.stdout.take().expect("child stdout"));
        let mut client = Self {
            child,
            stdin: Some(stdin),
            stdout,
        };
        client.initialize();
        client
    }

    fn initialize(&mut self) {
        self.send(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "test-client", "version": "0.1.0" },
            },
        }));
        let initialized = self.response_for(1);
        assert!(
            initialized["result"].is_object(),
            "initialize result: {initialized}"
        );
        self.send(serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        }));
    }

    fn call(&mut self, id: u64, name: &str, arguments: serde_json::Value) -> serde_json::Value {
        self.send(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        }));
        self.response_for(id)
    }

    fn tool_data(
        &mut self,
        id: u64,
        name: &str,
        arguments: serde_json::Value,
    ) -> serde_json::Value {
        let response = self.call(id, name, arguments);
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{name} response has no text content: {response}"));
        let tool_response: serde_json::Value =
            serde_json::from_str(text).unwrap_or_else(|_| panic!("{name} response text: {text}"));
        assert_eq!(
            tool_response["status"], "success",
            "{name} tool failed: {tool_response}"
        );
        tool_response["result"]["data"].clone()
    }

    fn send(&mut self, request: serde_json::Value) {
        let stdin = self.stdin.as_mut().expect("stdin is open");
        writeln!(stdin, "{request}").expect("send MCP request");
        stdin.flush().expect("flush MCP request");
    }

    fn response_for(&mut self, expected_id: u64) -> serde_json::Value {
        let mut line = String::new();
        loop {
            line.clear();
            let read = self.stdout.read_line(&mut line).expect("read MCP response");
            assert_ne!(
                read, 0,
                "server closed stdout before response {expected_id}"
            );
            let response: serde_json::Value =
                serde_json::from_str(line.trim_end()).expect("protocol stdout is JSON");
            if response["id"] == expected_id {
                return response;
            }
        }
    }

    fn close(mut self) {
        self.stdin.take();
        let status = self.child.wait().expect("wait for dotall-mcp");
        let mut remaining = String::new();
        self.stdout
            .read_to_string(&mut remaining)
            .expect("read remaining protocol stdout");
        for line in remaining.lines() {
            serde_json::from_str::<serde_json::Value>(line)
                .unwrap_or_else(|_| panic!("stdout must only contain protocol JSON: {line}"));
        }
        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .expect("child stderr")
            .read_to_string(&mut stderr)
            .expect("read stderr");
        assert!(status.success(), "dotall-mcp failed: {stderr}");
    }
}

fn workbook_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Revenue").expect("sheet name");
    worksheet.write_number(1, 0, 100.0).expect("value");
    worksheet
        .write_formula(1, 1, "=A2*2")
        .expect("formula fixture");
    workbook.save(path).expect("workbook fixture");
}

#[test]
fn stdio_server_initializes_and_lists_agent_tools() {
    let workspace = tempdir().expect("workspace");
    let mut child = Command::new(env!("CARGO_BIN_EXE_dotall-mcp"))
        .current_dir(workspace.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dotall-mcp");

    let request = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test-client","version":"0.1.0"}}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        "\n"
    );
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(request.as_bytes())
        .expect("send MCP requests");

    let output = child.wait_with_output().expect("wait for dotall-mcp");
    assert!(
        output.status.success(),
        "dotall-mcp failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let responses = String::from_utf8(output.stdout)
        .expect("protocol stdout is UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("protocol JSON"))
        .collect::<Vec<_>>();
    let tools = responses
        .iter()
        .find(|response| response["id"] == 2)
        .and_then(|response| response["result"]["tools"].as_array())
        .expect("tools/list result");

    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "dotall_apply",
            "dotall_capabilities",
            "dotall_deps",
            "dotall_diff",
            "dotall_discard",
            "dotall_edit",
            "dotall_history",
            "dotall_init",
            "dotall_inspect",
            "dotall_read",
            "dotall_revert",
            "dotall_staged",
            "dotall_status",
        ]
    );
    for tool in tools {
        assert!(
            tool["description"]
                .as_str()
                .is_some_and(|description| !description.is_empty())
        );
        assert_eq!(tool["inputSchema"]["type"], "object");
    }
}

#[tokio::test]
async fn agent_session_inspects_reads_edits_applies_histories_and_reverts() {
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("financials.xlsx");
    workbook_fixture(&file);

    let server =
        DotallServer::open_or_init(workspace.path(), FlushOnClose::default()).expect("server");
    let inspect = server
        .dotall_inspect(Parameters(FileParams {
            file: file.display().to_string(),
        }))
        .await
        .0;
    let ToolResponse::Success {
        result: inspect, ..
    } = inspect
    else {
        panic!("inspect should succeed");
    };
    let source_hash = inspect.data["source_hash"]
        .as_str()
        .expect("source hash")
        .to_owned();

    let read = server
        .dotall_read(Parameters(ReadParams {
            file: file.display().to_string(),
            selector_kind: Some("range".into()),
            selector: Some("Revenue!A2:B2".into()),
            max_tokens: Some(500),
            continuation: None,
        }))
        .await
        .0;
    let ToolResponse::Success { result: read, .. } = read else {
        panic!("read should succeed");
    };
    assert!(
        read.data["response"]["content"]
            .as_str()
            .expect("content")
            .contains("=A2*2")
    );

    let edit = server
        .dotall_edit(Parameters(EditParams {
            file: file.display().to_string(),
            expected_source_hash: source_hash,
            transaction_id: None,
            actor_id: "agent-1".into(),
            operations: vec![OperationParam {
                kind: "set_cell_formula".into(),
                payload: serde_json::json!({
                    "sheet": "Revenue",
                    "address": "B2",
                    "formula": "=A2*3",
                }),
            }],
        }))
        .await
        .0;
    let ToolResponse::Success { result: edit, .. } = edit else {
        panic!("edit should stage");
    };

    let applied = server
        .dotall_apply(Parameters(ApplyParams {
            file: file.display().to_string(),
            transaction_id: edit.data["tx_id"].as_str().map(str::to_owned),
            all: false,
        }))
        .await
        .0;
    assert!(matches!(applied, ToolResponse::Success { .. }));

    let history = server
        .dotall_history(Parameters(HistoryParams {
            file: file.display().to_string(),
        }))
        .await
        .0;
    let ToolResponse::Success {
        result: history, ..
    } = history
    else {
        panic!("history should succeed");
    };
    assert_eq!(
        history.data["entries"].as_array().expect("entries").len(),
        1
    );

    let inspect_after_apply = server
        .dotall_inspect(Parameters(FileParams {
            file: file.display().to_string(),
        }))
        .await
        .0;
    let ToolResponse::Success {
        result: inspect_after_apply,
        ..
    } = inspect_after_apply
    else {
        panic!("inspect after apply should succeed");
    };
    let revert = server
        .dotall_revert(Parameters(RevertParams {
            file: file.display().to_string(),
            version: 1,
            expected_source_hash: inspect_after_apply.data["source_hash"]
                .as_str()
                .expect("source hash")
                .to_owned(),
            transaction_id: None,
            actor_id: "agent-1".into(),
        }))
        .await
        .0;
    let ToolResponse::Success { result: revert, .. } = revert else {
        panic!("revert should stage");
    };

    let reverted = server
        .dotall_apply(Parameters(ApplyParams {
            file: file.display().to_string(),
            transaction_id: revert.data["tx_id"].as_str().map(str::to_owned),
            all: false,
        }))
        .await
        .0;
    assert!(matches!(reverted, ToolResponse::Success { .. }));

    let final_read = server
        .dotall_read(Parameters(ReadParams {
            file: file.display().to_string(),
            selector_kind: Some("range".into()),
            selector: Some("Revenue!A2:B2".into()),
            max_tokens: Some(500),
            continuation: None,
        }))
        .await
        .0;
    let ToolResponse::Success {
        result: final_read, ..
    } = final_read
    else {
        panic!("read after revert should succeed");
    };
    assert!(
        final_read.data["response"]["content"]
            .as_str()
            .expect("content")
            .contains("=A2*2")
    );
}

#[test]
fn stdio_agent_workflow_edits_applies_histories_and_reverts() {
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("financials.xlsx");
    workbook_fixture(&file);
    let file = file.display().to_string();
    let mut client = StdioClient::spawn(workspace.path(), &[]);

    let inspect = client.tool_data(2, "dotall_inspect", serde_json::json!({ "file": file }));
    let source_hash = inspect["source_hash"]
        .as_str()
        .expect("source hash")
        .to_owned();
    let read = client.tool_data(
        3,
        "dotall_read",
        serde_json::json!({
            "file": file,
            "selector_kind": "range",
            "selector": "Revenue!A2:B2",
            "max_tokens": 500,
            "continuation": null,
        }),
    );
    assert!(
        read["response"]["content"]
            .as_str()
            .expect("read content")
            .contains("=A2*2")
    );

    let edit = client.tool_data(
        4,
        "dotall_edit",
        serde_json::json!({
            "file": file,
            "expected_source_hash": source_hash,
            "transaction_id": null,
            "actor_id": "stdio-agent",
            "operations": [{
                "kind": "set_cell_formula",
                "payload": {
                    "sheet": "Revenue",
                    "address": "B2",
                    "formula": "=A2*3",
                },
            }],
        }),
    );
    let transaction_id = edit["tx_id"].as_str().expect("transaction ID").to_owned();
    let applied = client.tool_data(
        5,
        "dotall_apply",
        serde_json::json!({
            "file": file,
            "transaction_id": transaction_id,
            "all": false,
        }),
    );
    assert_eq!(applied["version"], 1);
    let history = client.tool_data(6, "dotall_history", serde_json::json!({ "file": file }));
    assert_eq!(
        history["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        1
    );

    let inspect_after_apply =
        client.tool_data(7, "dotall_inspect", serde_json::json!({ "file": file }));
    let revert = client.tool_data(
        8,
        "dotall_revert",
        serde_json::json!({
            "file": file,
            "version": 1,
            "expected_source_hash": inspect_after_apply["source_hash"],
            "transaction_id": null,
            "actor_id": "stdio-agent",
        }),
    );
    let reverted_transaction = revert["tx_id"].as_str().expect("revert transaction ID");
    client.tool_data(
        9,
        "dotall_apply",
        serde_json::json!({
            "file": file,
            "transaction_id": reverted_transaction,
            "all": false,
        }),
    );
    let final_read = client.tool_data(
        10,
        "dotall_read",
        serde_json::json!({
            "file": file,
            "selector_kind": "range",
            "selector": "Revenue!A2:B2",
            "max_tokens": 500,
            "continuation": null,
        }),
    );
    assert!(
        final_read["response"]["content"]
            .as_str()
            .expect("final read content")
            .contains("=A2*2")
    );
    client.close();
}

#[test]
fn stdio_flush_on_close_applies_or_preserves_staged_edits_by_policy() {
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("financials.xlsx");
    workbook_fixture(&file);
    let file = file.display().to_string();

    let mut flushing = StdioClient::spawn(workspace.path(), &[]);
    let inspect = flushing.tool_data(2, "dotall_inspect", serde_json::json!({ "file": file }));
    flushing.tool_data(
        3,
        "dotall_edit",
        serde_json::json!({
            "file": file,
            "expected_source_hash": inspect["source_hash"],
            "transaction_id": null,
            "actor_id": "stdio-agent",
            "operations": [{
                "kind": "set_cell_value",
                "payload": { "sheet": "Revenue", "address": "A2", "value": 125 },
            }],
        }),
    );
    flushing.close();

    let mut verify_flushed = StdioClient::spawn(workspace.path(), &["--no-flush-on-close"]);
    let history =
        verify_flushed.tool_data(2, "dotall_history", serde_json::json!({ "file": file }));
    assert_eq!(
        history["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        1
    );
    let applied_read = verify_flushed.tool_data(
        3,
        "dotall_read",
        serde_json::json!({
            "file": file,
            "selector_kind": "range",
            "selector": "Revenue!A2:B2",
            "max_tokens": 500,
            "continuation": null,
        }),
    );
    assert!(
        applied_read["response"]["content"]
            .as_str()
            .expect("applied read content")
            .contains("125")
    );
    verify_flushed.close();

    let disabled_workspace = tempdir().expect("disabled workspace");
    let disabled_file = disabled_workspace.path().join("financials.xlsx");
    workbook_fixture(&disabled_file);
    let disabled_file = disabled_file.display().to_string();
    let mut disabled = StdioClient::spawn(disabled_workspace.path(), &["--no-flush-on-close"]);
    let inspect = disabled.tool_data(
        2,
        "dotall_inspect",
        serde_json::json!({ "file": disabled_file }),
    );
    disabled.tool_data(
        3,
        "dotall_edit",
        serde_json::json!({
            "file": disabled_file,
            "expected_source_hash": inspect["source_hash"],
            "transaction_id": null,
            "actor_id": "stdio-agent",
            "operations": [{
                "kind": "set_cell_value",
                "payload": { "sheet": "Revenue", "address": "A2", "value": 125 },
            }],
        }),
    );
    disabled.close();

    let mut verify_pending =
        StdioClient::spawn(disabled_workspace.path(), &["--no-flush-on-close"]);
    let staged = verify_pending.tool_data(
        2,
        "dotall_staged",
        serde_json::json!({ "file": disabled_file }),
    );
    assert_eq!(staged["edits"].as_array().expect("staged edits").len(), 1);
    let unchanged_read = verify_pending.tool_data(
        3,
        "dotall_read",
        serde_json::json!({
            "file": disabled_file,
            "selector_kind": "range",
            "selector": "Revenue!A2:B2",
            "max_tokens": 500,
            "continuation": null,
        }),
    );
    assert!(
        unchanged_read["response"]["content"]
            .as_str()
            .expect("unchanged read content")
            .contains("100")
    );
    verify_pending.close();
}
