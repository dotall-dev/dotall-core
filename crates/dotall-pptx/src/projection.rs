use crate::model::{PresentationModel, SlideModel};
use dotall_core::read::apply_budget;
use dotall_core::registry::ReadResponse;

pub fn render_full(model: &PresentationModel, max_tokens: usize, offset: usize) -> ReadResponse {
    let mut content = String::from("# Presentation\n\n");
    for slide in &model.slides {
        content.push_str(&render_slide(slide));
        content.push('\n');
    }
    budget(content, max_tokens, offset)
}

pub fn render_slide_read(slide: &SlideModel, max_tokens: usize, offset: usize) -> ReadResponse {
    budget(render_slide(slide), max_tokens, offset)
}

pub fn render_notes(slide: &SlideModel, max_tokens: usize, offset: usize) -> ReadResponse {
    let notes = slide.notes.clone().unwrap_or_default();
    budget(
        format!("# {} notes\n\n{notes}\n", slide.name),
        max_tokens,
        offset,
    )
}

fn render_slide(slide: &SlideModel) -> String {
    let mut content = format!("# {}\n\n", slide.name);
    for shape in &slide.shapes {
        content.push_str(&format!("- **{}**: {}\n", shape.name, shape.text));
    }
    for table in &slide.tables {
        for cell in &table.cells {
            content.push_str(&format!(
                "- **{}[{},{}]**: {}\n",
                table.name, cell.row, cell.col, cell.text
            ));
        }
    }
    content
}

fn budget(content: String, max_tokens: usize, offset: usize) -> ReadResponse {
    let (sliced, truncated, continuation) = apply_budget(&content, max_tokens, offset)
        .unwrap_or_else(|_| (content.clone(), false, None));
    ReadResponse {
        content: sliced,
        estimated_tokens: (content.len() / 4).max(1),
        truncated,
        continuation,
        next_actions: Vec::new(),
    }
}
