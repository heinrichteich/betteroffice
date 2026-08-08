use std::collections::{BTreeMap, HashMap, HashSet};

use crate::model::{PackagePart, VsdxPackage};
use crate::patch::{
    AttributeSpan, CellEdit, MAX_PATCH_BYTES, MAX_PATCH_EDITS, SpanEdit, apply_span_edits,
    escape_attribute_value, scan_element_spans,
};
use crate::relationships::{Relationship, parse_relationships, relationship_types};
use crate::sheet::{parse_records, parse_sheet};
use crate::xml::{ParseBudget, XmlElement, XmlNode, parse_xml};
use crate::{CellAttribute, ParseLimits, Sheet, VsdxError};
use ooxml_drawingml::Theme;

/// Identifies the ShapeSheet containing a semantic cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellSheet {
    Document,
    Page(u32),
    Master(u32),
}

/// Identifies a row within a ShapeSheet section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CellRow {
    Index(u32),
    Name(String),
}

/// Stable semantic identity for a ShapeSheet cell.
///
/// Future CRDT entities can retain this locator and add their entity identity
/// alongside it without exposing lexical source spans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellLocator {
    pub sheet: CellSheet,
    pub shape_id: Option<u32>,
    pub section: Option<String>,
    pub row: Option<CellRow>,
    pub cell_name: String,
}

/// A semantic cell edit. `formula` and `value` independently select attributes to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticCellEdit {
    pub locator: CellLocator,
    pub gesture: MutationGesture,
    pub formula: Option<String>,
    pub value: Option<String>,
}

/// The user action that requested a ShapeSheet mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationGesture {
    CellEdit,
    MoveX,
    MoveY,
    ResizeWidth,
    ResizeHeight,
    ResizeAspect,
    TextEdit,
    Format,
    Delete,
}

/// A page-local structural change applied atomically with any cell edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StructuralEdit {
    DeleteShape { page_id: u32, shape_id: u32 },
}

type PendingInsertion = (crate::SourceSpan, u8, Vec<(CellAttribute, String)>);

struct NewCell {
    part_path: String,
    owner_span: crate::SourceSpan,
    name: String,
    formula: String,
    value: String,
}

struct NewContainerCell {
    part_path: String,
    owner_span: crate::SourceSpan,
    section: Option<String>,
    row: Option<CellRow>,
    name: String,
    formula: String,
    value: String,
}

pub fn parse_vsdx(data: &[u8]) -> Result<VsdxPackage, VsdxError> {
    parse_vsdx_with_limits(data, &ParseLimits::default())
}

pub fn parse_vsdx_with_limits(data: &[u8], limits: &ParseLimits) -> Result<VsdxPackage, VsdxError> {
    let source_parts = ooxml_opc::unzip_parts(data).map_err(VsdxError::Container)?;
    match ooxml_opc::detect_package_kind(&source_parts) {
        Ok(ooxml_opc::DocumentKind::Vsdx) => {}
        Ok(kind) => return Err(VsdxError::UnsupportedDocumentKind(kind)),
        Err(ooxml_opc::DocumentKindError::ConflictingDocumentKinds(kinds)) => {
            return Err(VsdxError::ConflictingDocumentKinds(kinds));
        }
        Err(error) => return Err(VsdxError::Container(error.to_string())),
    }
    let parts: HashMap<&str, &[u8]> = source_parts
        .iter()
        .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
        .collect();
    let mut budget = ParseBudget::new(limits);
    let mut xml_parts = HashMap::new();
    let mut relationships = BTreeMap::new();
    let root_relationships = load_relationships("", &parts, &mut relationships, &mut budget)?;
    let document_path = root_relationships
        .iter()
        .find(|relationship| relationship.relationship_type == relationship_types::DOCUMENT)
        .and_then(|relationship| relationship.resolved_target.clone())
        .ok_or_else(|| VsdxError::MissingPart("root Visio document relationship".to_owned()))?;
    require_part(&parts, &document_path)?;
    parse_part(&parts, &document_path, &mut xml_parts, &mut budget)?;
    let document_relationships =
        load_relationships(&document_path, &parts, &mut relationships, &mut budget)?;
    let pages_part_path = target_by_type(document_relationships, relationship_types::PAGES);
    let masters_part_path = target_by_type(document_relationships, relationship_types::MASTERS);
    let theme_part_paths = targets_by_type(document_relationships, relationship_types::THEME);
    let windows_part_path = target_by_type(document_relationships, relationship_types::WINDOWS);
    let mut page_part_paths = Vec::new();
    if let Some(path) = &pages_part_path {
        require_part(&parts, path)?;
        parse_part(&parts, path, &mut xml_parts, &mut budget)?;
        let rels = load_relationships(path, &parts, &mut relationships, &mut budget)?;
        page_part_paths = targets_by_type(rels, relationship_types::PAGE);
        for path in &page_part_paths {
            require_part(&parts, path)?;
            parse_part(&parts, path, &mut xml_parts, &mut budget)?;
        }
    }
    let mut master_part_paths = Vec::new();
    if let Some(path) = &masters_part_path {
        require_part(&parts, path)?;
        parse_part(&parts, path, &mut xml_parts, &mut budget)?;
        let rels = load_relationships(path, &parts, &mut relationships, &mut budget)?;
        master_part_paths = targets_by_type(rels, relationship_types::MASTER);
        for path in &master_part_paths {
            require_part(&parts, path)?;
            parse_part(&parts, path, &mut xml_parts, &mut budget)?;
        }
    }
    for path in theme_part_paths.iter().chain(windows_part_path.iter()) {
        require_part(&parts, path)?;
        parse_part(&parts, path, &mut xml_parts, &mut budget)?;
    }
    load_reachable_relationships(&parts, &mut relationships, &mut budget)?;
    let document = xml_parts.remove(&document_path).expect("document parsed");
    let document_sheet = document
        .children_named("DocumentSheet")
        .next()
        .map(|sheet| parse_sheet(sheet, &document_path, &mut budget))
        .transpose()?;
    let style_sheets = document
        .children_named("StyleSheets")
        .flat_map(|styles| styles.children_named("StyleSheet"))
        .map(|style| parse_sheet(style, &document_path, &mut budget))
        .collect::<Result<Vec<_>, _>>()?;
    let colors = parse_records(document.children_named("Colors").next());
    let face_names = parse_records(document.children_named("FaceNames").next());
    let page_sheets = parse_catalog_sheets(
        xml_parts.get(pages_part_path.as_deref().unwrap_or("")),
        "Page",
        &document_path,
        &mut budget,
    )?;
    let master_sheets = parse_catalog_sheets(
        xml_parts.get(masters_part_path.as_deref().unwrap_or("")),
        "Master",
        &document_path,
        &mut budget,
    )?;
    let page_part_ids = catalog_part_ids(
        xml_parts.get(pages_part_path.as_deref().unwrap_or("")),
        "Page",
        relationships
            .get(pages_part_path.as_deref().unwrap_or(""))
            .map(Vec::as_slice),
    );
    let master_part_ids = catalog_part_ids(
        xml_parts.get(masters_part_path.as_deref().unwrap_or("")),
        "Master",
        relationships
            .get(masters_part_path.as_deref().unwrap_or(""))
            .map(Vec::as_slice),
    );
    let page_contents = parse_part_sheets(&page_part_paths, &mut xml_parts, &mut budget)?;
    let master_contents = parse_part_sheets(&master_part_paths, &mut xml_parts, &mut budget)?;
    let themes = theme_part_paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let root = xml_parts
                .get(path)
                .ok_or_else(|| VsdxError::MissingPart(path.clone()))?;
            Ok(((index + 1) as u32, parse_theme(root, path)?))
        })
        .collect::<Result<_, VsdxError>>()?;
    let sheet_part_paths: HashSet<&str> = std::iter::once(document_path.as_str())
        .chain(pages_part_path.iter().map(String::as_str))
        .chain(masters_part_path.iter().map(String::as_str))
        .chain(page_part_paths.iter().map(String::as_str))
        .chain(master_part_paths.iter().map(String::as_str))
        .collect();
    let package_parts = source_parts
        .into_iter()
        .map(|(path, bytes)| {
            let spans = if sheet_part_paths.contains(path.as_str()) {
                scan_element_spans(&bytes).map_err(|_| VsdxError::MalformedXml {
                    part: path.clone(),
                    offset: 0,
                    message: "invalid lexical XML structure".to_owned(),
                })?
            } else {
                Vec::new()
            };
            Ok(PackagePart { path, bytes, spans })
        })
        .collect::<Result<Vec<_>, VsdxError>>()?;
    Ok(VsdxPackage {
        document_part_path: document_path,
        pages_part_path,
        masters_part_path,
        page_part_paths,
        master_part_paths,
        theme_part_paths,
        themes,
        windows_part_path,
        relationships,
        document_sheet,
        style_sheets,
        colors,
        face_names,
        page_sheets,
        master_sheets,
        page_part_ids,
        master_part_ids,
        page_contents,
        master_contents,
        parts: package_parts,
    })
}

fn parse_theme(root: &XmlElement, part: &str) -> Result<Theme, VsdxError> {
    let mut theme = Theme {
        name: root.attribute("name").unwrap_or("Office Theme").to_owned(),
        ..Theme::default()
    };
    let Some(theme_elements) = root.children_named("themeElements").next() else {
        return Ok(theme);
    };
    let scheme = theme_elements
        .children_named("clrScheme")
        .next()
        .ok_or_else(|| VsdxError::MalformedXml {
            part: part.to_owned(),
            offset: 0,
            message: "theme is missing themeElements/clrScheme".to_owned(),
        })?;
    for slot in [
        "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5",
        "accent6", "hlink", "folHlink",
    ] {
        let Some(value) = scheme.children_named(slot).next().and_then(|slot| {
            slot.children.iter().find_map(|child| match child {
                XmlNode::Element(value) => value
                    .attribute("lastClr")
                    .or_else(|| value.attribute("val")),
                XmlNode::Text(_) => None,
            })
        }) else {
            continue;
        };
        theme.color_scheme.set(slot, value.to_owned());
    }
    Ok(theme)
}

fn catalog_part_ids(
    root: Option<&XmlElement>,
    item: &str,
    relationships: Option<&[Relationship]>,
) -> BTreeMap<String, u32> {
    root.into_iter()
        .flat_map(|root| root.children_named(item))
        .filter_map(|element| {
            let id = element.attribute("ID")?.parse().ok()?;
            let relationship_id = element
                .children_named("Rel")
                .find_map(|rel| rel.attribute("r:id").or_else(|| rel.attribute("id")))
                .or_else(|| {
                    element
                        .attribute("r:id")
                        .or_else(|| element.attribute("id"))
                })?;
            let path = relationships?
                .iter()
                .find(|relationship| relationship.id == relationship_id)?
                .resolved_target
                .clone()?;
            Some((path, id))
        })
        .collect()
}

