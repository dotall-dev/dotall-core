use std::path::{Path, PathBuf};

use dotall_core::registry::{
    ArtifactEnvelope, ArtifactSchema, Capability, DetectionProbe, DetectionScore, EditCapability,
    EncodedSnapshot, FormatDescriptor, FormatHandler, Inspection, PatchedOutput, ReadRequest,
    ReadResponse, ReadSelector, ReadSuggestion, SemanticOperation, ValidatedEdit,
};
use dotall_core::{DotallError, Result};
use serde_json::json;

use crate::detection;
use crate::edits;
use crate::model::{PresentationModel, SCHEMA_ID, SCHEMA_VERSION};
use crate::{FORMAT_ID, parser, projection, selector};

const AVAILABLE_READS: [&str; 3] = ["read.full", "read.slide", "read.notes"];
const MANIFEST_SCHEMA_ID: &str = "pptx.snapshot-manifest";

pub struct PptxFormat;

impl FormatHandler for PptxFormat {
    fn descriptor(&self) -> FormatDescriptor {
        FormatDescriptor {
            id: FORMAT_ID.into(),
            version: SCHEMA_VERSION.to_string(),
            capabilities: capabilities(),
            edit_capabilities: edit_capabilities(),
        }
    }

    fn artifact_schema(&self) -> ArtifactSchema {
        ArtifactSchema {
            format_id: FORMAT_ID.into(),
            schema_id: SCHEMA_ID.into(),
            schema_version: SCHEMA_VERSION,
        }
    }

    fn detect(&self, probe: &DetectionProbe<'_>) -> DetectionScore {
        detection::score(probe)
    }

    fn parse(&self, source: &Path) -> Result<ArtifactEnvelope> {
        let payload =
            serde_json::to_value(parser::parse_presentation(source)?).map_err(|source| {
                DotallError::Serialization {
                    context: "PPTX presentation model".into(),
                    source,
                }
            })?;
        Ok(ArtifactEnvelope {
            format_id: FORMAT_ID.into(),
            schema_id: SCHEMA_ID.into(),
            schema_version: SCHEMA_VERSION,
            payload,
        })
    }

    fn inspect(&self, model: &ArtifactEnvelope) -> Result<Inspection> {
        let presentation = decode(model)?;
        let slides = presentation
            .slides
            .iter()
            .map(|slide| {
                json!({
                    "name": slide.name,
                    "index": slide.index,
                    "shape_count": slide.shapes.len(),
                    "table_count": slide.tables.len(),
                    "preview": slide.shapes.first().map(|shape| shape.text.as_str()).unwrap_or(""),
                })
            })
            .collect::<Vec<_>>();
        let suggested_reads = presentation
            .slides
            .iter()
            .map(|slide| ReadSuggestion {
                description: format!("Read {}", slide.name),
                selector: ReadSelector {
                    kind: "slide".into(),
                    value: slide.name.clone(),
                },
            })
            .collect();

        Ok(Inspection {
            format_id: FORMAT_ID.into(),
            summary: json!({
                "slides": slides,
                "media_parts": presentation.media_parts,
            }),
            capabilities: capabilities(),
            edit_capabilities: edit_capabilities(),
            suggested_reads,
        })
    }

    fn read(&self, model: &ArtifactEnvelope, request: &ReadRequest) -> Result<ReadResponse> {
        let presentation = decode(model)?;
        let offset = continuation_offset(request.continuation.as_deref())?;
        let kind = request
            .selector
            .as_ref()
            .map(|selector| selector.kind.as_str())
            .unwrap_or("full");
        let value = request
            .selector
            .as_ref()
            .map(|selector| selector.value.as_str())
            .unwrap_or("");
        match kind {
            "full" => Ok(projection::render_full(
                &presentation,
                request.max_tokens,
                offset,
            )),
            "slide" => {
                let slide = selector::resolve_slide(&presentation, value)
                    .ok_or_else(|| selector_error(format!("slide `{value}` was not found")))?;
                Ok(projection::render_slide_read(
                    slide,
                    request.max_tokens,
                    offset,
                ))
            }
            "notes" => {
                let slide = selector::resolve_slide(&presentation, value)
                    .ok_or_else(|| selector_error(format!("slide `{value}` was not found")))?;
                Ok(projection::render_notes(slide, request.max_tokens, offset))
            }
            other => Err(unsupported(other)),
        }
    }

