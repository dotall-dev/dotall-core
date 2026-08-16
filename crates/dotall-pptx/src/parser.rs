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
    PresentationModel, SCHEMA_VERSION, ShapeModel, SlideModel, TableCellModel, TableModel,
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
    let targets = relationship_targets(&rels_xml)?;

    let mut slides = Vec::new();
    for (index, rid) in rids.into_iter().enumerate() {
        let target = targets
            .get(&rid)
            .ok_or_else(|| format_error(format!("presentation relationship `{rid}` is missing")))?;
        let part_name = resolve_ppt_target(target);
        let slide_xml = zip_entry(&mut archive, &part_name)?;
        let name = format!("Slide {}", index + 1);
        let (shapes, tables) = parse_shapes_and_tables(&slide_xml, &name)?;
        let notes_part = format!("ppt/notesSlides/notesSlide{}.xml", index + 1);
        let (notes, notes_part_name) = match zip_entry(&mut archive, &notes_part) {
            Ok(xml) => {
                let text = collect_text(&xml);
                ((!text.is_empty()).then_some(text), Some(notes_part.clone()))
            }
            Err(_) => (None, None),
        };
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

fn relationship_targets(xml: &[u8]) -> Result<BTreeMap<String, String>> {
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
                if rel_type.ends_with("/slide")
                    && let (Some(id), Some(target)) = (id, target)
                {
                    targets.insert(id, target);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    Ok(targets)
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
