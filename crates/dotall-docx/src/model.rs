use serde::{Deserialize, Serialize};

pub const SCHEMA_ID: &str = "docx.document";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DocumentModel {
    pub document_id: String,
    pub paragraphs: Vec<ParagraphModel>,
    /// Always false when table cell paragraphs are included in `paragraphs`.
    #[serde(default)]
    pub skipped_tables: bool,
    /// Number of top-level `w:tbl` elements in `word/document.xml`.
    #[serde(default)]
    pub table_count: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ParagraphModel {
    pub element_id: String,
    pub index: u32,
    pub outline_level: Option<u32>,
    pub style_id: Option<String>,
    pub text: String,
    #[serde(default = "editable_default")]
    pub editable: bool,
}

fn editable_default() -> bool {
    true
}
