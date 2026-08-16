use crate::model::{DocumentModel, ParagraphModel};

pub fn resolve_range<'a>(
    model: &'a DocumentModel,
    value: &str,
) -> Result<Vec<&'a ParagraphModel>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(model.paragraphs.iter().collect());
    }
    let (start, end) = if let Some((left, right)) = trimmed.split_once(':') {
        let start = parse_index(left)?;
        let end = parse_index(right)?;
        if end < start {
            return Err(format!("paragraph range `{trimmed}` is inverted"));
        }
        (start, end)
    } else {
        let index = parse_index(trimmed)?;
        (index, index.saturating_add(1))
    };
    if start >= model.paragraphs.len() as u32 {
        return Err(format!("paragraph `{start}` was not found"));
    }
    let end = end.min(model.paragraphs.len() as u32);
    Ok(model.paragraphs[start as usize..end as usize]
        .iter()
        .collect())
}

fn parse_index(raw: &str) -> Result<u32, String> {
    raw.trim()
        .parse()
        .map_err(|_| format!("invalid paragraph index `{raw}`"))
}