    fn validate_edit(
        &self,
        model: &ArtifactEnvelope,
        operations: &[SemanticOperation],
    ) -> Result<ValidatedEdit> {
        edits::validate(&decode(model)?, operations)
    }

    fn apply_edit(&self, source: &Path, edit: &ValidatedEdit) -> Result<PatchedOutput> {
        edits::apply(source, edit)
    }

    fn encode_snapshot(&self, source_bytes: &[u8]) -> Result<EncodedSnapshot> {
        dotall_ooxml::encode_package(source_bytes, FORMAT_ID, MANIFEST_SCHEMA_ID, None)
    }

    fn decode_snapshot(&self, encoded: &EncodedSnapshot) -> Result<Vec<u8>> {
        if encoded.format_id != FORMAT_ID {
            return Err(DotallError::Format {
                format_id: FORMAT_ID.into(),
                path: "<pptx snapshot>".into(),
                message: format!("cannot decode {} snapshot as PPTX", encoded.format_id),
            });
        }
        dotall_ooxml::decode_package(encoded)
    }
}

fn decode(model: &ArtifactEnvelope) -> Result<PresentationModel> {
    if model.format_id != FORMAT_ID
        || model.schema_id != SCHEMA_ID
        || model.schema_version != SCHEMA_VERSION
    {
        return Err(DotallError::ArtifactSchemaMismatch {
            format_id: FORMAT_ID.into(),
            schema_id: model.schema_id.clone(),
            schema_version: model.schema_version,
        });
    }
    serde_json::from_value(model.payload.clone()).map_err(|source| DotallError::Serialization {
        context: "PPTX presentation artifact payload".into(),
        source,
    })
}

fn continuation_offset(continuation: Option<&str>) -> Result<usize> {
    continuation
        .map(|value| {
            value.parse::<usize>().map_err(|_| DotallError::Format {
                format_id: FORMAT_ID.into(),
                path: PathBuf::from("<read continuation>"),
                message: format!("invalid continuation cursor `{value}`"),
            })
        })
        .transpose()
        .map(|offset| offset.unwrap_or(0))
}

fn selector_error(message: String) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: PathBuf::from("<read selector>"),
        message,
    }
}

fn unsupported(capability: &str) -> DotallError {
    DotallError::UnsupportedCapability {
        format_id: FORMAT_ID.into(),
        capability: capability.into(),
        available: AVAILABLE_READS.iter().map(ToString::to_string).collect(),
    }
}

fn capabilities() -> Vec<Capability> {
    vec![
        Capability::Inspect,
        Capability::ReadFull,
        Capability::ReadSelector {
            kind: "slide".into(),
        },
        Capability::ReadSelector {
            kind: "notes".into(),
        },
    ]
}

