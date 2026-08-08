use std::collections::{BTreeMap, BTreeSet, HashSet};

use quick_xml::events::{BytesCData, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer, XmlVersion};

use crate::{rezip_parts, unzip_parts};

pub fn sanitize_package(data: &[u8]) -> Result<Vec<u8>, String> {
    sanitize_package_inner(data, None)
}

pub fn sanitize_package_for_format(data: &[u8], expected_format: &str) -> Result<Vec<u8>, String> {
    if !matches!(expected_format, "docx" | "xlsx" | "pptx" | "vsdx") {
        return Err(format!("unsupported OOXML format: {expected_format}"));
    }
    sanitize_package_inner(data, Some(expected_format))
}

fn sanitize_package_inner(data: &[u8], expected_format: Option<&str>) -> Result<Vec<u8>, String> {
    let mut parts = unzip_parts(data)?;
    let detected = detect_format(&parts)?;
    if let Some(expected) = expected_format
        && detected.format() != expected
    {
        return Err(format!(
            "claimed {expected} content does not match detected {} package",
            detected.format()
        ));
    }

    let mut removed: HashSet<String> = parts
        .iter()
        .filter(|(path, _)| dangerous_path(path))
        .map(|(path, _)| normalize_part_name(path))
        .collect();
    if let Some((_, content_types)) = parts
        .iter()
        .find(|(path, _)| path.eq_ignore_ascii_case("[Content_Types].xml"))
    {
        removed.extend(dangerous_content_type_parts(content_types)?);
    }

    parts.retain(|(path, _)| !removed.contains(&normalize_part_name(path)));
    for (path, bytes) in &mut parts {
        let lower = path.to_ascii_lowercase();
        if lower == "[content_types].xml" {
            *bytes = sanitize_content_types(bytes, &removed, path)?;
        } else if lower.ends_with(".rels") {
            *bytes = sanitize_relationships(bytes, &removed, path)?;
        } else if is_xml_part(&lower) {
            *bytes = neutralize_fields(bytes, path)?;
        }
    }
    rezip_parts(&parts)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DocumentKind {
    Docx,
    Xlsx,
    Pptx,
    Vsdx,
    Vsdm,
    Vssx,
    Vstx,
}

impl DocumentKind {
    pub fn format(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
            Self::Vsdx => "vsdx",
            Self::Vsdm => "vsdm",
            Self::Vssx => "vssx",
            Self::Vstx => "vstx",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentKindError {
    MissingContentTypes,
    InvalidContentTypes(String),
    MissingMainDocumentKind,
    ConflictingDocumentKinds(Vec<DocumentKind>),
}

impl std::fmt::Display for DocumentKindError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingContentTypes => formatter.write_str("missing [Content_Types].xml"),
            Self::InvalidContentTypes(error) => {
                write!(formatter, "invalid [Content_Types].xml: {error}")
            }
            Self::MissingMainDocumentKind => {
                formatter.write_str("missing recognized main document content type")
            }
            Self::ConflictingDocumentKinds(kinds) => write!(
                formatter,
                "conflicting main document content types: {kinds:?}"
            ),
        }
    }
}

impl std::error::Error for DocumentKindError {}

