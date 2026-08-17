use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::Path;

use dotall_core::{DotallError, Result};
use quick_xml::Reader;
use quick_xml::events::Event;
use zip::ZipArchive;

use crate::FORMAT_ID;
use crate::ids;
use crate::model::{
    ChartModel, CommentModel, PictureModel, PresentationModel, SCHEMA_VERSION, ShapeModel,
    SlideModel, TableCellModel, TableModel,
};

pub fn parse_presentation(source: &Path) -> Result<PresentationModel> {
    let bytes = std::fs::read(source).map_err(|error| DotallError::Io {
        path: source.to_path_buf(),
        source: error,
    })?;
    parse_presentation_bytes(&bytes)
}

pub fn parse_presentation_bytes(package: &[u8]) -> Result<PresentationModel> {
    let source_hash = blake3::hash(package).to_hex().to_string();
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| format_error(format!("invalid PPTX package: {error}")))?;
    let presentation_xml = zip_entry(&mut archive, "ppt/presentation.xml")?;
    let rels_xml = zip_entry(&mut archive, "ppt/_rels/presentation.xml.rels")?;
    let rids = slide_rids(&presentation_xml)?;
    let targets = relationship_targets(&rels_xml, "/slide")?;
    let authors = parse_comment_authors(&mut archive);

    let mut slides = Vec::new();
    let mut comments = Vec::new();
    let mut charts = Vec::new();
    let mut pictures = Vec::new();
    for (index, rid) in rids.into_iter().enumerate() {
        let target = targets
            .get(&rid)
            .ok_or_else(|| format_error(format!("presentation relationship `{rid}` is missing")))?;
        let part_name = resolve_ppt_target(target);
        let slide_xml = zip_entry(&mut archive, &part_name)?;
        let name = format!("Slide {}", index + 1);
        let (shapes, tables) = parse_shapes_and_tables(&slide_xml, &name)?;
        let (notes, notes_part_name) = resolve_notes(&mut archive, &part_name);
        let slide_rels = slide_relationship_part(&part_name);
        let slide_rel_targets = zip_entry(&mut archive, &slide_rels)
            .ok()
            .and_then(|xml| relationship_targets_all(&xml).ok())
            .unwrap_or_default();

        comments.extend(parse_slide_comments(
            &mut archive,
            &name,
            &slide_rel_targets,
            &authors,
        ));
        charts.extend(parse_slide_charts(
            &mut archive,
            &name,
            &slide_xml,
            &slide_rel_targets,
        ));
        pictures.extend(parse_slide_pictures(&name, &slide_xml, &slide_rel_targets));

        slides.push(SlideModel {
            element_id: ids::slide_id(&name, index as u32, SCHEMA_VERSION),
            name,
            index: index as u32,
            part_name,
            shapes,
            tables,
            notes,
            notes_part_name,
        });
    }

    Ok(PresentationModel {
        presentation_id: ids::presentation_id(&source_hash, SCHEMA_VERSION),
        slides,
        media_parts: media_part_names(&mut archive),
        comments,
        charts,
        pictures,
    })
}

fn media_part_names(archive: &mut ZipArchive<Cursor<&[u8]>>) -> Vec<String> {
    let mut names = Vec::new();
    for index in 0..archive.len() {
        if let Ok(entry) = archive.by_index(index) {
            let name = entry.name().to_owned();
            if name.starts_with("ppt/media/") && !name.ends_with('/') {
                names.push(name);
            }
        }
    }
    names.sort();
    names
}

