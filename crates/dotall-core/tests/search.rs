use std::fs;

use dotall_core::{
    ArtifactEnvelope, CachedView, DotallError, DotallStore, ReadResponse, SearchRequest,
    search_store,
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

#[test]
fn named_range_in_model_gets_a_selector() {
    let (_temp, store) = init_book();
    let hash = store.manifest().objects["book.xlsx"]
        .fingerprint
        .blake3
        .clone();
    store
        .write_model(
            "book.xlsx",
            &ArtifactEnvelope {
                format_id: "xlsx".into(),
                schema_id: "xlsx.workbook".into(),
                schema_version: 1,
                payload: serde_json::json!({
                    "named_ranges": [{
                        "name": "Rate",
                        "formula": "Inputs!$B$2",
                        "element_id": "n_rate"
                    }]
                }),
            },
        )
        .expect("model");

    let results = search_store(
        &store,
        &SearchRequest {
            query: "Rate".into(),
            glob: None,
        },
    )
    .expect("search");
    let hit = results
        .hits
        .iter()
        .find(|h| h.selector_kind.as_deref() == Some("named_ranges"))
        .expect("named range hit");
    assert_eq!(hit.selector.as_deref(), Some("Rate"));
    assert!(hit.snippet.contains("Inputs!$B$2"));
    let _ = hash;
}

#[test]
fn glob_limits_paths() {
    let temp = tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join("q3-pack")).expect("dir");
    fs::write(temp.path().join("other.xlsx"), b"a").expect("other");
    fs::write(temp.path().join("q3-pack/pack.xlsx"), b"b").expect("pack");
    let mut store = DotallStore::init(temp.path()).expect("store");
    store.register_source("other.xlsx", "xlsx").expect("reg");
    store
        .register_source("q3-pack/pack.xlsx", "xlsx")
        .expect("reg");
    for path in ["other.xlsx", "q3-pack/pack.xlsx"] {
        store
            .write_view(
                path,
                &CachedView {
                    source_hash: store.manifest().objects[path].fingerprint.blake3.clone(),
                    renderer_id: "t".into(),
                    renderer_version: "1".into(),
                    request_hash: "v".into(),
                    response: ReadResponse {
                        content: "Rate 10%".into(),
                        estimated_tokens: 2,
                        truncated: false,
                        continuation: None,
                        next_actions: vec![],
                    },
                },
            )
            .expect("view");
    }
    let results = search_store(
        &store,
        &SearchRequest {
            query: "10%".into(),
            glob: Some("q3-pack/*".into()),
        },
    )
    .expect("search");
    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].path, "q3-pack/pack.xlsx");
}

#[test]
fn snapshot_part_bytes_are_not_searched() {
    let (temp, store) = init_book();
    let part = temp
        .path()
        .join(".all/objects/book.xlsx/state/edits/history/snapshots/parts/deadbeef");
    fs::create_dir_all(part.parent().unwrap()).expect("dir");
    fs::write(&part, b"secret Rate in a ZIP part").expect("part");
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
