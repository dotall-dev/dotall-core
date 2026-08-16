use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use dotall_core::{DotallError, EncodedSnapshot, Result, SnapshotPart};
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct SnapshotManifest {
    schema_id: String,
    schema_version: u32,
    package_hash: String,
    entries: Vec<ManifestEntry>,
    tail_part_hash: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ManifestEntry {
    name: String,
    header_part_hash: String,
    part_hash: String,
    trailer_part_hash: String,
    compression: String,
    crc32: u32,
    compressed_size: u64,
    uncompressed_size: u64,
}

/// Explodes a ZIP package into content-addressed local-record slices.
pub fn encode_package(
    package: &[u8],
    format_id: &str,
    manifest_schema_id: &str,
    package_hash: Option<&str>,
) -> Result<EncodedSnapshot> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| snapshot_error(format_id, format!("invalid OOXML package: {error}")))?;
    let central_start = usize::try_from(archive.central_directory_start()).map_err(|_| {
        snapshot_error(
            format_id,
            "ZIP central-directory offset exceeds platform limits",
        )
    })?;
    if central_start > package.len() {
        return Err(snapshot_error(
            format_id,
            "ZIP central-directory offset exceeds package length",
        ));
    }

    let mut records = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|error| {
            snapshot_error(format_id, format!("cannot inspect ZIP entry: {error}"))
        })?;
        records.push((
            usize::try_from(entry.header_start()).map_err(|_| {
                snapshot_error(format_id, "ZIP entry offset exceeds platform limits")
            })?,
            usize::try_from(entry.data_start().ok_or_else(|| {
                snapshot_error(format_id, "ZIP entry data offset is unavailable")
            })?)
            .map_err(|_| {
                snapshot_error(format_id, "ZIP entry data offset exceeds platform limits")
            })?,
            entry.name().to_owned(),
            format!("{:?}", entry.compression()).to_ascii_lowercase(),
            entry.crc32(),
            entry.compressed_size(),
            entry.size(),
        ));
    }
    records.sort_by_key(|record| record.0);

    let mut parts = Vec::with_capacity(records.len() + 1);
    let mut seen_hashes = BTreeSet::new();
    let mut entries = Vec::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        let header_start = if index == 0 { 0 } else { record.0 };
        let data_start = record.1;
        let data_end = data_start
            .checked_add(usize::try_from(record.5).map_err(|_| {
                snapshot_error(format_id, "ZIP compressed size exceeds platform limits")
            })?)
            .ok_or_else(|| snapshot_error(format_id, "ZIP compressed payload range overflows"))?;
        let end = records.get(index + 1).map_or(central_start, |next| next.0);
        if header_start > data_start
            || data_start > data_end
            || data_end > end
            || end > central_start
        {
            return Err(snapshot_error(
                format_id,
                "ZIP local-file record offsets overlap",
            ));
        }
        let header_part_hash = push_part(
            &mut parts,
            &mut seen_hashes,
            &package[header_start..data_start],
        );
        let part_hash = push_part(&mut parts, &mut seen_hashes, &package[data_start..data_end]);
        let trailer_part_hash = push_part(&mut parts, &mut seen_hashes, &package[data_end..end]);
        entries.push(ManifestEntry {
            name: record.2.clone(),
            header_part_hash,
            part_hash,
            trailer_part_hash,
            compression: record.3.clone(),
            crc32: record.4,
            compressed_size: record.5,
            uncompressed_size: record.6,
        });
    }

    let tail_part_hash = push_part(
        &mut parts,
        &mut seen_hashes,
        &package[if records.is_empty() { 0 } else { central_start }..],
    );
    let package_hash = package_hash
        .map(str::to_owned)
        .unwrap_or_else(|| blake3::hash(package).to_hex().to_string());
    let manifest = SnapshotManifest {
        schema_id: manifest_schema_id.to_owned(),
        schema_version: MANIFEST_SCHEMA_VERSION,
        package_hash: package_hash.clone(),
        entries,
        tail_part_hash,
    };

    Ok(EncodedSnapshot {
        package_hash,
        format_id: format_id.to_owned(),
        manifest: serde_json::to_value(manifest).map_err(|source| DotallError::Serialization {
            context: "OOXML snapshot manifest".into(),
            source,
        })?,
        parts,
    })
}

