//! Compare full-file snapshot storage vs OOXML part-manifest storage.
//!
//! Usage:
//!   cargo run -p dotall-xlsx --example bench_snapshot_size -- <file.xlsx> [more.xlsx...]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dotall_core::registry::{FormatHandler, FormatRegistry};
use dotall_core::{Actor, ActorKind, DotallStore, EditRequest, Engine, SemanticOperation};
use dotall_xlsx::XlsxFormat;
use tempfile::tempdir;
use uuid::Uuid;

fn main() {
    let files: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if files.is_empty() {
        eprintln!("usage: bench_snapshot_size <file.xlsx> [more.xlsx...]");
        std::process::exit(2);
    }

    println!(
        "{:<40} {:>8} {:>5} {:>10} {:>10} {:>10} {:>10} {:>8}",
        "file", "src", "edits", "full×N", "parts", "manifests", "part_total", "savings"
    );

    for file in &files {
        match bench_file(file, 5) {
            Ok(row) => {
                let savings = if row.full_model_bytes == 0 {
                    0.0
                } else {
                    100.0 * (1.0 - row.part_total_bytes as f64 / row.full_model_bytes as f64)
                };
                println!(
                    "{:<40} {:>8} {:>5} {:>10} {:>10} {:>10} {:>10} {:>6.1}%",
                    truncate_name(file),
                    human(row.source_bytes),
                    row.edits,
                    human(row.full_model_bytes),
                    human(row.parts_bytes),
                    human(row.manifest_bytes),
                    human(row.part_total_bytes),
                    savings
                );
            }
            Err(error) => eprintln!("{}: {error}", file.display()),
        }
    }
}

struct BenchRow {
    source_bytes: u64,
    edits: usize,
    full_model_bytes: u64,
    parts_bytes: u64,
    manifest_bytes: u64,
    part_total_bytes: u64,
}

fn bench_file(path: &Path, edits: usize) -> Result<BenchRow, Box<dyn std::error::Error>> {
    let source_bytes = fs::metadata(path)?.len();
    let workspace = tempdir()?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("non-utf8 filename")?;
    let local = workspace.path().join(name);
    fs::copy(path, &local)?;

    let store = DotallStore::init(workspace.path())?;
    let mut registry = FormatRegistry::default();
    registry.register(Arc::new(XlsxFormat));
    let mut engine = Engine::new(store, registry);

    let inspection = engine.inspect(name)?;
    let sheet = inspection.inspection.summary["sheets"]
        .as_array()
        .and_then(|sheets| sheets.first())
        .and_then(|sheet| sheet["name"].as_str())
        .unwrap_or("Sheet1")
        .to_owned();

    let mut full_model_bytes = 0_u64;
    for index in 0..edits {
        // Old model: store a full copy of pre-apply bytes each apply.
        full_model_bytes += fs::metadata(&local)?.len();

        let loaded = engine.load_model(name)?;
        let tx = Uuid::new_v4();
        // Write into a far column to avoid colliding with existing headers/data.
        let address = format!("ZZ{}", 200 + index);
        engine.edit(
            name,
            &EditRequest {
                transaction_id: tx,
                expected_source_hash: loaded.source_hash,
                operations: vec![SemanticOperation {
                    kind: "set_cell_value".into(),
                    payload: serde_json::json!({
                        "sheet": sheet,
                        "address": address,
                        "value": index as i64 + 1,
                    }),
                }],
                actor: Actor {
                    kind: ActorKind::Cli,
                    id: Some("bench".into()),
                },
            },
        )?;
        engine.apply(name, tx)?;
    }

    // Confirm lossless reconstruct for the latest snapshot.
    let history = engine.history(name)?;
    let last = history.last().ok_or("expected history")?;
    let record = engine.diff(name, last.version)?;
    let store = DotallStore::open(workspace.path())?;
    let encoded = store.read_encoded_snapshot(name, &record.snapshot_ref)?;
    let reconstructed = XlsxFormat.decode_snapshot(&encoded)?;
    assert_eq!(
        blake3::hash(&reconstructed).to_hex().to_string(),
        record.snapshot_ref,
        "snapshot reconstruct must be lossless"
    );

    let snapshots = workspace
        .path()
        .join(".all/objects")
        .join(name)
        .join("state/edits/history/snapshots");
    let parts_bytes = dir_size(&snapshots.join("parts"))?;
    let manifest_bytes = dir_size(&snapshots.join("manifests"))?;

    Ok(BenchRow {
        source_bytes,
        edits: history.len(),
        full_model_bytes,
        parts_bytes,
        manifest_bytes,
        part_total_bytes: parts_bytes + manifest_bytes,
    })
}

fn dir_size(path: &Path) -> Result<u64, Box<dyn std::error::Error>> {
    let mut total = 0_u64;
    for entry in walkdir(path)? {
        total += fs::metadata(entry)?.len();
    }
    Ok(total)
}

fn walkdir(path: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    if !path.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(path)? {
        let path = entry?.path();
        if path.is_dir() {
            out.extend(walkdir(&path)?);
        } else {
            out.push(path);
        }
    }
    Ok(out)
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

fn truncate_name(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?");
    if name.len() <= 42 {
        name.to_owned()
    } else {
        format!("{}…", &name[..41])
    }
}