pub fn detect_package_kind(parts: &[(String, Vec<u8>)]) -> Result<DocumentKind, DocumentKindError> {
    let (_, bytes) = parts
        .iter()
        .find(|(path, _)| path.eq_ignore_ascii_case("[Content_Types].xml"))
        .ok_or(DocumentKindError::MissingContentTypes)?;
    let mut reader = Reader::from_reader(bytes.as_slice());
    let mut kinds = BTreeSet::new();
    let mut overrides = BTreeMap::new();
    let mut defaults = BTreeMap::new();
    let mut depth = 0_usize;
    let mut content_types_root = false;
    loop {
        match reader
            .read_event()
            .map_err(|error| DocumentKindError::InvalidContentTypes(error.to_string()))?
        {
            Event::Start(start) => {
                let local = start.name().local_name();
                if depth == 0 {
                    content_types_root =
                        local.as_ref() == b"Types" && has_content_types_namespace(&reader, &start)?;
                } else if content_types_root
                    && depth == 1
                    && matches!(local.as_ref(), b"Override" | b"Default")
                {
                    let values = content_type_attributes(&reader, &start)?;
                    if local.as_ref() == b"Override"
                        && let (Some(part_name), Some(content_type)) = (
                            attribute_value(&values, "PartName"),
                            attribute_value(&values, "ContentType"),
                        )
                    {
                        let part_name = normalize_part_name(part_name);
                        if let Some(kind) = declared_main_kind(&part_name, content_type) {
                            kinds.insert(kind);
                        }
                        overrides.insert(part_name, content_type.to_owned());
                    } else if local.as_ref() == b"Default"
                        && let (Some(extension), Some(content_type)) = (
                            attribute_value(&values, "Extension"),
                            attribute_value(&values, "ContentType"),
                        )
                    {
                        defaults.insert(extension.to_ascii_lowercase(), content_type.to_owned());
                    }
                }
                depth += 1;
            }
            Event::Empty(start) => {
                let local = start.name().local_name();
                if depth == 0 {
                    content_types_root =
                        local.as_ref() == b"Types" && has_content_types_namespace(&reader, &start)?;
                } else if content_types_root
                    && depth == 1
                    && matches!(local.as_ref(), b"Override" | b"Default")
                {
                    let values = content_type_attributes(&reader, &start)?;
                    if local.as_ref() == b"Override"
                        && let (Some(part_name), Some(content_type)) = (
                            attribute_value(&values, "PartName"),
                            attribute_value(&values, "ContentType"),
                        )
                    {
                        let part_name = normalize_part_name(part_name);
                        if let Some(kind) = declared_main_kind(&part_name, content_type) {
                            kinds.insert(kind);
                        }
                        overrides.insert(part_name, content_type.to_owned());
                    } else if local.as_ref() == b"Default"
                        && let (Some(extension), Some(content_type)) = (
                            attribute_value(&values, "Extension"),
                            attribute_value(&values, "ContentType"),
                        )
                    {
                        defaults.insert(extension.to_ascii_lowercase(), content_type.to_owned());
                    }
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::DocType(_) => {
                return Err(DocumentKindError::InvalidContentTypes(
                    "DTD is forbidden".to_owned(),
                ));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    for part_name in [
        "word/document.xml",
        "xl/workbook.xml",
        "ppt/presentation.xml",
        "visio/document.xml",
    ] {
        if has_part(parts, part_name)
            && !overrides.contains_key(part_name)
            && let Some((_, extension)) = part_name.rsplit_once('.')
            && let Some(content_type) = defaults.get(extension)
            && let Some(kind) = declared_main_kind(part_name, content_type)
        {
            kinds.insert(kind);
        }
    }
    // DOCX/XLSX/PPTX conflicts retain f1439b4's priority order for additive
    // compatibility. Any conflict involving a Visio main kind is rejected.
    if kinds.iter().all(|kind| {
        matches!(
            kind,
            DocumentKind::Docx | DocumentKind::Xlsx | DocumentKind::Pptx
        )
    }) {
        return [DocumentKind::Docx, DocumentKind::Xlsx, DocumentKind::Pptx]
            .into_iter()
            .find(|kind| kinds.contains(kind))
            .ok_or(DocumentKindError::MissingMainDocumentKind);
    }
    match kinds.len() {
        0 => Err(DocumentKindError::MissingMainDocumentKind),
        1 => Ok(*kinds.first().expect("length checked")),
        _ => Err(DocumentKindError::ConflictingDocumentKinds(
            kinds.into_iter().collect(),
        )),
    }
}

fn has_content_types_namespace(
    reader: &Reader<&[u8]>,
    start: &BytesStart<'_>,
) -> Result<bool, DocumentKindError> {
    Ok(content_type_attributes(reader, start)?
        .iter()
        .any(|(key, value)| {
            key == "xmlns"
                && value == "http://schemas.openxmlformats.org/package/2006/content-types"
        }))
}

fn content_type_attributes(
    reader: &Reader<&[u8]>,
    start: &BytesStart<'_>,
) -> Result<Vec<(String, String)>, DocumentKindError> {
    start
        .attributes()
        .map(|attribute| {
            let attribute = attribute
                .map_err(|error| DocumentKindError::InvalidContentTypes(error.to_string()))?;
            let key = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
            let value = attribute
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
                .map_err(|error| DocumentKindError::InvalidContentTypes(error.to_string()))?
                .into_owned();
            Ok((key, value))
        })
        .collect()
}

fn declared_main_kind(part_name: &str, content_type: &str) -> Option<DocumentKind> {
    let part_name = normalize_part_name(part_name);
    let content_type = content_type.to_ascii_lowercase();
    match (part_name.as_str(), content_type.as_str()) {
        (
            "word/document.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
            | "application/vnd.ms-word.document.macroenabled.main+xml",
        ) => Some(DocumentKind::Docx),
        (
            "xl/workbook.xml",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
            | "application/vnd.ms-excel.sheet.macroenabled.main+xml",
        ) => Some(DocumentKind::Xlsx),
        (
            "ppt/presentation.xml",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"
            | "application/vnd.ms-powerpoint.presentation.macroenabled.main+xml",
        ) => Some(DocumentKind::Pptx),
        ("visio/document.xml", "application/vnd.ms-visio.drawing.main+xml") => {
            Some(DocumentKind::Vsdx)
        }
        ("visio/document.xml", "application/vnd.ms-visio.drawing.macroenabled.main+xml") => {
            Some(DocumentKind::Vsdm)
        }
        (
            "visio/document.xml",
            "application/vnd.ms-visio.stencil.main+xml"
            | "application/vnd.ms-visio.stencil.macroenabled.main+xml",
        ) => Some(DocumentKind::Vssx),
        (
            "visio/document.xml",
            "application/vnd.ms-visio.template.main+xml"
            | "application/vnd.ms-visio.template.macroenabled.main+xml",
        ) => Some(DocumentKind::Vstx),
        _ => None,
    }
}

fn detect_format(parts: &[(String, Vec<u8>)]) -> Result<DocumentKind, String> {
    match detect_package_kind(parts) {
        Ok(kind) => return Ok(kind),
        Err(DocumentKindError::ConflictingDocumentKinds(kinds)) => {
            return Err(DocumentKindError::ConflictingDocumentKinds(kinds).to_string());
        }
        Err(_) => {}
    }
    if has_part(parts, "word/document.xml") {
        Ok(DocumentKind::Docx)
    } else if has_part(parts, "xl/workbook.xml") {
        Ok(DocumentKind::Xlsx)
    } else if has_part(parts, "ppt/presentation.xml") {
        Ok(DocumentKind::Pptx)
    } else {
        Err("could not detect DOCX, XLSX, PPTX, or VSDX package".to_owned())
    }
}

fn has_part(parts: &[(String, Vec<u8>)], expected: &str) -> bool {
    parts
        .iter()
        .any(|(path, _)| path.eq_ignore_ascii_case(expected))
}

fn dangerous_content_type_parts(xml: &[u8]) -> Result<HashSet<String>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut paths = HashSet::new();
    loop {
        match reader
            .read_event()
            .map_err(|error| format!("invalid [Content_Types].xml: {error}"))?
        {
            Event::Start(start) | Event::Empty(start)
                if start.name().local_name().as_ref() == b"Override" =>
            {
                let attributes = attributes(&reader, &start, "[Content_Types].xml")?;
                let part_name = attribute_value(&attributes, "PartName");
                let content_type = attribute_value(&attributes, "ContentType");
                if let (Some(part_name), Some(content_type)) = (part_name, content_type)
                    && dangerous_content_type(content_type)
                {
                    paths.insert(normalize_part_name(part_name));
                }
            }
            Event::DocType(_) => return Err("DTD is forbidden in [Content_Types].xml".to_owned()),
            Event::Eof => return Ok(paths),
            _ => {}
        }
    }
}

fn sanitize_content_types(
    xml: &[u8],
    removed: &HashSet<String>,
    path: &str,
) -> Result<Vec<u8>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut writer = Writer::new(Vec::with_capacity(xml.len()));
    let mut skip_depth = 0_usize;
    loop {
        let event = reader
            .read_event()
            .map_err(|error| format!("invalid XML in {path}: {error}"))?;
        if skip_depth > 0 {
            match event {
                Event::Start(_) => skip_depth += 1,
                Event::End(_) => skip_depth -= 1,
                Event::Eof => return Err(format!("unexpected EOF in {path}")),
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(start) => {
                let local = start.name().local_name();
                let name = local.as_ref();
                let values = attributes(&reader, &start, path)?;
                if remove_content_type_entry(name, &values, removed) {
                    skip_depth = 1;
                } else {
                    write_start(
                        &mut writer,
                        rewrite_content_type(start, values),
                        false,
                        path,
                    )?;
                }
            }
            Event::Empty(start) => {
                let local = start.name().local_name();
                let name = local.as_ref();
                let values = attributes(&reader, &start, path)?;
                if !remove_content_type_entry(name, &values, removed) {
                    write_start(&mut writer, rewrite_content_type(start, values), true, path)?;
                }
            }
            Event::DocType(_) => return Err(format!("DTD is forbidden in {path}")),
            Event::Comment(_) | Event::PI(_) => {}
            Event::Eof => return Ok(writer.into_inner()),
            other => write(&mut writer, other, path)?,
        }
    }
}

fn remove_content_type_entry(
    element: &[u8],
    attributes: &[(String, String)],
    removed: &HashSet<String>,
) -> bool {
    let content_type = attribute_value(attributes, "ContentType");
    if content_type.is_some_and(dangerous_content_type) {
        return true;
    }
    element == b"Override"
        && attribute_value(attributes, "PartName")
            .is_some_and(|path| removed.contains(&normalize_part_name(path)))
}

fn rewrite_content_type(
    start: BytesStart<'_>,
    attributes: Vec<(String, String)>,
) -> BytesStart<'static> {
    let mut output = start.into_owned();
    output.clear_attributes();
    for (key, value) in attributes {
        let value = if attribute_local(&key) == "ContentType" {
            macro_free_content_type(&value).to_owned()
        } else {
            value
        };
        output.push_attribute((key.as_str(), value.as_str()));
    }
    output
}

fn macro_free_content_type(content_type: &str) -> &str {
    match content_type.to_ascii_lowercase().as_str() {
        "application/vnd.ms-word.document.macroenabled.main+xml" => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
        }
        "application/vnd.ms-excel.sheet.macroenabled.main+xml" => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
        }
        "application/vnd.ms-powerpoint.presentation.macroenabled.main+xml" => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"
        }
        _ => content_type,
    }
}

