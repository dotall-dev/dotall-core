use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use dotall_core::{DotallError, Result};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use zip::ZipArchive;

use crate::FORMAT_ID;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactInventory {
    pub worksheets: Vec<String>,
    pub workbook: bool,
    pub workbook_relationships: bool,
    pub content_types: bool,
    pub shared_strings: bool,
    pub calculation_chain: bool,
    pub tables: Vec<String>,
    pub charts: Vec<String>,
    pub drawings: Vec<String>,
    pub unsupported: Vec<UnsupportedImpact>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnsupportedImpact {
    pub part: String,
    pub construct: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImpactOperation {
    InsertRow { sheet: String, at: u32, count: u32 },
}

pub fn inventory(package: &[u8], operation: &ImpactOperation) -> Result<ImpactInventory> {
    let (sheet, operation_name) = match operation {
        ImpactOperation::InsertRow { sheet, .. } => (sheet.as_str(), "insert_row"),
    };
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    let workbook_relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels")?;
    let relationship_targets = relationships(&workbook_relationships)?;
    let sheets = workbook_sheets(&workbook)?;
    let sheet_relationship_id = sheets
        .get(sheet)
        .ok_or_else(|| impact_error(format!("unknown sheet `{sheet}`")))?;
    let worksheet_target = relationship_targets
        .get(sheet_relationship_id)
        .ok_or_else(|| impact_error(format!("worksheet relationship `{sheet_relationship_id}` is missing")))?;
    let worksheet = resolve_target("xl/workbook.xml", &worksheet_target.target);

    let mut tables = BTreeSet::new();
    let mut charts = BTreeSet::new();
    let mut drawings = BTreeSet::new();
    let mut unsupported = BTreeSet::new();
    for relationship in relationships_for_part(package, &worksheet)? {
        if relationship.kind.ends_with("/table") {
            tables.insert(relationship.target);
        } else if relationship.kind.ends_with("/drawing") {
            drawings.insert(relationship.target.clone());
            for drawing_relationship in relationships_for_part(package, &relationship.target)? {
                if drawing_relationship.kind.ends_with("/chart") {
                    charts.insert(drawing_relationship.target);
                }
            }
        }
    }

    for relationship in relationships_for_part(package, "xl/workbook.xml")? {
        if relationship.kind.ends_with("/pivotCacheDefinition")
            && pivot_source_sheet(&entry_bytes(package, &relationship.target)?)?.as_deref() == Some(sheet)
        {
            unsupported.insert(UnsupportedImpact {
                part: relationship.target,
                construct: "pivot source reference".into(),
                reason: format!("{operation_name} changes rows referenced by the pivot cache"),
            });
        }
    }

    Ok(ImpactInventory {
        worksheets: vec![worksheet],
        workbook: false,
        workbook_relationships: false,
        content_types: false,
        shared_strings: false,
        calculation_chain: has_entry(package, "xl/calcChain.xml")?,
        tables: tables.into_iter().collect(),
        charts: charts.into_iter().collect(),
        drawings: drawings.into_iter().collect(),
        unsupported: unsupported.into_iter().collect(),
    })
}

pub fn validate_impact(package: &[u8], operation: &ImpactOperation) -> Result<ImpactInventory> {
    let inventory = inventory(package, operation)?;
    if let Some(unsupported) = inventory.unsupported.first() {
        let operation_name = match operation {
            ImpactOperation::InsertRow { .. } => "insert_row",
        };
        return Err(impact_error(format!(
            "{operation_name} is unsafe: unsupported {} in {}",
            unsupported.construct, unsupported.part
        )));
    }
    Ok(inventory)
}

#[derive(Debug)]
struct Relationship {
    kind: String,
    target: String,
}

fn workbook_sheets(xml: &[u8]) -> Result<BTreeMap<String, String>> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut sheets = BTreeMap::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| impact_error(format!("invalid workbook XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element) if element.name().as_ref() == b"sheet" => {
                let attributes = attributes(&element)?;
                if let (Some(name), Some(id)) = (attributes.get("name"), attributes.get("r:id")) {
                    sheets.insert(name.clone(), id.clone());
                }
            }
            Event::Eof => return Ok(sheets),
            _ => {}
        }
        buffer.clear();
    }
}

