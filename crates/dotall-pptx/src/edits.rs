use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Write};

use dotall_core::{
    DependencyImpact, DotallError, PatchedOutput, Result, SemanticChange, SemanticOperation,
    ValidatedEdit,
};
use zip::ZipArchive;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::FORMAT_ID;
use crate::model::{PresentationModel, SCHEMA_ID, SCHEMA_VERSION, TableCellModel, TableModel};
use crate::selector;

/// Blank slide template duplicated into new `ppt/slides/slideN.xml` parts.
const BLANK_SLIDE: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld></p:sld>"#;

struct PackagePatch {
    replacements: BTreeMap<String, Vec<u8>>,
    additions: BTreeMap<String, Vec<u8>>,
    removals: BTreeSet<String>,
}

pub fn validate(
    model: &PresentationModel,
    operations: &[SemanticOperation],
) -> Result<ValidatedEdit> {
    if operations.len() != 1 {
        return Err(format_error(
            "pptx supports a single edit operation per transaction",
        ));
    }
    let operation = &operations[0];
    match operation.kind.as_str() {
        "set_shape_text" => validate_set_shape_text(model, operation),
        "set_table_cell_text" => validate_set_table_cell_text(model, operation),
        "set_notes_text" => validate_set_notes_text(model, operation),
        "add_slide" => validate_add_slide(model, operation),
        "delete_slide" => validate_delete_slide(model, operation),
        "move_slide" => validate_move_slide(model, operation),
        "add_textbox" => validate_add_textbox(model, operation),
        "delete_shape" => validate_delete_shape(model, operation),
        "rename_shape" => validate_rename_shape(model, operation),
        "set_shape_bold" => validate_set_shape_bold(model, operation),
        "set_shape_italic" => validate_set_shape_italic(model, operation),
        "set_shape_underline" => validate_set_shape_underline(model, operation),
        other => Err(format_error(format!(
            "unsupported pptx edit `{other}`; use set_shape_text, set_table_cell_text, set_notes_text, set_shape_bold, set_shape_italic, set_shape_underline, add_slide, delete_slide, move_slide, add_textbox, delete_shape, or rename_shape"
        ))),
    }
}

