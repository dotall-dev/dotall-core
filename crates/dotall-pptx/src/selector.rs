use crate::model::PresentationModel;

pub fn resolve_slide<'a>(
    model: &'a PresentationModel,
    value: &str,
) -> Option<&'a crate::model::SlideModel> {
    let trimmed = value.trim();
    model.slides.iter().find(|slide| {
        slide.name.eq_ignore_ascii_case(trimmed)
            || slide.part_name.eq_ignore_ascii_case(trimmed)
            || format!("slide{}", slide.index + 1).eq_ignore_ascii_case(trimmed)
            || (slide.index + 1).to_string() == trimmed
    })
}
