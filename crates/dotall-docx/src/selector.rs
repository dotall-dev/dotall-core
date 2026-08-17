use crate::model::{DocumentModel, HeaderFooterParagraphModel, ParagraphModel};

pub fn resolve_range<'a>(
    model: &'a DocumentModel,
    value: &str,
) -> Result<Vec<&'a ParagraphModel>, String> {
    resolve_slice(&model.paragraphs, value, "paragraph")
}

pub fn resolve_header_range<'a>(
    model: &'a DocumentModel,
    value: &str,
) -> Result<Vec<&'a HeaderFooterParagraphModel>, String> {
    resolve_header_footer(&model.header_paragraphs, value, "header")
}

pub fn resolve_footer_range<'a>(
    model: &'a DocumentModel,
    value: &str,
) -> Result<Vec<&'a HeaderFooterParagraphModel>, String> {
    resolve_header_footer(&model.footer_paragraphs, value, "footer")
}

fn resolve_header_footer<'a>(
    paragraphs: &'a [HeaderFooterParagraphModel],
    value: &str,
    label: &str,
) -> Result<Vec<&'a HeaderFooterParagraphModel>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(paragraphs.iter().collect());
    }
    // `header1:0` or `header1:0:2` — part-qualified within-part indices.
    if let Some((part, rest)) = trimmed.split_once(':')
        && part.chars().any(|ch| !ch.is_ascii_digit())
    {
        let part_paragraphs: Vec<_> = paragraphs
            .iter()
            .filter(|paragraph| paragraph.part == part)
            .collect();
        if part_paragraphs.is_empty() {
            return Err(format!("{label} part `{part}` was not found"));
        }
        let (start, end) = parse_range(rest)?;
        let selected: Vec<_> = part_paragraphs
            .into_iter()
            .filter(|paragraph| paragraph.index >= start && paragraph.index < end)
            .collect();
        if selected.is_empty() {
            return Err(format!("{label} `{part}:{start}` was not found"));
        }
        return Ok(selected);
    }
    // Flat list order: sorted part name, then within-part index.
    resolve_slice(paragraphs, trimmed, label)
}

fn resolve_slice<'a, T>(items: &'a [T], value: &str, label: &str) -> Result<Vec<&'a T>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(items.iter().collect());
    }
    let (start, end) = parse_range(trimmed)?;
    if start >= items.len() as u32 {
        return Err(format!("{label} `{start}` was not found"));
    }
    let end = end.min(items.len() as u32);
    Ok(items[start as usize..end as usize].iter().collect())
}

fn parse_range(raw: &str) -> Result<(u32, u32), String> {
    if let Some((left, right)) = raw.split_once(':') {
        let start = parse_index(left)?;
        let end = parse_index(right)?;
        if end < start {
            return Err(format!("paragraph range `{raw}` is inverted"));
        }
        Ok((start, end))
    } else {
        let index = parse_index(raw)?;
        Ok((index, index.saturating_add(1)))
    }
}

fn parse_index(raw: &str) -> Result<u32, String> {
    raw.trim()
        .parse()
        .map_err(|_| format!("invalid paragraph index `{raw}`"))
}
