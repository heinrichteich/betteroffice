use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::{NsReader, XmlVersion};

use crate::rels::attribute_local;
use crate::{Format, RedactError};

pub(crate) struct VisioContentTypes {
    pub(crate) accepted_drawing: bool,
    pub(crate) accepted_template: bool,
    pub(crate) refused: bool,
}

pub(crate) fn classify_content_types(bytes: &[u8]) -> Result<VisioContentTypes, RedactError> {
    let mut reader = NsReader::from_reader(bytes);
    let error = |message: String| RedactError::Xml {
        part: "[Content_Types].xml".to_owned(),
        message,
    };
    let mut result = VisioContentTypes {
        accepted_drawing: false,
        accepted_template: false,
        refused: false,
    };
    loop {
        match reader
            .read_event()
            .map_err(|value| error(value.to_string()))?
        {
            Event::Start(start) | Event::Empty(start)
                if matches!(start.local_name().as_ref(), b"Override" | b"Default") =>
            {
                let namespace = reader.resolver().resolve_element(start.name()).0;
                if !matches!(namespace, ResolveResult::Unbound)
                    && !matches!(namespace, ResolveResult::Bound(ns) if ns.as_ref() == b"http://schemas.openxmlformats.org/package/2006/content-types")
                {
                    continue;
                }
                for attribute in start.attributes() {
                    let attribute = attribute.map_err(|value| error(value.to_string()))?;
                    if attribute.key.as_ref() == b"ContentType" {
                        let value = attribute
                            .decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
                            .map_err(|value| error(value.to_string()))?
                            .trim()
                            .to_ascii_lowercase();
                        if value == "application/vnd.ms-visio.drawing.main+xml" {
                            result.accepted_drawing = true;
                        } else if value == "application/vnd.ms-visio.template.main+xml" {
                            result.accepted_template = true;
                        } else if value.starts_with("application/vnd.ms-visio.")
                            && value.ends_with(".main+xml")
                        {
                            result.refused = true;
                        }
                    }
                }
            }
            Event::Eof => return Ok(result),
            _ => {}
        }
    }
}

pub(crate) fn is_visio(format: Format) -> bool {
    matches!(format, Format::Vsdx | Format::Vstx)
}

pub(crate) fn is_section(element: &str) -> bool {
    element.eq_ignore_ascii_case("Section")
}

pub(crate) fn is_cell(element: &str) -> bool {
    element.eq_ignore_ascii_case("Cell")
}

pub(crate) fn is_row(element: &str) -> bool {
    element.eq_ignore_ascii_case("Row")
}

pub(crate) fn contains_text(stack: &[String]) -> bool {
    stack.iter().any(|name| name.eq_ignore_ascii_case("Text"))
}

pub(crate) fn is_user_section(name: &str) -> bool {
    name.eq_ignore_ascii_case("Property") || name.eq_ignore_ascii_case("User")
}

pub(crate) fn is_data_cell(name: &str) -> bool {
    name.eq_ignore_ascii_case("Value")
        || name.eq_ignore_ascii_case("Prompt")
        || name.eq_ignore_ascii_case("Label")
}

pub(crate) fn attribute_named(attributes: &[(String, String)], expected: &str) -> Option<String> {
    for (key, value) in attributes {
        if attribute_local(key).eq_ignore_ascii_case(expected) {
            return Some(value.clone());
        }
    }
    None
}

pub(crate) fn ambiguous(part: &str, message: &str) -> RedactError {
    RedactError::AmbiguousVisio {
        part: part.to_owned(),
        message: message.to_owned(),
    }
}