fn sanitize_relationships(
    xml: &[u8],
    removed: &HashSet<String>,
    path: &str,
) -> Result<Vec<u8>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut writer = Writer::new(Vec::with_capacity(xml.len()));
    let mut skip_depth = 0_usize;
    loop {
        let event = reader
            .read_event()
            .map_err(|error| format!("invalid XML in {path}: {error}"))?;
        if skip_depth > 0 {
            match event {
                Event::Start(_) => skip_depth += 1,
                Event::End(_) => skip_depth -= 1,
                Event::Eof => return Err(format!("unexpected EOF in {path}")),
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(start) => {
                let remove = start.name().local_name().as_ref() == b"Relationship"
                    && remove_relationship(&attributes(&reader, &start, path)?, removed, path);
                if remove {
                    skip_depth = 1;
                } else {
                    write_start(&mut writer, start.into_owned(), false, path)?;
                }
            }
            Event::Empty(start) => {
                let remove = start.name().local_name().as_ref() == b"Relationship"
                    && remove_relationship(&attributes(&reader, &start, path)?, removed, path);
                if !remove {
                    write_start(&mut writer, start.into_owned(), true, path)?;
                }
            }
            Event::DocType(_) => return Err(format!("DTD is forbidden in {path}")),
            Event::Comment(_) | Event::PI(_) => {}
            Event::Eof => return Ok(writer.into_inner()),
            other => write(&mut writer, other, path)?,
        }
    }
}

