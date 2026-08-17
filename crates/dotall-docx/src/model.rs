use serde::{Deserialize, Serialize};

pub const SCHEMA_ID: &str = "docx.document";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DocumentModel {
    pub document_id: String,
    pub paragraphs: Vec<ParagraphModel>,
    /// Paragraphs from `word/header*.xml`, sorted by part then document order within the part.
    #[serde(default)]
    pub header_paragraphs: Vec<HeaderFooterParagraphModel>,
    /// Paragraphs from `word/footer*.xml`, sorted by part then document order within the part.
    #[serde(default)]
    pub footer_paragraphs: Vec<HeaderFooterParagraphModel>,
    /// Always false when table cell paragraphs are included in `paragraphs`.
    #[serde(default)]
    pub skipped_tables: bool,
    /// Number of top-level `w:tbl` elements in `word/document.xml`.
    #[serde(default)]
    pub table_count: u32,
    /// Comments from `word/comments.xml`, anchored to body paragraphs.
    #[serde(default)]
    pub comments: Vec<CommentModel>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommentModel {
    pub element_id: String,
    /// Anchored paragraph `element_id`.
    pub paragraph: String,
    /// Document-order paragraph index.
    pub index: u32,
    pub author: String,
    pub text: String,
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
    /// True when the paragraph lives inside a `w:tbl` (table cell).
    #[serde(default)]
    pub in_table: bool,
}

/// Paragraph living in a header or footer story part (not the body).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HeaderFooterParagraphModel {
    pub element_id: String,
    /// Part stem, e.g. `header1` or `footer1`.
    pub part: String,
    /// Index within that part (0-based).
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
