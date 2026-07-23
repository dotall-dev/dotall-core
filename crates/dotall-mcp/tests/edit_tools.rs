use dotall_mcp::params::{
    ApplyParams, DiffParams, EditParams, FileParams, HistoryParams, OperationParam, ReadParams,
    RevertParams, StagedParams,
};
use dotall_mcp::response::ToolResponse;
use dotall_mcp::server::{DotallServer, FlushOnClose};
use rmcp::handler::server::wrapper::Parameters;
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;

fn workbook_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Revenue").expect("sheet name");
    worksheet.write_string(0, 0, "Month").expect("header");
    worksheet.write_number(1, 0, 100.0).expect("value");
    workbook.save(path).expect("workbook fixture");
}

async fn inspect_hash(server: &DotallServer, file: &std::path::Path) -> String {
    let response = server
        .dotall_inspect(Parameters(FileParams {
            file: file.display().to_string(),
        }))
        .await
        .0;
    let ToolResponse::Success { result, .. } = response else {
        panic!("inspect should succeed");
    };
    result.data["source_hash"]
        .as_str()
        .expect("source hash")
        .to_owned()
}

#[tokio::test]
async fn edit_stages_then_apply_commits_a_version() {
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("financials.xlsx");
    workbook_fixture(&file);
    let server =
        DotallServer::open_or_init(workspace.path(), FlushOnClose::default()).expect("server");
    let source_hash = inspect_hash(&server, &file).await;

    let edit = server
        .dotall_edit(Parameters(EditParams {
            file: file.display().to_string(),
            expected_source_hash: source_hash,
            transaction_id: None,
            actor_id: "agent-1".into(),
            operations: vec![OperationParam {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Revenue",
                    "address": "A2",
                    "value": 125,
                }),
            }],
        }))
        .await
        .0;
    let ToolResponse::Success { result: edit, .. } = edit else {
        panic!("edit should stage");
    };
    let transaction_id = edit.data["tx_id"]
        .as_str()
        .expect("transaction ID")
        .to_owned();

    let staged = server
        .dotall_staged(Parameters(StagedParams {
            file: file.display().to_string(),
        }))
        .await
        .0;
    let ToolResponse::Success { result: staged, .. } = staged else {
        panic!("staged should succeed");
    };
    assert_eq!(staged.data["edits"].as_array().expect("edits").len(), 1);

    let applied = server
        .dotall_apply(Parameters(ApplyParams {
            file: file.display().to_string(),
            transaction_id: Some(transaction_id),
            all: false,
        }))
        .await
        .0;
    let ToolResponse::Success {
        result: applied, ..
    } = applied
    else {
        panic!("apply should succeed");
    };
    assert_eq!(applied.data["version"], 1);

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
    assert_eq!(history.data["entries"][0]["version"], 1);

    let diff = server
        .dotall_diff(Parameters(DiffParams {
            file: file.display().to_string(),
            version: 1,
        }))
        .await
        .0;
    let ToolResponse::Success { result: diff, .. } = diff else {
        panic!("diff should succeed");
    };
    assert_eq!(diff.data["version"], 1);
}

#[tokio::test]
async fn apply_all_rebases_same_hash_cell_edits() {
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("financials.xlsx");
    workbook_fixture(&file);
    let server =
        DotallServer::open_or_init(workspace.path(), FlushOnClose::default()).expect("server");
    let source_hash = inspect_hash(&server, &file).await;

    for (address, value) in [("A2", 125), ("B2", 200)] {
        let response = server
            .dotall_edit(Parameters(EditParams {
                file: file.display().to_string(),
                expected_source_hash: source_hash.clone(),
                transaction_id: None,
                actor_id: "agent-1".into(),
                operations: vec![OperationParam {
                    kind: "set_cell_value".into(),
                    payload: serde_json::json!({
                        "sheet": "Revenue",
                        "address": address,
                        "value": value,
                    }),
                }],
            }))
            .await
            .0;
        assert!(matches!(response, ToolResponse::Success { .. }));
    }

    let applied = server
        .dotall_apply(Parameters(ApplyParams {
            file: file.display().to_string(),
            transaction_id: None,
            all: true,
        }))
        .await
        .0;
    let ToolResponse::Success {
        result: applied, ..
    } = applied
    else {
        panic!("apply-all should succeed: {applied:?}");
    };
    assert_eq!(
        applied.data["applied"].as_array().expect("applied").len(),
        2
    );

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
        2
    );

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
    let content = read.data["response"]["content"].as_str().expect("content");
    assert!(content.contains("125"));
    assert!(content.contains("200"));
}

#[tokio::test]
async fn revert_stages_restore_until_apply() {
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("financials.xlsx");
    workbook_fixture(&file);
    let server =
        DotallServer::open_or_init(workspace.path(), FlushOnClose::default()).expect("server");
    let source_hash = inspect_hash(&server, &file).await;

    let edit = server
        .dotall_edit(Parameters(EditParams {
            file: file.display().to_string(),
            expected_source_hash: source_hash,
            transaction_id: None,
            actor_id: "agent-1".into(),
            operations: vec![OperationParam {
                kind: "set_cell_value".into(),
                payload: serde_json::json!({
                    "sheet": "Revenue",
                    "address": "A2",
                    "value": 125,
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

    let revert = server
        .dotall_revert(Parameters(RevertParams {
            file: file.display().to_string(),
            version: 1,
            expected_source_hash: inspect_hash(&server, &file).await,
            transaction_id: None,
            actor_id: None,
        }))
        .await
        .0;
    let ToolResponse::Success { result: revert, .. } = revert else {
        panic!("revert should stage");
    };
    assert_eq!(
        revert.data["preview"]["operations"][0]["kind"],
        "restore_snapshot"
    );

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
}

#[tokio::test]
async fn apply_requires_exactly_one_target() {
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("financials.xlsx");
    workbook_fixture(&file);
    let server =
        DotallServer::open_or_init(workspace.path(), FlushOnClose::default()).expect("server");

    for params in [
        ApplyParams {
            file: file.display().to_string(),
            transaction_id: None,
            all: false,
        },
        ApplyParams {
            file: file.display().to_string(),
            transaction_id: Some("00000000-0000-0000-0000-000000000000".into()),
            all: true,
        },
    ] {
        let response = server.dotall_apply(Parameters(params)).await.0;
        let ToolResponse::Error { code, .. } = response else {
            panic!("invalid apply request should fail");
        };
        assert_eq!(code, "invalid_source_path");
    }
}
