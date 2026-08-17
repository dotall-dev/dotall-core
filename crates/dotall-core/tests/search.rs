use std::fs;

use dotall_core::{
    CachedView, DotallError, DotallStore, ReadResponse, SearchRequest, search_store,
};
use tempfile::tempdir;

fn init_book() -> (tempfile::TempDir, DotallStore) {
    let temp = tempdir().expect("tempdir");
    fs::write(temp.path().join("book.xlsx"), b"source").expect("source");
    let mut store = DotallStore::init(temp.path()).expect("store");
    store
        .register_source("book.xlsx", "xlsx")
        .expect("register");
    (temp, store)
}

#[test]
fn empty_query_is_invalid_argument() {
    let (_temp, store) = init_book();
    let err = search_store(
        &store,
        &SearchRequest {
            query: "   ".into(),
            glob: None,
        },
    )
    .expect_err("empty");
    assert!(matches!(err, DotallError::InvalidArgument { .. }));
}

#[test]
fn missing_cache_lists_not_indexed_and_has_no_hits() {
    let (_temp, store) = init_book();
    let results = search_store(
        &store,
        &SearchRequest {
            query: "Rate".into(),
            glob: None,
        },
    )
    .expect("search");
    assert!(results.hits.is_empty());
    assert_eq!(results.not_indexed, vec!["book.xlsx".to_string()]);
}

#[test]
fn view_content_hit_is_case_insensitive() {
    let (_temp, store) = init_book();
    store
        .write_view(
            "book.xlsx",
            &CachedView {
                source_hash: store.manifest().objects["book.xlsx"]
                    .fingerprint
                    .blake3
                    .clone(),
                renderer_id: "test".into(),
                renderer_version: "1".into(),
                request_hash: "view1".into(),
                response: ReadResponse {
                    content: "Commission Rate is 10%\n".into(),
                    estimated_tokens: 4,
                    truncated: false,
                    continuation: None,
                    next_actions: vec![],
                },
            },
        )
        .expect("view");

    let results = search_store(
        &store,
        &SearchRequest {
            query: "rate".into(),
            glob: None,
        },
    )
    .expect("search");
    assert_eq!(results.not_indexed.len(), 0);
    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].path, "book.xlsx");
    assert_eq!(results.hits[0].format_id, "xlsx");
    assert!(results.hits[0].snippet.to_lowercase().contains("rate"));
    assert!(results.hits[0].selector_kind.is_none());
}
