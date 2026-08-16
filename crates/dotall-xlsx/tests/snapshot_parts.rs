use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use dotall_core::registry::FormatRegistry;
use dotall_core::{Actor, ActorKind, DotallStore, EditRequest, Engine, SemanticOperation};
use dotall_xlsx::XlsxFormat;
use rust_xlsxwriter::Workbook;
use tempfile::tempdir;

#[test]
fn two_small_edits_share_parts_and_store_far_less_than_two_workbooks() {
    let workspace = tempdir().expect("workspace");
    let source = workspace.path().join("book.xlsx");
    write_fixture(&source);
    let original_size = fs::metadata(&source).expect("source metadata").len();

    let store = DotallStore::init(workspace.path()).expect("store");
    let mut registry = FormatRegistry::default();
    registry.register(Arc::new(XlsxFormat));
    let mut engine = Engine::new(store, registry);

    let first_hash = engine
        .load_model("book.xlsx")
        .expect("load workbook")
        .source_hash;
    engine
        .edit(
            "book.xlsx",
            &set_cell_request(
                first_hash,
                "Inputs",
                "A1",
                11,
                "00000000-0000-0000-0000-000000000001",
            ),
        )
        .expect("stage first edit");
    engine
        .apply(
            "book.xlsx",
            "00000000-0000-0000-0000-000000000001"
                .parse()
                .expect("first transaction id"),
        )
        .expect("apply first edit");

    let second_hash = engine
        .load_model("book.xlsx")
        .expect("reload workbook")
        .source_hash;
    engine
        .edit(
            "book.xlsx",
            &set_cell_request(
                second_hash,
                "Outputs",
                "A1",
                22,
                "00000000-0000-0000-0000-000000000002",
            ),
        )
        .expect("stage second edit");
    engine
        .apply(
            "book.xlsx",
            "00000000-0000-0000-0000-000000000002"
                .parse()
                .expect("second transaction id"),
        )
        .expect("apply second edit");

    let snapshots = workspace
        .path()
        .join(".all/objects/book.xlsx/state/edits/history/snapshots");
    let first_manifest = read_part_hashes(
        &snapshots,
        &engine
            .diff("book.xlsx", 1)
            .expect("first history")
            .snapshot_ref,
    );
    let second_manifest = read_part_hashes(
        &snapshots,
        &engine
            .diff("book.xlsx", 2)
            .expect("second history")
            .snapshot_ref,
    );
    let shared_count = first_manifest.intersection(&second_manifest).count();
    assert!(
        shared_count >= 3,
        "expected unchanged OOXML records to share hashes, found {shared_count}"
    );

    let stored_size = directory_size(&snapshots);
    assert!(
        stored_size < original_size * 3 / 2,
        "snapshot storage {stored_size} should be far below two full copies of {original_size}"
    );
}

fn set_cell_request(
    expected_source_hash: String,
    sheet: &str,
    address: &str,
    value: i64,
    transaction_id: &str,
) -> EditRequest {
    EditRequest {
        transaction_id: transaction_id.parse().expect("transaction id"),
        expected_source_hash,
        actor: Actor {
            kind: ActorKind::Cli,
            id: Some("snapshot-test".into()),
        },
        operations: vec![SemanticOperation {
            kind: "set_cell_value".into(),
            payload: serde_json::json!({
                "sheet": sheet,
                "address": address,
                "value": value,
            }),
        }],
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
        .map(|entry| {
            entry["part_hash"]
                .as_str()
                .expect("entry part hash")
                .to_owned()
        })
        .collect::<BTreeSet<_>>();
    hashes.insert(
        value["manifest"]["tail_part_hash"]
            .as_str()
            .expect("tail part hash")
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

fn write_fixture(path: &Path) {
    let mut workbook = Workbook::new();
    workbook
        .add_worksheet()
        .set_name("Inputs")
        .expect("Inputs sheet")
        .write_number(0, 0, 1)
        .expect("Inputs A1");
    workbook
        .add_worksheet()
        .set_name("Outputs")
        .expect("Outputs sheet")
        .write_number(0, 0, 2)
        .expect("Outputs A1");
    let shared = workbook
        .add_worksheet()
        .set_name("Shared")
        .expect("Shared sheet");
    for row in 0..10_000 {
        shared
            .write_string(row, 0, format!("shared-row-{row:05}-0123456789abcdef"))
            .expect("shared value");
    }
    workbook.save(path).expect("save workbook");
}
