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

pub use fixture::{
    demo_deck_pptx, demo_q3_deck_pptx, minimal_pptx, minimal_pptx_with_media, pptx_two_text_shapes,
    pptx_with_chart, pptx_with_comment, pptx_with_notes, pptx_with_png, pptx_with_table,
};
pub use format::PptxFormat;
pub use model::{
    ChartModel, CommentModel, PictureModel, PresentationModel, SCHEMA_ID, SCHEMA_VERSION,
    ShapeModel, SlideModel, TableCellModel, TableModel,
};
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
    fn presentation_package_wins_even_with_xlsx_extension() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("deck.xlsx");
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file(
                "ppt/presentation.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("entry");
        std::io::Write::write_all(&mut writer, b"<p:presentation/>").expect("bytes");
        let package = writer.finish().expect("finish").into_inner();
        std::fs::write(&path, &package).expect("write");
        let score = detection::score(&DetectionProbe {
            path: &path,
            prefix: &package[..4],
        });
        assert!(
            score.0 >= 80,
            "package evidence should identify PPTX without a .pptx extension, got {}",
            score.0
        );
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
