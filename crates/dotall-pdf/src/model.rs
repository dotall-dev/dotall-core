use serde::{Deserialize, Serialize};

pub const SCHEMA_ID: &str = "pdf.document";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PdfDocumentModel {
    pub document_id: String,
    pub page_count: u32,
    pub pages: Vec<PdfPageModel>,
    pub fields: Vec<PdfFieldModel>,
    #[serde(default)]
    pub comments: Vec<PdfCommentModel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<PdfPictureModel>,
    pub outline: Vec<String>,
    pub encrypted: bool,
    /// True when the catalog has `/Perms` (DocMDP) or an AcroForm signature field.
    #[serde(default)]
    pub signed: bool,
    #[serde(default)]
    pub metadata: PdfMetadata,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PdfCommentModel {
    pub element_id: String,
    pub page: u32,
    pub subtype: String,
    pub contents: String,
    #[serde(default)]
    pub author: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PdfPictureModel {
    pub element_id: String,
    pub page: u32,
    pub subtype: String,
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
    /// Page `/Rotate` when non-zero (0/absent omitted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<u32>,
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
    #[serde(default)]
    pub password: bool,
    /// AcroForm Comb flag (`/Ff` bit 25) for text fields.
    #[serde(default)]
    pub comb: bool,
    /// AcroForm DoNotScroll flag (`/Ff` bit 24) for text fields.
    #[serde(default)]
    pub do_not_scroll: bool,
    /// AcroForm DoNotSpellCheck flag (`/Ff` bit 23) for text fields.
    #[serde(default)]
    pub do_not_spell_check: bool,
    /// AcroForm RichText flag (`/Ff` bit 26) for text fields.
    #[serde(default)]
    pub rich_text: bool,
    /// AcroForm NoExport flag (`/Ff` bit 3) for any field type.
    #[serde(default)]
    pub no_export: bool,
    /// AcroForm MultiSelect flag (`/Ff` bit 20) for choice fields.
    #[serde(default)]
    pub multi_select: bool,
    /// AcroForm Combo flag (`/Ff` bit 17) for choice fields.
    #[serde(default)]
    pub combo: bool,
    /// AcroForm Edit flag (`/Ff` bit 18) for choice fields.
    #[serde(default)]
    pub edit: bool,
    /// AcroForm `/MaxLen` for text fields when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
}