fn parse_part_sheets(
    paths: &[String],
    xml_parts: &mut HashMap<String, XmlElement>,
    budget: &mut ParseBudget<'_>,
) -> Result<BTreeMap<String, Sheet>, VsdxError> {
    paths
        .iter()
        .map(|path| {
            let root = xml_parts
                .remove(path)
                .ok_or_else(|| VsdxError::MissingPart(path.clone()))?;
            Ok((path.clone(), parse_sheet(&root, path, budget)?))
        })
        .collect()
}

fn parse_catalog_sheets(
    root: Option<&XmlElement>,
    item: &str,
    part: &str,
    budget: &mut ParseBudget<'_>,
) -> Result<BTreeMap<u32, Sheet>, VsdxError> {
    root.into_iter()
        .flat_map(|root| root.children_named(item))
        .map(|element| {
            let id = element
                .attribute("ID")
                .ok_or_else(|| VsdxError::MalformedXml {
                    part: part.to_owned(),
                    offset: 0,
                    message: format!("missing {item} ID"),
                })?
                .parse()
                .map_err(|_| VsdxError::MalformedXml {
                    part: part.to_owned(),
                    offset: 0,
                    message: format!("invalid {item} ID"),
                })?;
            let sheet = element
                .children_named("PageSheet")
                .next()
                .map(|sheet| parse_sheet(sheet, part, budget))
                .transpose()?
                .unwrap_or_default();
            Ok((id, sheet))
        })
        .collect()
}

fn parse_part(
    parts: &HashMap<&str, &[u8]>,
    path: &str,
    xml_parts: &mut HashMap<String, XmlElement>,
    budget: &mut ParseBudget<'_>,
) -> Result<(), VsdxError> {
    if !xml_parts.contains_key(path) {
        let bytes = parts
            .get(path)
            .ok_or_else(|| VsdxError::MissingPart(path.to_owned()))?;
        xml_parts.insert(path.to_owned(), parse_xml(bytes, path, budget)?);
    }
    Ok(())
}

fn load_reachable_relationships(
    parts: &HashMap<&str, &[u8]>,
    relationships: &mut BTreeMap<String, Vec<Relationship>>,
    budget: &mut ParseBudget<'_>,
) -> Result<(), VsdxError> {
    let mut pending: Vec<String> = relationships.keys().cloned().collect();
    let mut index = 0;
    while let Some(source) = pending.get(index).cloned() {
        index += 1;
        let targets: Vec<String> = relationships[&source]
            .iter()
            .filter_map(|relationship| relationship.resolved_target.clone())
            .collect();
        for target in targets {
            let path = relationship_path(&target);
            if parts.contains_key(path.as_str()) && !relationships.contains_key(&target) {
                load_relationships(&target, parts, relationships, budget)?;
                pending.push(target);
            }
        }
    }
    Ok(())
}

pub fn write_vsdx(package: &VsdxPackage) -> Result<Vec<u8>, VsdxError> {
    ooxml_opc::rezip_parts(
        &package
            .parts
            .iter()
            .map(|part| (part.path.clone(), part.bytes.clone()))
            .collect::<Vec<_>>(),
    )
    .map_err(VsdxError::Container)
}

/// Applies Cell@V and Cell@F attribute edits without mutating `package`.
pub(crate) fn save_cell_edits(
    package: &VsdxPackage,
    edits: &[CellEdit],
) -> Result<Vec<u8>, VsdxError> {
    save_cell_edits_with_new_cells(package, edits, &[], &[])
}

fn save_cell_edits_with_new_cells(
    package: &VsdxPackage,
    edits: &[CellEdit],
    new_cells: &[NewCell],
    new_container_cells: &[NewContainerCell],
) -> Result<Vec<u8>, VsdxError> {
    if edits.len() > MAX_PATCH_EDITS {
        return Err(VsdxError::PatchLimit { kind: "editCount" });
    }
    let mut replacement_bytes = 0_usize;
    let mut validated = Vec::with_capacity(edits.len());
    let mut insertions: BTreeMap<(&str, crate::SourceSpan), PendingInsertion> = BTreeMap::new();
    for edit in edits {
        if !is_shapesheet_part(package, &edit.part_path) {
            return Err(VsdxError::InvalidCellEdit {
                part: edit.part_path.clone(),
                message: "part is not an authoritative ShapeSheet part".to_owned(),
            });
        }
        let part = package
            .parts
            .iter()
            .find(|part| part.path == edit.part_path)
            .ok_or_else(|| VsdxError::InvalidCellEdit {
                part: edit.part_path.clone(),
                message: "part does not exist".to_owned(),
            })?;
        let cell = part
            .spans
            .iter()
            .find(|span| {
                span.name
                    .rsplit_once(':')
                    .map_or(span.name.as_str(), |(_, name)| name)
                    == "Cell"
                    && span.span == edit.cell_span
            })
            .ok_or_else(|| VsdxError::InvalidCellEdit {
                part: edit.part_path.clone(),
                message: "span is not an existing Cell".to_owned(),
            })?;
        if let Some(attribute) = cell.attributes.get(edit.attribute.name()) {
            let replacement = escape_attribute_value(&edit.value, attribute.quote)?;
            replacement_bytes = replacement_bytes
                .checked_add(replacement.len())
                .ok_or(VsdxError::PatchLimit { kind: "editBytes" })?;
            validated.push((
                part.path.as_str(),
                SpanEdit {
                    span: attribute.value,
                    replacement,
                },
            ));
        } else {
            let quote = cell
                .attributes
                .values()
                .next()
                .map(|attribute| attribute.quote)
                .ok_or_else(|| VsdxError::InvalidCellEdit {
                    part: edit.part_path.clone(),
                    message: "Cell has no attribute quote style".to_owned(),
                })?;
            let entry = insertions
                .entry((part.path.as_str(), cell.span))
                .or_insert_with(|| {
                    let end = cell.start_tag.end().expect("scanner span cannot overflow");
                    let offset = if part.bytes[end - 2] == b'/' {
                        end - 2
                    } else {
                        end - 1
                    };
                    (crate::SourceSpan { offset, length: 0 }, quote, Vec::new())
                });
            if entry
                .2
                .iter()
                .any(|(attribute, _)| *attribute == edit.attribute)
            {
                return Err(VsdxError::InvalidCellEdit {
                    part: edit.part_path.clone(),
                    message: format!("duplicate Cell@{} edit", edit.attribute.name()),
                });
            }
            entry.2.push((edit.attribute, edit.value.clone()));
        }
    }
    for ((path, _), (span, quote, attributes)) in insertions {
        let mut replacement = Vec::new();
        for (attribute, value) in attributes {
            replacement.push(b' ');
            replacement.extend_from_slice(attribute.name().as_bytes());
            replacement.push(b'=');
            replacement.push(quote);
            replacement.extend_from_slice(&escape_attribute_value(&value, quote)?);
            replacement.push(quote);
        }
        replacement_bytes = replacement_bytes
            .checked_add(replacement.len())
            .ok_or(VsdxError::PatchLimit { kind: "editBytes" })?;
        validated.push((path, SpanEdit { span, replacement }));
    }
    for new_cell in new_cells {
        let part = package
            .parts
            .iter()
            .find(|part| part.path == new_cell.part_path)
            .ok_or_else(|| VsdxError::InvalidCellEdit {
                part: new_cell.part_path.clone(),
                message: "part does not exist".to_owned(),
            })?;
        let owner = part
            .spans
            .iter()
            .find(|span| span.span == new_cell.owner_span)
            .ok_or_else(|| VsdxError::InvalidCellEdit {
                part: new_cell.part_path.clone(),
                message: "local cell owner does not exist".to_owned(),
            })?;
        let quote = owner
            .attributes
            .values()
            .next()
            .map(|attribute| attribute.quote)
            .ok_or_else(|| VsdxError::InvalidCellEdit {
                part: new_cell.part_path.clone(),
                message: "local cell owner has no attribute quote style".to_owned(),
            })?;
        let end = owner.span.end().ok_or(VsdxError::InvalidSpan)?;
        let (span, closes_owner) = if end >= 2 && part.bytes[end - 2..end] == *b"/>" {
            (
                crate::SourceSpan {
                    offset: end - 2,
                    length: 2,
                },
                true,
            )
        } else {
            (
                crate::SourceSpan {
                    offset: part.bytes[..end]
                        .iter()
                        .rposition(|byte| *byte == b'<')
                        .ok_or(VsdxError::InvalidSpan)?,
                    length: 0,
                },
                false,
            )
        };
        let mut replacement = b"<Cell N=".to_vec();
        if closes_owner {
            replacement.insert(0, b'>');
        }
        replacement.push(quote);
        replacement.extend_from_slice(&escape_attribute_value(&new_cell.name, quote)?);
        replacement.push(quote);
        for (name, value) in [("F", &new_cell.formula), ("V", &new_cell.value)] {
            replacement.push(b' ');
            replacement.extend_from_slice(name.as_bytes());
            replacement.push(b'=');
            replacement.push(quote);
            replacement.extend_from_slice(&escape_attribute_value(value, quote)?);
            replacement.push(quote);
        }
        replacement.extend_from_slice(b"/>");
        if closes_owner {
            replacement.extend_from_slice(format!("</{}>", local_name(&owner.name)).as_bytes());
        }
        replacement_bytes = replacement_bytes
            .checked_add(replacement.len())
            .ok_or(VsdxError::PatchLimit { kind: "editBytes" })?;
        validated.push((part.path.as_str(), SpanEdit { span, replacement }));
    }
    for new_cell in new_container_cells {
        let part = package
            .parts
            .iter()
            .find(|part| part.path == new_cell.part_path)
            .ok_or_else(|| VsdxError::InvalidCellEdit {
                part: new_cell.part_path.clone(),
                message: "part does not exist".to_owned(),
            })?;
        let owner = part
            .spans
            .iter()
            .find(|span| span.span == new_cell.owner_span)
            .ok_or_else(|| VsdxError::InvalidCellEdit {
                part: new_cell.part_path.clone(),
                message: "local container owner does not exist".to_owned(),
            })?;
        let quote = owner
            .attributes
            .values()
            .next()
            .map(|attribute| attribute.quote)
            .ok_or_else(|| VsdxError::InvalidCellEdit {
                part: new_cell.part_path.clone(),
                message: "local container owner has no attribute quote style".to_owned(),
            })?;
        let end = owner.span.end().ok_or(VsdxError::InvalidSpan)?;
        let (span, closes_owner) = if end >= 2 && part.bytes[end - 2..end] == *b"/>" {
            (
                crate::SourceSpan {
                    offset: end - 2,
                    length: 2,
                },
                true,
            )
        } else {
            (
                crate::SourceSpan {
                    offset: part.bytes[..end]
                        .iter()
                        .rposition(|byte| *byte == b'<')
                        .ok_or(VsdxError::InvalidSpan)?,
                    length: 0,
                },
                false,
            )
        };
        let mut replacement = Vec::new();
        if closes_owner {
            replacement.push(b'>');
        }
        if let Some(section) = &new_cell.section {
            replacement.extend_from_slice(b"<Section N=");
            push_quoted(&mut replacement, section, quote)?;
            replacement.push(b'>');
        }
        if let Some(row) = &new_cell.row {
            replacement.extend_from_slice(b"<Row ");
            match row {
                CellRow::Index(_) => replacement.extend_from_slice(b"IX="),
                CellRow::Name(_) => replacement.extend_from_slice(b"N="),
            }
            let row_value = match row {
                CellRow::Index(index) => index.to_string(),
                CellRow::Name(name) => name.clone(),
            };
            push_quoted(&mut replacement, &row_value, quote)?;
            replacement.push(b'>');
        }
        replacement.extend_from_slice(b"<Cell N=");
        push_quoted(&mut replacement, &new_cell.name, quote)?;
        replacement.extend_from_slice(b" F=");
        push_quoted(&mut replacement, &new_cell.formula, quote)?;
        replacement.extend_from_slice(b" V=");
        push_quoted(&mut replacement, &new_cell.value, quote)?;
        replacement.extend_from_slice(b"/>");
        if new_cell.row.is_some() {
            replacement.extend_from_slice(b"</Row>");
        }
        if new_cell.section.is_some() {
            replacement.extend_from_slice(b"</Section>");
        }
        if closes_owner {
            replacement.extend_from_slice(format!("</{}>", local_name(&owner.name)).as_bytes());
        }
        replacement_bytes = replacement_bytes
            .checked_add(replacement.len())
            .ok_or(VsdxError::PatchLimit { kind: "editBytes" })?;
        validated.push((part.path.as_str(), SpanEdit { span, replacement }));
    }
    if replacement_bytes > MAX_PATCH_BYTES {
        return Err(VsdxError::PatchLimit { kind: "editBytes" });
    }
    let mut part_edits: BTreeMap<&str, Vec<SpanEdit>> = BTreeMap::new();
    for (path, edit) in validated {
        part_edits.entry(path).or_default().push(edit);
    }
    let mut output = package.clone();
    for (path, edits) in part_edits {
        let part = output
            .parts
            .iter_mut()
            .find(|part| part.path == path)
            .expect("validated source part");
        part.bytes = apply_span_edits(&part.bytes, &edits)?;
    }
    let bytes = write_vsdx(&output)?;
    parse_vsdx(&bytes)?;
    Ok(bytes)
}

