use crate::model::{DocumentModel, HeaderFooterParagraphModel, ParagraphModel};
use dotall_core::read::apply_budget;
use dotall_core::registry::ReadResponse;

pub fn render_full(model: &DocumentModel, max_tokens: usize, offset: usize) -> ReadResponse {
    let mut content = String::from("# Document\n\n");
    content.push_str(&render_body_paragraphs(
        &model.paragraphs.iter().collect::<Vec<_>>(),
    ));
    if !model.header_paragraphs.is_empty() {
        content.push_str("\n## Headers\n\n");
        content.push_str(&render_header_footer_paragraphs(&model.header_paragraphs));
    }
    if !model.footer_paragraphs.is_empty() {
        content.push_str("\n## Footers\n\n");
        content.push_str(&render_header_footer_paragraphs(&model.footer_paragraphs));
    }
    budget(content, max_tokens, offset)
}

pub fn render_paragraphs_read(
    paragraphs: &[&ParagraphModel],
    max_tokens: usize,
    offset: usize,
) -> ReadResponse {
    budget(
        format!("# Document\n\n{}", render_body_paragraphs(paragraphs)),
        max_tokens,
        offset,
    )
}

pub fn render_headers_read(
    paragraphs: &[&HeaderFooterParagraphModel],
    max_tokens: usize,
    offset: usize,
) -> ReadResponse {
    budget(
        format!(
            "# Headers\n\n{}",
            render_header_footer_paragraphs_refs(paragraphs)
        ),
        max_tokens,
        offset,
    )
}

pub fn render_footers_read(
    paragraphs: &[&HeaderFooterParagraphModel],
    max_tokens: usize,
    offset: usize,
) -> ReadResponse {
    budget(
        format!(
            "# Footers\n\n{}",
            render_header_footer_paragraphs_refs(paragraphs)
        ),
        max_tokens,
        offset,
    )
}

fn render_body_paragraphs(paragraphs: &[&ParagraphModel]) -> String {
    let mut content = String::new();
    for paragraph in paragraphs {
        content.push_str(&format!("{}. {}\n", paragraph.index, paragraph.text));
    }
    content
}

fn render_header_footer_paragraphs(paragraphs: &[HeaderFooterParagraphModel]) -> String {
    render_header_footer_paragraphs_refs(&paragraphs.iter().collect::<Vec<_>>())
}

fn render_header_footer_paragraphs_refs(paragraphs: &[&HeaderFooterParagraphModel]) -> String {
    let mut content = String::new();
    for paragraph in paragraphs {
        content.push_str(&format!(
            "{}.{} {}\n",
            paragraph.part, paragraph.index, paragraph.text
        ));
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