fn remove_relationship(
    attributes: &[(String, String)],
    removed: &HashSet<String>,
    relationship_path: &str,
) -> bool {
    let target = attribute_value(attributes, "Target").unwrap_or_default();
    let target_mode = attribute_value(attributes, "TargetMode").unwrap_or_default();
    let relationship_type = attribute_value(attributes, "Type").unwrap_or_default();
    target_mode.eq_ignore_ascii_case("External")
        || external_target(target)
        || dangerous_relationship_type(relationship_type)
        || dangerous_path(target)
        || resolve_relationship_target(relationship_path, target)
            .is_some_and(|target| removed.contains(&target))
}

fn neutralize_fields(xml: &[u8], path: &str) -> Result<Vec<u8>, String> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::with_capacity(xml.len()));
    let mut stack = Vec::new();
    let mut skip_depth = 0_usize;
    loop {
        let event = reader
            .read_event()
            .map_err(|error| format!("invalid XML in {path}: {error}"))?;
        if skip_depth > 0 {
            match event {
                Event::Start(_) => skip_depth += 1,
                Event::End(_) => skip_depth -= 1,
                Event::Eof => return Err(format!("unexpected EOF in {path}")),
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(start) => {
                let local =
                    String::from_utf8_lossy(start.name().local_name().as_ref()).into_owned();
                if dangerous_element(&local) {
                    skip_depth = 1;
                    continue;
                }
                let output = if local == "fldSimple" {
                    neutralize_instruction_attribute(&reader, start, path)?
                } else {
                    start.into_owned()
                };
                stack.push(local);
                write_start(&mut writer, output, false, path)?;
            }
            Event::Empty(start) => {
                let local = start.name().local_name();
                if dangerous_element(&String::from_utf8_lossy(local.as_ref())) {
                    continue;
                }
                let output = if local.as_ref() == b"fldSimple" {
                    neutralize_instruction_attribute(&reader, start, path)?
                } else {
                    start.into_owned()
                };
                write_start(&mut writer, output, true, path)?;
            }
            Event::End(end) => {
                stack.pop();
                write(&mut writer, Event::End(end), path)?;
            }
            Event::Text(_) if stack.last().is_some_and(|name| field_element(name)) => {
                write(&mut writer, Event::Text(BytesText::new("0")), path)?;
            }
            Event::CData(_) if stack.last().is_some_and(|name| field_element(name)) => {
                write(&mut writer, Event::CData(BytesCData::new("0")), path)?;
            }
            Event::GeneralRef(_) if stack.last().is_some_and(|name| field_element(name)) => {
                write(&mut writer, Event::Text(BytesText::new("0")), path)?;
            }
            Event::DocType(_) => return Err(format!("DTD is forbidden in {path}")),
            Event::Comment(_) | Event::PI(_) => {}
            Event::Eof => return Ok(writer.into_inner()),
            other => write(&mut writer, other, path)?,
        }
    }
}