/// Resolves semantic cell edits to package-local lexical provenance and saves them.
pub fn save_semantic_cell_edits(
    package: &VsdxPackage,
    edits: &[SemanticCellEdit],
) -> Result<Vec<u8>, VsdxError> {
    let mut lexical = Vec::new();
    let mut new_cells = Vec::new();
    let mut new_container_cells = Vec::new();
    for edit in edits {
        let (Some(formula), Some(value)) = (&edit.formula, &edit.value) else {
            return Err(VsdxError::InvalidCellEdit {
                part: format!("{:?}", edit.locator.sheet),
                message: "semantic edits require both a formula and its evaluated cache".to_owned(),
            });
        };
        match resolve_cell_locator(package, &edit.locator)? {
            LocalCell::Existing(part_path, cell_span) => {
                lexical.push(CellEdit {
                    part_path: part_path.clone(),
                    cell_span,
                    attribute: CellAttribute::Formula,
                    value: formula.clone(),
                });
                lexical.push(CellEdit {
                    part_path,
                    cell_span,
                    attribute: CellAttribute::Value,
                    value: value.clone(),
                });
            }
            LocalCell::New(part_path, owner_span) => new_cells.push(NewCell {
                part_path,
                owner_span,
                name: edit.locator.cell_name.clone(),
                formula: formula.clone(),
                value: value.clone(),
            }),
            LocalCell::NewContainer(part_path, owner_span, section, row) => new_container_cells
                .push(NewContainerCell {
                    part_path,
                    owner_span,
                    section,
                    row,
                    name: edit.locator.cell_name.clone(),
                    formula: formula.clone(),
                    value: value.clone(),
                }),
        }
    }
    if new_cells.is_empty() && new_container_cells.is_empty() {
        save_cell_edits(package, &lexical)
    } else {
        save_cell_edits_with_new_cells(package, &lexical, &new_cells, &new_container_cells)
    }
}

/// Applies page-local structural edits through the lexical container fallback.
///
/// Deleting a shape also deletes every local Connect that names it. The source
/// package is never changed; the result is accepted only after reparsing and
/// referential-integrity validation.
pub fn save_structural_edits(
    package: &VsdxPackage,
    edits: &[StructuralEdit],
) -> Result<Vec<u8>, VsdxError> {
    let mut by_part: BTreeMap<String, HashSet<u32>> = BTreeMap::new();
    for edit in edits {
        let StructuralEdit::DeleteShape { page_id, shape_id } = edit;
        let path = package
            .page_part_ids
            .iter()
            .find_map(|(path, id)| (*id == *page_id).then(|| path.clone()))
            .ok_or_else(|| VsdxError::InvalidCellEdit {
                part: page_id.to_string(),
                message: "page does not exist".to_owned(),
            })?;
        by_part.entry(path).or_default().insert(*shape_id);
    }
    let mut output = package.clone();
    for (path, deleted) in by_part {
        let part = package
            .parts
            .iter()
            .find(|part| part.path == path)
            .ok_or_else(|| VsdxError::InvalidCellEdit {
                part: path.clone(),
                message: "part does not exist".to_owned(),
            })?;
        let shapes =
            direct_child(part, "Shapes", None).ok_or_else(|| VsdxError::InvalidCellEdit {
                part: path.clone(),
                message: "page has no Shapes container".to_owned(),
            })?;
        let mut replacements = vec![container_without(part, shapes, "Shape", |shape| {
            shape
                .attributes
                .get("ID")
                .and_then(|attribute| attribute_value(&part.bytes, attribute))
                .and_then(|value| value.parse::<u32>().ok())
                .is_some_and(|id| deleted.contains(&id))
        })?];
        if let Some(connects) = direct_child(part, "Connects", None) {
            replacements.push(container_without(part, connects, "Connect", |connect| {
                ["FromSheet", "ToSheet"].iter().any(|name| {
                    connect
                        .attributes
                        .get(*name)
                        .and_then(|attribute| attribute_value(&part.bytes, attribute))
                        .and_then(|value| value.parse::<u32>().ok())
                        .is_some_and(|id| deleted.contains(&id))
                })
            })?);
        }
        let target = output
            .parts
            .iter_mut()
            .find(|part| part.path == path)
            .expect("validated source part");
        target.bytes = apply_span_edits(&target.bytes, &replacements)?;
    }
    let bytes = write_vsdx(&output)?;
    let reparsed = parse_vsdx(&bytes)?;
    validate_structure(&reparsed)?;
    Ok(bytes)
}

fn direct_child<'a>(
    part: &'a PackagePart,
    name: &str,
    parent: Option<crate::SourceSpan>,
) -> Option<&'a crate::ElementSpan> {
    part.spans.iter().find(|candidate| {
        local_name(&candidate.name) == name
            && match parent {
                Some(parent) => nearest_parent(part, candidate.span, "Shapes")
                    .is_some_and(|owner| owner.span == parent),
                None => nearest_parent(part, candidate.span, "PageContents").is_some(),
            }
    })
}

fn container_without(
    part: &PackagePart,
    container: &crate::ElementSpan,
    child_name: &str,
    remove: impl Fn(&crate::ElementSpan) -> bool,
) -> Result<SpanEdit, VsdxError> {
    let mut children: Vec<_> = part
        .spans
        .iter()
        .filter(|candidate| {
            local_name(&candidate.name) == child_name
                && nearest_parent(part, candidate.span, &container.name)
                    .is_some_and(|parent| parent.span == container.span)
        })
        .collect();
    children.sort_by_key(|child| child.span.offset);
    let mut replacement = Vec::new();
    let mut cursor = container.span.offset;
    for child in children {
        if remove(child) {
            replacement.extend_from_slice(&part.bytes[cursor..child.span.offset]);
            cursor = child.span.end().ok_or(VsdxError::InvalidSpan)?;
        }
    }
    replacement.extend_from_slice(
        &part.bytes[cursor..container.span.end().ok_or(VsdxError::InvalidSpan)?],
    );
    Ok(SpanEdit {
        span: container.span,
        replacement,
    })
}

/// Checks page-local shape IDs and connector endpoints after structural edits.
pub fn validate_structure(package: &VsdxPackage) -> Result<(), VsdxError> {
    for (path, sheet) in &package.page_contents {
        let mut ids = HashSet::new();
        for shape in sheet.shapes() {
            if !ids.insert(shape.id) {
                return Err(VsdxError::InvalidCellEdit {
                    part: path.clone(),
                    message: format!("duplicate shape ID {}", shape.id),
                });
            }
        }
        for connect in sheet.connects() {
            if !ids.contains(&connect.from_sheet) || !ids.contains(&connect.to_sheet) {
                return Err(VsdxError::InvalidCellEdit {
                    part: path.clone(),
                    message: "Connect references a missing shape".to_owned(),
                });
            }
        }
    }
    Ok(())
}

enum LocalCell {
    Existing(String, crate::SourceSpan),
    New(String, crate::SourceSpan),
    NewContainer(String, crate::SourceSpan, Option<String>, Option<CellRow>),
}