fn zip_entry(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>> {
    let mut entry = archive
        .by_name(name)
        .map_err(|error| format_error(format!("missing `{name}`: {error}")))?;
    let mut bytes = Vec::new();
    entry
        .read_to_end(&mut bytes)
        .map_err(|error| format_error(format!("cannot read `{name}`: {error}")))?;
    Ok(bytes)
}

fn resolve_ppt_target(target: &str) -> String {
    if target.starts_with("ppt/") || target.starts_with('/') {
        target.trim_start_matches('/').to_owned()
    } else {
        format!("ppt/{target}")
    }
}

/// Resolve notes for a slide via its relationships, then fall back to the part-number
/// heuristic (`slideN.xml` → `notesSlideN.xml`). Never key notes by presentation index —
/// reorder (`move_slide`) changes order without renaming parts.
fn resolve_notes(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    slide_part: &str,
) -> (Option<String>, Option<String>) {
    let mut candidates = Vec::new();
    let slide_rels = slide_relationship_part(slide_part);
    if let Ok(rels_xml) = zip_entry(archive, &slide_rels) {
        candidates.extend(notes_parts_from_slide_rels(&rels_xml));
    }
    if let Some(number) = slide_part
        .strip_prefix("ppt/slides/slide")
        .and_then(|name| name.strip_suffix(".xml"))
    {
        candidates.push(format!("ppt/notesSlides/notesSlide{number}.xml"));
    }
    candidates.sort();
    candidates.dedup();
    for notes_part in candidates {
        if let Ok(xml) = zip_entry(archive, &notes_part) {
            let text = collect_text(&xml);
            return ((!text.is_empty()).then_some(text), Some(notes_part));
        }
    }
    (None, None)
}

fn slide_relationship_part(slide_part: &str) -> String {
    if let Some((dir, file)) = slide_part.rsplit_once('/') {
        format!("{dir}/_rels/{file}.rels")
    } else {
        format!("_rels/{slide_part}.rels")
    }
}

fn notes_parts_from_slide_rels(xml: &[u8]) -> Vec<String> {
    let Ok(targets) = relationship_targets(xml, "notesSlide") else {
        return Vec::new();
    };
    targets
        .into_values()
        .map(|target| resolve_slide_relative_target(&target))
        .collect()
}

fn slide_rids(xml: &[u8]) -> Result<Vec<String>> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut rids = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(format!("invalid presentation XML: {error}")))?
        {
            Event::Empty(tag) | Event::Start(tag) if tag.local_name().as_ref() == b"sldId" => {
                for attribute in tag.attributes() {
                    let attribute = attribute.map_err(|error| {
                        format_error(format!("invalid sldId attribute: {error}"))
                    })?;
                    let value = String::from_utf8_lossy(&attribute.value).into_owned();
                    if value.starts_with("rId") {
                        rids.push(value);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(rids)
}

fn relationship_targets(xml: &[u8], type_suffix: &str) -> Result<BTreeMap<String, String>> {
    let all = relationship_targets_all(xml)?;
    Ok(all
        .into_iter()
        .filter(|(_, (_, rel_type))| rel_type.ends_with(type_suffix))
        .map(|(id, (target, _))| (id, target))
        .collect())
}

fn relationship_targets_all(xml: &[u8]) -> Result<BTreeMap<String, (String, String)>> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut targets = BTreeMap::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(format!("invalid presentation rels: {error}")))?
        {
            Event::Empty(tag) | Event::Start(tag)
                if tag.local_name().as_ref() == b"Relationship" =>
            {
                let mut id = None;
                let mut target = None;
                let mut rel_type = String::new();
                for attribute in tag.attributes() {
                    let attribute = attribute
                        .map_err(|error| format_error(format!("invalid Relationship: {error}")))?;
                    let key = attribute.key.local_name();
                    let value = String::from_utf8_lossy(&attribute.value).into_owned();
                    match key.as_ref() {
                        b"Id" => id = Some(value),
                        b"Target" => target = Some(value),
                        b"Type" => rel_type = value,
                        _ => {}
                    }
                }
                if let (Some(id), Some(target)) = (id, target) {
                    targets.insert(id, (target, rel_type));
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(targets)
}

fn resolve_slide_relative_target(target: &str) -> String {
    if let Some(rest) = target.strip_prefix("../") {
        format!("ppt/{rest}")
    } else {
        resolve_ppt_target(target)
    }
}

fn parse_comment_authors(archive: &mut ZipArchive<Cursor<&[u8]>>) -> BTreeMap<u32, String> {
    let Ok(xml) = zip_entry(archive, "ppt/commentAuthors.xml") else {
        return BTreeMap::new();
    };
    let mut reader = Reader::from_reader(xml.as_slice());
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut authors = BTreeMap::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Empty(tag) | Event::Start(tag))
                if tag.local_name().as_ref() == b"cmAuthor" =>
            {
                let mut id = None;
                let mut name = None;
                for attribute in tag.attributes().flatten() {
                    let value = String::from_utf8_lossy(&attribute.value).into_owned();
                    match attribute.key.local_name().as_ref() {
                        b"id" => id = value.parse::<u32>().ok(),
                        b"name" => name = Some(value),
                        _ => {}
                    }
                }
                if let (Some(id), Some(name)) = (id, name) {
                    authors.insert(id, name);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buffer.clear();
    }
    authors
}

fn parse_slide_comments(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    slide_name: &str,
    slide_rel_targets: &BTreeMap<String, (String, String)>,
    authors: &BTreeMap<u32, String>,
) -> Vec<CommentModel> {
    let mut comments = Vec::new();
    for (target, rel_type) in slide_rel_targets.values() {
        if !rel_type.ends_with("/comments") {
            continue;
        }
        let part_name = resolve_slide_relative_target(target);
        let Ok(xml) = zip_entry(archive, &part_name) else {
            continue;
        };
        comments.extend(parse_comment_list(&xml, slide_name, authors));
    }
    comments
}

fn parse_comment_list(
    xml: &[u8],
    slide_name: &str,
    authors: &BTreeMap<u32, String>,
) -> Vec<CommentModel> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut comments = Vec::new();
    let mut in_cm = false;
    let mut author_id = 0u32;
    let mut idx = 0u32;
    let mut text = String::new();
    let mut shape = None;
    let mut depth = 0usize;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(tag)) if tag.local_name().as_ref() == b"cm" => {
                in_cm = true;
                depth = 1;
                author_id = 0;
                idx = 0;
                text.clear();
                shape = None;
                for attribute in tag.attributes().flatten() {
                    let value = String::from_utf8_lossy(&attribute.value).into_owned();
                    match attribute.key.local_name().as_ref() {
                        b"authorId" => {
                            if let Ok(parsed) = value.parse::<u32>() {
                                author_id = parsed;
                            }
                        }
                        b"idx" => {
                            if let Ok(parsed) = value.parse::<u32>() {
                                idx = parsed;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::Start(tag)) if in_cm && tag.local_name().as_ref() == b"text" => {
                if let Ok(value) = reader.read_text(tag.name()) {
                    text = decode_text(value);
                }
            }
            Ok(Event::Empty(tag) | Event::Start(tag))
                if in_cm && tag.local_name().as_ref() == b"shapeAnchor" =>
            {
                for attribute in tag.attributes().flatten() {
                    if attribute.key.local_name().as_ref() == b"name" {
                        shape = Some(String::from_utf8_lossy(&attribute.value).into_owned());
                    }
                }
            }
            Ok(Event::Start(_)) if in_cm => {
                depth += 1;
            }
            Ok(Event::End(tag)) if in_cm && tag.local_name().as_ref() == b"cm" => {
                let author = authors
                    .get(&author_id)
                    .cloned()
                    .unwrap_or_else(|| format!("author:{author_id}"));
                comments.push(CommentModel {
                    element_id: ids::comment_id(slide_name, &author, &text, idx, SCHEMA_VERSION),
                    slide: slide_name.to_owned(),
                    shape: shape.take(),
                    author,
                    text: text.clone(),
                });
                in_cm = false;
                depth = 0;
            }
            Ok(Event::End(_)) if in_cm => {
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buffer.clear();
    }
    let _ = depth;
    comments
}

fn parse_slide_charts(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    slide_name: &str,
    slide_xml: &[u8],
    slide_rel_targets: &BTreeMap<String, (String, String)>,
) -> Vec<ChartModel> {
    let chart_rids = chart_relationship_ids(slide_xml);
    let mut charts = Vec::new();
    for rid in chart_rids {
        let Some((target, rel_type)) = slide_rel_targets.get(&rid) else {
            continue;
        };
        if !rel_type.ends_with("/chart") {
            continue;
        }
        let part_name = resolve_slide_relative_target(target);
        let title = zip_entry(archive, &part_name)
            .map(|xml| chart_title(&xml))
            .unwrap_or_default();
        charts.push(ChartModel {
            element_id: ids::chart_id(slide_name, &part_name, SCHEMA_VERSION),
            slide: slide_name.to_owned(),
            title,
        });
    }
    charts
}

fn chart_relationship_ids(slide_xml: &[u8]) -> Vec<String> {
    let mut reader = Reader::from_reader(slide_xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut rids = Vec::new();
    let mut in_chart_graphic = false;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(tag)) if tag.local_name().as_ref() == b"graphicData" => {
                in_chart_graphic = tag.attributes().flatten().any(|attribute| {
                    attribute.key.local_name().as_ref() == b"uri"
                        && String::from_utf8_lossy(&attribute.value).contains("/chart")
                });
            }
            Ok(Event::End(tag)) if tag.local_name().as_ref() == b"graphicData" => {
                in_chart_graphic = false;
            }
            Ok(Event::Empty(tag) | Event::Start(tag))
                if in_chart_graphic && tag.local_name().as_ref() == b"chart" =>
            {
                for attribute in tag.attributes().flatten() {
                    if attribute.key.local_name().as_ref() == b"id" {
                        rids.push(String::from_utf8_lossy(&attribute.value).into_owned());
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buffer.clear();
    }
    rids
}

fn chart_title(xml: &[u8]) -> String {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut in_title = false;
    let mut texts = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(tag)) if tag.local_name().as_ref() == b"title" => {
                in_title = true;
            }
            Ok(Event::Start(tag)) if in_title && tag.local_name().as_ref() == b"t" => {
                if let Ok(text) = reader.read_text(tag.name()) {
                    texts.push(decode_text(text));
                }
            }
            Ok(Event::End(tag)) if in_title && tag.local_name().as_ref() == b"title" => {
                break;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buffer.clear();
    }
    texts.concat()
}

fn parse_slide_pictures(
    slide_name: &str,
    slide_xml: &[u8],
    slide_rel_targets: &BTreeMap<String, (String, String)>,
) -> Vec<PictureModel> {
    let mut reader = Reader::from_reader(slide_xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut pictures = Vec::new();
    let mut in_pic = false;
    let mut name = String::new();
    let mut embed_rid = None;
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(tag)) if tag.local_name().as_ref() == b"pic" => {
                in_pic = true;
                name.clear();
                embed_rid = None;
            }
            Ok(Event::Empty(tag) | Event::Start(tag))
                if in_pic && tag.local_name().as_ref() == b"cNvPr" =>
            {
                for attribute in tag.attributes().flatten() {
                    if attribute.key.local_name().as_ref() == b"name" {
                        name = String::from_utf8_lossy(&attribute.value).into_owned();
                    }
                }
            }
            Ok(Event::Empty(tag) | Event::Start(tag))
                if in_pic && tag.local_name().as_ref() == b"blip" =>
            {
                for attribute in tag.attributes().flatten() {
                    if attribute.key.local_name().as_ref() == b"embed" {
                        embed_rid = Some(String::from_utf8_lossy(&attribute.value).into_owned());
                    }
                }
            }
            Ok(Event::End(tag)) if in_pic && tag.local_name().as_ref() == b"pic" => {
                if let Some(rid) = embed_rid.take()
                    && let Some((target, rel_type)) = slide_rel_targets.get(&rid)
                    && rel_type.ends_with("/image")
                {
                    let part = resolve_slide_relative_target(target);
                    let display_name = if name.is_empty() {
                        format!("Picture {}", pictures.len() + 1)
                    } else {
                        name.clone()
                    };
                    pictures.push(PictureModel {
                        element_id: ids::picture_id(
                            slide_name,
                            &display_name,
                            &part,
                            SCHEMA_VERSION,
                        ),
                        slide: slide_name.to_owned(),
                        name: display_name,
                        part,
                    });
                }
                in_pic = false;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buffer.clear();
    }
    pictures
}

fn parse_shapes_and_tables(
    xml: &[u8],
    slide_name: &str,
) -> Result<(Vec<ShapeModel>, Vec<TableModel>)> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut shapes = Vec::new();
    let mut tables = Vec::new();

    let mut in_sp = false;
    let mut skipped_frame = false;
    let mut shape_name = String::new();
    let mut shape_texts = Vec::new();

    let mut in_graphic_frame = false;
    let mut frame_name = String::new();
    let mut in_tbl = false;
    let mut table_cells: Vec<(u32, u32, String)> = Vec::new();
    let mut row: i32 = -1;
    let mut col: i32 = -1;
    let mut in_tc = false;
    let mut cell_texts: Vec<String> = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| format_error(format!("invalid slide XML: {error}")))?
        {
            Event::Start(tag) if tag.local_name().as_ref() == b"graphicFrame" => {
                skipped_frame = true;
                in_graphic_frame = true;
                frame_name.clear();
                in_tbl = false;
                table_cells.clear();
                row = -1;
                col = -1;
                in_tc = false;
                cell_texts.clear();
            }
            Event::End(tag) if tag.local_name().as_ref() == b"graphicFrame" => {
                if in_tbl {
                    let name = if frame_name.is_empty() {
                        format!("table:{}", tables.len())
                    } else {
                        frame_name.clone()
                    };
                    let cells = table_cells
                        .iter()
                        .map(|(cell_row, cell_col, text)| TableCellModel {
                            element_id: ids::table_cell_id(
                                slide_name,
                                &name,
                                *cell_row,
                                *cell_col,
                                SCHEMA_VERSION,
                            ),
                            row: *cell_row,
                            col: *cell_col,
                            text: text.clone(),
                        })
                        .collect();
                    tables.push(TableModel {
                        element_id: ids::table_id(slide_name, &name, SCHEMA_VERSION),
                        name,
                        cells,
                    });
                }
                skipped_frame = false;
                in_graphic_frame = false;
                in_tbl = false;
                in_tc = false;
            }
            Event::Start(tag) if tag.local_name().as_ref() == b"tbl" && in_graphic_frame => {
                in_tbl = true;
                table_cells.clear();
                row = -1;
                col = -1;
            }
            Event::Start(tag) if tag.local_name().as_ref() == b"tr" && in_tbl => {
                row += 1;
                col = -1;
            }
            Event::Start(tag) if tag.local_name().as_ref() == b"tc" && in_tbl => {
                col += 1;
                in_tc = true;
                cell_texts.clear();
            }
            Event::End(tag) if tag.local_name().as_ref() == b"tc" && in_tc => {
                table_cells.push((row as u32, col as u32, cell_texts.concat()));
                in_tc = false;
            }
            Event::Start(tag) if tag.local_name().as_ref() == b"sp" && !skipped_frame => {
                in_sp = true;
                shape_name.clear();
                shape_texts.clear();
            }
            Event::Empty(tag) | Event::Start(tag)
                if (in_sp || in_graphic_frame) && tag.local_name().as_ref() == b"cNvPr" =>
            {
                for attribute in tag.attributes() {
                    let attribute = attribute
                        .map_err(|error| format_error(format!("invalid cNvPr: {error}")))?;
                    if attribute.key.local_name().as_ref() == b"name" {
                        let value = String::from_utf8_lossy(&attribute.value).into_owned();
                        if in_sp {
                            shape_name = value;
                        } else if in_graphic_frame && !in_tbl {
                            frame_name = value;
                        }
                    }
                }
            }
            Event::Start(tag) if tag.local_name().as_ref() == b"t" && (in_sp || in_tc) => {
                let text = reader
                    .read_text(tag.name())
                    .map_err(|error| format_error(format!("invalid a:t: {error}")))?;
                let decoded = decode_text(text);
                if in_tc {
                    cell_texts.push(decoded);
                } else {
                    shape_texts.push(decoded);
                }
            }
            Event::End(tag) if tag.local_name().as_ref() == b"sp" && in_sp => {
                if !skipped_frame {
                    let name = if shape_name.is_empty() {
                        format!("shape:{}", shapes.len())
                    } else {
                        shape_name.clone()
                    };
                    let text = shape_texts.concat();
                    shapes.push(ShapeModel {
                        element_id: ids::shape_id(slide_name, &name, SCHEMA_VERSION),
                        name,
                        text,
                    });
                }
                in_sp = false;
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok((shapes, tables))
}

fn collect_text(xml: &[u8]) -> String {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut texts = Vec::new();
    while let Ok(event) = reader.read_event_into(&mut buffer) {
        match event {
            Event::Start(tag) if tag.local_name().as_ref() == b"t" => {
                if let Ok(text) = reader.read_text(tag.name()) {
                    texts.push(decode_text(text));
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    texts.concat()
}

fn decode_text(text: quick_xml::events::BytesText<'_>) -> String {
    let decoded = text
        .decode()
        .map(|value| value.into_owned())
        .unwrap_or_else(|_| String::from_utf8_lossy(text.as_ref()).into_owned());
    quick_xml::escape::unescape(&decoded)
        .map(|value| value.into_owned())
        .unwrap_or(decoded)
}

fn format_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<pptx>".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_presentation_bytes;
    use crate::fixture::minimal_pptx;

    #[test]
    fn parses_title_shape_text() {
        let model = parse_presentation_bytes(&minimal_pptx()).expect("parse");
        assert_eq!(model.slides.len(), 1);
        assert_eq!(model.slides[0].shapes[0].name, "Title");
        assert_eq!(model.slides[0].shapes[0].text, "Hello");
        assert_eq!(model.slides[0].part_name, "ppt/slides/slide1.xml");
    }
}
