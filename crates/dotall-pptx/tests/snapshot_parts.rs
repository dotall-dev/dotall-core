use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use dotall_core::registry::{EncodedSnapshot, FormatHandler, FormatRegistry, SnapshotPart};
use dotall_core::{Actor, ActorKind, DotallStore, EditRequest, Engine, SemanticOperation};
use dotall_pptx::{PptxFormat, minimal_pptx_with_media};
use tempfile::tempdir;

#[test]
fn two_text_edits_share_snapshot_parts() {
    let workspace = tempdir().expect("workspace");
    let source = workspace.path().join("deck.pptx");
    let original = minimal_pptx_with_media(true);
    fs::write(&source, &original).expect("write fixture");
    let original_size = original.len() as u64;

    let store = DotallStore::init(workspace.path()).expect("store");
    let mut registry = FormatRegistry::default();
    registry.register(Arc::new(PptxFormat));
    let mut engine = Engine::new(store, registry);

    let first_hash = engine.load_model("deck.pptx").expect("load").source_hash;
    engine
        .edit("deck.pptx", &shape_text_request(&first_hash, "World", 1))
        .expect("stage first");
    engine.apply("deck.pptx", uuid(1)).expect("apply first");

    let handler = PptxFormat;
    let history = engine.diff("deck.pptx", 1).expect("history v1");
    let snapshots = workspace
        .path()
        .join(".all/objects/deck.pptx/state/edits/history/snapshots");
    let encoded = load_encoded(&snapshots, &history.snapshot_ref);
    let restored = handler.decode_snapshot(&encoded).expect("decode");
    assert_eq!(restored, original);

    let second_hash = engine.load_model("deck.pptx").expect("reload").source_hash;
    engine
        .edit("deck.pptx", &shape_text_request(&second_hash, "Again", 2))
        .expect("stage second");
    engine.apply("deck.pptx", uuid(2)).expect("apply second");

    let first_parts = read_part_hashes(&snapshots, &history.snapshot_ref);
    let second_parts = read_part_hashes(
        &snapshots,
        &engine
            .diff("deck.pptx", 2)
            .expect("history v2")
            .snapshot_ref,
    );
    let shared = first_parts.intersection(&second_parts).count();
    assert!(
        shared >= 3,
        "media and theme-like parts should be reused, found {shared}"
    );

    let stored_size = directory_size(&snapshots);
    assert!(
        stored_size < original_size * 3 / 2,
        "snapshot storage {stored_size} should be far below two full copies of {original_size}"
    );
}

fn shape_text_request(expected_source_hash: &str, text: &str, n: u8) -> EditRequest {
    EditRequest {
        transaction_id: uuid(n),
        expected_source_hash: expected_source_hash.to_owned(),
        actor: Actor {
            kind: ActorKind::Cli,
            id: Some("pptx-snapshot-test".into()),
        },
        operations: vec![SemanticOperation {
            kind: "set_shape_text".into(),
            payload: serde_json::json!({
                "slide": "Slide 1",
                "shape": "Title",
                "text": text,
            }),
        }],
    }
}

fn uuid(n: u8) -> uuid::Uuid {
    uuid::Uuid::parse_str(&format!("00000000-0000-0000-0000-00000000000{n}")).expect("uuid")
}

fn load_encoded(snapshots: &Path, package_hash: &str) -> EncodedSnapshot {
    let manifest_path = snapshots
        .join("manifests")
        .join(format!("{package_hash}.json"));
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(manifest_path).expect("manifest")).expect("json");
    let mut parts = Vec::new();
    let mut hashes = Vec::new();
    if let Some(entries) = value["manifest"]["entries"].as_array() {
        for entry in entries {
            for key in ["header_part_hash", "part_hash", "trailer_part_hash"] {
                if let Some(hash) = entry[key].as_str() {
                    hashes.push(hash.to_owned());
                }
            }
        }
    }
    if let Some(hash) = value["manifest"]["tail_part_hash"].as_str() {
        hashes.push(hash.to_owned());
    }
    hashes.sort();
    hashes.dedup();
    for hash in hashes {
        let bytes = fs::read(snapshots.join("parts").join(&hash)).expect("part bytes");
        parts.push(SnapshotPart { hash, bytes });
    }
    EncodedSnapshot {
        package_hash: package_hash.to_owned(),
        format_id: "pptx".into(),
        manifest: value["manifest"].clone(),
        parts,
    }
}

fn read_part_hashes(snapshots: &Path, package_hash: &str) -> BTreeSet<String> {
    let path = snapshots
        .join("manifests")
        .join(format!("{package_hash}.json"));
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(path).expect("snapshot manifest"))
            .expect("parse snapshot manifest");
    let mut hashes = value["manifest"]["entries"]
        .as_array()
        .expect("manifest entries")
        .iter()
        .flat_map(|entry| {
            [
                entry["header_part_hash"].as_str().unwrap().to_owned(),
                entry["part_hash"].as_str().unwrap().to_owned(),
                entry["trailer_part_hash"].as_str().unwrap().to_owned(),
            ]
        })
        .collect::<BTreeSet<_>>();
    hashes.insert(
        value["manifest"]["tail_part_hash"]
            .as_str()
            .expect("tail")
            .to_owned(),
    );
    hashes
}

fn directory_size(path: &Path) -> u64 {
    fs::read_dir(path)
        .expect("snapshot directory")
        .map(|entry| entry.expect("snapshot entry").path())
        .map(|path| {
            if path.is_dir() {
                directory_size(&path)
            } else {
                fs::metadata(path).expect("snapshot metadata").len()
            }
        })
        .sum()
}
