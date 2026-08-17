use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use dotall_core::{DotallError, Result};
use zip::ZipArchive;

use crate::FORMAT_ID;
use crate::model::column_name;
use crate::selector;

use super::workbook::PackagePatch;

/// Insert a legacy Excel Note on an existing cell (comments XML + VML + worksheet wiring).
pub(super) fn insert_comment(
    package: &[u8],
    sheet: &str,
    address: &str,
    text: &str,
    author: &str,
) -> Result<PackagePatch> {
    let worksheet_path = worksheet_path_for_sheet(package, sheet)?;
    let worksheet = entry_bytes(package, &worksheet_path)?;
    let rels_path = worksheet_rels_path(&worksheet_path);
    let existing_rels = optional_entry_bytes(package, &rels_path)?;
    let content_types = entry_bytes(package, "[Content_Types].xml")?;

    let parsed = selector::parse_cell_address(address).map_err(writer_error)?;
    let cell = format!("{}{}", column_name((parsed.col - 1) as usize), parsed.row);
    let zero_row = parsed.row.saturating_sub(1);
    let zero_col = parsed.col.saturating_sub(1);

    let mut replacements = BTreeMap::new();
    let mut additions = BTreeMap::new();

    let parts =
        resolve_or_create_comment_parts(package, &worksheet_path, existing_rels.as_deref())?;
    let comments_path = parts.comments_path;
    let comments_rid = parts.comments_rid;
    let vml_path = parts.vml_path;
    let vml_rid = parts.vml_rid;
    let rels_xml = parts.rels_xml;

    let comments_xml = if let Some(existing) = optional_entry_bytes(package, &comments_path)? {
        append_comment(&existing, &cell, text, author)?
    } else {
        render_comments_xml(&cell, text, author)
    };
    if has_entry(package, &comments_path)? {
        replacements.insert(comments_path.clone(), comments_xml);
    } else {
        additions.insert(comments_path.clone(), comments_xml);
    }

    let vml_xml = if let Some(existing) = optional_entry_bytes(package, &vml_path)? {
        append_vml_shape(&existing, zero_row, zero_col)?
    } else {
        render_vml(zero_row, zero_col)
    };
    if has_entry(package, &vml_path)? {
        replacements.insert(vml_path.clone(), vml_xml);
    } else {
        additions.insert(vml_path.clone(), vml_xml);
    }

    let patched_rels = ensure_comment_relationships(
        rels_xml.as_deref(),
        &comments_path,
        &comments_rid,
        &vml_path,
        &vml_rid,
        &worksheet_path,
    )?;
    if existing_rels.is_some() {
        replacements.insert(rels_path.clone(), patched_rels);
    } else {
        additions.insert(rels_path, patched_rels);
    }

    replacements.insert(
        worksheet_path.clone(),
        ensure_legacy_drawing(&worksheet, &vml_rid)?,
    );
    replacements.insert(
        "[Content_Types].xml".into(),
        ensure_content_types(&content_types, &comments_path)?,
    );

    Ok(PackagePatch {
        replacements,
        additions,
        removals: BTreeSet::new(),
    })
}

struct CommentParts {
    comments_path: String,
    comments_rid: String,
    vml_path: String,
    vml_rid: String,
    rels_xml: Option<Vec<u8>>,
}

fn resolve_or_create_comment_parts(
    package: &[u8],
    worksheet_path: &str,
    existing_rels: Option<&[u8]>,
) -> Result<CommentParts> {
    if let Some(rels) = existing_rels {
        let relationships = parse_relationships(rels)?;
        let comments = relationships
            .iter()
            .find(|rel| rel.kind.ends_with("/comments"));
        let vml = relationships
            .iter()
            .find(|rel| rel.kind.ends_with("/vmlDrawing"));
        if let (Some(comments), Some(vml)) = (comments, vml) {
            return Ok(CommentParts {
                comments_path: resolve_target(worksheet_path, &comments.target),
                comments_rid: comments.id.clone(),
                vml_path: resolve_target(worksheet_path, &vml.target),
                vml_rid: vml.id.clone(),
                rels_xml: Some(rels.to_vec()),
            });
        }
    }

    let comment_number = next_part_number(package, "xl/comments", ".xml")?;
    let vml_number = next_part_number(package, "xl/drawings/vmlDrawing", ".vml")?;
    let used_rids = existing_rels
        .map(parse_relationships)
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .filter_map(|rel| parse_rid_number(&rel.id))
        .collect::<BTreeSet<_>>();
    let vml_rid = format!("rId{}", lowest_unused(&used_rids));
    let mut used_rids = used_rids;
    used_rids.insert(parse_rid_number(&vml_rid).unwrap_or(1));
    let comments_rid = format!("rId{}", lowest_unused(&used_rids));

    Ok(CommentParts {
        comments_path: format!("xl/comments{comment_number}.xml"),
        comments_rid,
        vml_path: format!("xl/drawings/vmlDrawing{vml_number}.vml"),
        vml_rid,
        rels_xml: existing_rels.map(|bytes| bytes.to_vec()),
    })
}

