use std::io::{Cursor, Write};

use zip::ZipWriter;
use zip::write::SimpleFileOptions;

pub fn minimal_docx() -> Vec<u8> {
    minimal_docx_with_media(false)
}

pub fn minimal_docx_with_media(include_media: bool) -> Vec<u8> {
    package(DOCUMENT, include_media, None, None)
}

pub fn tracked_change_docx() -> Vec<u8> {
    package(DOCUMENT_WITH_INS, false, None, None)
}

/// Body paragraph plus a 1×2 table; includes header + media for surgical identity checks.
pub fn table_docx() -> Vec<u8> {
    package(DOCUMENT_WITH_TABLE, true, Some(HEADER), None)
}

/// Header + footer parts with a short body paragraph (footer edit smoke).
pub fn header_footer_docx() -> Vec<u8> {
    package(DOCUMENT_WITH_TABLE, false, Some(HEADER), Some(FOOTER))
}

/// Paragraph with two styled runs (bold then italic) for multi-run edit fidelity.
pub fn multi_run_docx() -> Vec<u8> {
    package(DOCUMENT_MULTI_RUN, false, None, None)
}

/// Multi-paragraph memo with heading + status table + confidential header. No media blob.
pub fn demo_memo_docx() -> Vec<u8> {
    package(DEMO_MEMO, false, Some(DEMO_HEADER), None)
}

/// Heavy Q3 memo: 24 decoy channel-mix paragraphs plus one live commission-rate line.
pub fn demo_q3_memo_docx() -> Vec<u8> {
    let mut body = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
    );
    for _ in 0..24 {
        body.push_str(r#"<w:p><w:r><w:t>Prior year channel mix stayed at 10%.</w:t></w:r></w:p>"#);
    }
    body.push_str(
        r#"<w:p><w:r><w:t>Q3 commission rate: 10%.</w:t></w:r></w:p></w:body></w:document>"#,
    );
    package(body.as_bytes(), false, None, None)
}

fn package(
    document: &[u8],
    include_media: bool,
    header: Option<&[u8]>,
    footer: Option<&[u8]>,
) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    let content_types = match (header.is_some(), footer.is_some()) {
        (true, true) => CONTENT_TYPES_WITH_HEADER_FOOTER,
        (true, false) => CONTENT_TYPES_WITH_HEADER,
        (false, true) => CONTENT_TYPES_WITH_FOOTER,
        (false, false) => CONTENT_TYPES,
    };
    add(&mut writer, "[Content_Types].xml", content_types, options);
    add(&mut writer, "_rels/.rels", ROOT_RELS, options);
    add(&mut writer, "word/document.xml", document, options);
    add(&mut writer, "word/styles.xml", STYLES, options);
    if header.is_some() || footer.is_some() {
        let rels = match (header.is_some(), footer.is_some()) {
            (true, true) => DOCUMENT_RELS_HEADER_FOOTER,
            (true, false) => DOCUMENT_RELS,
            (false, true) => DOCUMENT_RELS_FOOTER,
            (false, false) => unreachable!(),
        };
        add(&mut writer, "word/_rels/document.xml.rels", rels, options);
    }
    if let Some(header) = header {
        add(&mut writer, "word/header1.xml", header, options);
    }
    if let Some(footer) = footer {
        add(&mut writer, "word/footer1.xml", footer, options);
    }
    if include_media {
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let media = (0u8..=255).cycle().take(64 * 1024).collect::<Vec<_>>();
        add(&mut writer, "word/media/image1.bin", &media, stored);
    }
    writer.finish().expect("finish docx").into_inner()
}

fn add(
    writer: &mut ZipWriter<Cursor<Vec<u8>>>,
    name: &str,
    bytes: &[u8],
    options: SimpleFileOptions,
) {
    writer.start_file(name, options).expect("start");
    writer.write_all(bytes).expect("write");
}

const CONTENT_TYPES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;

const CONTENT_TYPES_WITH_HEADER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/></Types>"#;

const CONTENT_TYPES_WITH_FOOTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#;

const CONTENT_TYPES_WITH_HEADER_FOOTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#;

const ROOT_RELS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

const DOCUMENT_RELS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/></Relationships>"#;

const DOCUMENT_RELS_FOOTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/></Relationships>"#;

const DOCUMENT_RELS_HEADER_FOOTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/></Relationships>"#;

const STYLES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/></w:style></w:styles>"#;

const HEADER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>HeaderOnly</w:t></w:r></w:p></w:hdr>"#;

const DEMO_HEADER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>CONFIDENTIAL - Agent Pilot Memo</w:t></w:r></w:p></w:hdr>"#;

const FOOTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>FooterOnly</w:t></w:r></w:p></w:ftr>"#;

const DOCUMENT: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Alpha</w:t></w:r></w:p><w:p><w:r><w:t>Beta</w:t></w:r></w:p></w:body></w:document>"#;

const DOCUMENT_WITH_INS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Alpha</w:t></w:r></w:p><w:p><w:ins w:id="0"><w:r><w:t>Beta</w:t></w:r></w:ins></w:p></w:body></w:document>"#;

/// Intro body paragraph, then a 1×2 table (CellA | CellB) — three paragraphs in document order.
const DOCUMENT_WITH_TABLE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Intro</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>CellA</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>CellB</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;

/// Paragraph-mark rPr (should not steal run formatting) + bold run + italic run.
const DOCUMENT_MULTI_RUN: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:rPr><w:sz w:val="24"/></w:rPr></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>Bold</w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>Italic</w:t></w:r></w:p></w:body></w:document>"#;

/// Heading + multi-run body + body + 1×2 status table (indices 0..4 in document order).
/// Paragraph 1 has bold + italic runs for Wave 3 multi-run edit fidelity.
const DEMO_MEMO: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Internal Memo: Agent Pilot</w:t></w:r></w:p><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>We ran a two-week pilot</w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t> using Dotall for spreadsheet and document edits.</w:t></w:r></w:p><w:p><w:r><w:t>Please update the status table below before Friday standup.</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Owner</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Status</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>Platform</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>In progress</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;

#[cfg(test)]
mod q3_tests {
    use super::demo_q3_memo_docx;
    use std::io::{Cursor, Read};
    use zip::ZipArchive;

    #[test]
    fn q3_memo_contains_decoy_and_live_line() {
        let bytes = demo_q3_memo_docx();
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open docx zip");
        let mut document = archive.by_name("word/document.xml").expect("document.xml");
        let mut xml = String::new();
        document.read_to_string(&mut xml).expect("read document");
        let decoy = "Prior year channel mix stayed at 10%.";
        let live = "Q3 commission rate: 10%.";
        let decoy_count = xml.matches(decoy).count();
        assert!(
            decoy_count >= 24,
            "expected >=24 decoy paragraphs, got {decoy_count}"
        );
        assert!(xml.contains(live), "missing live Q3 commission line");
    }
}
