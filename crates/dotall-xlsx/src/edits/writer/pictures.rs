use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use dotall_core::{DotallError, Result};
use zip::ZipArchive;

use crate::FORMAT_ID;
use crate::selector;

use super::workbook::PackagePatch;

/// Insert a floating picture into a worksheet drawing (media + drawing + worksheet wiring).
pub(super) fn insert_picture(
    package: &[u8],
    sheet: &str,
    from_cell: &str,
    bytes: &[u8],
    content_type: &str,
) -> Result<PackagePatch> {
    if bytes.is_empty() {
        return Err(writer_error("insert_picture rejects empty image bytes"));
    }
    let extension = match content_type {
        "image/png" => "png",
        "image/jpeg" => "jpeg",
        other => {
            return Err(writer_error(format!(
                "insert_picture supports image/png and image/jpeg only, got `{other}`"
            )));
        }
    };

    let parsed = selector::parse_cell_address(from_cell).map_err(writer_error)?;
    let zero_row = parsed.row.saturating_sub(1);
    let zero_col = parsed.col.saturating_sub(1);

    let worksheet_path = worksheet_path_for_sheet(package, sheet)?;
    let worksheet = entry_bytes(package, &worksheet_path)?;
    let rels_path = worksheet_rels_path(&worksheet_path);
    let existing_rels = optional_entry_bytes(package, &rels_path)?;
    let content_types = entry_bytes(package, "[Content_Types].xml")?;

    let media_number = next_part_number(package, "xl/media/image", &format!(".{extension}"))?;
    let media_path = format!("xl/media/image{media_number}.{extension}");

    let mut replacements = BTreeMap::new();
    let mut additions = BTreeMap::new();
    additions.insert(media_path.clone(), bytes.to_vec());

    let drawing = resolve_or_create_drawing(package, &worksheet_path, existing_rels.as_deref())?;
    let picture_name = next_picture_name(package, &drawing.drawing_path)?;
    let picture_id = next_picture_nv_id(package, &drawing.drawing_path)?;

    if drawing.exists {
        let drawing_rels_path = worksheet_rels_path(&drawing.drawing_path);
        let existing_drawing_rels = optional_entry_bytes(package, &drawing_rels_path)?;
        let (patched_rels, rid) = append_image_relationship(
            existing_drawing_rels.as_deref(),
            &drawing.drawing_path,
            &media_path,
        )?;
        if existing_drawing_rels.is_some() {
            replacements.insert(drawing_rels_path, patched_rels);
        } else {
            additions.insert(drawing_rels_path, patched_rels);
        }

        let existing_drawing = entry_bytes(package, &drawing.drawing_path)?;
        let patched_drawing = append_one_cell_anchor(
            &existing_drawing,
            zero_col,
            zero_row,
            &rid,
            picture_id,
            &picture_name,
        )?;
        replacements.insert(drawing.drawing_path.clone(), patched_drawing);
    } else {
        let drawing_xml = render_drawing_xml(zero_col, zero_row, "rId1", picture_id, &picture_name);
        additions.insert(drawing.drawing_path.clone(), drawing_xml);
        let drawing_rels = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="{}"/></Relationships>"#,
            relative_target(&drawing.drawing_path, &media_path)
        );
        additions.insert(
            worksheet_rels_path(&drawing.drawing_path),
            drawing_rels.into_bytes(),
        );
    }

    let patched_sheet_rels = ensure_drawing_relationship(
        existing_rels.as_deref(),
        &drawing.drawing_path,
        &drawing.drawing_rid,
        &worksheet_path,
    )?;
    if existing_rels.is_some() {
        replacements.insert(rels_path.clone(), patched_sheet_rels);
    } else {
        additions.insert(rels_path, patched_sheet_rels);
    }

    replacements.insert(
        worksheet_path.clone(),
        ensure_drawing_element(&worksheet, &drawing.drawing_rid)?,
    );
    replacements.insert(
        "[Content_Types].xml".into(),
        ensure_content_types(
            &content_types,
            &drawing.drawing_path,
            extension,
            content_type,
        )?,
    );

    Ok(PackagePatch {
        replacements,
        additions,
        removals: BTreeSet::new(),
    })
}

struct DrawingParts {
    drawing_path: String,
    drawing_rid: String,
    exists: bool,
}