fn ensure_comment_relationships(
    existing: Option<&[u8]>,
    comments_path: &str,
    comments_rid: &str,
    vml_path: &str,
    vml_rid: &str,
    worksheet_path: &str,
) -> Result<Vec<u8>> {
    let comments_target = relative_target(worksheet_path, comments_path);
    let vml_target = relative_target(worksheet_path, vml_path);
    let vml_rel = format!(
        r#"<Relationship Id="{vml_rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing" Target="{vml_target}"/>"#
    );
    let comments_rel = format!(
        r#"<Relationship Id="{comments_rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="{comments_target}"/>"#
    );

    let Some(existing) = existing else {
        return Ok(format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{vml_rel}{comments_rel}</Relationships>"#
        )
        .into_bytes());
    };

    let text = std::str::from_utf8(existing)
        .map_err(|error| writer_error(format!("worksheet rels are not UTF-8: {error}")))?;
    let mut output = text.to_owned();
    if !output.contains("/comments\"") && !output.contains("/comments'") {
        output = insert_before_close(&output, "Relationships", &comments_rel)?;
    }
    if !output.contains("/vmlDrawing\"") && !output.contains("/vmlDrawing'") {
        output = insert_before_close(&output, "Relationships", &vml_rel)?;
    }
    Ok(output.into_bytes())
}

fn ensure_legacy_drawing(worksheet: &[u8], vml_rid: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(worksheet)
        .map_err(|error| writer_error(format!("worksheet XML is not UTF-8: {error}")))?;
    if text.contains("<legacyDrawing") {
        return Ok(worksheet.to_vec());
    }
    let insertion = format!(r#"<legacyDrawing r:id="{vml_rid}"/>"#);
    insert_before_close(text, "worksheet", &insertion).map(String::into_bytes)
}

fn ensure_content_types(xml: &[u8], comments_path: &str) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(xml)
        .map_err(|error| writer_error(format!("content types XML is not UTF-8: {error}")))?;
    let mut output = text.to_owned();
    if !output.contains(r#"Extension="vml""#) {
        output = insert_before_close(
            &output,
            "Types",
            r#"<Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/>"#,
        )?;
    }
    let part_name = format!("/{comments_path}");
    if !output.contains(&format!(r#"PartName="{part_name}""#)) {
        output = insert_before_close(
            &output,
            "Types",
            &format!(
                r#"<Override PartName="{part_name}" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/>"#
            ),
        )?;
    }
    Ok(output.into_bytes())
}

fn render_comments_xml(cell: &str, text: &str, author: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author>{}</author></authors><commentList><comment ref="{}" authorId="0"><text><t>{}</t></text></comment></commentList></comments>"#,
        escape_xml(author),
        escape_xml(cell),
        escape_xml(text)
    )
    .into_bytes()
}

fn append_comment(existing: &[u8], cell: &str, text: &str, author: &str) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(existing)
        .map_err(|error| writer_error(format!("comments XML is not UTF-8: {error}")))?;
    let author_id = ensure_author(source, author)?;
    let mut with_author = if source.contains(&format!("<author>{}</author>", escape_xml(author)))
        || author_list_contains(source, author)
    {
        source.to_owned()
    } else {
        insert_before_close(
            source,
            "authors",
            &format!("<author>{}</author>", escape_xml(author)),
        )?
    };
    let comment = format!(
        r#"<comment ref="{}" authorId="{}"><text><t>{}</t></text></comment>"#,
        escape_xml(cell),
        author_id,
        escape_xml(text)
    );
    with_author = insert_before_close(&with_author, "commentList", &comment)?;
    Ok(with_author.into_bytes())
}