fn neutralize_instruction_attribute(
    reader: &Reader<&[u8]>,
    start: BytesStart<'_>,
    path: &str,
) -> Result<BytesStart<'static>, String> {
    let mut output = start.into_owned();
    let values = attributes(reader, &output, path)?;
    output.clear_attributes();
    for (key, value) in values {
        let value = if attribute_local(&key) == "instr" {
            "0"
        } else {
            &value
        };
        output.push_attribute((key.as_str(), value));
    }
    Ok(output)
}

fn field_element(name: &str) -> bool {
    matches!(
        name,
        "instrText" | "delInstrText" | "f" | "formula" | "formula1" | "formula2"
    )
}

fn dangerous_element(name: &str) -> bool {
    matches!(
        name,
        "ddeLink" | "object" | "oleLink" | "oleObject" | "OLEObject" | "oleObj" | "control"
    )
}

fn attributes(
    reader: &Reader<&[u8]>,
    start: &BytesStart<'_>,
    path: &str,
) -> Result<Vec<(String, String)>, String> {
    start
        .attributes()
        .map(|attribute| {
            let attribute = attribute.map_err(|error| format!("invalid XML in {path}: {error}"))?;
            let key = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
            let value = attribute
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
                .map_err(|error| format!("invalid XML in {path}: {error}"))?
                .into_owned();
            Ok((key, value))
        })
        .collect()
}

fn attribute_value<'a>(attributes: &'a [(String, String)], expected: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(name, _)| attribute_local(name).eq_ignore_ascii_case(expected))
        .map(|(_, value)| value.as_str())
}