fn resolve_or_create_drawing(
    package: &[u8],
    worksheet_path: &str,
    existing_rels: Option<&[u8]>,
) -> Result<DrawingParts> {
    if let Some(rels) = existing_rels {
        let relationships = parse_relationships(rels)?;
        if let Some(drawing) = relationships
            .iter()
            .find(|rel| rel.kind.ends_with("/drawing"))
        {
            return Ok(DrawingParts {
                drawing_path: resolve_target(worksheet_path, &drawing.target),
                drawing_rid: drawing.id.clone(),
                exists: true,
            });
        }
    }

    let drawing_number = next_part_number(package, "xl/drawings/drawing", ".xml")?;
    let used_rids = existing_rels
        .map(parse_relationships)
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .filter_map(|rel| parse_rid_number(&rel.id))
        .collect::<BTreeSet<_>>();
    let drawing_rid = format!("rId{}", lowest_unused(&used_rids));
    Ok(DrawingParts {
        drawing_path: format!("xl/drawings/drawing{drawing_number}.xml"),
        drawing_rid,
        exists: false,
    })
}

fn ensure_drawing_relationship(
    existing: Option<&[u8]>,
    drawing_path: &str,
    drawing_rid: &str,
    worksheet_path: &str,
) -> Result<Vec<u8>> {
    let target = relative_target(worksheet_path, drawing_path);
    let drawing_rel = format!(
        r#"<Relationship Id="{drawing_rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="{target}"/>"#
    );
    let Some(existing) = existing else {
        return Ok(format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{drawing_rel}</Relationships>"#
        )
        .into_bytes());
    };
    let text = std::str::from_utf8(existing)
        .map_err(|error| writer_error(format!("worksheet rels are not UTF-8: {error}")))?;
    if text.contains("/drawing\"") || text.contains("/drawing'") {
        return Ok(existing.to_vec());
    }
    insert_before_close(text, "Relationships", &drawing_rel).map(String::into_bytes)
}

fn ensure_drawing_element(worksheet: &[u8], drawing_rid: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(worksheet)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    if text.contains("<drawing ") || text.contains("<drawing>") {
        return Ok(worksheet.to_vec());
    }
    let insertion = format!(r#"<drawing r:id="{drawing_rid}"/>"#);
    // Prefer before legacyDrawing so Wave 23 VML comments remain valid.
    if let Some(pos) = text.find("<legacyDrawing") {
        return Ok(format!("{}{}{}", &text[..pos], insertion, &text[pos..]).into_bytes());
    }
    insert_before_close(text, "worksheet", &insertion).map(String::into_bytes)
}

fn ensure_content_types(
    xml: &[u8],
    drawing_path: &str,
    extension: &str,
    content_type: &str,
) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("content types XML is not UTF-8: {error}")))?;
    let mut output = text.to_owned();
    let extension_attr = format!(r#"Extension="{extension}""#);
    if !output.contains(&extension_attr) {
        output = insert_before_close(
            &output,
            "Types",
            &format!(r#"<Default Extension="{extension}" ContentType="{content_type}"/>"#),
        )?;
    }
    let part_name = format!("/{drawing_path}");
    if !output.contains(&format!(r#"PartName="{part_name}""#)) {
        output = insert_before_close(
            &output,
            "Types",
            &format!(
                r#"<Override PartName="{part_name}" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>"#
            ),
        )?;
    }
    Ok(output.into_bytes())
}

fn append_image_relationship(
    existing: Option<&[u8]>,
    drawing_path: &str,
    media_path: &str,
) -> Result<(Vec<u8>, String)> {
    let target = relative_target(drawing_path, media_path);
    let used = existing
        .map(parse_relationships)
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .filter_map(|rel| parse_rid_number(&rel.id))
        .collect::<BTreeSet<_>>();
    let rid = format!("rId{}", lowest_unused(&used));
    let rel = format!(
        r#"<Relationship Id="{rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="{target}"/>"#
    );
    let Some(existing) = existing else {
        return Ok((
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rel}</Relationships>"#
            )
            .into_bytes(),
            rid,
        ));
    };
    let text = std::str::from_utf8(existing)
        .map_err(|error| writer_error(format!("drawing rels are not UTF-8: {error}")))?;
    let patched = insert_before_close(text, "Relationships", &rel)?;
    Ok((patched.into_bytes(), rid))
}

fn render_drawing_xml(
    col: u32,
    row: u32,
    embed_rid: &str,
    picture_id: u32,
    picture_name: &str,
) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">{}</xdr:wsDr>"#,
        one_cell_anchor_xml(col, row, embed_rid, picture_id, picture_name)
    )
    .into_bytes()
}

