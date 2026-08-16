use blake3::Hasher;

const ID_HASH_LENGTH: usize = 24;

pub fn document_id(source_hash: &str, schema_version: u32) -> String {
    opaque_id("doc", schema_version, &[source_hash])
}

pub fn paragraph_id(index: u32, text: &str, schema_version: u32) -> String {
    let index = index.to_string();
    opaque_id("p", schema_version, &[&index, text])
}

fn opaque_id(prefix: &str, schema_version: u32, components: &[&str]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"docx.document");
    hasher.update(&schema_version.to_le_bytes());
    for component in components {
        hasher.update(&(component.len() as u64).to_le_bytes());
        hasher.update(component.as_bytes());
    }
    let digest = hasher.finalize().to_hex();
    format!("{prefix}_{}", &digest[..ID_HASH_LENGTH])
}
