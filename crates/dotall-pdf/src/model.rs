use serde::{Deserialize, Serialize};

pub const SCHEMA_ID: &str = "pdf.document";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PdfDocumentModel {
    pub document_id: String,
    pub page_count: u32,
    pub pages: Vec<PdfPageModel>,
    pub fields: Vec<PdfFieldModel>,
    pub outline: Vec<String>,
    pub encrypted: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PdfPageModel {
    pub element_id: String,
    pub number: u32,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PdfFieldModel {
    pub element_id: String,
    pub name: String,
    pub field_type: String,
    pub value: String,
    pub page: Option<u32>,
    pub read_only: bool,
}