fn attribute_local(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

fn dangerous_path(path: &str) -> bool {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    path.ends_with("vbaproject.bin")
        || path.ends_with("vbadata.xml")
        || path.contains("/macrosheets/")
        || path.contains("/embeddings/")
        || path.contains("/activex/")
        || path.contains("/oleobject")
}

fn dangerous_content_type(content_type: &str) -> bool {
    let content_type = content_type.to_ascii_lowercase();
    content_type.contains("vbaproject")
        || content_type.contains("vbadata")
        || content_type.contains("macrosheet")
        || content_type.contains("oleobject")
        || content_type.contains("activex")
        || content_type.contains("ms-package")
}

fn dangerous_relationship_type(relationship_type: &str) -> bool {
    let relationship_type = relationship_type.to_ascii_lowercase();
    relationship_type.contains("vbaproject")
        || relationship_type.contains("macrosheet")
        || relationship_type.contains("oleobject")
        || relationship_type.ends_with("/package")
        || relationship_type.contains("activex")
        || relationship_type.ends_with("/control")
}

fn external_target(target: &str) -> bool {
    let lower = target.trim().to_ascii_lowercase();
    lower.starts_with("//")
        || lower.contains("://")
        || matches!(
            lower.split_once(':').map(|(scheme, _)| scheme),
            Some("file" | "mailto" | "ftp" | "javascript" | "data")
        )
}

fn resolve_relationship_target(relationship_path: &str, target: &str) -> Option<String> {
    if target.is_empty() || external_target(target) {
        return None;
    }
    let clean_target = target.split(['?', '#']).next().unwrap_or_default();
    let relationship_path = relationship_path.replace('\\', "/");
    let mut segments: Vec<String> =
        if clean_target.starts_with('/') || relationship_path.eq_ignore_ascii_case("_rels/.rels") {
            Vec::new()
        } else {
            relationship_path
                .split("/_rels/")
                .next()
                .unwrap_or_default()
                .split('/')
                .filter(|segment| !segment.is_empty())
                .map(str::to_owned)
                .collect()
        };
    let clean_target = clean_target.trim_start_matches('/').replace('\\', "/");
    for segment in clean_target
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
    {
        if segment == ".." {
            segments.pop()?;
        } else {
            segments.push(segment.to_owned());
        }
    }
    Some(segments.join("/").to_ascii_lowercase())
}

fn normalize_part_name(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

fn is_xml_part(path: &str) -> bool {
    path.ends_with(".xml") || path.ends_with(".vml")
}

fn write_start(
    writer: &mut Writer<Vec<u8>>,
    start: BytesStart<'_>,
    empty: bool,
    path: &str,
) -> Result<(), String> {
    if empty {
        write(writer, Event::Empty(start), path)
    } else {
        write(writer, Event::Start(start), path)
    }
}

fn write(writer: &mut Writer<Vec<u8>>, event: Event<'_>, path: &str) -> Result<(), String> {
    writer
        .write_event(event)
        .map_err(|error| format!("writing sanitized XML for {path}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_external_macro_and_embedded_attack_vectors() {
        let package = rezip_parts(&[
            (
                "[Content_Types].xml".to_owned(),
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="bin" ContentType="application/vnd.ms-office.vbaProject"/><Override PartName="/word/document.xml" ContentType="application/vnd.ms-word.document.macroEnabled.main+xml"/><Override PartName="/word/vbaProject.bin" ContentType="application/vnd.ms-office.vbaProject"/><Override PartName="/word/embeddings/object1.bin" ContentType="application/vnd.openxmlformats-officedocument.oleObject"/></Types>"#.to_vec(),
            ),
            (
                "word/document.xml".to_owned(),
                br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:o="urn:schemas-microsoft-com:office:office"><w:body><w:p><w:fldSimple w:instr="DDEAUTO secret"><w:r><w:instrText>HYPERLINK secret.example</w:instrText></w:r></w:fldSimple><w:object><o:OLEObject ProgID="Package"/></w:object></w:p></w:body></w:document>"#.to_vec(),
            ),
            (
                "word/_rels/document.xml.rels".to_owned(),
                br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://secret.example" TargetMode="External"/><Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/oleObject" Target="embeddings/object1.bin"/></Relationships>"#.to_vec(),
            ),
            ("word/vbaProject.bin".to_owned(), b"macro secret".to_vec()),
            (
                "word/embeddings/object1.bin".to_owned(),
                b"embedded secret".to_vec(),
            ),
        ])
        .unwrap();

        let sanitized = sanitize_package_for_format(&package, "docx").unwrap();
        let parts = unzip_parts(&sanitized).unwrap();
        assert_eq!(
            parts
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "[Content_Types].xml",
                "word/document.xml",
                "word/_rels/document.xml.rels"
            ]
        );
        let all = parts
            .iter()
            .map(|(_, bytes)| String::from_utf8_lossy(bytes))
            .collect::<String>();
        assert!(!all.contains("secret"));
        assert!(!all.contains("TargetMode"));
        assert!(!all.contains("vbaProject"));
        assert!(!all.contains("oleObject"));
        assert!(!all.contains("ProgID"));
        assert!(all.contains("wordprocessingml.document.main+xml"));
        assert!(all.contains("w:instr=\"0\""));
        assert!(all.contains("<w:instrText>0</w:instrText>"));
    }

    #[test]
    fn validates_claimed_format() {
        let package = rezip_parts(&[
            (
                "[Content_Types].xml".to_owned(),
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#.to_vec(),
            ),
            ("xl/workbook.xml".to_owned(), b"<workbook/>".to_vec()),
        ])
        .unwrap();
        assert!(sanitize_package_for_format(&package, "docx").is_err());
    }

    #[test]
    fn accepts_and_rezips_all_three_formats() {
        let cases = [
            (
                "docx",
                "word/document.xml",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
                "<w:document/>",
            ),
            (
                "xlsx",
                "xl/workbook.xml",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
                "<workbook><f>DDE secret</f><ddeLink ddeService=\"DDE secret\"/></workbook>",
            ),
            (
                "pptx",
                "ppt/presentation.xml",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
                "<p:presentation/>",
            ),
        ];
        for (format, part_name, content_type, main_xml) in cases {
            let content_types = format!(
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/{part_name}" ContentType="{content_type}"/></Types>"#
            );
            let package = rezip_parts(&[
                ("[Content_Types].xml".to_owned(), content_types.into_bytes()),
                (part_name.to_owned(), main_xml.as_bytes().to_vec()),
            ])
            .unwrap();
            let sanitized = sanitize_package_for_format(&package, format).unwrap();
            let parts = unzip_parts(&sanitized).unwrap();
            assert_eq!(parts.len(), 2);
            assert!(
                parts
                    .iter()
                    .all(|(_, bytes)| !String::from_utf8_lossy(bytes).contains("DDE secret"))
            );
        }
    }

    #[test]
    fn accepts_vsdx_and_preserves_visio_fields_and_formulas() {
        let package = rezip_parts(&[
            (
                "[Content_Types].xml".to_owned(),
                br#"<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/visio/document.xml' ContentType='application/vnd.ms-visio.drawing.main+xml'/></Types>"#.to_vec(),
            ),
            ("visio/document.xml".to_owned(), b"<VisioDocument/>".to_vec()),
            (
                "visio/pages/page1.xml".to_owned(),
                br#"<PageContents><Shapes><Shape><Cell N='PinX' F='Width*0.5' V='1'/><Text>Page <fld IX='0'/></Text></Shape></Shapes></PageContents>"#.to_vec(),
            ),
        ])
        .unwrap();
        let sanitized = sanitize_package_for_format(&package, "vsdx").unwrap();
        let parts = unzip_parts(&sanitized).unwrap();
        let page = parts
            .iter()
            .find(|(path, _)| path == "visio/pages/page1.xml")
            .unwrap();
        let xml = String::from_utf8_lossy(&page.1);
        assert!(xml.contains("F='Width*0.5'"));
        assert!(xml.contains("<fld IX='0'/>"));
    }

    #[test]
    fn rejects_recognized_non_drawing_visio_kinds() {
        for content_type in [
            "application/vnd.ms-visio.drawing.macroEnabled.main+xml",
            "application/vnd.ms-visio.stencil.main+xml",
            "application/vnd.ms-visio.template.main+xml",
        ] {
            let package = rezip_parts(&[
                (
                    "[Content_Types].xml".to_owned(),
                    format!("<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/visio/document.xml' ContentType='{content_type}'/></Types>").into_bytes(),
                ),
                ("visio/document.xml".to_owned(), b"<VisioDocument/>".to_vec()),
            ])
            .unwrap();
            assert!(sanitize_package_for_format(&package, "vsdx").is_err());
        }
    }

    #[test]
    fn content_type_is_authoritative_for_visio_kind_detection() {
        let macro_enabled = rezip_parts(&[
            ("[Content_Types].xml".to_owned(), br#"<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/visio/document.xml' ContentType='application/vnd.ms-visio.drawing.macroEnabled.main+xml'/></Types>"#.to_vec()),
            ("visio/document.xml".to_owned(), b"<VisioDocument/>".to_vec()),
        ]).unwrap();
        assert!(sanitize_package_for_format(&macro_enabled, "vsdx").is_err());

        let no_content_type = rezip_parts(&[(
            "visio/document.xml".to_owned(),
            b"<VisioDocument/>".to_vec(),
        )])
        .unwrap();
        assert!(sanitize_package_for_format(&no_content_type, "vsdx").is_err());
    }

    #[test]
    fn keeps_existing_format_detection_results() {
        let cases = [
            (
                "word/document.xml",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
                DocumentKind::Docx,
            ),
            (
                "xl/workbook.xml",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
                DocumentKind::Xlsx,
            ),
            (
                "ppt/presentation.xml",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml",
                DocumentKind::Pptx,
            ),
        ];
        for (part_name, content_type, expected) in cases {
            let parts = vec![
                ("[Content_Types].xml".to_owned(), format!("<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/{part_name}' ContentType='{content_type}'/></Types>").into_bytes()),
                (part_name.to_owned(), b"<root/>".to_vec()),
            ];
            assert_eq!(detect_format(&parts).unwrap(), expected);
        }
    }

    #[test]
    fn preserves_shipping_format_sanitizer_goldens() {
        // These goldens were generated by f1439b4: add a worktree at that commit,
        // run a throwaway sanitizer bin over these fixtures, then copy its outputs here.
        for (format, source, expected) in [
            (
                "docx",
                include_bytes!("../tests/fixtures/betteroffice-demo.docx").as_slice(),
                include_bytes!("../tests/fixtures/betteroffice-demo.docx.sanitized").as_slice(),
            ),
            (
                "xlsx",
                include_bytes!("../tests/fixtures/sample.xlsx").as_slice(),
                include_bytes!("../tests/fixtures/sample.xlsx.sanitized").as_slice(),
            ),
            (
                "pptx",
                include_bytes!("../tests/fixtures/betteroffice-demo.pptx").as_slice(),
                include_bytes!("../tests/fixtures/betteroffice-demo.pptx.sanitized").as_slice(),
            ),
        ] {
            assert_eq!(
                sanitize_package_for_format(source, format).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn validates_content_types_and_resolves_conflicts() {
        let macro_enabled = vec![(
            "[Content_Types].xml".to_owned(),
            br#"<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/visio/document.xml' ContentType='application/vnd.ms-visio.drawing.macro&#69;nabled.main+xml'/></Types>"#.to_vec(),
        )];
        assert_eq!(detect_package_kind(&macro_enabled), Ok(DocumentKind::Vsdm));

        let comment = vec![(
            "[Content_Types].xml".to_owned(),
            br#"<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><!-- application/vnd.ms-visio.drawing.main+xml --><Override PartName='/word/document.xml' ContentType='application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml'/></Types>"#.to_vec(),
        )];
        assert_eq!(detect_package_kind(&comment), Ok(DocumentKind::Docx));

        let conflicting = vec![(
            "[Content_Types].xml".to_owned(),
            br#"<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/word/document.xml' ContentType='application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml'/><Override PartName='/ppt/presentation.xml' ContentType='application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml'/></Types>"#.to_vec(),
        )];
        assert_eq!(detect_package_kind(&conflicting), Ok(DocumentKind::Docx));
        assert_eq!(detect_format(&conflicting).unwrap(), DocumentKind::Docx);

        let visio_conflict = vec![(
            "[Content_Types].xml".to_owned(),
            br#"<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Override PartName='/visio/document.xml' ContentType='application/vnd.ms-visio.drawing.main+xml'/><Override PartName='/visio/document.xml' ContentType='application/vnd.ms-visio.drawing.macroEnabled.main+xml'/></Types>"#.to_vec(),
        )];
        assert!(matches!(
            detect_package_kind(&visio_conflict),
            Err(DocumentKindError::ConflictingDocumentKinds(_))
        ));
    }

    #[test]
    fn ignores_nested_and_foreign_content_type_entries() {
        let nested = vec![(
            "[Content_Types].xml".to_owned(),
            br#"<Types xmlns='http://schemas.openxmlformats.org/package/2006/content-types'><Group><Override PartName='/visio/document.xml' ContentType='application/vnd.ms-visio.drawing.main+xml'/></Group></Types>"#.to_vec(),
        )];
        assert_eq!(
            detect_package_kind(&nested),
            Err(DocumentKindError::MissingMainDocumentKind)
        );

        let foreign = vec![(
            "[Content_Types].xml".to_owned(),
            br#"<Types xmlns='urn:foreign'><Override PartName='/visio/document.xml' ContentType='application/vnd.ms-visio.drawing.main+xml'/></Types>"#.to_vec(),
        )];
        assert_eq!(
            detect_package_kind(&foreign),
            Err(DocumentKindError::MissingMainDocumentKind)
        );
    }

    #[test]
    fn rejects_dtd_in_xml_parts() {
        let package = rezip_parts(&[
            (
                "[Content_Types].xml".to_owned(),
                br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_vec(),
            ),
            (
                "word/document.xml".to_owned(),
                b"<!DOCTYPE x><w:document/>".to_vec(),
            ),
        ])
        .unwrap();
        assert!(sanitize_package(&package).unwrap_err().contains("DTD"));
    }
}
