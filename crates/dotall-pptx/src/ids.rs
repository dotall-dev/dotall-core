use blake3::Hasher;

const ID_HASH_LENGTH: usize = 24;

pub fn presentation_id(source_hash: &str, schema_version: u32) -> String {
    opaque_id("pr", schema_version, &[source_hash])
}

pub fn slide_id(name: &str, index: u32, schema_version: u32) -> String {
    let index = index.to_string();
    opaque_id("sl", schema_version, &[name, &index])
}

pub fn shape_id(slide_name: &str, shape_name: &str, schema_version: u32) -> String {
    opaque_id("sp", schema_version, &[slide_name, shape_name])
}

fn opaque_id(prefix: &str, schema_version: u32, components: &[&str]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"pptx.presentation");
    hasher.update(&schema_version.to_le_bytes());
    for component in components {
        hasher.update(&(component.len() as u64).to_le_bytes());
        hasher.update(component.as_bytes());
    }
    let digest = hasher.finalize().to_hex();
    format!("{prefix}_{}", &digest[..ID_HASH_LENGTH])
}