fn edit_capabilities() -> Vec<EditCapability> {
    vec![
        EditCapability {
            operation: "set_shape_text".into(),
            schema_version: SCHEMA_VERSION,
            description: "Replace text in a simple text-frame shape.".into(),
            example: json!({
                "kind": "set_shape_text",
                "payload": { "slide": "Slide 1", "shape": "Title", "text": "World" }
            }),
            safety: "Text frames only. Patches the target slide part; rejects SmartArt, charts, and grouped drawingML.".into(),
        },
        EditCapability {
            operation: "set_table_cell_text".into(),
            schema_version: SCHEMA_VERSION,
            description: "Replace text in one cell of a slide table (a:tbl inside p:graphicFrame).".into(),
            example: json!({
                "kind": "set_table_cell_text",
                "payload": {
                    "slide": "Slide 1",
                    "table": "Table 1",
                    "row": 0,
                    "col": 1,
                    "text": "NEW"
                }
            }),
            safety: "Patches only the slide part that owns the table. Rejects out-of-range row/col. Media and other slides stay byte-identical.".into(),
        },
        EditCapability {
            operation: "set_notes_text".into(),
            schema_version: SCHEMA_VERSION,
            description: "Replace speaker notes text for a slide that already has a notes slide part.".into(),
            example: json!({
                "kind": "set_notes_text",
                "payload": { "slide": "Slide 1", "text": "Updated speaker notes" }
            }),
            safety: "Patches only the notes slide part. Rejects slides without notes. Slide XML and media stay byte-identical.".into(),
        },
        EditCapability {
            operation: "add_slide".into(),
            schema_version: SCHEMA_VERSION,
            description: "Insert a blank slide after an existing slide (or at the end when `after` is omitted).".into(),
            example: json!({
                "kind": "add_slide",
                "payload": { "after": "Slide 1" }
            }),
            safety: "Updates presentation.xml, presentation.xml.rels, and [Content_Types].xml; duplicates a blank slide template. Prior slide parts stay byte-identical.".into(),
        },
        EditCapability {
            operation: "delete_slide".into(),
            schema_version: SCHEMA_VERSION,
            description: "Remove a slide by identity. Rejects deleting the sole remaining slide.".into(),
            example: json!({
                "kind": "delete_slide",
                "payload": { "slide": "Slide 2" }
            }),
            safety: "Updates presentation.xml, presentation.xml.rels, and [Content_Types].xml; removes the slide part (and notes/rels when present). Other slides and media stay byte-identical.".into(),
        },
        EditCapability {
            operation: "move_slide".into(),
            schema_version: SCHEMA_VERSION,
            description: "Reorder a slide to a 0-based `to_index` by rewriting only presentation.xml sldIdLst order.".into(),
            example: json!({
                "kind": "move_slide",
                "payload": { "slide": "Slide 2", "to_index": 0 }
            }),
            safety: "Patches only ppt/presentation.xml (p:sldId order). Slide parts, notes, rels, and Content_Types stay byte-identical. Rejects no-op and single-slide decks.".into(),
        },
        EditCapability {
            operation: "add_textbox".into(),
            schema_version: SCHEMA_VERSION,
            description: "Insert a simple text-box shape (p:sp with txBox) onto a slide.".into(),
            example: json!({
                "kind": "add_textbox",
                "payload": { "slide": "Slide 1", "name": "Callout", "text": "Agent note" }
            }),
            safety: "Patches only the target slide part. Optional `name` defaults to TextBox N; rejects duplicate names. Other slides and media stay byte-identical.".into(),
        },
        EditCapability {
            operation: "delete_shape".into(),
            schema_version: SCHEMA_VERSION,
            description: "Remove a text shape (p:sp) from a slide by name or element_id.".into(),
            example: json!({
                "kind": "delete_shape",
                "payload": { "slide": "Slide 1", "shape": "Callout" }
            }),
            safety: "Patches only the target slide part by removing the matching p:sp. Tables (graphicFrame) are not deleted via this op. Other slides and media stay byte-identical.".into(),
        },
        EditCapability {
            operation: "rename_shape".into(),
            schema_version: SCHEMA_VERSION,
            description: "Rename a text shape (p:sp cNvPr name) on a slide.".into(),
            example: json!({
                "kind": "rename_shape",
                "payload": { "slide": "Slide 1", "shape": "Title", "name": "Headline" }
            }),
            safety: "Patches only the target slide part. Rejects duplicate names on the same slide (including table names). Other slides and media stay byte-identical.".into(),
        },
        EditCapability {
            operation: "set_shape_bold".into(),
            schema_version: SCHEMA_VERSION,
            description: "Set or clear bold on all text runs inside a slide shape (a:rPr b).".into(),
            example: json!({
                "kind": "set_shape_bold",
                "payload": { "slide": "Slide 1", "shape": "Title", "bold": true }
            }),
            safety: "Upserts a:rPr b on each a:r in the shape txBody. Patches only the target slide part. Rejects graphicFrame/SmartArt/charts.".into(),
        },
    ]
}
