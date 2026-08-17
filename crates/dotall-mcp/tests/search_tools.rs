use dotall_mcp::params::SearchParams;
use dotall_mcp::response::ToolResponse;
use dotall_mcp::server::{DotallServer, FlushOnClose};
use rmcp::handler::server::wrapper::Parameters;
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;

#[tokio::test]
async fn search_returns_named_range_after_inspect() {
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("book.xlsx");
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.set_name("Inputs").unwrap();
    sheet.write_string(0, 0, "Rate").unwrap();
    workbook.define_name("Rate", "=Inputs!$A$1").unwrap();
    workbook.save(&file).unwrap();

    let server =
        DotallServer::open_or_init(workspace.path(), FlushOnClose::default()).expect("server");
    let _ = server
        .dotall_inspect(Parameters(dotall_mcp::params::FileParams {
            file: file.display().to_string(),
        }))
        .await;

    let response = server
        .dotall_search(Parameters(SearchParams {
            query: "Rate".into(),
            glob: None,
        }))
        .await
        .0;
    let ToolResponse::Success { result, .. } = response else {
        panic!("search should succeed");
    };
    assert!(!result.data["hits"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn search_reports_not_indexed_for_tracked_uncached_file() {
    // search_store reports tracked files with no cached model/views as not_indexed.
    // A file becomes tracked by inspecting it; clearing its cache directory
    // leaves it tracked but not indexed.
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("book.xlsx");
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.set_name("Inputs").unwrap();
    sheet.write_string(0, 0, "Rate").unwrap();
    workbook.save(&file).unwrap();

    let server =
        DotallServer::open_or_init(workspace.path(), FlushOnClose::default()).expect("server");
    let _ = server
        .dotall_inspect(Parameters(dotall_mcp::params::FileParams {
            file: file.display().to_string(),
        }))
        .await;

    // Remove the cached model so the file is tracked but not indexed.
    let cache_model = workspace
        .path()
        .join(".all/objects/book.xlsx/cache/model/model.json");
    assert!(
        cache_model.is_file(),
        "model cache should exist after inspect"
    );
    std::fs::remove_file(&cache_model).expect("remove model cache");

    let response = server
        .dotall_search(Parameters(SearchParams {
            query: "Rate".into(),
            glob: None,
        }))
        .await
        .0;
    let ToolResponse::Success { result, .. } = response else {
        panic!("search should succeed");
    };
    let not_indexed = result.data["not_indexed"].as_array().expect("not_indexed");
    assert!(
        not_indexed.iter().any(|path| path == "book.xlsx"),
        "tracked-but-uncached file should be reported as not_indexed, got {not_indexed:?}"
    );
    assert!(result.data["hits"].as_array().expect("hits").is_empty());
}

#[tokio::test]
async fn search_rejects_empty_query() {
    let workspace = tempdir().expect("workspace");
    let server =
        DotallServer::open_or_init(workspace.path(), FlushOnClose::default()).expect("server");

    let response = server
        .dotall_search(Parameters(SearchParams {
            query: "   ".into(),
            glob: None,
        }))
        .await
        .0;
    assert!(
        matches!(response, ToolResponse::Error { .. }),
        "empty query should error"
    );
}

#[tokio::test]
async fn search_glob_skips_non_matching_paths() {
    let workspace = tempdir().expect("workspace");
    let file = workspace.path().join("book.xlsx");
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.set_name("Inputs").unwrap();
    sheet.write_string(0, 0, "Rate").unwrap();
    workbook.define_name("Rate", "=Inputs!$A$1").unwrap();
    workbook.save(&file).unwrap();

    let server =
        DotallServer::open_or_init(workspace.path(), FlushOnClose::default()).expect("server");
    let _ = server
        .dotall_inspect(Parameters(dotall_mcp::params::FileParams {
            file: file.display().to_string(),
        }))
        .await;

    let response = server
        .dotall_search(Parameters(SearchParams {
            query: "Rate".into(),
            glob: Some("other/*.xlsx".into()),
        }))
        .await
        .0;
    let ToolResponse::Success { result, .. } = response else {
        panic!("search should succeed");
    };
    assert!(result.data["hits"].as_array().expect("hits").is_empty());
    assert!(
        result.data["not_indexed"]
            .as_array()
            .expect("not_indexed")
            .is_empty()
    );
}
