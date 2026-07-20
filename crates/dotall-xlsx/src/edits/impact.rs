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
    DeleteRow { sheet: String, at: u32, count: u32 },
    InsertColumn { sheet: String, at: u32, count: u32 },
    DeleteColumn { sheet: String, at: u32, count: u32 },
}

pub fn inventory(package: &[u8], operation: &ImpactOperation) -> Result<ImpactInventory> {
    let (sheet, operation_name) = match operation {
        ImpactOperation::InsertRow { sheet, .. } => (sheet.as_str(), "insert_row"),
        ImpactOperation::DeleteRow { sheet, .. } => (sheet.as_str(), "delete_row"),
        ImpactOperation::InsertColumn { sheet, .. } => (sheet.as_str(), "insert_column"),
        ImpactOperation::DeleteColumn { sheet, .. } => (sheet.as_str(), "delete_column"),
    };
    let workbook = entry_bytes(package, "xl/workbook.xml")?;
    let workbook_relationships = entry_bytes(package, "xl/_rels/workbook.xml.rels")?;
    let relationship_targets = relationships(&workbook_relationships)?;
    let sheets = workbook_sheets(&workbook)?;
    let defined_names = workbook_defined_names(&workbook)?;
    let (canonical_sheet, sheet_relationship_id) = find_workbook_sheet(&sheets, sheet)?;
    let worksheet_target = relationship_targets
        .get(sheet_relationship_id)
        .ok_or_else(|| {
            impact_error(format!(
                "worksheet relationship `{sheet_relationship_id}` is missing"
            ))
        })?;
    let worksheet = resolve_target("xl/workbook.xml", &worksheet_target.target);
    let worksheet_xml = entry_bytes(package, &worksheet)?;

    let mut tables = BTreeSet::new();
    let mut charts = BTreeSet::new();
    let mut drawings = BTreeSet::new();
    let mut unsupported = BTreeSet::new();
    for relationship in relationships_for_part(package, &worksheet)? {
        if relationship.kind.ends_with("/table") {
            tables.insert(relationship.target.clone());
            unsupported.insert(UnsupportedImpact {
                part: relationship.target,
                construct: "table reference".into(),
                reason: format!("{operation_name} requires rewriting the table range"),
            });
        } else if relationship.kind.ends_with("/drawing") {
            drawings.insert(relationship.target.clone());
            for drawing_relationship in relationships_for_part(package, &relationship.target)? {
                if drawing_relationship.kind.ends_with("/chart") {
                    charts.insert(drawing_relationship.target.clone());
                    unsupported.insert(UnsupportedImpact {
                        part: drawing_relationship.target,
                        construct: "chart series references".into(),
                        reason: format!(
                            "{operation_name} can require rewriting chart series formulas"
                        ),
                    });
                }
            }
        }
    }
    unsupported.extend(unsupported_worksheet_constructs(
        &worksheet_xml,
        &worksheet,
        operation_name,
    )?);

    for relationship in relationships_for_part(package, "xl/workbook.xml")? {
        if !relationship.kind.ends_with("/pivotCacheDefinition") {
            continue;
        }
        let pivot_xml = entry_bytes(package, &relationship.target)?;
        match pivot_source_sheet(&pivot_xml, &defined_names)? {
            PivotSourceResolution::Sheet(source_sheet)
                if source_sheet.eq_ignore_ascii_case(canonical_sheet) =>
            {
                unsupported.insert(UnsupportedImpact {
                    part: relationship.target,
                    construct: "pivot source reference".into(),
                    reason: format!("{operation_name} changes rows referenced by the pivot cache"),
                });
            }
            PivotSourceResolution::UnresolvedDefinedName(name) => {
                unsupported.insert(UnsupportedImpact {
                    part: relationship.target,
                    construct: "pivot source reference".into(),
                    reason: format!(
                        "{operation_name} cannot verify pivot cache source: defined name `{name}` could not be resolved to a worksheet"
                    ),
                });
            }
            PivotSourceResolution::Sheet(_) | PivotSourceResolution::Missing => {}
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

fn unsupported_worksheet_constructs(
    xml: &[u8],
    part: &str,
    operation_name: &str,
) -> Result<BTreeSet<UnsupportedImpact>> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut unsupported = BTreeSet::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| impact_error(format!("invalid worksheet XML: {error}")))?
        {
            Event::Empty(element) | Event::Start(element) => {
                let construct = match element.name().as_ref() {
                    b"autoFilter" => Some("autoFilter"),
                    b"dataValidations" => Some("dataValidations"),
                    b"conditionalFormatting" => Some("conditionalFormatting"),
                    b"hyperlinks" => Some("hyperlinks"),
                    _ => None,
                };
                if let Some(construct) = construct {
                    unsupported.insert(UnsupportedImpact {
                        part: part.into(),
                        construct: construct.into(),
                        reason: format!("{operation_name} requires rewriting worksheet references"),
                    });
                }
            }
            Event::Eof => return Ok(unsupported),
            _ => {}
        }
        buffer.clear();
    }
}

