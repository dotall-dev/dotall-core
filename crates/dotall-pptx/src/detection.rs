use dotall_core::registry::{DetectionProbe, DetectionScore};

pub fn score(probe: &DetectionProbe<'_>) -> DetectionScore {
    DetectionScore(dotall_ooxml::score_office_package(
        probe,
        "pptx",
        "ppt/presentation.xml",
    ))
}
