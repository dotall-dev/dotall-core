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
use crate::model::{DocumentModel, SCHEMA_ID, SCHEMA_VERSION};
use crate::{FORMAT_ID, parser, projection, selector};

const AVAILABLE_READS: [&str; 4] = [
    "read.full",
    "read.paragraphs",
    "read.headers",
    "read.footers",
];
const MANIFEST_SCHEMA_ID: &str = "docx.snapshot-manifest";

pub struct DocxFormat;

impl FormatHandler for DocxFormat {
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
        let payload = serde_json::to_value(parser::parse_document(source)?).map_err(|source| {
            DotallError::Serialization {
                context: "DOCX document model".into(),
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
        let document = decode(model)?;
        let headings = document
            .paragraphs
            .iter()
            .filter(|paragraph| {
                paragraph
                    .style_id
                    .as_deref()
                    .is_some_and(|style| style.to_ascii_lowercase().starts_with("heading"))
                    || paragraph.outline_level.is_some()
            })
            .map(|paragraph| {
                json!({
                    "index": paragraph.index,
                    "text": paragraph.text,
                    "style_id": paragraph.style_id,
                })
            })
            .collect::<Vec<_>>();
        let headers = document
            .header_paragraphs
            .iter()
            .map(|paragraph| {
                json!({
                    "part": paragraph.part,
                    "index": paragraph.index,
                    "text": paragraph.text,
                })
            })
            .collect::<Vec<_>>();
        let footers = document
            .footer_paragraphs
            .iter()
            .map(|paragraph| {
                json!({
                    "part": paragraph.part,
                    "index": paragraph.index,
                    "text": paragraph.text,
                })
            })
            .collect::<Vec<_>>();
        Ok(Inspection {
            format_id: FORMAT_ID.into(),
            summary: json!({
                "paragraph_count": document.paragraphs.len(),
                "header_paragraph_count": document.header_paragraphs.len(),
                "footer_paragraph_count": document.footer_paragraphs.len(),
                "headings": headings,
                "headers": headers,
                "footers": footers,
                "table_count": document.table_count,
                "skipped_tables": document.skipped_tables,
            }),
            capabilities: capabilities(),
            edit_capabilities: edit_capabilities(),
            suggested_reads: vec![ReadSuggestion {
                description: "Read body paragraphs".into(),
                selector: ReadSelector {
                    kind: "paragraphs".into(),
                    value: if document.paragraphs.is_empty() {
                        "0".into()
                    } else {
                        format!("0:{}", document.paragraphs.len())
                    },
                },
            }],
        })
    }

    fn read(&self, model: &ArtifactEnvelope, request: &ReadRequest) -> Result<ReadResponse> {
        let document = decode(model)?;
        let offset = continuation_offset(request.continuation.as_deref())?;
        let kind = request
            .selector
            .as_ref()
            .map(|selector| selector.kind.as_str())
            .unwrap_or("full");
        match kind {
            "full" => Ok(projection::render_full(
                &document,
                request.max_tokens,
                offset,
            )),
            "paragraphs" => {
                let value = request
                    .selector
                    .as_ref()
                    .map(|selector| selector.value.as_str())
                    .unwrap_or("");
                let paragraphs =
                    selector::resolve_range(&document, value).map_err(selector_error)?;
                Ok(projection::render_paragraphs_read(
                    &paragraphs,
                    request.max_tokens,
                    offset,
                ))
            }
            "headers" => {
                let value = request
                    .selector
                    .as_ref()
                    .map(|selector| selector.value.as_str())
                    .unwrap_or("");
                let paragraphs =
                    selector::resolve_header_range(&document, value).map_err(selector_error)?;
                Ok(projection::render_headers_read(
                    &paragraphs,
                    request.max_tokens,
                    offset,
                ))
            }
            "footers" => {
                let value = request
                    .selector
                    .as_ref()
                    .map(|selector| selector.value.as_str())
                    .unwrap_or("");
                let paragraphs =
                    selector::resolve_footer_range(&document, value).map_err(selector_error)?;
                Ok(projection::render_footers_read(
                    &paragraphs,
                    request.max_tokens,
                    offset,
                ))
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
                path: "<docx snapshot>".into(),
                message: format!("cannot decode {} snapshot as DOCX", encoded.format_id),
            });
        }
        dotall_ooxml::decode_package(encoded)
    }
}

fn decode(model: &ArtifactEnvelope) -> Result<DocumentModel> {
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
        context: "DOCX document artifact payload".into(),
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
            kind: "paragraphs".into(),
        },
        Capability::ReadSelector {
            kind: "headers".into(),
        },
        Capability::ReadSelector {
            kind: "footers".into(),
        },
    ]
}

