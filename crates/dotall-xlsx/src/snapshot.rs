use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use dotall_core::{DotallError, EncodedSnapshot, Result, SnapshotPart};
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use crate::FORMAT_ID;

const MANIFEST_SCHEMA_ID: &str = "xlsx.snapshot-manifest";
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
    part_hash: String,
    compression: String,
    crc32: u32,
    compressed_size: u64,
    uncompressed_size: u64,
}

pub(crate) fn encode(package: &[u8]) -> Result<EncodedSnapshot> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| snapshot_error(format!("invalid XLSX package: {error}")))?;
    let central_start = usize::try_from(archive.central_directory_start())
        .map_err(|_| snapshot_error("ZIP central-directory offset exceeds platform limits"))?;
    if central_start > package.len() {
        return Err(snapshot_error(
            "ZIP central-directory offset exceeds package length",
        ));
    }

    let mut records = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| snapshot_error(format!("cannot inspect ZIP entry: {error}")))?;
        records.push((
            usize::try_from(entry.header_start())
                .map_err(|_| snapshot_error("ZIP entry offset exceeds platform limits"))?,
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
        let start = if index == 0 { 0 } else { record.0 };
        let end = records
            .get(index + 1)
            .map_or(central_start, |next| next.0);
        if start > end || end > central_start {
            return Err(snapshot_error("ZIP local-file record offsets overlap"));
        }
        let part_hash = push_part(&mut parts, &mut seen_hashes, &package[start..end]);
        entries.push(ManifestEntry {
            name: record.1.clone(),
            part_hash,
            compression: record.2.clone(),
            crc32: record.3,
            compressed_size: record.4,
            uncompressed_size: record.5,
        });
    }

    let tail_part_hash = push_part(
        &mut parts,
        &mut seen_hashes,
        &package[if records.is_empty() {
            0
        } else {
            central_start
        }..],
    );
    let package_hash = blake3::hash(package).to_hex().to_string();
    let manifest = SnapshotManifest {
        schema_id: MANIFEST_SCHEMA_ID.into(),
        schema_version: MANIFEST_SCHEMA_VERSION,
        package_hash: package_hash.clone(),
        entries,
        tail_part_hash,
    };

    Ok(EncodedSnapshot {
        package_hash,
        format_id: FORMAT_ID.into(),
        manifest: serde_json::to_value(manifest).map_err(|source| DotallError::Serialization {
            context: "XLSX snapshot manifest".into(),
            source,
        })?,
        parts,
    })
}

pub(crate) fn decode(encoded: &EncodedSnapshot) -> Result<Vec<u8>> {
    if encoded.format_id != FORMAT_ID {
        return Err(snapshot_error(format!(
            "cannot decode {} snapshot as XLSX",
            encoded.format_id
        )));
    }
    let manifest: SnapshotManifest =
        serde_json::from_value(encoded.manifest.clone()).map_err(|source| {
            DotallError::Serialization {
                context: "XLSX snapshot manifest".into(),
                source,
            }
        })?;
    if manifest.schema_id != MANIFEST_SCHEMA_ID
        || manifest.schema_version != MANIFEST_SCHEMA_VERSION
        || manifest.package_hash != encoded.package_hash
    {
        return Err(snapshot_error("unsupported or inconsistent snapshot manifest"));
    }

    let parts = encoded
        .parts
        .iter()
        .map(|part| (part.hash.as_str(), part.bytes.as_slice()))
        .collect::<BTreeMap<_, _>>();
    let mut package = Vec::new();
    for entry in &manifest.entries {
        append_part(&mut package, &parts, &entry.part_hash)?;
    }
    append_part(&mut package, &parts, &manifest.tail_part_hash)?;
    let actual_hash = blake3::hash(&package).to_hex().to_string();
    if actual_hash != encoded.package_hash {
        return Err(snapshot_error(format!(
            "reconstructed package hash {actual_hash} does not match {}",
            encoded.package_hash
        )));
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
    output: &mut Vec<u8>,
    parts: &BTreeMap<&str, &[u8]>,
    hash: &str,
) -> Result<()> {
    let bytes = parts
        .get(hash)
        .ok_or_else(|| DotallError::SnapshotMissing {
            path: "<xlsx snapshot>".into(),
            hash: hash.to_owned(),
        })?;
    if blake3::hash(bytes).to_hex().as_str() != hash {
        return Err(snapshot_error(format!(
            "snapshot part {hash} does not match its content hash"
        )));
    }
    output.extend_from_slice(bytes);
    Ok(())
}

fn snapshot_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<xlsx snapshot>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::io::{Cursor, Write};

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::{decode, encode};

    #[test]
    fn snapshot_parts_reconstruct_the_exact_zip_bytes() {
        let package = package(b"one", b"shared");

        let encoded = encode(&package).expect("encode package");
        let decoded = decode(&encoded).expect("decode package");

        assert_eq!(decoded, package);
        assert_eq!(
            blake3::hash(&decoded).to_hex().to_string(),
            encoded.package_hash
        );
        assert_eq!(encoded.manifest["schema_id"], "xlsx.snapshot-manifest");
        assert!(encoded.parts.len() > 1);
    }

    #[test]
    fn packages_with_one_changed_entry_share_local_record_parts() {
        let first = encode(&package(b"one", b"shared")).expect("encode first");
        let second = encode(&package(b"two", b"shared")).expect("encode second");
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
