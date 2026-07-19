use dotall_core::registry::{DetectionProbe, DetectionScore};

/// Scores XLSX candidates using both their OOXML ZIP signature and extension.
pub fn score(probe: &DetectionProbe<'_>) -> DetectionScore {
    let has_xlsx_extension = probe
        .path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx"));
    let has_zip_magic = probe.prefix.starts_with(b"PK\x03\x04")
        || probe.prefix.starts_with(b"PK\x05\x06")
        || probe.prefix.starts_with(b"PK\x07\x08");

    DetectionScore(match (has_xlsx_extension, has_zip_magic) {
        (true, true) => 100,
        (false, true) => 60,
        (true, false) => 40,
        (false, false) => 0,
    })
}