fn validate_set_shape_text(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let slide_ref = required_str(&operation.payload, "slide")?;
    let shape_ref = required_str(&operation.payload, "shape")?;
    let text = required_str(&operation.payload, "text")?;
    let slide = selector::resolve_slide(model, slide_ref)
        .ok_or_else(|| format_error(format!("slide `{slide_ref}` was not found")))?;
    let shape = slide
        .shapes
        .iter()
        .find(|shape| shape.name == shape_ref || shape.element_id == shape_ref)
        .ok_or_else(|| format_error(format!("shape `{shape_ref}` was not found")))?;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_shape_text".into(),
            payload: serde_json::json!({
                "slide": slide.name,
                "shape": shape.name,
                "text": text,
                "part_name": slide.part_name,
                "element_id": shape.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{}!{}", slide.name, shape.name),
            element_id: shape.element_id.clone(),
            change: "set_shape_text".into(),
            before: Some(shape.text.clone()),
            after: Some(text.to_owned()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_table_cell_text(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let slide_ref = required_str(&operation.payload, "slide")?;
    let table_ref = table_ref(&operation.payload)?;
    let row = required_u32(&operation.payload, "row")?;
    let col = required_u32(&operation.payload, "col")?;
    let text = required_str(&operation.payload, "text")?;
    let slide = selector::resolve_slide(model, slide_ref)
        .ok_or_else(|| format_error(format!("slide `{slide_ref}` was not found")))?;
    let table = resolve_table(slide, table_ref)
        .ok_or_else(|| format_error(format!("table `{table_ref}` was not found")))?;
    let cell = resolve_cell(table, row, col).ok_or_else(|| {
        format_error(format!(
            "table cell row={row} col={col} is out of range for `{}`",
            table.name
        ))
    })?;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_table_cell_text".into(),
            payload: serde_json::json!({
                "slide": slide.name,
                "table": table.name,
                "row": row,
                "col": col,
                "text": text,
                "part_name": slide.part_name,
                "element_id": cell.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{}!{}!r{}c{}", slide.name, table.name, row, col),
            element_id: cell.element_id.clone(),
            change: "set_table_cell_text".into(),
            before: Some(cell.text.clone()),
            after: Some(text.to_owned()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_notes_text(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let slide_ref = required_str(&operation.payload, "slide")?;
    let text = required_str(&operation.payload, "text")?;
    let slide = selector::resolve_slide(model, slide_ref)
        .ok_or_else(|| format_error(format!("slide `{slide_ref}` was not found")))?;
    let notes_part_name = slide.notes_part_name.as_deref().ok_or_else(|| {
        format_error(format!(
            "slide `{}` has no notes slide part to edit",
            slide.name
        ))
    })?;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "set_notes_text".into(),
            payload: serde_json::json!({
                "slide": slide.name,
                "text": text,
                "part_name": notes_part_name,
                "element_id": slide.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{}!notes", slide.name),
            element_id: slide.element_id.clone(),
            change: "set_notes_text".into(),
            before: slide.notes.clone(),
            after: Some(text.to_owned()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_add_slide(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let after = optional_str(&operation.payload, "after")?;
    let insertion_index = match after {
        Some(after_ref) => {
            let slide = selector::resolve_slide(model, after_ref)
                .ok_or_else(|| format_error(format!("slide `{after_ref}` was not found")))?;
            (slide.index as usize) + 1
        }
        None => model.slides.len(),
    };
    let after_name = after
        .map(|value| {
            selector::resolve_slide(model, value)
                .map(|slide| slide.name.clone())
                .ok_or_else(|| format_error(format!("slide `{value}` was not found")))
        })
        .transpose()?;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "add_slide".into(),
            payload: serde_json::json!({
                "after": after_name,
                "insertion_index": insertion_index,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("Slide {}", insertion_index + 1),
            element_id: String::new(),
            change: "add_slide".into(),
            before: None,
            after: Some("blank".into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_set_shape_bold(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    validate_set_shape_run_bool(model, operation, "set_shape_bold", "bold")
}

fn validate_set_shape_italic(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    validate_set_shape_run_bool(model, operation, "set_shape_italic", "italic")
}

fn validate_set_shape_underline(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    validate_set_shape_run_bool(model, operation, "set_shape_underline", "underline")
}

fn validate_set_shape_run_bool(
    model: &PresentationModel,
    operation: &SemanticOperation,
    kind: &str,
    flag: &str,
) -> Result<ValidatedEdit> {
    let slide_ref = required_str(&operation.payload, "slide")?;
    let shape_ref = required_str(&operation.payload, "shape")?;
    let enabled = required_bool(&operation.payload, flag)?;
    let slide = selector::resolve_slide(model, slide_ref)
        .ok_or_else(|| format_error(format!("slide `{slide_ref}` was not found")))?;
    let shape = slide
        .shapes
        .iter()
        .find(|shape| shape.name == shape_ref || shape.element_id == shape_ref)
        .ok_or_else(|| format_error(format!("shape `{shape_ref}` was not found")))?;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: kind.into(),
            payload: serde_json::json!({
                "slide": slide.name,
                "shape": shape.name,
                flag: enabled,
                "part_name": slide.part_name,
                "element_id": shape.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{}!{}", slide.name, shape.name),
            element_id: shape.element_id.clone(),
            change: kind.into(),
            before: None,
            after: Some(if enabled { "true" } else { "false" }.into()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_delete_slide(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    if model.slides.len() <= 1 {
        return Err(format_error(
            "cannot delete the sole slide in a presentation",
        ));
    }
    let slide_ref = required_str(&operation.payload, "slide")?;
    let slide = selector::resolve_slide(model, slide_ref)
        .ok_or_else(|| format_error(format!("slide `{slide_ref}` was not found")))?;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "delete_slide".into(),
            payload: serde_json::json!({
                "slide": slide.name,
                "part_name": slide.part_name,
                "index": slide.index,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: slide.name.clone(),
            element_id: slide.element_id.clone(),
            change: "delete_slide".into(),
            before: Some(slide.name.clone()),
            after: None,
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_move_slide(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    if model.slides.len() <= 1 {
        return Err(format_error(
            "cannot move_slide in a presentation with fewer than two slides",
        ));
    }
    let slide_ref = required_str(&operation.payload, "slide")?;
    let to_index = required_usize(&operation.payload, "to_index")?;
    let slide = selector::resolve_slide(model, slide_ref)
        .ok_or_else(|| format_error(format!("slide `{slide_ref}` was not found")))?;
    let from_index = slide.index as usize;
    if to_index >= model.slides.len() {
        return Err(format_error(format!(
            "to_index {to_index} is out of range for {} slides",
            model.slides.len()
        )));
    }
    if to_index == from_index {
        return Err(format_error(format!(
            "slide `{}` is already at index {to_index}",
            slide.name
        )));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "move_slide".into(),
            payload: serde_json::json!({
                "slide": slide.name,
                "part_name": slide.part_name,
                "from_index": from_index,
                "to_index": to_index,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: slide.name.clone(),
            element_id: slide.element_id.clone(),
            change: "move_slide".into(),
            before: Some(from_index.to_string()),
            after: Some(to_index.to_string()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_delete_shape(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let slide_ref = required_str(&operation.payload, "slide")?;
    let shape_ref = required_str(&operation.payload, "shape")?;
    let slide = selector::resolve_slide(model, slide_ref)
        .ok_or_else(|| format_error(format!("slide `{slide_ref}` was not found")))?;
    let shape = slide
        .shapes
        .iter()
        .find(|shape| shape.name == shape_ref || shape.element_id == shape_ref)
        .ok_or_else(|| format_error(format!("shape `{shape_ref}` was not found")))?;
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "delete_shape".into(),
            payload: serde_json::json!({
                "slide": slide.name,
                "shape": shape.name,
                "part_name": slide.part_name,
                "element_id": shape.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{}!{}", slide.name, shape.name),
            element_id: shape.element_id.clone(),
            change: "delete_shape".into(),
            before: Some(shape.text.clone()),
            after: None,
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_rename_shape(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let slide_ref = required_str(&operation.payload, "slide")?;
    let shape_ref = required_str(&operation.payload, "shape")?;
    let new_name = required_str(&operation.payload, "name")?;
    if new_name.trim().is_empty() {
        return Err(format_error("`name` must be non-empty"));
    }
    let slide = selector::resolve_slide(model, slide_ref)
        .ok_or_else(|| format_error(format!("slide `{slide_ref}` was not found")))?;
    let shape = slide
        .shapes
        .iter()
        .find(|shape| shape.name == shape_ref || shape.element_id == shape_ref)
        .ok_or_else(|| format_error(format!("shape `{shape_ref}` was not found")))?;
    if slide.shapes.iter().any(|other| {
        other.element_id != shape.element_id
            && (other.name == new_name || other.element_id == new_name)
    }) || slide
        .tables
        .iter()
        .any(|table| table.name == new_name || table.element_id == new_name)
    {
        return Err(format_error(format!(
            "shape `{new_name}` already exists on `{}`",
            slide.name
        )));
    }
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "rename_shape".into(),
            payload: serde_json::json!({
                "slide": slide.name,
                "shape": shape.name,
                "name": new_name,
                "part_name": slide.part_name,
                "element_id": shape.element_id,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{}!{}", slide.name, shape.name),
            element_id: shape.element_id.clone(),
            change: "rename_shape".into(),
            before: Some(shape.name.clone()),
            after: Some(new_name.to_owned()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn validate_add_textbox(
    model: &PresentationModel,
    operation: &SemanticOperation,
) -> Result<ValidatedEdit> {
    let slide_ref = required_str(&operation.payload, "slide")?;
    let text = required_str(&operation.payload, "text")?;
    let slide = selector::resolve_slide(model, slide_ref)
        .ok_or_else(|| format_error(format!("slide `{slide_ref}` was not found")))?;
    let name = match optional_str(&operation.payload, "name")? {
        Some(name) => {
            if slide
                .shapes
                .iter()
                .any(|shape| shape.name == name || shape.element_id == name)
            {
                return Err(format_error(format!(
                    "shape `{name}` already exists on `{}`",
                    slide.name
                )));
            }
            name.to_owned()
        }
        None => {
            let mut n = 1u32;
            loop {
                let candidate = format!("TextBox {n}");
                if !slide.shapes.iter().any(|shape| shape.name == candidate) {
                    break candidate;
                }
                n = n.saturating_add(1);
            }
        }
    };
    Ok(ValidatedEdit {
        format_id: FORMAT_ID.into(),
        schema_id: SCHEMA_ID.into(),
        schema_version: SCHEMA_VERSION,
        operations: vec![SemanticOperation {
            kind: "add_textbox".into(),
            payload: serde_json::json!({
                "slide": slide.name,
                "name": name,
                "text": text,
                "part_name": slide.part_name,
            }),
        }],
        semantic_diff: vec![SemanticChange {
            target: format!("{}!{}", slide.name, name),
            element_id: String::new(),
            change: "add_textbox".into(),
            before: None,
            after: Some(text.to_owned()),
        }],
        dependency_impact: DependencyImpact {
            forward: Vec::new(),
            notes: Vec::new(),
        },
    })
}

fn resolve_table<'a>(
    slide: &'a crate::model::SlideModel,
    table_ref: &str,
) -> Option<&'a TableModel> {
    slide
        .tables
        .iter()
        .find(|table| table.name == table_ref || table.element_id == table_ref)
}

fn resolve_cell(table: &TableModel, row: u32, col: u32) -> Option<&TableCellModel> {
    table
        .cells
        .iter()
        .find(|cell| cell.row == row && cell.col == col)
}

pub fn apply_bytes(package: &[u8], edit: &ValidatedEdit) -> Result<PatchedOutput> {
    let operation = edit
        .operations
        .first()
        .ok_or_else(|| format_error("validated edit is missing operations"))?;
    let patch = match operation.kind.as_str() {
        "set_shape_text" => {
            let part_name = required_str(&operation.payload, "part_name")?;
            let shape = required_str(&operation.payload, "shape")?;
            let text = required_str(&operation.payload, "text")?;
            let original = entry_bytes(package, part_name)?;
            PackagePatch {
                replacements: BTreeMap::from([(
                    part_name.to_owned(),
                    patch_shape_text(&original, shape, text)?,
                )]),
                additions: BTreeMap::new(),
                removals: BTreeSet::new(),
            }
        }
        "set_table_cell_text" => {
            let part_name = required_str(&operation.payload, "part_name")?;
            let table = required_str(&operation.payload, "table")?;
            let row = required_u32(&operation.payload, "row")?;
            let col = required_u32(&operation.payload, "col")?;
            let text = required_str(&operation.payload, "text")?;
            let original = entry_bytes(package, part_name)?;
            PackagePatch {
                replacements: BTreeMap::from([(
                    part_name.to_owned(),
                    patch_table_cell_text(&original, table, row, col, text)?,
                )]),
                additions: BTreeMap::new(),
                removals: BTreeSet::new(),
            }
        }
        "set_notes_text" => {
            let part_name = required_str(&operation.payload, "part_name")?;
            let text = required_str(&operation.payload, "text")?;
            let original = entry_bytes(package, part_name)?;
            PackagePatch {
                replacements: BTreeMap::from([(
                    part_name.to_owned(),
                    patch_notes_text(&original, text)?,
                )]),
                additions: BTreeMap::new(),
                removals: BTreeSet::new(),
            }
        }
        "add_slide" => {
            let insertion_index = required_usize(&operation.payload, "insertion_index")?;
            add_slide_patch(package, insertion_index)?
        }
        "delete_slide" => {
            let part_name = required_str(&operation.payload, "part_name")?;
            delete_slide_patch(package, part_name)?
        }
        "move_slide" => {
            let from_index = required_usize(&operation.payload, "from_index")?;
            let to_index = required_usize(&operation.payload, "to_index")?;
            move_slide_patch(package, from_index, to_index)?
        }
        "add_textbox" => {
            let part_name = required_str(&operation.payload, "part_name")?;
            let name = required_str(&operation.payload, "name")?;
            let text = required_str(&operation.payload, "text")?;
            let original = entry_bytes(package, part_name)?;
            PackagePatch {
                replacements: BTreeMap::from([(
                    part_name.to_owned(),
                    patch_add_textbox(&original, name, text)?,
                )]),
                additions: BTreeMap::new(),
                removals: BTreeSet::new(),
            }
        }
        "delete_shape" => {
            let part_name = required_str(&operation.payload, "part_name")?;
            let shape = required_str(&operation.payload, "shape")?;
            let original = entry_bytes(package, part_name)?;
            PackagePatch {
                replacements: BTreeMap::from([(
                    part_name.to_owned(),
                    patch_delete_shape(&original, shape)?,
                )]),
                additions: BTreeMap::new(),
                removals: BTreeSet::new(),
            }
        }
        "rename_shape" => {
            let part_name = required_str(&operation.payload, "part_name")?;
            let shape = required_str(&operation.payload, "shape")?;
            let name = required_str(&operation.payload, "name")?;
            let original = entry_bytes(package, part_name)?;
            PackagePatch {
                replacements: BTreeMap::from([(
                    part_name.to_owned(),
                    patch_rename_shape(&original, shape, name)?,
                )]),
                additions: BTreeMap::new(),
                removals: BTreeSet::new(),
            }
        }
        "set_shape_bold" => {
            let part_name = required_str(&operation.payload, "part_name")?;
            let shape = required_str(&operation.payload, "shape")?;
            let bold = operation
                .payload
                .get("bold")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`bold` boolean is required"))?;
            let original = entry_bytes(package, part_name)?;
            PackagePatch {
                replacements: BTreeMap::from([(
                    part_name.to_owned(),
                    patch_shape_run_bool(&original, shape, "b", bold)?,
                )]),
                additions: BTreeMap::new(),
                removals: BTreeSet::new(),
            }
        }
        "set_shape_italic" => {
            let part_name = required_str(&operation.payload, "part_name")?;
            let shape = required_str(&operation.payload, "shape")?;
            let italic = operation
                .payload
                .get("italic")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`italic` boolean is required"))?;
            let original = entry_bytes(package, part_name)?;
            PackagePatch {
                replacements: BTreeMap::from([(
                    part_name.to_owned(),
                    patch_shape_run_bool(&original, shape, "i", italic)?,
                )]),
                additions: BTreeMap::new(),
                removals: BTreeSet::new(),
            }
        }
        "set_shape_underline" => {
            let part_name = required_str(&operation.payload, "part_name")?;
            let shape = required_str(&operation.payload, "shape")?;
            let underline = operation
                .payload
                .get("underline")
                .and_then(serde_json::Value::as_bool)
                .ok_or_else(|| format_error("`underline` boolean is required"))?;
            let original = entry_bytes(package, part_name)?;
            PackagePatch {
                replacements: BTreeMap::from([(
                    part_name.to_owned(),
                    patch_shape_underline(&original, shape, underline)?,
                )]),
                additions: BTreeMap::new(),
                removals: BTreeSet::new(),
            }
        }
        other => {
            return Err(format_error(format!(
                "cannot apply unsupported pptx edit `{other}`"
            )));
        }
    };
    let bytes = rebuild_package(
        package,
        &patch.replacements,
        &patch.removals,
        &patch.additions,
    )?;
    Ok(PatchedOutput {
        after_source_hash: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
    })
}

fn add_slide_patch(package: &[u8], insertion_index: usize) -> Result<PackagePatch> {
    let presentation = entry_bytes(package, "ppt/presentation.xml")?;
    let relationships = entry_bytes(package, "ppt/_rels/presentation.xml.rels")?;
    let content_types = entry_bytes(package, "[Content_Types].xml")?;
    let slide_ids = sld_id_numbers(&presentation)?;
    let relationship_ids = relationship_id_numbers(&relationships)?;
    let part_numbers = slide_part_numbers(package)?;
    let sld_id = slide_ids
        .iter()
        .copied()
        .max()
        .unwrap_or(255)
        .saturating_add(1);
    let relationship_id = format!("rId{}", lowest_unused_number(&relationship_ids));
    let part_number = lowest_unused_number(&part_numbers);
    let part_name = format!("ppt/slides/slide{part_number}.xml");
    let target = format!("slides/slide{part_number}.xml");
    let sld_count = count_sld_ids(&presentation)?;
    if insertion_index > sld_count {
        return Err(format_error(format!(
            "insertion_index {insertion_index} is out of range for {sld_count} slides"
        )));
    }

    let mut replacements = BTreeMap::new();
    replacements.insert(
        "ppt/presentation.xml".into(),
        insert_sld_id(
            &presentation,
            insertion_index,
            &format!(r#"<p:sldId id="{sld_id}" r:id="{relationship_id}"/>"#),
        )?,
    );
    replacements.insert(
        "ppt/_rels/presentation.xml.rels".into(),
        insert_before_close(
            &relationships,
            "Relationships",
            &format!(
                r#"<Relationship Id="{relationship_id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="{target}"/>"#
            ),
        )?,
    );
    replacements.insert(
        "[Content_Types].xml".into(),
        insert_before_close(
            &content_types,
            "Types",
            &format!(
                r#"<Override PartName="/{part_name}" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#
            ),
        )?,
    );

    Ok(PackagePatch {
        replacements,
        additions: BTreeMap::from([(part_name, BLANK_SLIDE.to_vec())]),
        removals: BTreeSet::new(),
    })
}

fn move_slide_patch(package: &[u8], from_index: usize, to_index: usize) -> Result<PackagePatch> {
    let presentation = entry_bytes(package, "ppt/presentation.xml")?;
    let patched = reorder_sld_id(&presentation, from_index, to_index)?;
    Ok(PackagePatch {
        replacements: BTreeMap::from([("ppt/presentation.xml".into(), patched)]),
        additions: BTreeMap::new(),
        removals: BTreeSet::new(),
    })
}

fn delete_slide_patch(package: &[u8], part_name: &str) -> Result<PackagePatch> {
    let presentation = entry_bytes(package, "ppt/presentation.xml")?;
    let relationships = entry_bytes(package, "ppt/_rels/presentation.xml.rels")?;
    let content_types = entry_bytes(package, "[Content_Types].xml")?;
    let targets = relationship_targets(&relationships)?;
    let rid = targets
        .iter()
        .find(|(_, target)| resolve_ppt_target(target) == part_name)
        .map(|(id, _)| id.clone())
        .ok_or_else(|| {
            format_error(format!(
                "presentation relationship for `{part_name}` is missing"
            ))
        })?;
    if count_sld_ids(&presentation)? <= 1 {
        return Err(format_error(
            "cannot delete the sole slide in a presentation",
        ));
    }

    let mut removals = BTreeSet::from([part_name.to_owned()]);
    let slide_rels = slide_relationship_part(part_name);
    if has_entry(package, &slide_rels)? {
        let slide_rels_xml = entry_bytes(package, &slide_rels)?;
        for notes in notes_parts_from_slide_rels(&slide_rels_xml) {
            let notes_rels = relationship_part_name(&notes);
            if has_entry(package, &notes_rels)? {
                removals.insert(notes_rels);
            }
            removals.insert(notes);
        }
        removals.insert(slide_rels);
    }
    // Heuristic notes path used by our parser when no slide rels exist.
    if let Some(number) = part_name
        .strip_prefix("ppt/slides/slide")
        .and_then(|name| name.strip_suffix(".xml"))
    {
        let notes = format!("ppt/notesSlides/notesSlide{number}.xml");
        if has_entry(package, &notes)? {
            removals.insert(notes.clone());
            let notes_rels = relationship_part_name(&notes);
            if has_entry(package, &notes_rels)? {
                removals.insert(notes_rels);
            }
        }
    }

    let mut replacements = BTreeMap::new();
    replacements.insert(
        "ppt/presentation.xml".into(),
        remove_sld_id_for_rid(&presentation, &rid)?,
    );
    replacements.insert(
        "ppt/_rels/presentation.xml.rels".into(),
        remove_relationship(&relationships, &rid)?,
    );
    replacements.insert(
        "[Content_Types].xml".into(),
        remove_content_type_overrides(&content_types, &removals)?,
    );

    Ok(PackagePatch {
        replacements,
        additions: BTreeMap::new(),
        removals,
    })
}

pub fn patch_delete_shape(xml: &[u8], shape: &str) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("slide XML is not UTF-8: {error}")))?;
    let needle = format!("name=\"{shape}\"");
    let name_at = source
        .find(&needle)
        .ok_or_else(|| format_error(format!("shape `{shape}` was not found in slide XML")))?;
    let sp_start = source[..name_at]
        .rfind("<p:sp")
        .ok_or_else(|| format_error("shape is missing a p:sp wrapper"))?;
    let after_open = &source[sp_start + "<p:sp".len()..];
    if after_open.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Err(format_error("shape name matched a non-p:sp element"));
    }
    let rel_end = source[name_at..]
        .find("</p:sp>")
        .ok_or_else(|| format_error("shape is missing a closing p:sp"))?;
    let sp_end = name_at + rel_end + "</p:sp>".len();
    Ok(format!("{}{}", &source[..sp_start], &source[sp_end..]).into_bytes())
}

pub fn patch_rename_shape(xml: &[u8], shape: &str, new_name: &str) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("slide XML is not UTF-8: {error}")))?;
    let needle = format!("name=\"{shape}\"");
    let name_at = source
        .find(&needle)
        .ok_or_else(|| format_error(format!("shape `{shape}` was not found in slide XML")))?;
    let value_start = name_at + "name=\"".len();
    let value_end = value_start + shape.len();
    if &source[value_start..value_end] != shape {
        return Err(format_error(format!(
            "shape `{shape}` name attribute mismatch"
        )));
    }
    Ok(format!(
        "{}{}{}",
        &source[..value_start],
        xml_escape(new_name),
        &source[value_end..]
    )
    .into_bytes())
}

pub fn patch_add_textbox(xml: &[u8], name: &str, text: &str) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("slide XML is not UTF-8: {error}")))?;
    let close = "</p:spTree>";
    let position = source
        .rfind(close)
        .ok_or_else(|| format_error("slide is missing closing p:spTree"))?;
    let shape_id = next_c_nv_pr_id(source);
    let shape = format!(
        concat!(
            r#"<p:sp>"#,
            r#"<p:nvSpPr>"#,
            r#"<p:cNvPr id="{id}" name="{name}"/>"#,
            r#"<p:cNvSpPr txBox="1"/>"#,
            r#"<p:nvPr/>"#,
            r#"</p:nvSpPr>"#,
            r#"<p:spPr>"#,
            r#"<a:xfrm>"#,
            r#"<a:off x="457200" y="4572000"/>"#,
            r#"<a:ext cx="8229600" cy="914400"/>"#,
            r#"</a:xfrm>"#,
            r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>"#,
            r#"<a:noFill/>"#,
            r#"<a:ln><a:noFill/></a:ln>"#,
            r#"</p:spPr>"#,
            r#"<p:txBody>"#,
            r#"<a:bodyPr wrap="square"/>"#,
            r#"<a:lstStyle/>"#,
            r#"<a:p><a:r><a:t>{text}</a:t></a:r></a:p>"#,
            r#"</p:txBody>"#,
            r#"</p:sp>"#
        ),
        id = shape_id,
        name = xml_escape(name),
        text = xml_escape(text)
    );
    Ok(format!("{}{}{}", &source[..position], shape, &source[position..]).into_bytes())
}

fn next_c_nv_pr_id(source: &str) -> u32 {
    let mut max_id = 1u32;
    let mut cursor = 0;
    while let Some(rel) = source[cursor..].find("id=\"") {
        let start = cursor + rel + 4;
        if let Some(end_rel) = source[start..].find('"') {
            let value = &source[start..start + end_rel];
            if let Ok(id) = value.parse::<u32>() {
                max_id = max_id.max(id);
            }
            cursor = start + end_rel + 1;
        } else {
            break;
        }
    }
    max_id.saturating_add(1)
}

pub fn patch_shape_text(xml: &[u8], shape: &str, text: &str) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("slide XML is not UTF-8: {error}")))?;
    let needle = format!("name=\"{shape}\"");
    let name_at = source
        .find(&needle)
        .ok_or_else(|| format_error(format!("shape `{shape}` was not found in slide XML")))?;
    let sp_start = source[..name_at]
        .rfind("<p:sp")
        .ok_or_else(|| format_error("shape is missing a p:sp wrapper"))?;
    let rel_end = source[name_at..]
        .find("</p:sp>")
        .ok_or_else(|| format_error("shape is missing a closing p:sp"))?;
    let sp_end = name_at + rel_end + "</p:sp>".len();
    let sp = &source[sp_start..sp_end];
    if sp.contains("<p:graphicFrame")
        || sp.contains("<a:graphic")
        || sp.contains("<mc:AlternateContent")
    {
        return Err(format_error("cannot edit non-text shape"));
    }
    let Some(t_rel) = find_tag(sp, 0, "a:t") else {
        return Err(format_error("shape has no text run to patch"));
    };
    let t_open_end = sp[t_rel..]
        .find('>')
        .map(|offset| t_rel + offset + 1)
        .ok_or_else(|| format_error("unterminated a:t"))?;
    let t_close_rel = sp[t_open_end..]
        .find("</a:t>")
        .ok_or_else(|| format_error("unterminated a:t content"))?;
    let t_close = t_open_end + t_close_rel;
    let mut patched_sp = String::new();
    patched_sp.push_str(&sp[..t_open_end]);
    patched_sp.push_str(&xml_escape(text));
    patched_sp.push_str(&sp[t_close..]);
    // Drop extra a:t contents after the first run.
    patched_sp = clear_later_text_runs(&patched_sp);
    let mut output = String::new();
    output.push_str(&source[..sp_start]);
    output.push_str(&patched_sp);
    output.push_str(&source[sp_end..]);
    Ok(output.into_bytes())
}

pub fn patch_shape_bold(xml: &[u8], shape: &str, bold: bool) -> Result<Vec<u8>> {
    patch_shape_run_bool(xml, shape, "b", bold)
}

pub fn patch_shape_italic(xml: &[u8], shape: &str, italic: bool) -> Result<Vec<u8>> {
    patch_shape_run_bool(xml, shape, "i", italic)
}

pub fn patch_shape_underline(xml: &[u8], shape: &str, underline: bool) -> Result<Vec<u8>> {
    let value = if underline { "sng" } else { "none" };
    patch_shape_run_attr(xml, shape, "u", value)
}

fn patch_shape_run_bool(xml: &[u8], shape: &str, attr: &str, enabled: bool) -> Result<Vec<u8>> {
    let value = if enabled { "1" } else { "0" };
    patch_shape_run_attr(xml, shape, attr, value)
}

fn patch_shape_run_attr(xml: &[u8], shape: &str, attr: &str, value: &str) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("slide XML is not UTF-8: {error}")))?;
    let needle = format!("name=\"{shape}\"");
    let name_at = source
        .find(&needle)
        .ok_or_else(|| format_error(format!("shape `{shape}` was not found in slide XML")))?;
    let sp_start = source[..name_at]
        .rfind("<p:sp")
        .ok_or_else(|| format_error("shape is missing a p:sp wrapper"))?;
    let rel_end = source[name_at..]
        .find("</p:sp>")
        .ok_or_else(|| format_error("shape is missing a closing p:sp"))?;
    let sp_end = name_at + rel_end + "</p:sp>".len();
    let sp = &source[sp_start..sp_end];
    if sp.contains("<p:graphicFrame")
        || sp.contains("<a:graphic")
        || sp.contains("<mc:AlternateContent")
    {
        return Err(format_error("cannot edit non-text shape"));
    }
    let patched_sp = set_shape_runs_attr(sp, attr, value)?;
    let mut output = String::new();
    output.push_str(&source[..sp_start]);
    output.push_str(&patched_sp);
    output.push_str(&source[sp_end..]);
    Ok(output.into_bytes())
}

fn set_shape_runs_attr(sp: &str, attr: &str, value: &str) -> Result<String> {
    let mut output = String::with_capacity(sp.len() + 32);
    let mut cursor = 0;
    let mut patched_any = false;
    while let Some(rel) = sp[cursor..].find("<a:r") {
        let start = cursor + rel;
        let after = sp.as_bytes().get(start + 4).copied().unwrap_or(0);
        if after != b' ' && after != b'>' && after != b'/' {
            output.push_str(&sp[cursor..start + 4]);
            cursor = start + 4;
            continue;
        }
        let end = element_end_drawing(sp, start, "a:r")?;
        output.push_str(&sp[cursor..start]);
        output.push_str(&upsert_drawing_run_attr(&sp[start..end], attr, value)?);
        cursor = end;
        patched_any = true;
    }
    output.push_str(&sp[cursor..]);
    if !patched_any {
        return Err(format_error(format!(
            "shape has no text run to set `{attr}`"
        )));
    }
    Ok(output)
}

fn upsert_drawing_run_attr(run: &str, attr: &str, value: &str) -> Result<String> {
    let open_end = run
        .find('>')
        .map(|offset| offset + 1)
        .ok_or_else(|| format_error("unterminated a:r"))?;
    if run[..open_end].ends_with("/>") {
        return Ok(run.to_owned());
    }
    let rest = &run[open_end..];
    if let Some(r_pr_start) = find_tag(rest, 0, "a:rPr") {
        let r_pr_end = element_end_drawing(rest, r_pr_start, "a:rPr")?;
        let patched = set_rpr_attr(&rest[r_pr_start..r_pr_end], attr, value)?;
        return Ok(format!(
            "{}{}{}{}",
            &run[..open_end],
            &rest[..r_pr_start],
            patched,
            &rest[r_pr_end..]
        ));
    }
    let r_pr = format!(r#"<a:rPr {attr}="{value}"/>"#);
    Ok(format!("{}{}{}", &run[..open_end], r_pr, rest))
}

fn set_rpr_attr(r_pr: &str, attr: &str, value: &str) -> Result<String> {
    let needle = format!("{attr}=\"");
    if let Some(rel) = r_pr.find(&needle) {
        let value_start = rel + needle.len();
        let value_end = r_pr[value_start..]
            .find('"')
            .map(|offset| value_start + offset)
            .ok_or_else(|| format_error(format!("unterminated {attr} attribute")))?;
        return Ok(format!(
            "{}{}{}",
            &r_pr[..value_start],
            value,
            &r_pr[value_end..]
        ));
    }
    // Insert attr= before the tag close.
    if r_pr.ends_with("/>") {
        let open = r_pr.trim_end_matches("/>").trim_end();
        return Ok(format!("{open} {attr}=\"{value}\"/>"));
    }
    if let Some(gt) = r_pr.find('>') {
        let open = r_pr[..gt].trim_end();
        return Ok(format!("{open} {attr}=\"{value}\">{}", &r_pr[gt + 1..]));
    }
    Err(format_error("unterminated a:rPr"))
}

fn element_end_drawing(xml: &str, start: usize, local: &str) -> Result<usize> {
    let open = &xml[start..];
    let open_end = open
        .find('>')
        .map(|offset| start + offset + 1)
        .ok_or_else(|| format_error(format!("unterminated <{local}")))?;
    if xml[start..open_end].ends_with("/>") {
        return Ok(open_end);
    }
    let close = format!("</{local}>");
    xml[open_end..]
        .find(&close)
        .map(|offset| open_end + offset + close.len())
        .ok_or_else(|| format_error(format!("missing closing </{local}>")))
}

pub fn patch_table_cell_text(
    xml: &[u8],
    table: &str,
    row: u32,
    col: u32,
    text: &str,
) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("slide XML is not UTF-8: {error}")))?;
    let needle = format!("name=\"{table}\"");
    let name_at = source
        .find(&needle)
        .ok_or_else(|| format_error(format!("table `{table}` was not found in slide XML")))?;
    let frame_start = source[..name_at]
        .rfind("<p:graphicFrame")
        .ok_or_else(|| format_error("table is missing a p:graphicFrame wrapper"))?;
    let rel_end = source[name_at..]
        .find("</p:graphicFrame>")
        .ok_or_else(|| format_error("table is missing a closing p:graphicFrame"))?;
    let frame_end = name_at + rel_end + "</p:graphicFrame>".len();
    let frame = &source[frame_start..frame_end];
    let tbl_rel = find_tag(frame, 0, "a:tbl")
        .ok_or_else(|| format_error(format!("graphicFrame `{table}` has no a:tbl")))?;
    let tbl_close_rel = frame[tbl_rel..]
        .find("</a:tbl>")
        .ok_or_else(|| format_error("unterminated a:tbl"))?;
    let tbl_end = tbl_rel + tbl_close_rel + "</a:tbl>".len();
    let tbl = &frame[tbl_rel..tbl_end];
    let cell = nth_table_cell(tbl, row, col)?;
    let Some(t_rel) = find_tag(cell, 0, "a:t") else {
        return Err(format_error("table cell has no text run to patch"));
    };
    let t_open_end = cell[t_rel..]
        .find('>')
        .map(|offset| t_rel + offset + 1)
        .ok_or_else(|| format_error("unterminated a:t"))?;
    let t_close_rel = cell[t_open_end..]
        .find("</a:t>")
        .ok_or_else(|| format_error("unterminated a:t content"))?;
    let t_close = t_open_end + t_close_rel;
    let mut patched_cell = String::new();
    patched_cell.push_str(&cell[..t_open_end]);
    patched_cell.push_str(&xml_escape(text));
    patched_cell.push_str(&cell[t_close..]);
    patched_cell = clear_later_text_runs(&patched_cell);

    let cell_abs_start = frame_start + tbl_rel + cell_offset_in_tbl(tbl, row, col)?;
    let cell_abs_end = cell_abs_start + cell.len();
    let mut output = String::new();
    output.push_str(&source[..cell_abs_start]);
    output.push_str(&patched_cell);
    output.push_str(&source[cell_abs_end..]);
    Ok(output.into_bytes())
}

/// Replace the first `a:t` in a notes slide part and clear later text runs.
pub fn patch_notes_text(xml: &[u8], text: &str) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("notes XML is not UTF-8: {error}")))?;
    let Some(t_rel) = find_tag(source, 0, "a:t") else {
        return Err(format_error("notes slide has no text run to patch"));
    };
    let t_open_end = source[t_rel..]
        .find('>')
        .map(|offset| t_rel + offset + 1)
        .ok_or_else(|| format_error("unterminated a:t"))?;
    let t_close_rel = source[t_open_end..]
        .find("</a:t>")
        .ok_or_else(|| format_error("unterminated a:t content"))?;
    let t_close = t_open_end + t_close_rel;
    let mut patched = String::new();
    patched.push_str(&source[..t_open_end]);
    patched.push_str(&xml_escape(text));
    patched.push_str(&source[t_close..]);
    patched = clear_later_text_runs(&patched);
    Ok(patched.into_bytes())
}

fn nth_table_cell(tbl: &str, row: u32, col: u32) -> Result<&str> {
    let (start, end) = cell_span(tbl, row, col)?;
    Ok(&tbl[start..end])
}

fn cell_offset_in_tbl(tbl: &str, row: u32, col: u32) -> Result<usize> {
    let (start, _) = cell_span(tbl, row, col)?;
    Ok(start)
}

fn cell_span(tbl: &str, row: u32, col: u32) -> Result<(usize, usize)> {
    let mut cursor = 0usize;
    let mut current_row = 0u32;
    while let Some(tr_start) = find_tag(tbl, cursor, "a:tr") {
        let open_end = tbl[tr_start..]
            .find('>')
            .map(|offset| tr_start + offset + 1)
            .ok_or_else(|| format_error("unterminated a:tr"))?;
        let close_rel = tbl[open_end..]
            .find("</a:tr>")
            .ok_or_else(|| format_error("unterminated a:tr content"))?;
        let tr_end = open_end + close_rel + "</a:tr>".len();
        if current_row == row {
            let tr = &tbl[tr_start..tr_end];
            let (cell_start, cell_end) = nth_cell_in_row(tr, col)?;
            return Ok((tr_start + cell_start, tr_start + cell_end));
        }
        current_row += 1;
        cursor = tr_end;
    }
    Err(format_error(format!(
        "table cell row={row} col={col} is out of range"
    )))
}

fn nth_cell_in_row(tr: &str, col: u32) -> Result<(usize, usize)> {
    let mut cursor = 0usize;
    let mut current_col = 0u32;
    while let Some(tc_start) = find_tag(tr, cursor, "a:tc") {
        let open_end = tr[tc_start..]
            .find('>')
            .map(|offset| tc_start + offset + 1)
            .ok_or_else(|| format_error("unterminated a:tc"))?;
        let close_rel = tr[open_end..]
            .find("</a:tc>")
            .ok_or_else(|| format_error("unterminated a:tc content"))?;
        let tc_end = open_end + close_rel + "</a:tc>".len();
        if current_col == col {
            return Ok((tc_start, tc_end));
        }
        current_col += 1;
        cursor = tc_end;
    }
    Err(format_error(format!(
        "table cell row col={col} is out of range"
    )))
}

fn clear_later_text_runs(fragment: &str) -> String {
    let Some(first) = find_tag(fragment, 0, "a:t") else {
        return fragment.to_owned();
    };
    let Some(first_close_rel) = fragment[first..].find("</a:t>") else {
        return fragment.to_owned();
    };
    let mut cursor = first + first_close_rel + "</a:t>".len();
    let mut output = fragment[..cursor].to_owned();
    while let Some(start) = find_tag(fragment, cursor, "a:t") {
        let Some(open_end_rel) = fragment[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel + 1;
        let Some(close_rel) = fragment[open_end..].find("</a:t>") else {
            break;
        };
        output.push_str(&fragment[cursor..open_end]);
        output.push_str(&fragment[open_end + close_rel..open_end + close_rel + "</a:t>".len()]);
        cursor = open_end + close_rel + "</a:t>".len();
    }
    output.push_str(&fragment[cursor..]);
    output
}

/// Find `<local` / `<local …>` without matching longer names (e.g. `a:t` vs `a:tc`).
fn find_tag(haystack: &str, start: usize, local: &str) -> Option<usize> {
    let needle = format!("<{local}");
    let mut cursor = start;
    while let Some(rel) = haystack[cursor..].find(&needle) {
        let at = cursor + rel;
        let next = *haystack.as_bytes().get(at + needle.len())?;
        if next == b'>' || next == b' ' || next == b'/' {
            return Some(at);
        }
        cursor = at + needle.len();
    }
    None
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn entry_bytes(package: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| format_error(format!("invalid PPTX package: {error}")))?;
    let mut entry = archive
        .by_name(name)
        .map_err(|error| format_error(format!("missing `{name}`: {error}")))?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut bytes)
        .map_err(|error| format_error(format!("cannot read `{name}`: {error}")))?;
    Ok(bytes)
}

fn rebuild_package(
    original: &[u8],
    replacements: &BTreeMap<String, Vec<u8>>,
    removals: &BTreeSet<String>,
    additions: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(original))
        .map_err(|error| format_error(format!("invalid PPTX package: {error}")))?;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format_error(format!("cannot read ZIP entry: {error}")))?;
        let name = entry.name().to_owned();
        if removals.contains(&name) {
            continue;
        }
        if let Some(replacement) = replacements.get(&name) {
            let options = SimpleFileOptions::default()
                .compression_method(entry.compression())
                .last_modified_time(entry.last_modified().unwrap_or_default());
            writer.start_file(&name, options).map_err(|error| {
                format_error(format!("cannot start patched ZIP entry: {error}"))
            })?;
            writer.write_all(replacement).map_err(|error| {
                format_error(format!("cannot write patched ZIP entry: {error}"))
            })?;
        } else {
            writer
                .raw_copy_file(entry)
                .map_err(|error| format_error(format!("cannot copy ZIP entry: {error}")))?;
        }
    }
    for (name, bytes) in additions {
        writer
            .start_file(name, SimpleFileOptions::default())
            .map_err(|error| format_error(format!("cannot start added ZIP entry: {error}")))?;
        writer
            .write_all(bytes)
            .map_err(|error| format_error(format!("cannot write added ZIP entry: {error}")))?;
    }
    writer
        .finish()
        .map_err(|error| format_error(format!("cannot finish PPTX package: {error}")))
        .map(|cursor| cursor.into_inner())
}

fn insert_sld_id(xml: &[u8], insertion_index: usize, tag: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("presentation XML is not UTF-8: {error}")))?;
    let spans = sld_id_spans(text)?;
    if insertion_index > spans.len() {
        return Err(format_error("cannot insert slide past end of sldIdLst"));
    }
    let position = if insertion_index == 0 {
        let list_open = text
            .find("<p:sldIdLst>")
            .or_else(|| text.find("<p:sldIdLst "))
            .ok_or_else(|| format_error("presentation is missing p:sldIdLst"))?;
        text[list_open..]
            .find('>')
            .map(|offset| list_open + offset + 1)
            .ok_or_else(|| format_error("unterminated p:sldIdLst"))?
    } else {
        spans[insertion_index - 1].1
    };
    Ok(format!("{}{}{}", &text[..position], tag, &text[position..]).into_bytes())
}

fn reorder_sld_id(xml: &[u8], from_index: usize, to_index: usize) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("presentation XML is not UTF-8: {error}")))?;
    let spans = sld_id_spans(text)?;
    if from_index >= spans.len() {
        return Err(format_error(format!(
            "from_index {from_index} is out of range for {} slides",
            spans.len()
        )));
    }
    if to_index >= spans.len() {
        return Err(format_error(format!(
            "to_index {to_index} is out of range for {} slides",
            spans.len()
        )));
    }
    if from_index == to_index {
        return Ok(xml.to_vec());
    }
    let (from_start, from_end) = spans[from_index];
    let tag = text[from_start..from_end].to_owned();
    let without = format!("{}{}", &text[..from_start], &text[from_end..]);
    let remaining = sld_id_spans(&without)?;
    let insert_at = if to_index == 0 {
        let list_open = without
            .find("<p:sldIdLst>")
            .or_else(|| without.find("<p:sldIdLst "))
            .ok_or_else(|| format_error("presentation is missing p:sldIdLst"))?;
        without[list_open..]
            .find('>')
            .map(|offset| list_open + offset + 1)
            .ok_or_else(|| format_error("unterminated p:sldIdLst"))?
    } else if to_index > remaining.len() {
        return Err(format_error("cannot move slide past end of sldIdLst"));
    } else {
        // After removal, insert so the slide lands at final `to_index`.
        remaining[to_index - 1].1
    };
    Ok(format!("{}{}{}", &without[..insert_at], tag, &without[insert_at..]).into_bytes())
}

fn remove_sld_id_for_rid(xml: &[u8], rid: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("presentation XML is not UTF-8: {error}")))?;
    let needle = format!(r#"r:id="{rid}""#);
    let rid_at = text
        .find(&needle)
        .ok_or_else(|| format_error(format!("sldId for `{rid}` was not found")))?;
    let start = text[..rid_at]
        .rfind("<p:sldId")
        .ok_or_else(|| format_error("malformed sldId element"))?;
    let end = if let Some(rel) = text[rid_at..].find("/>") {
        rid_at + rel + 2
    } else if let Some(rel) = text[rid_at..].find("</p:sldId>") {
        rid_at + rel + "</p:sldId>".len()
    } else {
        return Err(format_error("unterminated sldId element"));
    };
    Ok(format!("{}{}", &text[..start], &text[end..]).into_bytes())
}

fn insert_before_close(xml: &[u8], element: &str, insertion: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("XML is not UTF-8: {error}")))?;
    let closing = format!("</{element}>");
    let position = text
        .rfind(&closing)
        .ok_or_else(|| format_error(format!("XML is missing closing `{element}`")))?;
    Ok(format!("{}{}{}", &text[..position], insertion, &text[position..]).into_bytes())
}

fn remove_relationship(xml: &[u8], id: &str) -> Result<Vec<u8>> {
    remove_matching_tag(xml, "Relationship", |tag| {
        tag_attribute(tag, "Id").as_deref() == Some(id)
    })
}

fn remove_content_type_overrides(xml: &[u8], parts: &BTreeSet<String>) -> Result<Vec<u8>> {
    remove_matching_tag(xml, "Override", |tag| {
        tag_attribute(tag, "PartName")
            .is_some_and(|part| parts.contains(part.trim_start_matches('/')))
    })
}

fn remove_matching_tag(
    xml: &[u8],
    tag_name: &str,
    matches: impl Fn(&str) -> bool,
) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("XML is not UTF-8: {error}")))?;
    let needle = format!("<{tag_name}");
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find(&needle) {
        let start = cursor + offset;
        let after_name = &text[start + needle.len()..];
        if after_name.starts_with(|character: char| character.is_ascii_alphabetic()) {
            output.push_str(&text[cursor..start + needle.len()]);
            cursor = start + needle.len();
            continue;
        }
        let end = text[start..]
            .find('>')
            .map(|rel| start + rel + 1)
            .ok_or_else(|| format_error(format!("unterminated `{tag_name}`")))?;
        let tag = &text[start..end];
        let self_closing = tag.ends_with("/>");
        let span_end = if self_closing {
            end
        } else {
            let close = format!("</{tag_name}>");
            text[end..]
                .find(&close)
                .map(|rel| end + rel + close.len())
                .ok_or_else(|| format_error(format!("missing close for `{tag_name}`")))?
        };
        if matches(tag) {
            output.push_str(&text[cursor..start]);
            cursor = span_end;
        } else {
            output.push_str(&text[cursor..span_end]);
            cursor = span_end;
        }
    }
    output.push_str(&text[cursor..]);
    Ok(output.into_bytes())
}

