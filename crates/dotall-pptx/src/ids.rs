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

pub fn table_id(slide_name: &str, table_name: &str, schema_version: u32) -> String {
    opaque_id("tb", schema_version, &[slide_name, table_name])
}

pub fn table_cell_id(
    slide_name: &str,
    table_name: &str,
    row: u32,
    col: u32,
    schema_version: u32,
) -> String {
    let row = row.to_string();
    let col = col.to_string();
    opaque_id("tc", schema_version, &[slide_name, table_name, &row, &col])
}

pub fn comment_id(
    slide_name: &str,
    author: &str,
    text: &str,
    idx: u32,
    schema_version: u32,
) -> String {
    let idx = idx.to_string();
    opaque_id("cm", schema_version, &[slide_name, author, text, &idx])
}

pub fn chart_id(slide_name: &str, part_name: &str, schema_version: u32) -> String {
    opaque_id("ch", schema_version, &[slide_name, part_name])
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