fn append_one_cell_anchor(
    existing: &[u8],
    col: u32,
    row: u32,
    embed_rid: &str,
    picture_id: u32,
    picture_name: &str,
) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(existing)
        .map_err(|error| writer_error(format!("drawing XML is not UTF-8: {error}")))?;
    let anchor = one_cell_anchor_xml(col, row, embed_rid, picture_id, picture_name);
    if let Some(pos) = text.rfind("</xdr:wsDr>") {
        Ok(format!("{}{}{}", &text[..pos], anchor, &text[pos..]).into_bytes())
    } else {
        Err(writer_error("drawing XML is missing closing </xdr:wsDr>"))
    }
}

fn one_cell_anchor_xml(
    col: u32,
    row: u32,
    embed_rid: &str,
    picture_id: u32,
    picture_name: &str,
) -> String {
    format!(
        r#"<xdr:oneCellAnchor><xdr:from><xdr:col>{col}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{row}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx="914400" cy="914400"/><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="{picture_id}" name="{picture_name}"/><xdr:cNvPicPr><a:picLocks noChangeAspect="1"/></xdr:cNvPicPr></xdr:nvPicPr><xdr:blipFill><a:blip xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:embed="{embed_rid}"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic><xdr:clientData/></xdr:oneCellAnchor>"#,
        col = col,
        row = row,
        picture_id = picture_id,
        picture_name = escape_xml(picture_name),
        embed_rid = embed_rid
    )
}

fn next_picture_name(package: &[u8], drawing_path: &str) -> Result<String> {
    let mut max = 0_u32;
    if let Some(existing) = optional_entry_bytes(package, drawing_path)? {
        let text = std::str::from_utf8(&existing).unwrap_or("");
        let mut cursor = 0;
        while let Some(relative) = text[cursor..].find("name=\"Picture ") {
            let start = cursor + relative + "name=\"Picture ".len();
            let end = text[start..]
                .find('"')
                .map(|offset| start + offset)
                .unwrap_or(start);
            if let Ok(value) = text[start..end].parse::<u32>() {
                max = max.max(value);
            }
            cursor = end;
        }
    }
    Ok(format!("Picture {}", max + 1))
}

fn next_picture_nv_id(package: &[u8], drawing_path: &str) -> Result<u32> {
    let mut max = 1_u32;
    if let Some(existing) = optional_entry_bytes(package, drawing_path)? {
        let text = std::str::from_utf8(&existing).unwrap_or("");
        let mut cursor = 0;
        while let Some(relative) = text[cursor..].find("<xdr:cNvPr ") {
            let start = cursor + relative;
            let end = text[start..]
                .find('>')
                .map(|offset| start + offset)
                .unwrap_or(start);
            let tag = &text[start..=end];
            if let Some(id) = tag_attribute(tag, "id").and_then(|value| value.parse().ok()) {
                max = max.max(id);
            }
            cursor = end + 1;
        }
    }
    Ok(max + 1)
}

fn worksheet_path_for_sheet(package: &[u8], sheet: &str) -> Result<String> {
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    let relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels")?;
    let sheets = parse_workbook_sheets(&workbook)?;
    let rels = parse_relationships(&relationships)?;
    let relationship_id = sheets
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(sheet))
        .map(|(_, id)| id)
        .ok_or_else(|| writer_error(format!("worksheet `{sheet}` was not found")))?;
    let target = rels
        .into_iter()
        .find(|rel| rel.id == relationship_id)
        .map(|rel| rel.target)
        .ok_or_else(|| {
            writer_error(format!("missing workbook relationship `{relationship_id}`"))
        })?;
    Ok(normalize_workbook_target(&target))
}

fn parse_workbook_sheets(xml: &[u8]) -> Result<Vec<(String, String)>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("workbook XML is not UTF-8: {error}")))?;
    let mut sheets = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find("<sheet ") {
        let start = cursor + relative;
        let end = text[start..]
            .find('>')
            .map(|offset| start + offset)
            .ok_or_else(|| writer_error("unterminated workbook sheet tag"))?;
        let tag = &text[start..=end];
        let name = tag_attribute(tag, "name")
            .ok_or_else(|| writer_error("workbook sheet is missing name"))?;
        let id = tag_attribute(tag, "r:id")
            .or_else(|| tag_attribute(tag, "id"))
            .ok_or_else(|| writer_error("workbook sheet is missing r:id"))?;
        sheets.push((unescape_xml(&name)?, id));
        cursor = end + 1;
    }
    Ok(sheets)
}

#[derive(Clone, Debug)]
struct Relationship {
    id: String,
    kind: String,
    target: String,
}