fn sld_id_spans(text: &str) -> Result<Vec<(usize, usize)>> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = text[cursor..].find("<p:sldId") {
        let start = cursor + rel;
        let after = &text[start + "<p:sldId".len()..];
        if after.starts_with(|c: char| c.is_ascii_alphabetic()) {
            cursor = start + "<p:sldId".len();
            continue;
        }
        let end = if let Some(close_rel) = text[start..].find("/>") {
            start + close_rel + 2
        } else if let Some(close_rel) = text[start..].find("</p:sldId>") {
            start + close_rel + "</p:sldId>".len()
        } else {
            return Err(format_error("unterminated p:sldId"));
        };
        spans.push((start, end));
        cursor = end;
    }
    Ok(spans)
}

fn count_sld_ids(xml: &[u8]) -> Result<usize> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("presentation XML is not UTF-8: {error}")))?;
    Ok(sld_id_spans(text)?.len())
}

fn sld_id_numbers(xml: &[u8]) -> Result<BTreeSet<u32>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("presentation XML is not UTF-8: {error}")))?;
    let mut numbers = BTreeSet::new();
    for (start, end) in sld_id_spans(text)? {
        if let Some(id) =
            tag_attribute(&text[start..end], "id").and_then(|value| value.parse().ok())
        {
            numbers.insert(id);
        }
    }
    Ok(numbers)
}

