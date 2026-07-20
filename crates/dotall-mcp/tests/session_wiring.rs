use std::fs;
use std::sync::MutexGuard;

use dotall_core::Engine;
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

    let server = DotallServer::open_or_init(temp.path(), FlushOnClose::default()).expect("init");

    assert!(temp.path().join(".all/manifest.json").is_file());
    assert!(temp.path().join(".all/objects").is_dir());

    let relative = relative_path_for_file(&server, &file).expect("relative path");
    assert_eq!(relative, "data/book.xlsx");

    let engine = server.engine();
    let guard: MutexGuard<'_, Engine> = engine.lock().expect("engine mutex");
    drop(guard);
}