pub fn validate_impact(package: &[u8], operation: &ImpactOperation) -> Result<ImpactInventory> {
    let inventory = inventory(package, operation)?;
    if let Some(unsupported) = inventory.unsupported.first() {
        let operation_name = match operation {
            ImpactOperation::InsertRow { .. } => "insert_row",
            ImpactOperation::DeleteRow { .. } => "delete_row",
            ImpactOperation::InsertColumn { .. } => "insert_column",
            ImpactOperation::DeleteColumn { .. } => "delete_column",
        };
        return Err(impact_error(format!(
            "{operation_name} is unsafe: unsupported {} in {} ({})",
            unsupported.construct, unsupported.part, unsupported.reason
        )));
    }
    Ok(inventory)
}

#[derive(Debug)]
struct Relationship {
    kind: String,
    target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PivotSourceResolution {
    Sheet(String),
    UnresolvedDefinedName(String),
    Missing,
}

fn find_workbook_sheet<'a>(
    sheets: &'a BTreeMap<String, String>,
    name: &str,
) -> Result<(&'a str, &'a str)> {
    sheets
        .iter()
        .find(|(sheet_name, _)| sheet_name.eq_ignore_ascii_case(name))
        .map(|(sheet_name, relationship_id)| (sheet_name.as_str(), relationship_id.as_str()))
        .ok_or_else(|| impact_error(format!("unknown sheet `{name}`")))
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
            Event::Empty(element) | Event::Start(element)
                if element.name().as_ref() == b"sheet" =>
            {
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
        .into_values()
        .map(|relationship| {
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

fn workbook_defined_names(xml: &[u8]) -> Result<BTreeMap<String, String>> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut defined_names = BTreeMap::new();
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| impact_error(format!("invalid workbook XML: {error}")))?
        {
            Event::Start(element) if element.name().as_ref() == b"definedName" => {
                let attributes = attributes(&element)?;
                let name = attributes.get("name").cloned();
                let mut formula = String::new();
                loop {
                    match reader
                        .read_event_into(&mut buffer)
                        .map_err(|error| impact_error(format!("invalid workbook XML: {error}")))?
                    {
                        Event::Text(text) => {
                            formula.push_str(&String::from_utf8_lossy(text.as_ref()));
                        }
                        Event::CData(text) => {
                            formula.push_str(&String::from_utf8_lossy(text.as_ref()));
                        }
                        Event::End(end) if end.name().as_ref() == b"definedName" => break,
                        Event::Eof => break,
                        _ => {}
                    }
                    buffer.clear();
                }
                if let Some(name) = name {
                    defined_names.insert(name, formula);
                }
            }
            Event::Eof => return Ok(defined_names),
            _ => {}
        }
        buffer.clear();
    }
}

fn pivot_source_sheet(
    xml: &[u8],
    defined_names: &BTreeMap<String, String>,
) -> Result<PivotSourceResolution> {
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
                let attributes = attributes(&element)?;
                if let Some(sheet) = attributes.get("sheet") {
                    return Ok(PivotSourceResolution::Sheet(sheet.clone()));
                }
                if let Some(name) = attributes.get("name") {
                    return Ok(resolve_defined_name_sheet(defined_names, name)
                        .map(PivotSourceResolution::Sheet)
                        .unwrap_or_else(|| {
                            PivotSourceResolution::UnresolvedDefinedName(name.clone())
                        }));
                }
                return Ok(PivotSourceResolution::Missing);
            }
            Event::Eof => return Ok(PivotSourceResolution::Missing),
            _ => {}
        }
        buffer.clear();
    }
}

fn resolve_defined_name_sheet(
    defined_names: &BTreeMap<String, String>,
    name: &str,
) -> Option<String> {
    defined_names
        .iter()
        .find(|(defined_name, _)| defined_name.eq_ignore_ascii_case(name))
        .and_then(|(_, formula)| sheet_from_defined_name_formula(formula))
}

fn sheet_from_defined_name_formula(formula: &str) -> Option<String> {
    let formula = formula.trim();
    let (sheet_part, _) = formula.rsplit_once('!')?;
    let sheet = sheet_part.trim();
    if sheet.starts_with('\'') && sheet.ends_with('\'') && sheet.len() >= 2 {
        Some(sheet[1..sheet.len() - 1].replace("''", "'"))
    } else if !sheet.is_empty() {
        Some(sheet.to_owned())
    } else {
        None
    }
}

fn attributes(element: &BytesStart<'_>) -> Result<BTreeMap<String, String>> {
    element
        .attributes()
        .map(|attribute| {
            let attribute = attribute
                .map_err(|error| impact_error(format!("invalid XML attribute: {error}")))?;
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
    let mut components = base
        .split('/')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
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
