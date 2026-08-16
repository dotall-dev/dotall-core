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
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ShapeModel {
    pub element_id: String,
    pub name: String,
    pub text: String,
}