fn parse_relationships(xml: &[u8]) -> Result<Vec<Relationship>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("relationships XML is not UTF-8: {error}")))?;
    let mut relationships = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = text[cursor..].find("<Relationship ") {
        let start = cursor + relative;
        let end = text[start..]
            .find('>')
            .map(|offset| start + offset)
            .ok_or_else(|| writer_error("unterminated Relationship tag"))?;
        let tag = &text[start..=end];
        let id =
            tag_attribute(tag, "Id").ok_or_else(|| writer_error("Relationship is missing Id"))?;
        let kind = tag_attribute(tag, "Type")
            .ok_or_else(|| writer_error("Relationship is missing Type"))?;
        let target = tag_attribute(tag, "Target")
            .ok_or_else(|| writer_error("Relationship is missing Target"))?;
        relationships.push(Relationship {
            id: unescape_xml(&id)?,
            kind: unescape_xml(&kind)?,
            target: unescape_xml(&target)?,
        });
        cursor = end + 1;
    }
    Ok(relationships)
}

fn next_part_number(package: &[u8], prefix: &str, suffix: &str) -> Result<u32> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| writer_error(format!("invalid XLSX package: {error}")))?;
    let mut used = BTreeSet::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| writer_error(format!("cannot read ZIP entry: {error}")))?;
        let name = entry.name();
        if let Some(number) = name
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(suffix))
            .and_then(|digits| digits.parse::<u32>().ok())
        {
            used.insert(number);
        }
    }
    Ok(lowest_unused(&used))
}

fn lowest_unused(used: &BTreeSet<u32>) -> u32 {
    let mut candidate = 1;
    while used.contains(&candidate) {
        candidate += 1;
    }
    candidate
}

fn parse_rid_number(id: &str) -> Option<u32> {
    id.strip_prefix("rId")?.parse().ok()
}

fn worksheet_rels_path(part_path: &str) -> String {
    match part_path.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part_path}.rels"),
    }
}

fn normalize_workbook_target(target: &str) -> String {
    if target.starts_with('/') {
        target.trim_start_matches('/').to_owned()
    } else {
        format!("xl/{target}")
    }
}

fn resolve_target(base_part: &str, target: &str) -> String {
    if target.starts_with('/') {
        return target.trim_start_matches('/').to_owned();
    }
    let base_dir = base_part.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let mut components: Vec<&str> = base_dir
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    components.join("/")
}

fn relative_target(from_part: &str, to_part: &str) -> String {
    let from_dir = from_part.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let from_components: Vec<&str> = from_dir
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let to_components: Vec<&str> = to_part.split('/').filter(|part| !part.is_empty()).collect();
    let mut shared = 0;
    while shared < from_components.len()
        && shared < to_components.len()
        && from_components[shared] == to_components[shared]
    {
        shared += 1;
    }
    let mut relative = vec![".."; from_components.len() - shared];
    relative.extend(to_components[shared..].iter().copied());
    relative.join("/")
}

fn insert_before_close(text: &str, element: &str, insertion: &str) -> Result<String> {
    let closing = format!("</{element}>");
    let position = text
        .rfind(&closing)
        .ok_or_else(|| writer_error(format!("XML is missing closing `{element}`")))?;
    Ok(format!(
        "{}{}{}",
        &text[..position],
        insertion,
        &text[position..]
    ))
}

fn tag_attribute(tag: &str, attribute: &str) -> Option<String> {
    let needle = format!("{attribute}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_owned())
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn unescape_xml(value: &str) -> Result<String> {
    quick_xml::escape::unescape(value)
        .map(|value| value.into_owned())
        .map_err(|error| writer_error(format!("invalid XML value: {error}")))
}

fn has_entry(package: &[u8], name: &str) -> Result<bool> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| writer_error(format!("invalid XLSX package: {error}")))?;
    Ok(archive.by_name(name).is_ok())
}

fn entry_bytes(package: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| writer_error(format!("invalid XLSX package: {error}")))?;
    let mut entry = archive
        .by_name(name)
        .map_err(|error| writer_error(format!("XLSX package is missing `{name}`: {error}")))?;
    let mut bytes = Vec::new();
    entry
        .read_to_end(&mut bytes)
        .map_err(|error| writer_error(format!("cannot read `{name}`: {error}")))?;
    Ok(bytes)
}

fn optional_entry_bytes(package: &[u8], name: &str) -> Result<Option<Vec<u8>>> {
    if has_entry(package, name)? {
        Ok(Some(entry_bytes(package, name)?))
    } else {
        Ok(None)
    }
}

fn writer_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<xlsx picture writer>".into(),
        message: message.into(),
    }
}
