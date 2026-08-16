use serde::{Deserialize, Serialize};

pub const SCHEMA_ID: &str = "pptx.presentation";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PresentationModel {
    pub presentation_id: String,
    pub slides: Vec<SlideModel>,
    #[serde(default)]
    pub media_parts: Vec<String>,
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
