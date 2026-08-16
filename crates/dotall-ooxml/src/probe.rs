/// Returns true when `prefix` looks like a ZIP local/end/span header.
pub fn has_zip_magic(prefix: &[u8]) -> bool {
    prefix.starts_with(b"PK\x03\x04")
        || prefix.starts_with(b"PK\x05\x06")
        || prefix.starts_with(b"PK\x07\x08")
}

/// Returns true when the ZIP central directory lists `name` (forward slashes).
pub fn zip_contains_entry(package: &[u8], name: &str) -> bool {
    let Ok(mut archive) = zip::ZipArchive::new(std::io::Cursor::new(package)) else {
        return false;
    };
    (0..archive.len()).any(|index| {
        archive
            .by_index(index)
            .map(|entry| entry.name() == name)
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::{has_zip_magic, zip_contains_entry};

    #[test]
    fn zip_magic_matches_local_file_header() {
        assert!(has_zip_magic(b"PK\x03\x04rest"));
        assert!(!has_zip_magic(b"%PDF-1.7"));
    }

    #[test]
    fn zip_contains_entry_finds_workbook_part() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("xl/workbook.xml", SimpleFileOptions::default())
            .expect("entry");
        writer.write_all(b"<workbook/>").expect("bytes");
        let package = writer.finish().expect("finish").into_inner();
        assert!(zip_contains_entry(&package, "xl/workbook.xml"));
        assert!(!zip_contains_entry(&package, "ppt/presentation.xml"));
    }
}