fn author_list_contains(source: &str, author: &str) -> bool {
    source
        .split("<author>")
        .skip(1)
        .any(|chunk| chunk.split("</author>").next() == Some(author))
}

fn ensure_author(source: &str, author: &str) -> Result<usize> {
    let mut authors = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find("<author>") {
        let start = cursor + relative + "<author>".len();
        let end = source[start..]
            .find("</author>")
            .map(|offset| start + offset)
            .ok_or_else(|| writer_error("unterminated author element"))?;
        authors.push(source[start..end].to_owned());
        cursor = end + "</author>".len();
    }
    if let Some(index) = authors.iter().position(|existing| existing == author) {
        return Ok(index);
    }
    Ok(authors.len())
}

fn render_vml(row: u32, col: u32) -> Vec<u8> {
    let anchor = format!(
        "{}, 15, {}, 10, {}, 15, {}, 4",
        col + 1,
        row,
        col + 3,
        row + 3
    );
    format!(
        r##"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="1"/></o:shapelayout><v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202" path="m,l,21600r21600,l21600,xe"><v:stroke joinstyle="miter"/><v:path gradientshapeok="t" o:connecttype="rect"/></v:shapetype><v:shape id="_x0000_s1025" type="#_x0000_t202" style="position:absolute;margin-left:107.25pt;margin-top:7.5pt;width:96pt;height:55.5pt;z-index:1;visibility:hidden" fillcolor="#ffffe1" o:insetmode="auto"><v:fill color2="#ffffe1"/><v:shadow on="t" color="black" obscured="t"/><v:path o:connecttype="none"/><v:textbox style="mso-direction-alt:auto"><div style="text-align:left"></div></v:textbox><x:ClientData ObjectType="Note"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>{anchor}</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>{row}</x:Row><x:Column>{col}</x:Column></x:ClientData></v:shape></xml>"##,
        anchor = anchor,
        row = row,
        col = col
    )
    .into_bytes()
}

fn append_vml_shape(existing: &[u8], row: u32, col: u32) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(existing)
        .map_err(|error| writer_error(format!("VML is not UTF-8: {error}")))?;
    let shape_id = next_vml_shape_id(source);
    let anchor = format!(
        "{}, 15, {}, 10, {}, 15, {}, 4",
        col + 1,
        row,
        col + 3,
        row + 3
    );
    let shape = format!(
        r##"<v:shape id="_x0000_s{shape_id}" type="#_x0000_t202" style="position:absolute;margin-left:107.25pt;margin-top:7.5pt;width:96pt;height:55.5pt;z-index:1;visibility:hidden" fillcolor="#ffffe1" o:insetmode="auto"><v:fill color2="#ffffe1"/><v:shadow on="t" color="black" obscured="t"/><v:path o:connecttype="none"/><v:textbox style="mso-direction-alt:auto"><div style="text-align:left"></div></v:textbox><x:ClientData ObjectType="Note"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>{anchor}</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>{row}</x:Row><x:Column>{col}</x:Column></x:ClientData></v:shape>"##,
        shape_id = shape_id,
        anchor = anchor,
        row = row,
        col = col
    );
    if let Some(pos) = source.rfind("</xml>") {
        Ok(format!("{}{}{}", &source[..pos], shape, &source[pos..]).into_bytes())
    } else {
        Err(writer_error("VML drawing is missing closing </xml>"))
    }
}

fn next_vml_shape_id(source: &str) -> u32 {
    let mut max = 1024_u32;
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find(r#"id="_x0000_s"#) {
        let start = cursor + relative + r#"id="_x0000_s"#.len();
        let end = source[start..]
            .find('"')
            .map(|offset| start + offset)
            .unwrap_or(start);
        if let Ok(value) = source[start..end].parse::<u32>() {
            max = max.max(value);
        }
        cursor = end;
    }
    max + 1
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
        path: "<xlsx comment writer>".into(),
        message: message.into(),
    }
}
