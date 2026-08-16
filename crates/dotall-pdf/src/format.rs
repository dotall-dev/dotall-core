use std::path::{Path, PathBuf};

use dotall_core::registry::{
    ArtifactEnvelope, ArtifactSchema, Capability, DetectionProbe, DetectionScore, EditCapability,
    FormatDescriptor, FormatHandler, Inspection, PatchedOutput, ReadRequest, ReadResponse,
    ReadSelector, ReadSuggestion, SemanticOperation, ValidatedEdit,
};
use dotall_core::{DotallError, Result};
use serde_json::json;

use crate::detection;
use crate::edits;
use crate::model::{PdfDocumentModel, SCHEMA_ID, SCHEMA_VERSION};
use crate::{FORMAT_ID, parser, projection, selector};

const AVAILABLE_READS: [&str; 3] = ["read.full", "read.page", "read.field"];

pub struct PdfFormat;

impl FormatHandler for PdfFormat {
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
        let payload = serde_json::to_value(parser::parse_pdf(source)?).map_err(|source| {
            DotallError::Serialization {
                context: "PDF document model".into(),
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
        Ok(Inspection {
            format_id: FORMAT_ID.into(),
            summary: json!({
                "page_count": document.page_count,
                "field_names": document.fields.iter().map(|field| &field.name).collect::<Vec<_>>(),
                "encrypted": document.encrypted,
                "has_signature": document.fields.iter().any(|field| field.field_type == "sig"),
            }),
            capabilities: capabilities(),
            edit_capabilities: edit_capabilities(),
            suggested_reads: vec![
                ReadSuggestion {
                    description: "Read page 1".into(),
                    selector: ReadSelector {
                        kind: "page".into(),
                        value: "1".into(),
                    },
                },
                ReadSuggestion {
                    description: "Read form fields".into(),
                    selector: ReadSelector {
                        kind: "full".into(),
                        value: String::new(),
                    },
                },
            ],
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
        let value = request
            .selector
            .as_ref()
            .map(|selector| selector.value.as_str())
            .unwrap_or("");
        match kind {
            "full" => Ok(projection::render_full(
                &document,
                request.max_tokens,
                offset,
            )),
            "page" => {
                let page = selector::resolve_page(&document, value).map_err(selector_error)?;
                Ok(projection::render_page(
                    &page.text,
                    page.number,
                    request.max_tokens,
                    offset,
                ))
            }
            "field" => {
                let field = selector::resolve_field(&document, value).map_err(selector_error)?;
                Ok(projection::render_field(
                    &field.name,
                    &field.field_type,
                    &field.value,
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
}

fn decode(model: &ArtifactEnvelope) -> Result<PdfDocumentModel> {
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
        context: "PDF document artifact payload".into(),
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
            kind: "page".into(),
        },
        Capability::ReadSelector {
            kind: "field".into(),
        },
    ]
}

fn edit_capabilities() -> Vec<EditCapability> {
    vec![EditCapability {
        operation: "set_form_field".into(),
        schema_version: SCHEMA_VERSION,
        description: "Set an AcroForm text (or string choice) field value.".into(),
        example: json!({
            "kind": "set_form_field",
            "payload": { "name": "Name", "value": "Grace" }
        }),
        safety: "Form fill only. Rejects encrypted and signed PDFs. Does not rewrite page content streams.".into(),
    }]
}