fn resolve_cell_locator(
    package: &VsdxPackage,
    locator: &CellLocator,
) -> Result<LocalCell, VsdxError> {
    let path = match locator.sheet {
        CellSheet::Document => Some(package.document_part_path.clone()),
        CellSheet::Page(id) if locator.shape_id.is_some() => package
            .page_part_ids
            .iter()
            .find_map(|(path, candidate)| (*candidate == id).then(|| path.clone())),
        CellSheet::Page(id) => package.parts.iter().find_map(|part| {
            part.spans.iter().find_map(|page| {
                (local_name(&page.name) == "Page"
                    && attribute_equals(&part.bytes, page, "ID", &id.to_string()))
                .then(|| {
                    part.spans
                        .iter()
                        .any(|sheet| {
                            local_name(&sheet.name) == "PageSheet"
                                && contains(page.span, sheet.span)
                        })
                        .then(|| part.path.clone())
                })
                .flatten()
            })
        }),
        CellSheet::Master(id) => package
            .master_part_ids
            .iter()
            .find_map(|(path, candidate)| (*candidate == id).then(|| path.clone())),
    }
    .ok_or_else(|| VsdxError::InvalidCellEdit {
        part: format!("{:?}", locator.sheet),
        message: "sheet does not exist".to_owned(),
    })?;
    let part = package
        .parts
        .iter()
        .find(|part| part.path == path)
        .expect("catalogued part exists");
    let cell = part
        .spans
        .iter()
        .filter(|span| {
            local_name(&span.name) == "Cell"
                && attribute_equals(&part.bytes, span, "N", &locator.cell_name)
        })
        .find(|cell| {
            let nearest = |name: &str| {
                part.spans
                    .iter()
                    .filter(|parent| {
                        local_name(&parent.name) == name && contains(parent.span, cell.span)
                    })
                    .min_by_key(|parent| parent.span.length)
            };
            let shape_matches = match (locator.shape_id, nearest("Shape")) {
                (None, None) => true,
                (Some(id), Some(shape)) => {
                    attribute_equals(&part.bytes, shape, "ID", &id.to_string())
                }
                _ => false,
            };
            let section_matches = match (&locator.section, nearest("Section")) {
                (None, None) => true,
                (Some(name), Some(section)) => attribute_equals(&part.bytes, section, "N", name),
                _ => false,
            };
            let row_matches = match (&locator.row, nearest("Row")) {
                (None, None) => true,
                (Some(CellRow::Index(index)), Some(row)) => {
                    attribute_equals(&part.bytes, row, "IX", &index.to_string())
                }
                (Some(CellRow::Name(name)), Some(row)) => {
                    attribute_equals(&part.bytes, row, "N", name)
                }
                _ => false,
            };
            shape_matches && section_matches && row_matches
        });
    if let Some(cell) = cell {
        return Ok(LocalCell::Existing(path, cell.span));
    }
    if let (Some(section_name), Some(row)) = (&locator.section, &locator.row) {
        let shape_matches = |candidate: &crate::ElementSpan| match locator.shape_id {
            Some(id) => nearest_parent(part, candidate.span, "Shape")
                .is_some_and(|shape| attribute_equals(&part.bytes, shape, "ID", &id.to_string())),
            None => nearest_parent(part, candidate.span, "Shape").is_none(),
        };
        if let Some(existing_row) = part.spans.iter().find(|candidate| {
            local_name(&candidate.name) == "Row"
                && nearest_parent(part, candidate.span, "Section").is_some_and(|section| {
                    attribute_equals(&part.bytes, section, "N", section_name)
                })
                && shape_matches(candidate)
                && match row {
                    CellRow::Index(index) => {
                        attribute_equals(&part.bytes, candidate, "IX", &index.to_string())
                    }
                    CellRow::Name(name) => attribute_equals(&part.bytes, candidate, "N", name),
                }
        }) {
            return Ok(LocalCell::New(path, existing_row.span));
        }
        if let Some(section) = part.spans.iter().find(|candidate| {
            local_name(&candidate.name) == "Section"
                && attribute_equals(&part.bytes, candidate, "N", section_name)
                && shape_matches(candidate)
        }) {
            return Ok(LocalCell::NewContainer(
                path,
                section.span,
                None,
                Some(row.clone()),
            ));
        }
        if let Some(shape) = part.spans.iter().find(|candidate| {
            local_name(&candidate.name) == "Shape"
                && locator.shape_id.is_some_and(|id| {
                    attribute_equals(&part.bytes, candidate, "ID", &id.to_string())
                })
        }) {
            return Ok(LocalCell::NewContainer(
                path,
                shape.span,
                Some(section_name.clone()),
                Some(row.clone()),
            ));
        }
    }
    let owner_name = if locator.section.is_some() {
        "Row"
    } else if locator.shape_id.is_some() {
        "Shape"
    } else if matches!(locator.sheet, CellSheet::Page(_)) {
        "PageSheet"
    } else {
        "DocumentSheet"
    };
    let owner = part
        .spans
        .iter()
        .filter(|span| local_name(&span.name) == owner_name)
        .find(|owner| {
            let owner = *owner;
            let shape = (local_name(&owner.name) == "Shape")
                .then_some(owner)
                .or_else(|| nearest_parent(part, owner.span, "Shape"));
            let shape_matches = match (locator.shape_id, shape) {
                (None, None) => true,
                (Some(id), Some(shape)) => {
                    attribute_equals(&part.bytes, shape, "ID", &id.to_string())
                }
                _ => false,
            };
            let section_matches = match (
                &locator.section,
                nearest_parent(part, owner.span, "Section"),
            ) {
                (None, None) => true,
                (Some(name), Some(section)) => attribute_equals(&part.bytes, section, "N", name),
                _ => false,
            };
            let row_matches = match (&locator.row, Some(owner)) {
                (None, _) => true,
                (Some(CellRow::Index(index)), Some(row)) => {
                    attribute_equals(&part.bytes, row, "IX", &index.to_string())
                }
                (Some(CellRow::Name(name)), Some(row)) => {
                    attribute_equals(&part.bytes, row, "N", name)
                }
                _ => false,
            };
            shape_matches && section_matches && row_matches
        });
    owner.map(|owner| LocalCell::New(path.clone(), owner.span)).ok_or_else(|| VsdxError::InvalidCellEdit {
        part: path,
        message: "semantic cell does not exist locally; creating inherited Section or Row is unsupported".to_owned(),
    })
}

fn push_quoted(output: &mut Vec<u8>, value: &str, quote: u8) -> Result<(), VsdxError> {
    output.push(quote);
    output.extend_from_slice(&escape_attribute_value(value, quote)?);
    output.push(quote);
    Ok(())
}

fn nearest_parent<'a>(
    part: &'a PackagePart,
    child: crate::SourceSpan,
    name: &str,
) -> Option<&'a crate::ElementSpan> {
    part.spans
        .iter()
        .filter(|parent| local_name(&parent.name) == name && contains(parent.span, child))
        .min_by_key(|parent| parent.span.length)
}

fn local_name(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, name)| name)
}

fn contains(parent: crate::SourceSpan, child: crate::SourceSpan) -> bool {
    parent.offset < child.offset
        && parent
            .end()
            .is_some_and(|end| child.end().is_some_and(|child_end| child_end <= end))
}

fn attribute_equals(source: &[u8], span: &crate::ElementSpan, name: &str, expected: &str) -> bool {
    span.attributes.get(name).is_some_and(|attribute| {
        attribute_value(source, attribute).is_some_and(|value| value == expected)
    })
}

fn attribute_value<'a>(
    source: &'a [u8],
    attribute: &AttributeSpan,
) -> Option<std::borrow::Cow<'a, str>> {
    let value = source.get(attribute.value.offset..attribute.value.end()?)?;
    quick_xml::escape::unescape(std::str::from_utf8(value).ok()?).ok()
}

fn is_shapesheet_part(package: &VsdxPackage, path: &str) -> bool {
    path == package.document_part_path
        || package.pages_part_path.as_deref() == Some(path)
        || package.masters_part_path.as_deref() == Some(path)
        || package
            .page_part_paths
            .iter()
            .any(|candidate| candidate == path)
        || package
            .master_part_paths
            .iter()
            .any(|candidate| candidate == path)
}

