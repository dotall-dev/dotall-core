use blake3::Hasher;

const ID_HASH_LENGTH: usize = 24;

/// Returns a deterministic opaque identifier for a workbook source hash.
pub fn workbook_id(source_hash: &str, schema_version: u32) -> String {
    opaque_id("wb", schema_version, &[source_hash])
}

/// Returns a deterministic opaque identifier for a worksheet.
pub fn sheet_id(sheet_name: &str, index: u32, schema_version: u32) -> String {
    let index = index.to_string();
    opaque_id("sh", schema_version, &[sheet_name, &index])
}

/// Returns a deterministic opaque identifier for a cell location.
pub fn cell_id(sheet_name: &str, address: &str, schema_version: u32) -> String {
    opaque_id("c", schema_version, &[sheet_name, address])
}

/// Returns a deterministic opaque identifier for a named range.
pub fn named_range_id(name: &str, formula: &str, schema_version: u32) -> String {
    opaque_id("nr", schema_version, &[name, formula])
}

/// Returns a deterministic opaque identifier for a style entry.
pub fn style_id(index: u32, schema_version: u32) -> String {
    let index = index.to_string();
    opaque_id("st", schema_version, &[&index])
}

/// Returns a deterministic opaque identifier for a legacy Excel comment/note.
pub fn comment_id(sheet_name: &str, cell: &str, schema_version: u32) -> String {
    opaque_id("cm", schema_version, &[sheet_name, cell])
}

/// Returns a deterministic opaque identifier for a chart part.
pub fn chart_id(part_path: &str, schema_version: u32) -> String {
    opaque_id("ch", schema_version, &[part_path])
}

/// Returns a deterministic opaque identifier for a worksheet picture.
pub fn picture_id(sheet_name: &str, media_path: &str, schema_version: u32) -> String {
    opaque_id("pic", schema_version, &[sheet_name, media_path])
}

fn opaque_id(prefix: &str, schema_version: u32, components: &[&str]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(b"xlsx.workbook");
    hasher.update(&schema_version.to_le_bytes());

    for component in components {
        hasher.update(&(component.len() as u64).to_le_bytes());
        hasher.update(component.as_bytes());
    }

    let digest = hasher.finalize().to_hex();
    format!("{prefix}_{}", &digest[..ID_HASH_LENGTH])
}
