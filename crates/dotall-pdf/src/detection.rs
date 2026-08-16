use dotall_core::registry::{DetectionProbe, DetectionScore};

pub fn score(probe: &DetectionProbe<'_>) -> DetectionScore {
    let has_pdf_extension = probe
        .path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
    let has_magic = probe.prefix.starts_with(b"%PDF-");
    DetectionScore(match (has_pdf_extension, has_magic) {
        (true, true) => 100,
        (false, true) => 80,
        (true, false) => 40,
        (false, false) => 0,
    })
}
