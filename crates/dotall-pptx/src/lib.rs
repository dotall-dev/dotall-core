pub mod detection;
pub mod edits;
pub mod fixture;
pub mod format;
pub mod ids;
pub mod model;
pub mod parser;
pub mod projection;
pub mod selector;

pub const FORMAT_ID: &str = "pptx";

pub use fixture::{minimal_pptx, minimal_pptx_with_media};
pub use format::PptxFormat;
pub use model::{PresentationModel, SCHEMA_ID, SCHEMA_VERSION, ShapeModel, SlideModel};
pub use parser::parse_presentation_bytes;

#[cfg(test)]
mod tests {
    use super::{FORMAT_ID, detection, ids};
    use dotall_core::registry::DetectionProbe;
    use std::path::Path;

    #[test]
    fn format_id_is_stable() {
        assert_eq!(FORMAT_ID, "pptx");
    }

    #[test]
    fn pptx_extension_plus_zip_magic_scores_100() {
        let score = detection::score(&DetectionProbe {
            path: Path::new("deck.pptx"),
            prefix: b"PK\x03\x04",
        });
        assert_eq!(score.0, 100);
    }

    #[test]
    fn xlsx_path_does_not_match_pptx_handler() {
        let score = detection::score(&DetectionProbe {
            path: Path::new("book.xlsx"),
            prefix: b"PK\x03\x04",
        });
        assert_eq!(score.0, 0);
    }

    #[test]
    fn ids_are_deterministic_and_opaque() {
        let first = ids::shape_id("Slide 1", "Title", 1);
        let second = ids::shape_id("Slide 1", "Title", 1);
        assert_eq!(first, second);
        assert!(first.starts_with("sp_"));
        assert!(!first.contains("Title"));
    }
}
