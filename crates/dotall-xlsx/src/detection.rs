use dotall_core::registry::{DetectionProbe, DetectionScore};

/// Scores XLSX candidates using both their OOXML ZIP signature and extension.
///
/// A generic ZIP is not treated as XLSX so PPTX/DOCX can win on their extensions.
pub fn score(probe: &DetectionProbe<'_>) -> DetectionScore {
    let has_xlsx_extension = probe
        .path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx"));
    let has_zip_magic = dotall_ooxml::has_zip_magic(probe.prefix);

    DetectionScore(match (has_xlsx_extension, has_zip_magic) {
        (true, true) => 100,
        (true, false) => 40,
        (false, _) => 0,
    })
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
}
