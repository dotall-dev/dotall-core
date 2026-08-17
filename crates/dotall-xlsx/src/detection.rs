use dotall_core::registry::{DetectionProbe, DetectionScore};

/// Scores XLSX candidates using extension, ZIP magic, and package evidence.
///
/// A generic ZIP is not treated as XLSX so PPTX/DOCX can win on their parts.
pub fn score(probe: &DetectionProbe<'_>) -> DetectionScore {
    DetectionScore(dotall_ooxml::score_office_package(
        probe,
        "xlsx",
        "xl/workbook.xml",
    ))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::score;
    use dotall_core::registry::DetectionProbe;

    #[test]
    fn pptx_zip_is_not_a_strong_xlsx_match() {
        let score = score(&DetectionProbe {
            path: Path::new("deck.pptx"),
            prefix: b"PK\x03\x04",
        });
        assert_eq!(score.0, 0, "ZIP without .xlsx must not be claimed as XLSX");
    }

    #[test]
    fn xlsx_zip_still_scores_100() {
        let score = score(&DetectionProbe {
            path: Path::new("book.xlsx"),
            prefix: b"PK\x03\x04",
        });
        assert_eq!(score.0, 100);
    }

    #[test]
    fn readable_pptx_package_with_xlsx_extension_is_a_weak_xlsx_match() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("deck.xlsx");
        write_zip(&path, "ppt/presentation.xml");
        let bytes = std::fs::read(&path).expect("zip bytes");
        let score = score(&DetectionProbe {
            path: &path,
            prefix: &bytes[..4],
        });
        assert!(
            score.0 < 80,
            "wrong-family package must not win as XLSX, got {}",
            score.0
        );
    }

    fn write_zip(path: &Path, entry: &str) {
        use std::io::{Cursor, Write};

        use zip::ZipWriter;
        use zip::write::SimpleFileOptions;

        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(entry, SimpleFileOptions::default())
            .expect("entry");
        writer.write_all(b"<root/>").expect("bytes");
        let package = writer.finish().expect("finish").into_inner();
        std::fs::write(path, package).expect("write zip");
    }
}
