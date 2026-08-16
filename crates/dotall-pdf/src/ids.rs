use blake3::Hasher;

const ID_HASH_LENGTH: usize = 24;

pub fn document_id(source_hash: &str, schema_version: u32) -> String {
    opaque_id("pdf", schema_version, &[source_hash])
}

pub fn page_id(number: u32, schema_version: u32) -> String {
    let number = number.to_string();
    opaque_id("pg", schema_version, &[&number])
}

pub fn field_id(name: &str, schema_version: u32) -> String {
    opaque_id("fl", schema_version, &[name])
}

fn opaque_id(prefix: &str, schema_version: u32, components: &[&str]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"pdf.document");
    hasher.update(&schema_version.to_le_bytes());
    for component in components {
        hasher.update(&(component.len() as u64).to_le_bytes());
        hasher.update(component.as_bytes());
    }
    let digest = hasher.finalize().to_hex();
    format!("{prefix}_{}", &digest[..ID_HASH_LENGTH])
}
