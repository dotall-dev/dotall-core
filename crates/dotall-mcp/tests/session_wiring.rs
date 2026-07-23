use std::fs;

use dotall_mcp::server::{DotallServer, FlushOnClose};
use dotall_mcp::tools::relative_path_for_file;
use tempfile::tempdir;

#[test]
fn open_or_init_initializes_store_and_engine() {
    let temp = tempdir().expect("tempdir");
    let nested = temp.path().join("data");
    fs::create_dir_all(&nested).expect("nested directory");
    let file = nested.join("book.xlsx");
    fs::write(&file, b"initial").expect("source file");

    let _server = DotallServer::open_or_init(temp.path(), FlushOnClose::default()).expect("init");

    assert!(temp.path().join(".all/manifest.json").is_file());
    assert!(temp.path().join(".all/objects").is_dir());

    let workspace_root = temp.path().canonicalize().expect("workspace root");
    let relative = relative_path_for_file(&workspace_root, &file).expect("relative path");
    assert_eq!(relative, "data/book.xlsx");
}