fn relationship_targets(xml: &[u8]) -> Result<BTreeMap<String, String>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| format_error(format!("relationships XML is not UTF-8: {error}")))?;
    let mut targets = BTreeMap::new();
    let mut cursor = 0;
    while let Some(rel) = text[cursor..].find("<Relationship") {
        let start = cursor + rel;
        let end = text[start..]
            .find('>')
            .map(|offset| start + offset + 1)
            .ok_or_else(|| format_error("unterminated Relationship"))?;
        let tag = &text[start..end];
        if let (Some(id), Some(target)) = (tag_attribute(tag, "Id"), tag_attribute(tag, "Target")) {
            targets.insert(id, target);
        }
        cursor = end;
    }
    Ok(targets)
}

fn relationship_id_numbers(xml: &[u8]) -> Result<BTreeSet<u32>> {
    Ok(relationship_targets(xml)?
        .keys()
        .filter_map(|id| id.strip_prefix("rId")?.parse().ok())
        .collect())
}

fn slide_part_numbers(package: &[u8]) -> Result<BTreeSet<u32>> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| format_error(format!("invalid PPTX package: {error}")))?;
    let mut numbers = BTreeSet::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format_error(format!("cannot read ZIP entry: {error}")))?;
        if let Some(number) = entry
            .name()
            .strip_prefix("ppt/slides/slide")
            .and_then(|name| name.strip_suffix(".xml"))
            .and_then(|number| number.parse().ok())
        {
            numbers.insert(number);
        }
    }
    Ok(numbers)
}