/// Reconstructs exact ZIP bytes from a part snapshot.
pub fn decode_package(encoded: &EncodedSnapshot) -> Result<Vec<u8>> {
    let format_id = encoded.format_id.as_str();
    let manifest: SnapshotManifest =
        serde_json::from_value(encoded.manifest.clone()).map_err(|source| {
            DotallError::Serialization {
                context: "OOXML snapshot manifest".into(),
                source,
            }
        })?;
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION
        || manifest.package_hash != encoded.package_hash
    {
        return Err(snapshot_error(
            format_id,
            "unsupported or inconsistent snapshot manifest",
        ));
    }

    let parts = encoded
        .parts
        .iter()
        .map(|part| (part.hash.as_str(), part.bytes.as_slice()))
        .collect::<BTreeMap<_, _>>();
    let mut package = Vec::new();
    for entry in &manifest.entries {
        append_part(format_id, &mut package, &parts, &entry.header_part_hash)?;
        append_part(format_id, &mut package, &parts, &entry.part_hash)?;
        append_part(format_id, &mut package, &parts, &entry.trailer_part_hash)?;
    }
    append_part(format_id, &mut package, &parts, &manifest.tail_part_hash)?;
    let actual_hash = blake3::hash(&package).to_hex().to_string();
    if actual_hash != encoded.package_hash {
        return Err(snapshot_error(
            format_id,
            format!(
                "reconstructed package hash {actual_hash} does not match {}",
                encoded.package_hash
            ),
        ));
    }
    Ok(package)
}

fn push_part(
    parts: &mut Vec<SnapshotPart>,
    seen_hashes: &mut BTreeSet<String>,
    bytes: &[u8],
) -> String {
    let hash = blake3::hash(bytes).to_hex().to_string();
    if seen_hashes.insert(hash.clone()) {
        parts.push(SnapshotPart {
            hash: hash.clone(),
            bytes: bytes.to_vec(),
        });
    }
    hash
}

fn append_part(
    format_id: &str,
    output: &mut Vec<u8>,
    parts: &BTreeMap<&str, &[u8]>,
    hash: &str,
) -> Result<()> {
    let bytes = parts
        .get(hash)
        .ok_or_else(|| DotallError::SnapshotMissing {
            path: "<ooxml snapshot>".into(),
            hash: hash.to_owned(),
        })?;
    if blake3::hash(bytes).to_hex().as_str() != hash {
        return Err(snapshot_error(
            format_id,
            format!("snapshot part {hash} does not match its content hash"),
        ));
    }
    output.extend_from_slice(bytes);
    Ok(())
}

fn snapshot_error(format_id: &str, message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: format_id.into(),
        path: "<ooxml snapshot>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::io::{Cursor, Write};

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::{decode_package, encode_package};

    #[test]
    fn snapshot_parts_reconstruct_the_exact_zip_bytes() {
        let package = package(b"one", b"shared");

        let encoded = encode_package(&package, "xlsx", "xlsx.snapshot-manifest", None)
            .expect("encode package");
        let decoded = decode_package(&encoded).expect("decode package");

        assert_eq!(decoded, package);
        assert_eq!(
            blake3::hash(&decoded).to_hex().to_string(),
            encoded.package_hash
        );
        assert_eq!(encoded.format_id, "xlsx");
        assert_eq!(encoded.manifest["schema_id"], "xlsx.snapshot-manifest");
        assert!(encoded.parts.len() > 1);
    }

    #[test]
    fn encode_records_caller_format_and_schema_ids() {
        let package = package(b"one", b"shared");
        let encoded = encode_package(&package, "pptx", "pptx.snapshot-manifest", None)
            .expect("encode package");
        assert_eq!(encoded.format_id, "pptx");
        assert_eq!(encoded.manifest["schema_id"], "pptx.snapshot-manifest");
        assert_eq!(decode_package(&encoded).expect("decode"), package);
    }

    #[test]
    fn packages_with_one_changed_entry_share_local_record_parts() {
        let first = encode_package(
            &package(b"one", b"shared"),
            "xlsx",
            "xlsx.snapshot-manifest",
            None,
        )
        .expect("encode first");
        let second = encode_package(
            &package(b"two", b"shared"),
            "xlsx",
            "xlsx.snapshot-manifest",
            None,
        )
        .expect("encode second");
        let first_hashes = first
            .parts
            .iter()
            .map(|part| part.hash.as_str())
            .collect::<BTreeSet<_>>();
        let shared = second
            .parts
            .iter()
            .filter(|part| first_hashes.contains(part.hash.as_str()))
            .count();

        assert!(shared >= 1, "unchanged local records should deduplicate");
    }

    fn package(changed: &[u8], shared: &[u8]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default();
        writer
            .start_file("xl/worksheets/sheet1.xml", options)
            .expect("first entry");
        writer.write_all(changed).expect("first bytes");
        writer
            .start_file("xl/styles.xml", options)
            .expect("shared entry");
        writer.write_all(shared).expect("shared bytes");
        writer.finish().expect("finish package").into_inner()
    }
}
