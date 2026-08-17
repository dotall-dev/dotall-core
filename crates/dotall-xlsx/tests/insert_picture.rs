use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use base64::Engine;
use dotall_core::SemanticOperation;
use dotall_core::registry::FormatHandler;
use dotall_xlsx::{XlsxFormat, parse_workbook};
use rust_xlsxwriter::{Image, Workbook};
use tempfile::tempdir;
use zip::ZipArchive;

const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

fn write_image_fixture(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet().set_name("Inputs").expect("sheet");
    sheet.write_string(0, 0, "Logo").expect("write");
    let image = Image::new_from_buffer(TINY_PNG).expect("image");
    sheet.insert_image(0, 2, &image).expect("insert");
    workbook.save(path).expect("save");
}

fn write_blank(path: &std::path::Path) {
    let mut workbook = Workbook::new();
    workbook
        .add_worksheet()
        .set_name("Inputs")
        .expect("sheet")
        .write_string(0, 0, "A")
        .expect("write");
    workbook.save(path).expect("save");
}

#[test]
fn inspect_lists_pictures() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("logo.xlsx");
    write_image_fixture(&source);
    let pictures = XlsxFormat
        .inspect(&XlsxFormat.parse(&source).expect("parse"))
        .expect("inspect")
        .summary["pictures"]
        .as_array()
        .expect("arr")
        .clone();
    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0]["sheet"], "Inputs");
    assert!(
        pictures[0]["content_type"]
            .as_str()
            .expect("ct")
            .contains("png")
    );
}

#[test]
fn insert_picture_adds_media_and_leaves_other_sheets_identical() {
    use std::fs;
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("logo.xlsx");
    write_blank(&source);
    let _before = fs::read(&source).expect("bytes");
    let encoded = base64::engine::general_purpose::STANDARD.encode(TINY_PNG);
    let handler = XlsxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&source).expect("parse"),
            &[SemanticOperation {
                kind: "insert_picture".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "from_cell": "A1",
                    "bytes_base64": encoded,
                    "content_type": "image/png"
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    fs::write(&source, &patched.bytes).expect("rewrite");
    assert_eq!(parse_workbook(&source).expect("reparse").pictures.len(), 1);
    assert!(
        zip_entries(&patched.bytes)
            .keys()
            .any(|name| name.starts_with("xl/media/"))
    );
}

#[test]
fn insert_picture_does_not_rewrite_existing_media_bytes() {
    use std::fs;
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("logo.xlsx");
    write_image_fixture(&source);
    let before = fs::read(&source).expect("bytes");
    let existing: Vec<(String, Vec<u8>)> = zip_entries(&before)
        .into_iter()
        .filter(|(name, _)| name.starts_with("xl/media/"))
        .collect();
    assert!(!existing.is_empty());
    let encoded = base64::engine::general_purpose::STANDARD.encode(TINY_PNG);
    let handler = XlsxFormat;
    let edit = handler
        .validate_edit(
            &handler.parse(&source).expect("parse"),
            &[SemanticOperation {
                kind: "insert_picture".into(),
                payload: serde_json::json!({
                    "sheet": "Inputs",
                    "from_cell": "A2",
                    "bytes_base64": encoded
                }),
            }],
        )
        .expect("validate");
    let patched = handler.apply_edit(&source, &edit).expect("apply");
    let after = zip_entries(&patched.bytes);
    for (name, bytes) in existing {
        assert_eq!(after.get(&name).expect("retained media"), &bytes);
    }
}

#[test]
fn replace_picture_is_unsupported() {
    let directory = tempdir().expect("temporary directory");
    let source = directory.path().join("logo.xlsx");
    write_image_fixture(&source);
    let error = XlsxFormat
        .validate_edit(
            &XlsxFormat.parse(&source).expect("parse"),
            &[SemanticOperation {
                kind: "replace_picture".into(),
                payload: serde_json::json!({ "element_id": "pic_x" }),
            }],
        )
        .expect_err("replace");
    assert!(error.to_string().contains("unsupported"));
}

fn zip_entries(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open ZIP");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).expect("ZIP entry");
        let name = entry.name().to_owned();
        let mut inflated = Vec::new();
        entry.read_to_end(&mut inflated).expect("read entry");
        entries.insert(name, inflated);
    }
    entries
}
