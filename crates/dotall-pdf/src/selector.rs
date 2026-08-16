use crate::model::{PdfDocumentModel, PdfFieldModel, PdfPageModel};

pub fn resolve_page<'a>(
    model: &'a PdfDocumentModel,
    value: &str,
) -> Result<&'a PdfPageModel, String> {
    let number: u32 = value
        .trim()
        .parse()
        .map_err(|_| format!("invalid page `{value}`"))?;
    model
        .pages
        .iter()
        .find(|page| page.number == number)
        .ok_or_else(|| format!("page `{number}` was not found"))
}

pub fn resolve_field<'a>(
    model: &'a PdfDocumentModel,
    value: &str,
) -> Result<&'a PdfFieldModel, String> {
    model
        .fields
        .iter()
        .find(|field| field.name == value.trim() || field.element_id == value.trim())
        .ok_or_else(|| format!("field `{value}` was not found"))
}
