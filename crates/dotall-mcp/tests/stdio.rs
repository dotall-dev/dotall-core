use std::io::Write;
use std::process::{Command, Stdio};

use dotall_mcp::params::{
    ApplyParams, EditParams, FileParams, HistoryParams, OperationParam, ReadParams, RevertParams,
};
use dotall_mcp::response::ToolResponse;
use dotall_mcp::server::{DotallServer, FlushOnClose};
use rmcp::handler::server::wrapper::Parameters;
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;

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
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Revenue").expect("sheet name");
    worksheet.write_number(1, 0, 100.0).expect("value");
    worksheet
        .write_formula(1, 1, "=A2*2")
        .expect("formula fixture");
    workbook.save(&file).expect("workbook fixture");

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
