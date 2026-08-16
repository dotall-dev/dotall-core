use dotall_core::{EncodedSnapshot, Result};

use crate::FORMAT_ID;

const MANIFEST_SCHEMA_ID: &str = "xlsx.snapshot-manifest";

pub(crate) fn encode(package: &[u8]) -> Result<EncodedSnapshot> {
    dotall_ooxml::encode_package(package, FORMAT_ID, MANIFEST_SCHEMA_ID, None)
}

pub(crate) fn decode(encoded: &EncodedSnapshot) -> Result<Vec<u8>> {
    if encoded.format_id != FORMAT_ID {
        return Err(dotall_core::DotallError::Format {
            format_id: FORMAT_ID.into(),
            path: "<xlsx snapshot>".into(),
            message: format!("cannot decode {} snapshot as XLSX", encoded.format_id),
        });
    }
    dotall_ooxml::decode_package(encoded)
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
