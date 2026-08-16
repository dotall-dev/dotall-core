pub mod detection;
pub mod edits;
pub mod fixture;
pub mod format;
pub mod ids;
pub mod model;
pub mod parser;
pub mod projection;
pub mod selector;

pub const FORMAT_ID: &str = "docx";

pub use fixture::{minimal_docx, minimal_docx_with_media};
pub use format::DocxFormat;
pub use model::{DocumentModel, ParagraphModel, SCHEMA_ID, SCHEMA_VERSION};
pub use parser::parse_document_bytes;

#[cfg(test)]
mod tests {
    use super::{FORMAT_ID, detection, ids};
    use dotall_core::registry::DetectionProbe;
    use std::path::Path;

    #[test]
    fn format_id_is_stable() {
        assert_eq!(FORMAT_ID, "docx");
    }

    #[test]
    fn docx_extension_plus_zip_magic_scores_100() {
        let score = detection::score(&DetectionProbe {
            path: Path::new("memo.docx"),
            prefix: b"PK\x03\x04",
        });
        assert_eq!(score.0, 100);
    }

    #[test]
    fn pptx_path_does_not_match_docx_handler() {
        let score = detection::score(&DetectionProbe {
            path: Path::new("deck.pptx"),
            prefix: b"PK\x03\x04",
        });
        assert_eq!(score.0, 0);
    }

    #[test]
    fn ids_are_deterministic_and_opaque() {
        let first = ids::paragraph_id(0, "Alpha", 1);
        let second = ids::paragraph_id(0, "Alpha", 1);
        assert_eq!(first, second);
        assert!(first.starts_with("p_"));
        assert!(!first.contains("Alpha"));
    }
}
