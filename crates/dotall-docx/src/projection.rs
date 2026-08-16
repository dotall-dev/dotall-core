use crate::model::{DocumentModel, ParagraphModel};
use dotall_core::read::apply_budget;
use dotall_core::registry::ReadResponse;

pub fn render_full(model: &DocumentModel, max_tokens: usize, offset: usize) -> ReadResponse {
    budget(
        render_paragraphs(&model.paragraphs.iter().collect::<Vec<_>>()),
        max_tokens,
        offset,
    )
}

pub fn render_paragraphs_read(
    paragraphs: &[&ParagraphModel],
    max_tokens: usize,
    offset: usize,
) -> ReadResponse {
    budget(render_paragraphs(paragraphs), max_tokens, offset)
}

fn render_paragraphs(paragraphs: &[&ParagraphModel]) -> String {
    let mut content = String::from("# Document\n\n");
    for paragraph in paragraphs {
        content.push_str(&format!("{}. {}\n", paragraph.index, paragraph.text));
    }
    content
}

fn budget(content: String, max_tokens: usize, offset: usize) -> ReadResponse {
    let (sliced, truncated, continuation) = apply_budget(&content, max_tokens, offset);
    ReadResponse {
        content: sliced,
        estimated_tokens: (content.len() / 4).max(1),
        truncated,
        continuation,
        next_actions: Vec::new(),
    }
}
