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
    #[serde(default)]
    pub metadata: PdfMetadata,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PdfMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
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
    /// Appearance-state names from `/AP /N` (e.g. `Yes`, `Off`) when discoverable.
    #[serde(default)]
    pub export_values: Vec<String>,
    /// Choice (`/Ch`) options from `/Opt` (export values agents can set).
    #[serde(default)]
    pub options: Vec<String>,
    pub page: Option<u32>,
    pub read_only: bool,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub multiline: bool,
}
