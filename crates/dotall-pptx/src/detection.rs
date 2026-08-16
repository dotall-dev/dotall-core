use dotall_core::registry::{DetectionProbe, DetectionScore};

pub fn score(probe: &DetectionProbe<'_>) -> DetectionScore {
    let has_pptx_extension = probe
        .path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pptx"));
    let has_zip_magic = dotall_ooxml::has_zip_magic(probe.prefix);
    DetectionScore(match (has_pptx_extension, has_zip_magic) {
        (true, true) => 100,
        (true, false) => 40,
        (false, _) => 0,
    })
}