fn load_relationships<'a>(
    source: &str,
    parts: &HashMap<&str, &[u8]>,
    relationships: &'a mut BTreeMap<String, Vec<Relationship>>,
    budget: &mut ParseBudget<'_>,
) -> Result<&'a [Relationship], VsdxError> {
    if !relationships.contains_key(source) {
        let path = relationship_path(source);
        let bytes = parts
            .get(path.as_str())
            .ok_or_else(|| VsdxError::MissingPart(path.clone()))?;
        let parsed = parse_relationships(bytes, &path, source, budget)?;
        relationships.insert(source.to_owned(), parsed);
    }
    Ok(relationships
        .get(source)
        .map(Vec::as_slice)
        .expect("inserted relationship entry"))
}
fn relationship_path(source: &str) -> String {
    if source.is_empty() {
        "_rels/.rels".to_owned()
    } else if let Some((directory, name)) = source.rsplit_once('/') {
        format!("{directory}/_rels/{name}.rels")
    } else {
        format!("_rels/{source}.rels")
    }
}
fn require_part(parts: &HashMap<&str, &[u8]>, path: &str) -> Result<(), VsdxError> {
    if parts.contains_key(path) {
        Ok(())
    } else {
        Err(VsdxError::MissingPart(path.to_owned()))
    }
}
fn target_by_type(relationships: &[Relationship], kind: &str) -> Option<String> {
    relationships
        .iter()
        .find(|relationship| relationship.has_type(kind))
        .and_then(|relationship| relationship.resolved_target.clone())
}
fn targets_by_type(relationships: &[Relationship], kind: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    relationships
        .iter()
        .filter(|relationship| relationship.has_type(kind))
        .filter_map(|relationship| relationship.resolved_target.clone())
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet::serialize_sheet;
    use crate::xml::{XmlNode, parse_xml};
    use crate::{CellAttribute, CellEdit, Shape, SourceSpan};
    use ooxml_opc::{rezip_parts, unzip_parts};

    fn first_value_edit(package: &VsdxPackage, part_path: &str, value: &str) -> CellEdit {
        let cell = package
            .element_spans(part_path)
            .unwrap()
            .iter()
            .find(|span| {
                span.name
                    .rsplit_once(':')
                    .map_or(span.name.as_str(), |(_, name)| name)
                    == "Cell"
                    && span.attributes.contains_key("V")
            })
            .unwrap();
        CellEdit {
            part_path: part_path.to_owned(),
            cell_span: cell.span,
            attribute: CellAttribute::Value,
            value: value.to_owned(),
        }
    }

    fn package_with_page_xml(xml: &[u8]) -> (VsdxPackage, String) {
        let source = include_bytes!("../tests/fixtures/foundation.vsdx");
        let mut parts = unzip_parts(source).unwrap();
        let path = parse_vsdx(source).unwrap().page_part_paths.remove(0);
        parts
            .iter_mut()
            .find(|(candidate, _)| candidate == &path)
            .unwrap()
            .1 = xml.to_vec();
        (parse_vsdx(&rezip_parts(&parts).unwrap()).unwrap(), path)
    }

    fn cell_span(package: &VsdxPackage, part_path: &str) -> SourceSpan {
        package
            .element_spans(part_path)
            .unwrap()
            .iter()
            .find(|span| span.name == "Cell")
            .unwrap()
            .span
    }

    #[test]
    fn inserts_a_missing_local_singleton_cell() {
        let source = b"<PageContents><Shapes><Shape ID='1'><Cell N='Other' V='1'/></Shape></Shapes></PageContents>";
        let (package, path) = package_with_page_xml(source);
        let page_id = package.page_part_ids[&path];
        let saved = save_semantic_cell_edits(
            &package,
            &[SemanticCellEdit {
                locator: CellLocator {
                    sheet: CellSheet::Page(page_id),
                    shape_id: Some(1),
                    section: None,
                    row: None,
                    cell_name: "Width".to_owned(),
                },
                gesture: MutationGesture::CellEdit,
                formula: Some("4".to_owned()),
                value: Some("4".to_owned()),
            }],
        )
        .unwrap();
        let saved = parse_vsdx(&saved).unwrap();
        let after = saved.part_bytes(&path).unwrap();
        assert_eq!(after, b"<PageContents><Shapes><Shape ID='1'><Cell N='Other' V='1'/><Cell N='Width' F='4' V='4'/></Shape></Shapes></PageContents>");
    }

    #[test]
    fn inserts_cell_in_missing_row_without_rewriting_section_descendants() {
        let source = b"<PageContents><Shapes><Shape ID='1'><Section N='User'><Row N='Keep'><Cell N='Value' V='old'/></Row></Section></Shape></Shapes></PageContents>";
        let (package, path) = package_with_page_xml(source);
        let page_id = package.page_part_ids[&path];
        let saved = save_semantic_cell_edits(
            &package,
            &[semantic_edit(
                page_id,
                "User",
                CellRow::Name("New".to_owned()),
            )],
        )
        .unwrap();
        let after = parse_vsdx(&saved)
            .unwrap()
            .part_bytes(&path)
            .unwrap()
            .to_vec();
        assert_eq!(after, b"<PageContents><Shapes><Shape ID='1'><Section N='User'><Row N='Keep'><Cell N='Value' V='old'/></Row><Row N='New'><Cell N='Value' F='4' V='4'/></Row></Section></Shape></Shapes></PageContents>");
    }

    #[test]
    fn inserts_cell_with_missing_section_and_row() {
        let source = b"<PageContents><Shapes><Shape ID='1'><Cell N='Keep' V='old'/></Shape></Shapes></PageContents>";
        let (package, path) = package_with_page_xml(source);
        let page_id = package.page_part_ids[&path];
        let saved = save_semantic_cell_edits(
            &package,
            &[semantic_edit(
                page_id,
                "User",
                CellRow::Name("New".to_owned()),
            )],
        )
        .unwrap();
        let after = parse_vsdx(&saved)
            .unwrap()
            .part_bytes(&path)
            .unwrap()
            .to_vec();
        assert_eq!(after, b"<PageContents><Shapes><Shape ID='1'><Cell N='Keep' V='old'/><Section N='User'><Row N='New'><Cell N='Value' F='4' V='4'/></Row></Section></Shape></Shapes></PageContents>");
    }

    #[test]
    fn inserts_cell_into_self_closing_local_row() {
        let source = b"<PageContents><Shapes><Shape ID='1'><Section N='User'><Row N='New'/></Section></Shape></Shapes></PageContents>";
        let (package, path) = package_with_page_xml(source);
        let page_id = package.page_part_ids[&path];
        let saved = save_semantic_cell_edits(
            &package,
            &[semantic_edit(
                page_id,
                "User",
                CellRow::Name("New".to_owned()),
            )],
        )
        .unwrap();
        let after = parse_vsdx(&saved)
            .unwrap()
            .part_bytes(&path)
            .unwrap()
            .to_vec();
        assert_eq!(after, b"<PageContents><Shapes><Shape ID='1'><Section N='User'><Row N='New'><Cell N='Value' F='4' V='4'/></Row></Section></Shape></Shapes></PageContents>");
    }

    #[test]
    fn deletes_a_shape_and_its_connects_without_rewriting_siblings() {
        let source = b"<PageContents><Shapes><Shape ID='1'><Cell N='Keep' V='one'/></Shape><Shape ID='2'><Cell N='Keep' V='two'/></Shape><Shape ID='3'><Cell N='Keep' V='three'/></Shape></Shapes><Connects><Connect FromSheet='1' ToSheet='2'/><Connect FromSheet='2' ToSheet='3'/></Connects></PageContents>";
        let (package, path) = package_with_page_xml(source);
        let page_id = package.page_part_ids[&path];
        let saved = save_structural_edits(
            &package,
            &[StructuralEdit::DeleteShape {
                page_id,
                shape_id: 2,
            }],
        )
        .unwrap();
        let after = parse_vsdx(&saved)
            .unwrap()
            .part_bytes(&path)
            .unwrap()
            .to_vec();
        assert_eq!(after, b"<PageContents><Shapes><Shape ID='1'><Cell N='Keep' V='one'/></Shape><Shape ID='3'><Cell N='Keep' V='three'/></Shape></Shapes><Connects></Connects></PageContents>");
    }

    fn semantic_edit(page_id: u32, section: &str, row: CellRow) -> SemanticCellEdit {
        SemanticCellEdit {
            locator: CellLocator {
                sheet: CellSheet::Page(page_id),
                shape_id: Some(1),
                section: Some(section.to_owned()),
                row: Some(row),
                cell_name: "Value".to_owned(),
            },
            gesture: MutationGesture::CellEdit,
            formula: Some("4".to_owned()),
            value: Some("4".to_owned()),
        }
    }

    fn assert_only_span_changed(
        before: &[u8],
        after: &[u8],
        span: SourceSpan,
        replacement_length: usize,
    ) {
        assert_eq!(&before[..span.offset], &after[..span.offset]);
        assert_eq!(
            &before[span.offset + span.length..],
            &after[span.offset + replacement_length..]
        );
    }

    fn sheet_has_value(sheet: &Sheet, value: &str) -> bool {
        sheet
            .cells()
            .any(|cell| cell.value.as_deref() == Some(value))
            || sheet.shapes().any(|shape| shape_has_value(shape, value))
    }

    fn shape_has_value(shape: &Shape, value: &str) -> bool {
        shape
            .cells()
            .any(|cell| cell.value.as_deref() == Some(value))
            || shape.sections().any(|section| {
                section
                    .rows()
                    .flat_map(crate::Row::cells)
                    .any(|cell| cell.value.as_deref() == Some(value))
            })
            || shape.shapes().any(|shape| shape_has_value(shape, value))
    }

    #[test]
    fn records_source_spans_for_shapesheet_entities() {
        let package = parse_vsdx(include_bytes!("../tests/fixtures/foundation.vsdx")).unwrap();
        let part_path = package.page_part_paths.first().unwrap();
        let bytes = package.part_bytes(part_path).unwrap();
        let spans = package.element_spans(part_path).unwrap();
        for name in ["Shape", "Section", "Row", "Cell", "Text"] {
            assert!(spans.iter().any(|span| span.name == name), "{name}");
        }
        for span in spans {
            assert_eq!(bytes[span.span.offset], b'<');
            assert_eq!(bytes[span.span.offset + span.span.length - 1], b'>');
        }
    }

    #[test]
    fn records_exact_spans_for_shapesheet_entities_and_mixed_text() {
        let source = b"<PageContents><Shapes><Shape ID='7'><Section N='Geometry'><Row IX='2'><Cell N='X'/></Row></Section><Text>before<cp IX='0'/>after</Text></Shape></Shapes></PageContents>";
        let spans = scan_element_spans(source).unwrap();
        for (name, expected) in [
            (
                "Shape",
                "<Shape ID='7'><Section N='Geometry'><Row IX='2'><Cell N='X'/></Row></Section><Text>before<cp IX='0'/>after</Text></Shape>",
            ),
            (
                "Section",
                "<Section N='Geometry'><Row IX='2'><Cell N='X'/></Row></Section>",
            ),
            ("Row", "<Row IX='2'><Cell N='X'/></Row>"),
            ("Cell", "<Cell N='X'/>"),
            ("Text", "<Text>before<cp IX='0'/>after</Text>"),
        ] {
            let span = spans.iter().find(|span| span.name == name).unwrap().span;
            assert_eq!(
                &source[span.offset..span.end().unwrap()],
                expected.as_bytes(),
                "{name}"
            );
        }
    }

    #[test]
    fn inserts_missing_cell_attributes_with_existing_quote_style() {
        for (xml, edits, expected) in [
            (b"<PageContents><Shapes><Shape ID='1'><Cell N='X' F='a'/></Shape></Shapes></PageContents>".as_slice(), vec![(CellAttribute::Value, "v")], "<Cell N='X' F='a' V='v'/>"),
            (b"<PageContents><Shapes><Shape ID='1'><Cell N='X' V='v'/></Shape></Shapes></PageContents>".as_slice(), vec![(CellAttribute::Formula, "a")], "<Cell N='X' V='v' F='a'/>"),
            (b"<PageContents><Shapes><Shape ID='1'><Cell N='X'/></Shape></Shapes></PageContents>".as_slice(), vec![(CellAttribute::Formula, "a"), (CellAttribute::Value, "v")], "<Cell N='X' F='a' V='v'/>"),
        ] {
            let (package, path) = package_with_page_xml(xml);
            let span = cell_span(&package, &path);
            let edits = edits.into_iter().map(|(attribute, value)| CellEdit { part_path: path.clone(), cell_span: span, attribute, value: value.to_owned() }).collect::<Vec<_>>();
            let saved = save_cell_edits(&package, &edits).unwrap();
            let part = unzip_parts(&saved).unwrap().into_iter().find(|(candidate, _)| candidate == &path).unwrap().1;
            assert!(std::str::from_utf8(&part).unwrap().contains(expected));
        }
    }

    #[test]
    fn saves_cell_value_without_rewriting_other_parts_or_lexical_spans() {
        let source = include_bytes!("../tests/fixtures/foundation.vsdx");
        let package = parse_vsdx(source).unwrap();
        let part_path = package.page_part_paths.first().unwrap();
        let edit = first_value_edit(&package, part_path, "patched");
        let before: BTreeMap<_, _> = unzip_parts(source).unwrap().into_iter().collect();
        let saved = save_cell_edits(&package, std::slice::from_ref(&edit)).unwrap();
        let after: BTreeMap<_, _> = unzip_parts(&saved).unwrap().into_iter().collect();
        for (path, bytes) in &before {
            if path == &edit.part_path {
                let original = package.part_bytes(path).unwrap();
                let cell = package
                    .element_spans(path)
                    .unwrap()
                    .iter()
                    .find(|span| span.span == edit.cell_span)
                    .unwrap();
                assert_only_span_changed(
                    original,
                    &after[path],
                    cell.attributes["V"].value,
                    edit.value.len(),
                );
            } else {
                assert_eq!(&after[path], bytes, "{path}");
            }
        }
        let reparsed = parse_vsdx(&saved).unwrap();
        assert!(
            reparsed
                .page_contents
                .values()
                .any(|sheet| sheet_has_value(sheet, "patched"))
        );
    }

    #[test]
    fn resolves_semantic_locators_with_escaped_attributes() {
        let (package, path) = package_with_page_xml(
            b"<PageContents><Shapes><Shape ID='&#49;'><Cell N='A&amp;B' V='direct'/><Section N='A&#x26;B'><Row IX='0'><Cell N='SectionCell' V='section'/></Row></Section><Cell N='IdCell' V='id'/></Shape></Shapes></PageContents>",
        );
        let cases = [
            (
                CellLocator {
                    sheet: CellSheet::Page(*package.page_part_ids.get(&path).unwrap()),
                    shape_id: Some(1),
                    section: None,
                    row: None,
                    cell_name: "A&B".to_owned(),
                },
                "A&B",
                "patched-direct",
            ),
            (
                CellLocator {
                    sheet: CellSheet::Page(*package.page_part_ids.get(&path).unwrap()),
                    shape_id: Some(1),
                    section: Some("A&B".to_owned()),
                    row: Some(CellRow::Index(0)),
                    cell_name: "SectionCell".to_owned(),
                },
                "SectionCell",
                "patched-section",
            ),
            (
                CellLocator {
                    sheet: CellSheet::Page(*package.page_part_ids.get(&path).unwrap()),
                    shape_id: Some(1),
                    section: None,
                    row: None,
                    cell_name: "IdCell".to_owned(),
                },
                "IdCell",
                "patched-id",
            ),
        ];
        for (locator, _, new_value) in cases {
            let saved = save_semantic_cell_edits(
                &package,
                &[SemanticCellEdit {
                    locator,
                    gesture: MutationGesture::CellEdit,
                    formula: Some("2".to_owned()),
                    value: Some(new_value.to_owned()),
                }],
            )
            .unwrap();
            let reparsed = parse_vsdx(&saved).unwrap();
            assert!(
                reparsed
                    .page_contents
                    .values()
                    .any(|sheet| sheet_has_value(sheet, new_value))
            );
        }
    }

    #[test]
    fn saves_existing_cell_formula() {
        let source = include_bytes!("../tests/fixtures/foundation.vsdx");
        let package = parse_vsdx(source).unwrap();
        let part_path = package.page_part_paths.first().unwrap();
        let cell = package
            .element_spans(part_path)
            .unwrap()
            .iter()
            .find(|span| {
                span.name == "Cell"
                    && span.attributes.contains_key("F")
                    && span.attributes.contains_key("V")
            })
            .unwrap();
        let saved = save_cell_edits(
            &package,
            &[CellEdit {
                part_path: part_path.to_owned(),
                cell_span: cell.span,
                attribute: CellAttribute::Formula,
                value: "Width*3".to_owned(),
            }],
        )
        .unwrap();
        let reparsed = parse_vsdx(&saved).unwrap();
        assert!(
            reparsed
                .page_contents
                .values()
                .flat_map(Sheet::shapes)
                .any(|shape| shape
                    .cells()
                    .any(|cell| cell.formula.as_deref() == Some("Width*3")))
        );
    }

    #[test]
    fn saves_real_corpus_cells_without_rewriting_other_parts() {
        let Some(directory) = std::env::var_os("VSDX_CORPUS_DIR") else {
            eprintln!("SKIPPED REVIEW CORPUS TEST: VSDX_CORPUS_DIR is unset");
            return;
        };
        for name in ["lichtsysteme.vsdx", "soundplan.vsdx"] {
            let source = std::fs::read(std::path::Path::new(&directory).join(name)).unwrap();
            let package = parse_vsdx(&source).unwrap();
            let part_path = package.page_part_paths.first().unwrap();
            let edit = first_value_edit(&package, part_path, "patched");
            let before: BTreeMap<_, _> = unzip_parts(&source).unwrap().into_iter().collect();
            let saved = save_cell_edits(&package, std::slice::from_ref(&edit)).unwrap();
            let after: BTreeMap<_, _> = unzip_parts(&saved).unwrap().into_iter().collect();
            for (path, bytes) in before {
                if path == part_path.as_str() {
                    let cell = package
                        .element_spans(&path)
                        .unwrap()
                        .iter()
                        .find(|span| span.span == edit.cell_span)
                        .unwrap();
                    assert_only_span_changed(
                        &bytes,
                        &after[&path],
                        cell.attributes["V"].value,
                        edit.value.len(),
                    );
                } else {
                    assert_eq!(after[&path], bytes, "{name}: {path}");
                }
            }
        }
    }

    #[test]
    fn cell_edits_preserve_quotes_escape_values_and_are_transactional() {
        let source = include_bytes!("../tests/fixtures/foundation.vsdx");
        let package = parse_vsdx(source).unwrap();
        let part_path = package.page_part_paths.first().unwrap();
        let mut edit = first_value_edit(&package, part_path, "<&\"'");
        let cell = package
            .element_spans(part_path)
            .unwrap()
            .iter()
            .find(|span| span.span == edit.cell_span)
            .unwrap();
        assert_eq!(cell.attributes["V"].quote, b'\'');
        let saved = save_cell_edits(&package, &[edit.clone()]).unwrap();
        let mut parts: BTreeMap<_, _> = unzip_parts(&saved).unwrap().into_iter().collect();
        let part = parts.remove(part_path).unwrap();
        assert!(
            std::str::from_utf8(&part)
                .unwrap()
                .contains("V='&lt;&amp;&quot;&apos;'")
        );
        let reparsed = parse_vsdx(&saved).unwrap();
        assert!(
            reparsed
                .page_contents
                .values()
                .any(|sheet| sheet_has_value(sheet, "<&\"'"))
        );

        edit.cell_span.offset = usize::MAX;
        let before = package.part_bytes(part_path).unwrap().to_vec();
        assert!(matches!(
            save_cell_edits(
                &package,
                &[first_value_edit(&package, part_path, "ok"), edit]
            ),
            Err(VsdxError::InvalidCellEdit { .. })
        ));
        assert_eq!(package.part_bytes(part_path).unwrap(), before);
    }

    #[test]
    fn only_authoritative_shapesheet_parts_receive_spans_or_edits() {
        let source = include_bytes!("../tests/fixtures/foundation.vsdx");
        let mut parts = unzip_parts(source).unwrap();
        parts.push((
            "visio/media/forged.png".to_owned(),
            b"not PNG <Cell V='1'/>".to_vec(),
        ));
        parts.push((
            "customXml/item1.xml".to_owned(),
            b"<root><Cell V='1'/></root>".to_vec(),
        ));
        let package = parse_vsdx(&rezip_parts(&parts).unwrap()).unwrap();
        for path in ["visio/media/forged.png", "customXml/item1.xml"] {
            assert_eq!(package.element_spans(path), Some([].as_slice()));
            assert!(matches!(
                save_cell_edits(
                    &package,
                    &[CellEdit {
                        part_path: path.to_owned(),
                        cell_span: SourceSpan {
                            offset: 0,
                            length: 13
                        },
                        attribute: CellAttribute::Value,
                        value: "changed".to_owned(),
                    }]
                ),
                Err(VsdxError::InvalidCellEdit { .. })
            ));
        }
    }

    #[test]
    fn replacing_a_part_invalidates_its_edit_provenance() {
        let source = include_bytes!("../tests/fixtures/foundation.vsdx");
        let mut package = parse_vsdx(source).unwrap();
        let part_path = package.page_part_paths.first().unwrap().clone();
        let edit = first_value_edit(&package, &part_path, "changed");
        let replacement = package.part_bytes(&part_path).unwrap().to_vec();
        assert!(package.replace_part(&part_path, replacement.clone()));
        assert_eq!(package.element_spans(&part_path), Some([].as_slice()));
        assert!(matches!(
            save_cell_edits(&package, &[edit]),
            Err(VsdxError::InvalidCellEdit { .. })
        ));
        assert_eq!(package.part_bytes(&part_path), Some(replacement.as_slice()));
    }

    #[test]
    fn rejects_an_aggregate_over_limit_before_mutating_the_package() {
        let source = include_bytes!("../tests/fixtures/foundation.vsdx");
        let package = parse_vsdx(source).unwrap();
        let part_path = package.page_part_paths.first().unwrap();
        let edit = first_value_edit(&package, part_path, "changed");
        let edits = vec![edit; MAX_PATCH_EDITS + 1];
        let before = package.part_bytes(part_path).unwrap().to_vec();
        assert!(matches!(
            save_cell_edits(&package, &edits),
            Err(VsdxError::PatchLimit { kind: "editCount" })
        ));
        assert_eq!(package.part_bytes(part_path), Some(before.as_slice()));

        let oversized = "x".repeat(MAX_PATCH_BYTES / 2 + 1);
        let edits = [
            first_value_edit(&package, part_path, &oversized),
            first_value_edit(&package, part_path, &oversized),
        ];
        assert!(matches!(
            save_cell_edits(&package, &edits),
            Err(VsdxError::PatchLimit { kind: "editBytes" })
        ));
        assert_eq!(package.part_bytes(part_path), Some(before.as_slice()));
    }

    #[test]
    fn failed_edits_after_a_valid_edit_leave_the_package_unchanged() {
        let (mut package, part_path) = package_with_page_xml(
            b"<PageContents><Shapes><Shape ID='1'><Cell N='Valid' V='old'/><Cell N='Invalid'/></Shape></Shapes></PageContents>",
        );
        let invalid_span = package
            .element_spans(&part_path)
            .unwrap()
            .iter()
            .find(|span| {
                span.name == "Cell"
                    && span.attributes.contains_key("N")
                    && !span.attributes.contains_key("V")
            })
            .unwrap()
            .span;
        package
            .parts
            .iter_mut()
            .find(|part| part.path == part_path)
            .unwrap()
            .spans
            .iter_mut()
            .find(|span| span.span == invalid_span)
            .unwrap()
            .attributes
            .clear();
        let valid = first_value_edit(&package, &part_path, "valid");
        let before = package.part_bytes(&part_path).unwrap().to_vec();
        let stale = CellEdit {
            cell_span: SourceSpan {
                offset: usize::MAX,
                length: 0,
            },
            ..valid.clone()
        };
        let invalid_insertion = CellEdit {
            part_path: part_path.clone(),
            cell_span: invalid_span,
            attribute: CellAttribute::Formula,
            value: "x".to_owned(),
        };
        let cases = vec![
            vec![valid.clone(), valid.clone()],
            vec![
                valid.clone(),
                CellEdit {
                    value: "x".repeat(MAX_PATCH_BYTES),
                    ..valid.clone()
                },
            ],
            vec![
                valid.clone(),
                CellEdit {
                    value: "bad\0".to_owned(),
                    ..valid.clone()
                },
            ],
            vec![valid.clone(), stale],
            vec![valid.clone(), invalid_insertion],
        ];
        for edits in cases {
            assert!(save_cell_edits(&package, &edits).is_err());
            assert_eq!(package.part_bytes(&part_path), Some(before.as_slice()));
        }
    }

    #[test]
    fn saves_xml_whitespace_character_references_exactly() {
        let source = include_bytes!("../tests/fixtures/foundation.vsdx");
        let package = parse_vsdx(source).unwrap();
        let part_path = package.page_part_paths.first().unwrap();
        let value = "a\tb\nc\rd";
        let saved =
            save_cell_edits(&package, &[first_value_edit(&package, part_path, value)]).unwrap();
        let reparsed = parse_vsdx(&saved).unwrap();
        assert!(
            reparsed
                .page_contents
                .values()
                .any(|sheet| sheet_has_value(sheet, value))
        );
    }

    #[test]
    fn rejects_incomplete_theme_parts() {
        let limits = ParseLimits::default();
        let mut budget = ParseBudget::new(&limits);
        let part = "visio/theme/theme1.xml";
        let root = parse_xml(
            br#"<a:theme xmlns:a='http://schemas.openxmlformats.org/drawingml/2006/main'><a:themeElements/></a:theme>"#,
            part,
            &mut budget,
        )
        .unwrap();
        assert!(matches!(
            parse_theme(&root, part),
            Err(VsdxError::MalformedXml { message, .. }) if message == "theme is missing themeElements/clrScheme"
        ));
    }

    #[test]
    fn maps_catalog_ids_through_child_relationships() {
        let limits = ParseLimits::default();
        let mut budget = ParseBudget::new(&limits);
        let root = parse_xml(
            br#"<Masters><Master ID='15'><Rel r:id='rId2'/></Master></Masters>"#,
            "visio/masters/masters.xml",
            &mut budget,
        )
        .unwrap();
        let relationships = [Relationship {
            id: "rId2".into(),
            relationship_type: relationship_types::MASTER.into(),
            target: "master2.xml".into(),
            target_mode: crate::TargetMode::Internal,
            resolved_target: Some("visio/masters/master2.xml".into()),
        }];

        assert_eq!(
            catalog_part_ids(Some(&root), "Master", Some(&relationships)),
            [("visio/masters/master2.xml".into(), 15)].into()
        );
    }
    use std::path::PathBuf;

    #[test]
    fn round_trips_committed_fixture_parts_in_order_and_byte_for_byte() {
        let source = include_bytes!("../tests/fixtures/foundation.vsdx");
        let written = write_vsdx(&parse_vsdx(source).unwrap()).unwrap();
        assert_eq!(unzip_parts(&written).unwrap(), unzip_parts(source).unwrap());
    }

    #[test]
    fn preserves_external_corpus_parts_when_available() {
        let Some(directory) = std::env::var_os("VSDX_CORPUS_DIR") else {
            eprintln!("WARNING: VSDX corpus test skipped; VSDX_CORPUS_DIR is unset");
            return;
        };
        for file in ["lichtsysteme.vsdx", "soundplan.vsdx"] {
            let path = PathBuf::from(&directory).join(file);
            assert!(path.is_file(), "missing corpus file: {}", path.display());
            let source = std::fs::read(&path).unwrap();
            let written = write_vsdx(&parse_vsdx(&source).unwrap()).unwrap();
            assert_eq!(
                parse_vsdx(&written).unwrap().page_contents,
                parse_vsdx(&source).unwrap().page_contents,
                "{}",
                path.display()
            );
            assert_eq!(
                unzip_parts(&written).unwrap(),
                unzip_parts(&source).unwrap(),
                "{}",
                path.display()
            );
        }
    }

    #[test]
    fn models_lossless_shapesheet_features() {
        let package = parse_vsdx(include_bytes!("../tests/fixtures/foundation.vsdx")).unwrap();
        let sheet = &package.page_contents["visio/pages/page1.xml"];
        let shape = sheet.shapes().next().unwrap();
        assert!(
            shape
                .cells()
                .any(|cell| cell.name == "LineWeight" && cell.del)
        );
        assert!(
            shape
                .sections()
                .any(|section| section.name == "Scratch" && section.del)
        );
        let geometry = shape
            .sections()
            .find(|section| section.name == "Geometry")
            .unwrap();
        let geometry_rows: Vec<_> = geometry.rows().collect();
        assert_eq!(geometry_rows[1].name.as_deref(), Some("LineTo"));
        assert_eq!(geometry_rows[2].index, Some(2));
        assert!(geometry_rows[2].del);
        assert_eq!(
            shape.text(),
            Some(
                [
                    crate::TextToken::Literal(" A".to_owned()),
                    crate::TextToken::CharacterRun(1),
                    crate::TextToken::Literal("B".to_owned()),
                    crate::TextToken::ParagraphRun(2),
                    crate::TextToken::Tab(3),
                    crate::TextToken::Field(0),
                    crate::TextToken::Literal(" C ".to_owned())
                ]
                .as_slice()
            )
        );
        assert_eq!(sheet.connects().next().unwrap().from_part, Some(9));
        assert_eq!(sheet.connects().next().unwrap().to_part, Some(3));
        assert_eq!(
            package.page_sheets[&1].cells().next().unwrap().name,
            "PageWidth"
        );
        assert_eq!(
            package.master_sheets[&1].cells().next().unwrap().name,
            "PageHeight"
        );
        assert_eq!(geometry_rows[0].local_name.as_deref(), Some("Start"));
        assert!(
            shape
                .cells()
                .any(|cell| cell.name == "FOnly" && cell.formula.is_some() && cell.value.is_none())
        );
        assert!(
            shape
                .cells()
                .any(|cell| cell.name == "VOnly" && cell.formula.is_none() && cell.value.is_some())
        );
        assert!(
            shape
                .cells()
                .any(|cell| cell.name == "Both" && cell.formula.is_some() && cell.value.is_some())
        );
        let user = shape
            .sections()
            .find(|section| section.name == "User")
            .unwrap();
        let user_rows: Vec<_> = user.rows().collect();
        assert_eq!(user_rows[0].name.as_deref(), Some("visVersion"));
        assert_eq!(user_rows[1].index, Some(3));
        assert_eq!(user_rows[1].row_type.as_deref(), Some("UnknownRow"));
        assert_eq!(
            user_rows[1].cells().next().unwrap().other_attrs,
            [("UnknownAttr".to_owned(), "kept".to_owned())]
        );
        let written = write_vsdx(&package).unwrap();
        assert_eq!(
            parse_vsdx(&written).unwrap().page_contents,
            package.page_contents
        );
    }

    #[test]
    fn model_serializer_matches_original_parts_and_reparse() {
        assert_model_serialization(
            include_bytes!("../tests/fixtures/foundation.vsdx"),
            "foundation.vsdx",
        );
        let Some(directory) = std::env::var_os("VSDX_CORPUS_DIR") else {
            eprintln!("WARNING: VSDX corpus comparator skipped; VSDX_CORPUS_DIR is unset");
            return;
        };
        for file in ["lichtsysteme.vsdx", "soundplan.vsdx"] {
            let path = PathBuf::from(&directory).join(file);
            assert!(path.is_file(), "missing corpus file: {}", path.display());
            assert_model_serialization(&std::fs::read(&path).unwrap(), &path.display().to_string());
        }
    }

    fn assert_model_serialization(source: &[u8], label: &str) {
        let package = parse_vsdx(source).unwrap();
        let original = unzip_parts(source).unwrap();
        let document_bytes = original
            .iter()
            .find(|(path, _)| path == &package.document_part_path)
            .unwrap()
            .1
            .as_slice();
        let limits = ParseLimits::default();
        let mut budget = ParseBudget::new(&limits);
        let document = parse_xml(document_bytes, &package.document_part_path, &mut budget).unwrap();
        if let Some(sheet) = &package.document_sheet {
            assert_sheet_matches_element(
                sheet,
                "DocumentSheet",
                document.children_named("DocumentSheet").next().unwrap(),
                label,
            );
        }
        for (sheet, original) in package.style_sheets.iter().zip(
            document
                .children_named("StyleSheets")
                .flat_map(|styles| styles.children_named("StyleSheet")),
        ) {
            assert_sheet_matches_element(sheet, "StyleSheet", original, label);
        }
        if let Some(path) = &package.pages_part_path {
            assert_catalog_sheets(&original, path, "Page", &package.page_sheets, label);
        }
        if let Some(path) = &package.masters_part_path {
            assert_catalog_sheets(&original, path, "Master", &package.master_sheets, label);
        }
        for (path, sheet) in &package.page_contents {
            let original = original
                .iter()
                .find(|(candidate, _)| candidate == path)
                .unwrap()
                .1
                .as_slice();
            let serialized = serialize_sheet("PageContents", sheet);
            assert_canonical_xml_eq(original, serialized.as_bytes(), &format!("{label}:{path}"));
            let reparsed = parse_sheet_for_test(serialized.as_bytes(), path);
            assert_eq!(&reparsed, sheet, "{label}:{path}");
        }
        for (path, sheet) in &package.master_contents {
            let original = original
                .iter()
                .find(|(candidate, _)| candidate == path)
                .unwrap()
                .1
                .as_slice();
            let serialized = serialize_sheet("MasterContents", sheet);
            assert_canonical_xml_eq(original, serialized.as_bytes(), &format!("{label}:{path}"));
            let reparsed = parse_sheet_for_test(serialized.as_bytes(), path);
            assert_eq!(&reparsed, sheet, "{label}:{path}");
        }
    }

    fn assert_catalog_sheets(
        original: &[(String, Vec<u8>)],
        path: &str,
        item: &str,
        sheets: &BTreeMap<u32, Sheet>,
        label: &str,
    ) {
        let bytes = original
            .iter()
            .find(|(candidate, _)| candidate == path)
            .unwrap()
            .1
            .as_slice();
        let limits = ParseLimits::default();
        let mut budget = ParseBudget::new(&limits);
        let root = parse_xml(bytes, path, &mut budget).unwrap();
        for item_element in root.children_named(item) {
            if let Some(original) = item_element.children_named("PageSheet").next() {
                let id = item_element.attribute("ID").unwrap().parse().unwrap();
                assert_sheet_matches_element(&sheets[&id], "PageSheet", original, label);
            }
        }
    }

    fn assert_sheet_matches_element(sheet: &Sheet, root: &str, original: &XmlElement, label: &str) {
        let serialized = serialize_sheet(root, sheet);
        let limits = ParseLimits::default();
        let mut budget = ParseBudget::new(&limits);
        let serialized = parse_xml(serialized.as_bytes(), label, &mut budget).unwrap();
        assert_eq!(canonical(original), canonical(&serialized), "{label}");
        assert_eq!(
            parse_sheet_for_test(serialize_sheet(root, sheet).as_bytes(), label),
            *sheet,
            "{label}"
        );
    }

    fn parse_sheet_for_test(bytes: &[u8], part: &str) -> Sheet {
        let limits = ParseLimits::default();
        let mut budget = ParseBudget::new(&limits);
        let root = parse_xml(bytes, part, &mut budget).unwrap();
        parse_sheet(&root, part, &mut budget).unwrap()
    }

    fn assert_canonical_xml_eq(left: &[u8], right: &[u8], label: &str) {
        let limits = ParseLimits::default();
        let mut budget = ParseBudget::new(&limits);
        let left = parse_xml(left, label, &mut budget).unwrap();
        let mut budget = ParseBudget::new(&limits);
        let right = parse_xml(right, label, &mut budget).unwrap();
        assert_eq!(canonical(&left), canonical(&right), "{label}");
    }

    #[derive(Debug, PartialEq, Eq)]
    enum CanonicalNode {
        Element(String, Vec<(String, String)>, Vec<CanonicalNode>),
        Text(String),
    }
    fn canonical(element: &XmlElement) -> CanonicalNode {
        let mut attributes = canonical_attributes(element);
        attributes.sort();
        let mut children = Vec::new();
        for child in &element.children {
            match child {
                XmlNode::Element(element) => children.push(CanonicalNode::Element(
                    element.local_name().to_owned(),
                    canonical_attributes(element),
                    canonical_children(element),
                )),
                XmlNode::Text(text) if !text.is_empty() => {
                    if let Some(CanonicalNode::Text(previous)) = children.last_mut() {
                        previous.push_str(text);
                    } else {
                        children.push(CanonicalNode::Text(text.clone()));
                    }
                }
                XmlNode::Text(_) => {}
            }
        }
        CanonicalNode::Element(element.local_name().to_owned(), attributes, children)
    }
    fn canonical_attributes(element: &XmlElement) -> Vec<(String, String)> {
        let mut attributes = element
            .attributes
            .iter()
            .filter(|(name, _)| name != "xmlns" && !name.starts_with("xmlns:"))
            .map(|(name, value)| {
                (
                    name.rsplit_once(':')
                        .map_or(name.as_str(), |(_, local)| local)
                        .to_owned(),
                    value.clone(),
                )
            })
            .collect::<Vec<_>>();
        attributes.sort();
        attributes
    }
    fn canonical_children(element: &XmlElement) -> Vec<CanonicalNode> {
        match canonical(element) {
            CanonicalNode::Element(_, _, children) => children,
            CanonicalNode::Text(_) => unreachable!(),
        }
    }

    #[test]
    fn enforces_sheet_resource_boundaries() {
        let source = include_bytes!("../tests/fixtures/foundation.vsdx");
        for limits in [
            ParseLimits {
                max_cells: 0,
                ..ParseLimits::default()
            },
            ParseLimits {
                max_sections: 0,
                ..ParseLimits::default()
            },
            ParseLimits {
                max_rows: 0,
                ..ParseLimits::default()
            },
            ParseLimits {
                max_shapes: 0,
                ..ParseLimits::default()
            },
        ] {
            assert!(matches!(
                parse_vsdx_with_limits(source, &limits),
                Err(VsdxError::ResourceLimit { .. })
            ));
        }
    }

    #[test]
    fn enforces_exact_package_wide_sheet_budgets() {
        let source = rezip_parts(&[
            ("[Content_Types].xml".to_owned(), br#"<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/visio/document.xml' ContentType='application/vnd.ms-visio.drawing.main+xml'/></Types>"#.to_vec()),
            ("_rels/.rels".to_owned(), br#"<Relationships><Relationship Id='r1' Type='http://schemas.microsoft.com/visio/2010/relationships/document' Target='visio/document.xml'/></Relationships>"#.to_vec()),
            ("visio/document.xml".to_owned(), br#"<VisioDocument><DocumentSheet><Cell N='DocumentCell'/><Section N='DocumentSection'><Row IX='0'><Cell N='DocumentRowCell'/></Row></Section></DocumentSheet><StyleSheets><StyleSheet ID='1'><Cell N='StyleCell'/><Section N='StyleSection'><Row IX='0'><Cell N='StyleRowCell'/></Row></Section></StyleSheet></StyleSheets></VisioDocument>"#.to_vec()),
            ("visio/_rels/document.xml.rels".to_owned(), br#"<Relationships><Relationship Id='r1' Type='http://schemas.microsoft.com/visio/2010/relationships/pages' Target='pages/pages.xml'/></Relationships>"#.to_vec()),
            ("visio/pages/pages.xml".to_owned(), br#"<Pages><Page ID='1'/></Pages>"#.to_vec()),
            ("visio/pages/_rels/pages.xml.rels".to_owned(), br#"<Relationships><Relationship Id='r1' Type='http://schemas.microsoft.com/visio/2010/relationships/page' Target='page1.xml'/></Relationships>"#.to_vec()),
            ("visio/pages/page1.xml".to_owned(), br#"<PageContents><Cell N='PageCell'/><Section N='PageSection'><Row IX='0'><Cell N='PageRowCell'/></Row></Section><Shapes><Shape ID='1'><Cell N='ShapeCell'/><Section N='ShapeSection'><Row IX='0'><Cell N='ShapeRowCell0'/></Row><Row IX='1'><Cell N='ShapeRowCell1'/></Row></Section><Shapes><Shape ID='2'><Cell N='NestedShapeCell'/></Shape></Shapes></Shape></Shapes></PageContents>"#.to_vec()),
        ]).unwrap();
        // Fixture totals: 10 cells, 4 sections, 5 rows, 2 shapes.
        let limits_for = |kind: &str, value: usize| match kind {
            "cells" => ParseLimits {
                max_cells: value,
                ..ParseLimits::default()
            },
            "sections" => ParseLimits {
                max_sections: value,
                ..ParseLimits::default()
            },
            "rows" => ParseLimits {
                max_rows: value,
                ..ParseLimits::default()
            },
            "shapes" => ParseLimits {
                max_shapes: value,
                ..ParseLimits::default()
            },
            _ => unreachable!(),
        };
        for (kind, minimum) in [("cells", 10), ("sections", 4), ("rows", 5), ("shapes", 2)] {
            assert!(parse_vsdx_with_limits(&source, &limits_for(kind, minimum)).is_ok());
            assert!(matches!(
                parse_vsdx_with_limits(&source, &limits_for(kind, minimum - 1)),
                Err(VsdxError::ResourceLimit { kind: actual, .. }) if actual == kind
            ));
        }
    }

    #[test]
    fn models_namespaced_deleted_sheet_content() {
        let source = include_bytes!("../tests/fixtures/foundation.vsdx");
        let mut parts = unzip_parts(source).unwrap();
        let page = parts
            .iter_mut()
            .find(|(path, _)| path == "visio/pages/page1.xml")
            .unwrap();
        page.1 = br#"<v:PageContents xmlns:v='urn:visio'><v:Shapes><v:Shape ID='1' Del='1'><v:Cell N='PinX' Del='1'/><v:Section N='Geometry' Del='1'><v:Row IX='0' Del='1'/></v:Section></v:Shape></v:Shapes></v:PageContents>"#.to_vec();
        let package = parse_vsdx(&rezip_parts(&parts).unwrap()).unwrap();
        let shape = package.page_contents["visio/pages/page1.xml"]
            .shapes()
            .next()
            .unwrap();
        assert!(shape.del);
        assert!(shape.cells().next().unwrap().del);
        assert!(shape.sections().next().unwrap().del);
        assert!(shape.sections().next().unwrap().rows().next().unwrap().del);
    }

    #[test]
    fn discovers_parts_only_through_relationships() {
        let package = rezip_parts(&[
            ("[Content_Types].xml".to_owned(), br#"<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/visio/document.xml' ContentType='application/vnd.ms-visio.drawing.main+xml'/></Types>"#.to_vec()),
            ("_rels/.rels".to_owned(), br#"<Relationships><Relationship Id='r1' Type='http://schemas.microsoft.com/visio/2010/relationships/document' Target='visio/document.xml'/></Relationships>"#.to_vec()),
            ("visio/document.xml".to_owned(), b"<VisioDocument/>".to_vec()),
            ("visio/_rels/document.xml.rels".to_owned(), br#"<Relationships><Relationship Id='r1' Type='http://schemas.microsoft.com/visio/2010/relationships/pages' Target='pages/pages.xml'/></Relationships>"#.to_vec()),
            ("visio/pages/pages.xml".to_owned(), b"<Pages/>".to_vec()),
            ("visio/pages/_rels/pages.xml.rels".to_owned(), br#"<Relationships><Relationship Id='r1' Type='http://schemas.microsoft.com/visio/2010/relationships/page' Target='page9.xml'/></Relationships>"#.to_vec()),
            ("visio/pages/page9.xml".to_owned(), b"<PageContents/>".to_vec()),
        ]).unwrap();
        let parsed = parse_vsdx(&package).unwrap();
        assert_eq!(parsed.page_part_paths, ["visio/pages/page9.xml"]);
    }

    #[test]
    fn rejects_dangling_relationship_targets() {
        let package = rezip_parts(&[
            ("[Content_Types].xml".to_owned(), br#"<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/visio/document.xml' ContentType='application/vnd.ms-visio.drawing.main+xml'/></Types>"#.to_vec()),
            (
                "_rels/.rels".to_owned(),
                br#"<Relationships><Relationship Id='r1' Type='http://schemas.microsoft.com/visio/2010/relationships/document' Target='visio/document.xml'/></Relationships>"#.to_vec(),
            ),
        ])
        .unwrap();
        assert!(matches!(
            parse_vsdx(&package),
            Err(VsdxError::MissingPart(_))
        ));
    }

    #[test]
    fn validates_reachable_page_relationship_parts() {
        let package = rezip_parts(&[
            ("[Content_Types].xml".to_owned(), br#"<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/visio/document.xml' ContentType='application/vnd.ms-visio.drawing.main+xml'/></Types>"#.to_vec()),
            ("_rels/.rels".to_owned(), br#"<Relationships><Relationship Id='r1' Type='http://schemas.microsoft.com/visio/2010/relationships/document' Target='visio/document.xml'/></Relationships>"#.to_vec()),
            ("visio/document.xml".to_owned(), b"<VisioDocument/>".to_vec()),
            ("visio/_rels/document.xml.rels".to_owned(), br#"<Relationships><Relationship Id='r1' Type='http://schemas.microsoft.com/visio/2010/relationships/pages' Target='pages/pages.xml'/></Relationships>"#.to_vec()),
            ("visio/pages/pages.xml".to_owned(), b"<Pages/>".to_vec()),
            ("visio/pages/_rels/pages.xml.rels".to_owned(), br#"<Relationships><Relationship Id='r1' Type='http://schemas.microsoft.com/visio/2010/relationships/page' Target='page1.xml'/></Relationships>"#.to_vec()),
            ("visio/pages/page1.xml".to_owned(), b"<PageContents/>".to_vec()),
            ("visio/pages/_rels/page1.xml.rels".to_owned(), br#"<Relationships><Relationship Id='r1' Type='x/image' Target='javascript:x'/></Relationships>"#.to_vec()),
        ]).unwrap();
        assert!(matches!(
            parse_vsdx(&package),
            Err(VsdxError::InvalidRelationship { .. })
        ));
    }

    #[test]
    fn rejects_unsupported_or_conflicting_document_kinds() {
        for content_type in [
            "application/vnd.ms-visio.drawing.macroEnabled.main+xml",
            "application/vnd.ms-visio.stencil.main+xml",
            "application/vnd.ms-visio.template.main+xml",
        ] {
            let package = rezip_parts(&[(
                "[Content_Types].xml".to_owned(),
                format!("<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/visio/document.xml' ContentType='{content_type}'/></Types>").into_bytes(),
            )]).unwrap();
            assert!(matches!(
                parse_vsdx(&package),
                Err(VsdxError::UnsupportedDocumentKind(_))
            ));
        }
        let package = rezip_parts(&[(
            "[Content_Types].xml".to_owned(),
            br#"<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/visio/document.xml' ContentType='application/vnd.ms-visio.drawing.main+xml'/><Override PartName='/word/document.xml' ContentType='application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml'/></Types>"#.to_vec(),
        )]).unwrap();
        assert!(matches!(
            parse_vsdx(&package),
            Err(VsdxError::ConflictingDocumentKinds(_))
        ));
    }

    #[test]
    fn rejects_missing_or_wrong_content_types() {
        let missing = rezip_parts(&[(
            "visio/document.xml".to_owned(),
            b"<VisioDocument/>".to_vec(),
        )])
        .unwrap();
        assert!(parse_vsdx(&missing).is_err());
        let wrong = rezip_parts(&[
            ("[Content_Types].xml".to_owned(), br#"<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/word/document.xml' ContentType='application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml'/></Types>"#.to_vec()),
            ("visio/document.xml".to_owned(), b"<VisioDocument/>".to_vec()),
        ]).unwrap();
        assert!(matches!(
            parse_vsdx(&wrong),
            Err(VsdxError::UnsupportedDocumentKind(_))
        ));
    }
}
