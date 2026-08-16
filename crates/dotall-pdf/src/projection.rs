use crate::model::PdfDocumentModel;
use dotall_core::read::apply_budget;
use dotall_core::registry::ReadResponse;

pub fn render_full(model: &PdfDocumentModel, max_tokens: usize, offset: usize) -> ReadResponse {
    let mut content = format!("# PDF\n\nPages: {}\n\n", model.page_count);
    for page in &model.pages {
        content.push_str(&format!("## Page {}\n\n{}\n\n", page.number, page.text));
    }
    content.push_str("## Fields\n\n");
    for field in &model.fields {
        content.push_str(&format!(
            "- `{}` ({}) = {}\n",
            field.name, field.field_type, field.value
        ));
    }
    budget(content, max_tokens, offset)
}

pub fn render_page(text: &str, number: u32, max_tokens: usize, offset: usize) -> ReadResponse {
    budget(format!("## Page {number}\n\n{text}\n"), max_tokens, offset)
}

pub fn render_field(
    name: &str,
    field_type: &str,
    value: &str,
    max_tokens: usize,
    offset: usize,
) -> ReadResponse {
    budget(
        format!("`{name}` ({field_type}) = {value}\n"),
        max_tokens,
        offset,
    )
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