fn edit_capabilities() -> Vec<EditCapability> {
    vec![
        EditCapability {
            operation: "set_paragraph_text".into(),
            schema_version: SCHEMA_VERSION,
            description:
                "Replace paragraph text (body or table cell). Plain `text` keeps first-run rPr; optional `runs` clones per-run rPr."
                    .into(),
            example: json!({
                "kind": "set_paragraph_text",
                "payload": {
                    "index": 1,
                    "runs": [
                        { "text": "Pilot complete; " },
                        { "text": "expanding to PPTX and PDF." }
                    ]
                }
            }),
            safety:
                "Rejects tracked changes, content controls, and fields. Patches only word/document.xml. Does not steal paragraph-mark rPr from w:pPr."
                    .into(),
        },
        EditCapability {
            operation: "insert_paragraph".into(),
            schema_version: SCHEMA_VERSION,
            description:
                "Insert a new paragraph immediately after the given document-order index (body or table cell)."
                    .into(),
            example: json!({
                "kind": "insert_paragraph",
                "payload": { "after": 2, "text": "Action: confirm owners before Friday." }
            }),
            safety:
                "Patches only word/document.xml. New paragraph is a single plain run; subsequent indices shift."
                    .into(),
        },
        EditCapability {
            operation: "delete_paragraph".into(),
            schema_version: SCHEMA_VERSION,
            description:
                "Delete a paragraph by document-order index or element_id (body or table cell)."
                    .into(),
            example: json!({
                "kind": "delete_paragraph",
                "payload": { "index": 3 }
            }),
            safety:
                "Patches only word/document.xml. Removes the entire w:p; subsequent indices shift down."
                    .into(),
        },
        EditCapability {
            operation: "set_paragraph_style".into(),
            schema_version: SCHEMA_VERSION,
            description:
                "Set paragraph style_id (w:pStyle) without rewriting styles.xml."
                    .into(),
            example: json!({
                "kind": "set_paragraph_style",
                "payload": { "index": 2, "style_id": "Heading1" }
            }),
            safety:
                "Upserts w:pStyle inside w:pPr on the target body/table paragraph. Patches only word/document.xml. Style must already exist in styles.xml for Word to resolve it."
                    .into(),
        },
        EditCapability {
            operation: "set_header_paragraph_text".into(),
            schema_version: SCHEMA_VERSION,
            description:
                "Replace the text of one header paragraph (`part` + within-part `index`)."
                    .into(),
            example: json!({
                "kind": "set_header_paragraph_text",
                "payload": { "part": "header1", "index": 0, "text": "CONFIDENTIAL" }
            }),
            safety:
                "Rejects tracked changes, content controls, and fields. Patches only the target word/header*.xml part."
                    .into(),
        },
        EditCapability {
            operation: "set_footer_paragraph_text".into(),
            schema_version: SCHEMA_VERSION,
            description:
                "Replace the text of one footer paragraph (`part` + within-part `index`)."
                    .into(),
            example: json!({
                "kind": "set_footer_paragraph_text",
                "payload": { "part": "footer1", "index": 0, "text": "Page 1" }
            }),
            safety:
                "Rejects tracked changes, content controls, and fields. Patches only the target word/footer*.xml part."
                    .into(),
        },
    ]
}
