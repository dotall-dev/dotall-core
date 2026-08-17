use crate::model::{PdfDocumentModel, PdfFieldModel};
use dotall_core::read::apply_budget;
use dotall_core::registry::ReadResponse;

pub fn render_full(model: &PdfDocumentModel, max_tokens: usize, offset: usize) -> ReadResponse {
    let mut content = format!("# PDF\n\nPages: {}\n\n", model.page_count);
    if let Some(title) = &model.metadata.title {
        content.push_str(&format!("Title: {title}\n"));
    }
    if let Some(author) = &model.metadata.author {
        content.push_str(&format!("Author: {author}\n"));
    }
    if model.metadata.title.is_some() || model.metadata.author.is_some() {
        content.push('\n');
    }
    for page in &model.pages {
        content.push_str(&format!("## Page {}\n\n{}\n\n", page.number, page.text));
    }
    content.push_str("## Fields\n\n");
    for field in &model.fields {
        content.push_str(&format!(
            "- `{}` ({}) = {}",
            field.name, field.field_type, field.value
        ));
        if !field.options.is_empty() {
            content.push_str(&format!(" options=[{}]", field.options.join(", ")));
        }
        content.push('\n');
    }
    budget(content, max_tokens, offset)
}

pub fn render_page(text: &str, number: u32, max_tokens: usize, offset: usize) -> ReadResponse {
    budget(format!("## Page {number}\n\n{text}\n"), max_tokens, offset)
}

pub fn render_field(field: &PdfFieldModel, max_tokens: usize, offset: usize) -> ReadResponse {
    let mut content = format!(
        "`{}` ({}) = {}\n",
        field.name, field.field_type, field.value
    );
    if !field.options.is_empty() {
        content.push_str(&format!("options: {}\n", field.options.join(", ")));
    }
    if !field.export_values.is_empty() {
        content.push_str(&format!(
            "export_values: {}\n",
            field.export_values.join(", ")
        ));
    }
    budget(content, max_tokens, offset)
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
