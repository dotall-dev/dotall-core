pub mod detection;
pub mod edits;
pub mod fixture;
pub mod format;
pub mod ids;
pub mod model;
pub mod parser;
pub mod projection;
pub mod selector;

pub const FORMAT_ID: &str = "pdf";

pub use fixture::{minimal_checkbox_pdf, minimal_form_pdf};
pub use format::PdfFormat;
pub use model::{PdfDocumentModel, PdfFieldModel, PdfPageModel, SCHEMA_ID, SCHEMA_VERSION};
pub use parser::parse_pdf_bytes;

#[cfg(test)]
mod tests {
    use super::{FORMAT_ID, detection};
    use dotall_core::registry::DetectionProbe;
    use std::path::Path;

    #[test]
    fn format_id_is_stable() {
        assert_eq!(FORMAT_ID, "pdf");
    }

    #[test]
    fn pdf_magic_and_extension_scores_100() {
        let score = detection::score(&DetectionProbe {
            path: Path::new("form.pdf"),
            prefix: b"%PDF-1.4",
        });
        assert_eq!(score.0, 100);
    }

    #[test]
    fn zip_magic_does_not_match_pdf_handler() {
        let score = detection::score(&DetectionProbe {
            path: Path::new("deck.pptx"),
            prefix: b"PK\x03\x04",
        });
        assert_eq!(score.0, 0);
    }
}