fn relationships_for_part(package: &[u8], part: &str) -> Result<Vec<Relationship>> {
    let relationship_part = relationship_part_name(part);
    if !has_entry(package, &relationship_part)? {
        return Ok(Vec::new());
    }
    let xml = entry_bytes(package, &relationship_part)?;
    relationships(&xml)?
        .into_iter()
        .map(|(_, relationship)| {
            Ok(Relationship {
                kind: relationship.kind,
                target: resolve_target(part, &relationship.target),
            })
        })
        .collect()
}

#[derive(Debug)]
struct RawRelationship {
    kind: String,
    target: String,
}

fn relationships(xml: &[u8]) -> Result<BTreeMap<String, RawRelationship>> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut parsed = BTreeMap::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| impact_error(format!("invalid relationships XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if element.name().as_ref() == b"Relationship" =>
            {
                let attributes = attributes(&element)?;
                if let (Some(id), Some(kind), Some(target)) = (
                    attributes.get("Id"),
                    attributes.get("Type"),
                    attributes.get("Target"),
                ) {
                    parsed.insert(
                        id.clone(),
                        RawRelationship {
                            kind: kind.clone(),
                            target: target.clone(),
                        },
                    );
                }
            }
            Event::Eof => return Ok(parsed),
            _ => {}
        }
        buffer.clear();
    }
}

fn pivot_source_sheet(xml: &[u8]) -> Result<Option<String>> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| impact_error(format!("invalid pivot cache XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element)
                if element.name().as_ref() == b"worksheetSource" =>
            {
                return Ok(attributes(&element)?.get("sheet").cloned());
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
        buffer.clear();
    }
}

fn attributes(element: &BytesStart<'_>) -> Result<BTreeMap<String, String>> {
    element
        .attributes()
        .map(|attribute| {
            let attribute =
                attribute.map_err(|error| impact_error(format!("invalid XML attribute: {error}")))?;
            Ok((
                String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                String::from_utf8_lossy(attribute.value.as_ref()).into_owned(),
            ))
        })
        .collect()
}

fn relationship_part_name(part: &str) -> String {
    let (directory, name) = part.rsplit_once('/').unwrap_or(("", part));
    if directory.is_empty() {
        format!("_rels/{name}.rels")
    } else {
        format!("{directory}/_rels/{name}.rels")
    }
}

fn resolve_target(part: &str, target: &str) -> String {
    if target.starts_with('/') {
        return target.trim_start_matches('/').into();
    }
    let base = part.rsplit_once('/').map_or("", |(directory, _)| directory);
    let mut components = base.split('/').filter(|component| !component.is_empty()).collect::<Vec<_>>();
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            component => components.push(component),
        }
    }
    components.join("/")
}

fn has_entry(package: &[u8], name: &str) -> Result<bool> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| impact_error(format!("invalid XLSX package: {error}")))?;
    Ok(archive.by_name(name).is_ok())
}

fn entry_bytes(package: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(package))
        .map_err(|error| impact_error(format!("invalid XLSX package: {error}")))?;
    let mut entry = archive
        .by_name(name)
        .map_err(|error| impact_error(format!("XLSX package is missing `{name}`: {error}")))?;
    let mut bytes = Vec::new();
    entry
        .read_to_end(&mut bytes)
        .map_err(|error| impact_error(format!("cannot read `{name}`: {error}")))?;
    Ok(bytes)
}

fn impact_error(message: impl Into<String>) -> DotallError {
    DotallError::Format {
        format_id: FORMAT_ID.into(),
        path: "<xlsx impact inventory>".into(),
        message: message.into(),
    }
}
