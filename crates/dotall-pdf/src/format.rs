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
use crate::model::{PdfDocumentModel, PdfFieldModel, PdfMetadata, SCHEMA_ID, SCHEMA_VERSION};
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
                "fields": document.fields.iter().map(field_summary).collect::<Vec<_>>(),
                "encrypted": document.encrypted,
                "has_signature": document.fields.iter().any(|field| field.field_type == "sig"),
                "metadata": metadata_summary(&document.metadata),
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
                Ok(projection::render_field(field, request.max_tokens, offset))
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

fn field_summary(field: &PdfFieldModel) -> serde_json::Value {
    let mut summary = json!({
        "name": field.name,
        "field_type": field.field_type,
        "value": field.value,
        "read_only": field.read_only,
        "required": field.required,
        "multiline": field.multiline,
    });
    if !field.export_values.is_empty() {
        summary["export_values"] = json!(field.export_values);
    }
    if !field.options.is_empty() {
        summary["options"] = json!(field.options);
    }
    summary
}

fn metadata_summary(metadata: &PdfMetadata) -> serde_json::Value {
    json!({
        "title": metadata.title,
        "author": metadata.author,
        "subject": metadata.subject,
        "creator": metadata.creator,
        "producer": metadata.producer,
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
    vec![
        EditCapability {
            operation: "set_form_field".into(),
            schema_version: SCHEMA_VERSION,
            description: "Set an AcroForm text, choice, checkbox, or radio (Btn) field value.".into(),
            example: json!({
                "kind": "set_form_field",
                "payload": { "name": "Priority", "value": "High" }
            }),
            safety: "Form fill only. Checkboxes accept On/Off or export values; radios require an explicit export value when multiple on-states exist. Rejects encrypted and signed PDFs. Does not rewrite page content streams.".into(),
        },
        EditCapability {
            operation: "clear_form_field".into(),
            schema_version: SCHEMA_VERSION,
            description: "Clear an AcroForm field (blank text/choice, Off for checkbox/radio)."
                .into(),
            example: json!({
                "kind": "clear_form_field",
                "payload": { "name": "Name" }
            }),
            safety: "Sets Tx/Ch to empty string and Btn to Off. Rejects encrypted, signed, and read-only fields. Does not rewrite page content streams.".into(),
        },
        EditCapability {
            operation: "clear_all_form_fields".into(),
            schema_version: SCHEMA_VERSION,
            description: "Clear every non-read-only AcroForm field in one transaction.".into(),
            example: json!({
                "kind": "clear_all_form_fields",
                "payload": {}
            }),
            safety: "Applies clear_form_field semantics to each editable field. Skips read-only fields. Rejects encrypted and signed PDFs.".into(),
        },
        EditCapability {
            operation: "set_form_fields".into(),
            schema_version: SCHEMA_VERSION,
            description: "Set multiple AcroForm fields in one transaction via a name→value map."
                .into(),
            example: json!({
                "kind": "set_form_fields",
                "payload": { "fields": { "Name": "Ada", "Agree": "On" } }
            }),
            safety: "Validates each field like set_form_field (including Btn export values). Rejects empty maps, missing names, encrypted/signed PDFs, and read-only fields.".into(),
        },
        EditCapability {
            operation: "set_form_field_readonly".into(),
            schema_version: SCHEMA_VERSION,
            description: "Set or clear the AcroForm field ReadOnly flag (/Ff bit 1).".into(),
            example: json!({
                "kind": "set_form_field_readonly",
                "payload": { "name": "Name", "readonly": true }
            }),
            safety: "Toggles /Ff ReadOnly on the named field. Inspect surfaces read_only. Value edits reject read-only fields. Rejects encrypted and signed PDFs.".into(),
        },
        EditCapability {
            operation: "set_form_field_required".into(),
            schema_version: SCHEMA_VERSION,
            description: "Set or clear the AcroForm field Required flag (/Ff bit 2).".into(),
            example: json!({
                "kind": "set_form_field_required",
                "payload": { "name": "Name", "required": true }
            }),
            safety: "Toggles /Ff Required on the named field. Inspect surfaces required. Rejects encrypted and signed PDFs.".into(),
        },
        EditCapability {
            operation: "set_form_field_multiline".into(),
            schema_version: SCHEMA_VERSION,
            description: "Set or clear the AcroForm text-field Multiline flag (/Ff bit 13)."
                .into(),
            example: json!({
                "kind": "set_form_field_multiline",
                "payload": { "name": "Name", "multiline": true }
            }),
            safety: "Toggles /Ff Multiline on tx fields only. Inspect surfaces multiline. Rejects non-text fields, encrypted and signed PDFs.".into(),
        },
        EditCapability {
            operation: "set_document_metadata".into(),
            schema_version: SCHEMA_VERSION,
            description: "Set PDF /Info Title, Author, and/or Subject.".into(),
            example: json!({
                "kind": "set_document_metadata",
                "payload": {
                    "title": "Updated Intake",
                    "author": "Wave 4 Agent",
                    "subject": "Onboarding refresh"
                }
            }),
            safety: "Patches trailer /Info only. Omit keys you want to leave unchanged. Rejects encrypted and signed PDFs.".into(),
        },
        EditCapability {
            operation: "clear_document_metadata".into(),
            schema_version: SCHEMA_VERSION,
            description: "Clear PDF /Info Title, Author, and Subject.".into(),
            example: json!({
                "kind": "clear_document_metadata",
                "payload": {}
            }),
            safety: "Removes Title/Author/Subject from trailer /Info. Leaves Creator/Producer untouched. Rejects encrypted and signed PDFs.".into(),
        },
    ]
}
