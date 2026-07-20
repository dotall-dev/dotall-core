use dotall_mcp::params::{EditParams, FileParams, OperationParam, StagedParams};
use dotall_mcp::response::ToolResponse;
use dotall_mcp::server::{DotallServer, FlushOnClose};
use rmcp::handler::server::wrapper::Parameters;
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;

#[test]
fn flush_on_close_defaults_to_enabled() {
    assert!(FlushOnClose::resolve(false, None).enabled());
}

#[test]
fn no_flush_flag_disables_flush_on_close() {
    assert!(!FlushOnClose::resolve(true, None).enabled());
}

#[test]
fn zero_value_in_environment_disables_flush_on_close() {
    assert!(!FlushOnClose::resolve(false, Some("0")).enabled());
}

#[test]
fn no_flush_flag_wins_over_environment_enable() {
    assert!(!FlushOnClose::resolve(true, Some("1")).enabled());
}

fn workbook_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Revenue").expect("sheet name");
    worksheet.write_number(1, 0, 100.0).expect("value");
    workbook.save(path).expect("workbook fixture");
}

async fn stage_value_edit(server: &DotallServer, file: &std::path::Path) {
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

    let staged = server
        .dotall_edit(Parameters(EditParams {
            file: file.display().to_string(),
            expected_source_hash: inspect.data["source_hash"]
                .as_str()
                .expect("source hash")
                .to_owned(),
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
    assert!(matches!(staged, ToolResponse::Success { .. }));
}

#[tokio::test]
async fn flush_on_close_applies_staged_edits_when_enabled() {
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("financials.xlsx");
    workbook_fixture(&file);
    let server =
        DotallServer::open_or_init(workspace.path(), FlushOnClose::default()).expect("server");
    stage_value_edit(&server, &file).await;

    server.flush_staged();

    let staged = server
        .dotall_staged(Parameters(StagedParams {
            file: file.display().to_string(),
        }))
        .await
        .0;
    let ToolResponse::Success { result, .. } = staged else {
        panic!("staged should succeed");
    };
    assert!(result.data["edits"].as_array().expect("edits").is_empty());
}

#[tokio::test]
async fn flush_on_close_leaves_staged_edits_when_disabled() {
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("financials.xlsx");
    workbook_fixture(&file);
    let server = DotallServer::open_or_init(workspace.path(), FlushOnClose::resolve(true, None))
        .expect("server");
    stage_value_edit(&server, &file).await;

    server.flush_staged();

    let staged = server
        .dotall_staged(Parameters(StagedParams {
            file: file.display().to_string(),
        }))
        .await
        .0;
    let ToolResponse::Success { result, .. } = staged else {
        panic!("staged should succeed");
    };
    assert_eq!(result.data["edits"].as_array().expect("edits").len(), 1);
}
