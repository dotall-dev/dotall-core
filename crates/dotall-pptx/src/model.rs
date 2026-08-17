use serde::{Deserialize, Serialize};

pub const SCHEMA_ID: &str = "pptx.presentation";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PresentationModel {
    pub presentation_id: String,
    pub slides: Vec<SlideModel>,
    #[serde(default)]
    pub media_parts: Vec<String>,
    #[serde(default)]
    pub comments: Vec<CommentModel>,
    #[serde(default)]
    pub charts: Vec<ChartModel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<PictureModel>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SlideModel {
    pub element_id: String,
    pub name: String,
    pub index: u32,
    pub part_name: String,
    pub shapes: Vec<ShapeModel>,
    #[serde(default)]
    pub tables: Vec<TableModel>,
    pub notes: Option<String>,
    /// Present when a notes slide part exists (even if notes text is empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_part_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ShapeModel {
    pub element_id: String,
    pub name: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TableModel {
    pub element_id: String,
    pub name: String,
    pub cells: Vec<TableCellModel>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TableCellModel {
    pub element_id: String,
    pub row: u32,
    pub col: u32,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommentModel {
    pub element_id: String,
    pub slide: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    pub author: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ChartModel {
    pub element_id: String,
    pub slide: String,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PictureModel {
    pub element_id: String,
    pub slide: String,
    pub name: String,
    pub part: String,
}