fn lowest_unused_number(used: &BTreeSet<u32>) -> u32 {
    let mut number = 1;
    while used.contains(&number) {
        number += 1;
    }
    number
}

fn resolve_ppt_target(target: &str) -> String {
    if target.starts_with("ppt/") || target.starts_with('/') {
        target.trim_start_matches('/').to_owned()
    } else {
        format!("ppt/{target}")
    }
}

fn slide_relationship_part(slide_part: &str) -> String {
    // ppt/slides/slide1.xml -> ppt/slides/_rels/slide1.xml.rels
    if let Some((dir, file)) = slide_part.rsplit_once('/') {
        format!("{dir}/_rels/{file}.rels")
    } else {
        format!("_rels/{slide_part}.rels")
    }
}

fn relationship_part_name(part: &str) -> String {
    if let Some((dir, file)) = part.rsplit_once('/') {
        format!("{dir}/_rels/{file}.rels")
    } else {
        format!("_rels/{part}.rels")
    }
}

fn notes_parts_from_slide_rels(xml: &[u8]) -> Vec<String> {
    let Ok(targets) = relationship_targets(xml) else {
        return Vec::new();
    };
    targets
        .into_values()
        .filter(|target| target.contains("notesSlide"))
        .map(|target| {
            if target.starts_with("../") {
                format!("ppt/{}", target.trim_start_matches("../"))
            } else {
                resolve_ppt_target(&target)
            }
        })
        .collect()
}

