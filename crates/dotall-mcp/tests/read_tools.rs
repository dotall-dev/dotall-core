use dotall_core::DotallStore;
use dotall_mcp::params::{
    CapabilitiesParams, DepsParams, FileParams, InitParams, ReadParams, StatusParams,
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
    worksheet.write_formula(1, 1, "=A2*1.1").expect("formula");
    workbook.save(path).expect("workbook fixture");
}

#[tokio::test]
async fn capabilities_returns_xlsx_selectors_and_edit_examples() {
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("financials.xlsx");
    workbook_fixture(&file);
    let server =
        DotallServer::open_or_init(workspace.path(), FlushOnClose::default()).expect("server");

    let response = server
        .dotall_capabilities(Parameters(CapabilitiesParams {
            file: file.display().to_string(),
        }))
        .await
        .0;

    let ToolResponse::Success { result, .. } = response else {
        panic!("capabilities should succeed");
    };
    assert_eq!(result.data["format_id"], "xlsx");
    assert!(
        result.data["selectors"]
            .as_array()
            .expect("selectors")
            .iter()
            .any(|selector| selector == "range")
    );
    assert!(
        result.data["edit_capabilities"]
            .as_array()
            .expect("edit capabilities")
            .iter()
            .any(|capability| capability["operation"] == "set_cell_value")
    );
    assert!(
        result.data["source_hash"]
            .as_str()
            .is_some_and(|h| !h.is_empty()),
        "capabilities must return source_hash for edit guards"
    );
}

#[tokio::test]
async fn inspect_read_status_and_deps_return_engine_data() {
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("financials.xlsx");
    workbook_fixture(&file);
    let server =
        DotallServer::open_or_init(workspace.path(), FlushOnClose::default()).expect("server");
    let file_param = FileParams {
        file: file.display().to_string(),
    };

    let inspect = server
        .dotall_inspect(Parameters(file_param.clone()))
        .await
        .0;
    let ToolResponse::Success {
        result: inspect, ..
    } = inspect
    else {
        panic!("inspect should succeed");
    };
    assert_eq!(inspect.data["inspection"]["format_id"], "xlsx");
    assert!(
        inspect.data["inspection"]["capabilities"]
            .as_array()
            .expect("capabilities")
            .iter()
            .any(|capability| capability["read_selector"]["kind"] == "range")
    );

    let read = server
        .dotall_read(Parameters(ReadParams {
            file: file_param.file.clone(),
            selector_kind: None,
            selector: None,
            max_tokens: Some(8),
            continuation: None,
        }))
        .await
        .0;
    let ToolResponse::Success { result: read, .. } = read else {
        panic!("read should succeed");
    };
    assert_eq!(read.data["response"]["truncated"], true);
    assert!(read.data["response"]["continuation"].is_string());

    let deps = server
        .dotall_deps(Parameters(DepsParams {
            file: file_param.file.clone(),
            cell: "Revenue!B2".into(),
            dependents: false,
        }))
        .await
        .0;
    let ToolResponse::Success { result: deps, .. } = deps else {
        panic!("deps should succeed");
    };
    assert_eq!(deps.data["direction"], "forward");
    assert_eq!(deps.data["edges"].as_array().expect("edges").len(), 1);

    let status = server
        .dotall_status(Parameters(StatusParams {
            workspace: workspace.path().display().to_string(),
        }))
        .await
        .0;
    let ToolResponse::Success { result: status, .. } = status else {
        panic!("status should succeed");
    };
    assert_eq!(status.data["tracked_count"], 1);
    assert!(status.data["objects"][0]["source_hash"].is_string());
    assert_eq!(status.data["objects"][0]["version_count"], 0);
}

#[tokio::test]
async fn init_rebinds_the_session_for_subsequent_file_tools() {
    let workspace_a = tempdir().expect("workspace A");
    let workspace_b = tempdir().expect("workspace B");
    let file_b = workspace_b.path().join("financials.xlsx");
    workbook_fixture(&file_b);
    let server =
        DotallServer::open_or_init(workspace_a.path(), FlushOnClose::default()).expect("server");

    let init = server
        .dotall_init(Parameters(InitParams {
            workspace: workspace_b.path().display().to_string(),
        }))
        .await
        .0;
    assert!(matches!(init, ToolResponse::Success { .. }));

    let inspect = server
        .dotall_inspect(Parameters(FileParams {
            file: file_b.display().to_string(),
        }))
        .await
        .0;
    let ToolResponse::Success {
        result: inspect, ..
    } = inspect
    else {
        panic!("inspect should use workspace B after init");
    };
    assert_eq!(inspect.data["inspection"]["format_id"], "xlsx");
}

#[tokio::test]
async fn status_rebinds_the_session_for_subsequent_file_tools() {
    let workspace_a = tempdir().expect("workspace A");
    let workspace_b = tempdir().expect("workspace B");
    let file_b = workspace_b.path().join("financials.xlsx");
    workbook_fixture(&file_b);
    DotallStore::init(workspace_b.path()).expect("initialize workspace B");
    let server =
        DotallServer::open_or_init(workspace_a.path(), FlushOnClose::default()).expect("server");

    let status = server
        .dotall_status(Parameters(StatusParams {
            workspace: workspace_b.path().display().to_string(),
        }))
        .await
        .0;
    assert!(matches!(status, ToolResponse::Success { .. }));

    let inspect = server
        .dotall_inspect(Parameters(FileParams {
            file: file_b.display().to_string(),
        }))
        .await
        .0;
    let ToolResponse::Success {
        result: inspect, ..
    } = inspect
    else {
        panic!("inspect should use workspace B after status");
    };
    assert_eq!(inspect.data["inspection"]["format_id"], "xlsx");
}
