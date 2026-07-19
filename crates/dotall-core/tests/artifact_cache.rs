use std::fs;

use dotall_core::{ArtifactEnvelope, CachedDerived, CachedView, DotallStore, ReadResponse};
use tempfile::tempdir;

fn fixture() -> (tempfile::TempDir, DotallStore) {
    let temp = tempdir().expect("tempdir");
    fs::write(temp.path().join("book.xlsx"), b"source").expect("source");
    let mut store = DotallStore::init(temp.path()).expect("store");
    store
        .register_source("book.xlsx", "xlsx")
        .expect("register source");
    (temp, store)
}

#[test]
fn model_artifact_round_trips_under_the_tracked_object() {
    let (_temp, store) = fixture();
    let artifact = ArtifactEnvelope {
        format_id: "xlsx".into(),
        schema_id: "xlsx.workbook".into(),
        schema_version: 1,
        payload: serde_json::json!({"sheets": []}),
    };

    store
        .write_model("book.xlsx", &artifact)
        .expect("write model");
    let loaded = store
        .read_model("book.xlsx")
        .expect("read model")
        .expect("present model");

    assert_eq!(loaded, artifact);
}

#[test]
fn model_cache_misses_when_its_source_hash_differs() {
    let (temp, store) = fixture();
    let artifact = ArtifactEnvelope {
        format_id: "xlsx".into(),
        schema_id: "xlsx.workbook".into(),
        schema_version: 1,
        payload: serde_json::json!({"sheets": []}),
    };

    store
        .write_model("book.xlsx", &artifact)
        .expect("write model");
    let model_path = temp
        .path()
        .join(".all/objects/book.xlsx/cache/model/model.json");
    let mut cached: serde_json::Value =
        serde_json::from_slice(&fs::read(&model_path).expect("read cached model"))
            .expect("decode cached model");
    cached["source_hash"] = serde_json::json!("wrong-source-hash");
    fs::write(
        &model_path,
        serde_json::to_vec(&cached).expect("encode cached model"),
    )
    .expect("overwrite cached model");

    assert_eq!(store.read_model("book.xlsx").expect("read model"), None);
}

#[test]
fn view_and_derived_artifacts_round_trip() {
    let (_temp, store) = fixture();
    let view = CachedView {
        source_hash: store.manifest().objects["book.xlsx"]
            .fingerprint
            .blake3
            .clone(),
        renderer_id: "xlsx.markdown".into(),
        renderer_version: "1".into(),
        request_hash: "read-summary".into(),
        response: ReadResponse {
            content: "# Workbook".into(),
            estimated_tokens: 3,
            truncated: false,
            continuation: None,
            next_actions: vec!["Read a sheet".into()],
        },
    };
    let derived = CachedDerived {
        source_hash: view.source_hash.clone(),
        model_schema_id: "xlsx.workbook".into(),
        model_schema_version: 1,
        processor_id: "xlsx.structure".into(),
        processor_version: "1".into(),
        payload: serde_json::json!({"sheets": []}),
    };

    store.write_view("book.xlsx", &view).expect("write view");
    store
        .write_derived("book.xlsx", "structure", &derived)
        .expect("write derived");

    assert_eq!(
        store
            .read_view("book.xlsx", "read-summary")
            .expect("read view"),
        Some(view)
    );
    assert_eq!(
        store
            .read_derived("book.xlsx", "structure")
            .expect("read derived"),
        Some(derived)
    );
}