fn has_entry(package: &[u8], name: &str) -> Result<bool> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| format_error(format!("invalid PPTX package: {error}")))?;
    Ok(archive.by_name(name).is_ok())
}

fn tag_attribute(tag: &str, attribute: &str) -> Option<String> {
    let needle = format!("{attribute}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

fn table_ref(payload: &serde_json::Value) -> Result<&str> {
    if let Ok(value) = required_str(payload, "table") {
        return Ok(value);
    }
    required_str(payload, "shape").map_err(|_| format_error("`table` is required"))
}

fn required_bool(payload: &serde_json::Value, key: &str) -> Result<bool> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| format_error(format!("`{key}` boolean is required")))
}

fn optional_str<'a>(payload: &'a serde_json::Value, key: &str) -> Result<Option<&'a str>> {
    match payload.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .filter(|text| !text.is_empty())
            .map(Some)
            .ok_or_else(|| format_error(format!("`{key}` must be a non-empty string"))),
    }
}

fn required_str<'a>(payload: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format_error(format!("`{key}` is required")))
}

fn required_u32(payload: &serde_json::Value, key: &str) -> Result<u32> {
    payload
        .get(key)
        .and_then(|value| {
            value
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .or_else(|| {
                    value
                        .as_i64()
                        .and_then(|n| if n >= 0 { u32::try_from(n).ok() } else { None })
                })
        })
        .ok_or_else(|| format_error(format!("`{key}` is required")))
}

fn required_usize(payload: &serde_json::Value, key: &str) -> Result<usize> {
    payload
        .get(key)
        .and_then(|value| {
            value
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .or_else(|| {
                    value.as_i64().and_then(|n| {
                        if n >= 0 {
                            usize::try_from(n).ok()
                        } else {
                            None
                        }
                    })
                })
        })
        .ok_or_else(|| format_error(format!("`{key}` is required")))
}

pub fn apply(source: &std::path::Path, edit: &ValidatedEdit) -> Result<PatchedOutput> {
    let bytes = std::fs::read(source).map_err(|error| DotallError::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    apply_bytes(&bytes, edit)
}

fn format_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<pptx edit>".into(),
        message: message.into(),
    }
}
