use std::collections::BTreeMap;

use crate::VsdxError;

pub const MAX_PATCH_EDITS: usize = 16_384;
pub const MAX_PATCH_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpan {
    pub offset: usize,
    pub length: usize,
}

impl SourceSpan {
    pub fn end(self) -> Option<usize> {
        self.offset.checked_add(self.length)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeSpan {
    pub value: SourceSpan,
    pub quote: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementSpan {
    pub name: String,
    pub span: SourceSpan,
    pub attributes: BTreeMap<String, AttributeSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpanEdit {
    pub span: SourceSpan,
    pub replacement: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellAttribute {
    Formula,
    Value,
}

impl CellAttribute {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Formula => "F",
            Self::Value => "V",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellEdit {
    pub part_path: String,
    pub cell_span: SourceSpan,
    pub attribute: CellAttribute,
    pub value: String,
}

pub fn apply_span_edits(source: &[u8], edits: &[SpanEdit]) -> Result<Vec<u8>, VsdxError> {
    if edits.len() > MAX_PATCH_EDITS {
        return Err(VsdxError::PatchLimit { kind: "editCount" });
    }
    let replacement_bytes = edits.iter().try_fold(0_usize, |total, edit| {
        total
            .checked_add(edit.replacement.len())
            .ok_or(VsdxError::PatchLimit { kind: "editBytes" })
    })?;
    if replacement_bytes > MAX_PATCH_BYTES {
        return Err(VsdxError::PatchLimit { kind: "editBytes" });
    }
    let mut ordered = edits.to_vec();
    ordered.sort_by_key(|edit| edit.span.offset);
    let mut cursor = 0;
    let mut output = Vec::with_capacity(source.len().saturating_add(replacement_bytes));
    for edit in ordered {
        let Some(end) = edit.span.end() else {
            return Err(VsdxError::InvalidSpan);
        };
        if end > source.len() || edit.span.offset < cursor {
            return Err(VsdxError::InvalidSpan);
        }
        if !utf8_boundary(source, edit.span.offset) || !utf8_boundary(source, end) {
            return Err(VsdxError::InvalidSpan);
        }
        output.extend_from_slice(&source[cursor..edit.span.offset]);
        output.extend_from_slice(&edit.replacement);
        cursor = end;
    }
    output.extend_from_slice(&source[cursor..]);
    Ok(output)
}

pub fn escape_attribute_value(value: &str, _quote: u8) -> Result<Vec<u8>, VsdxError> {
    if !value.is_char_boundary(value.len()) {
        return Err(VsdxError::InvalidSpan);
    }
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(character),
        }
    }
    Ok(output.into_bytes())
}

pub(crate) fn scan_element_spans(source: &[u8]) -> Vec<ElementSpan> {
    let mut spans = Vec::new();
    let mut stack: Vec<(usize, String, BTreeMap<String, AttributeSpan>)> = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].iter().position(|byte| *byte == b'<') {
        let start = cursor + relative;
        let Some(end) = tag_end(source, start) else {
            break;
        };
        let tag = &source[start..=end];
        if tag.starts_with(b"<!--")
            || tag.starts_with(b"<![")
            || tag.starts_with(b"<?")
            || tag.starts_with(b"<!")
        {
            cursor = end + 1;
            continue;
        }
        if tag.starts_with(b"</") {
            if let Some((offset, name, attributes)) = stack.pop() {
                spans.push(ElementSpan {
                    name,
                    span: SourceSpan {
                        offset,
                        length: end + 1 - offset,
                    },
                    attributes,
                });
            }
        } else if let Some((name, attributes, empty)) = opening_tag(source, start, end) {
            if empty {
                spans.push(ElementSpan {
                    name,
                    span: SourceSpan {
                        offset: start,
                        length: end + 1 - start,
                    },
                    attributes,
                });
            } else {
                stack.push((start, name, attributes));
            }
        }
        cursor = end + 1;
    }
    spans
}

fn tag_end(source: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 1;
    let mut quote = None;
    while index < source.len() {
        let byte = source[index];
        if let Some(active) = quote {
            if byte == active {
                quote = None;
            }
        } else if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
        } else if byte == b'>' {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn opening_tag(
    source: &[u8],
    start: usize,
    end: usize,
) -> Option<(String, BTreeMap<String, AttributeSpan>, bool)> {
    let mut index = start + 1;
    skip_space(source, &mut index, end);
    let name_start = index;
    while index < end && !source[index].is_ascii_whitespace() && source[index] != b'/' {
        index += 1;
    }
    if name_start == index {
        return None;
    }
    let name = String::from_utf8(source[name_start..index].to_vec()).ok()?;
    let mut attributes = BTreeMap::new();
    while index < end {
        skip_space(source, &mut index, end);
        if index >= end || source[index] == b'/' {
            break;
        }
        let key_start = index;
        while index < end && !source[index].is_ascii_whitespace() && source[index] != b'=' {
            index += 1;
        }
        let key = String::from_utf8(source[key_start..index].to_vec()).ok()?;
        skip_space(source, &mut index, end);
        if index >= end || source[index] != b'=' {
            return None;
        }
        index += 1;
        skip_space(source, &mut index, end);
        if index >= end || !matches!(source[index], b'\'' | b'"') {
            return None;
        }
        let quote = source[index];
        let value_start = index + 1;
        index = value_start;
        while index < end && source[index] != quote {
            index += 1;
        }
        if index >= end {
            return None;
        }
        attributes.insert(
            key,
            AttributeSpan {
                value: SourceSpan {
                    offset: value_start,
                    length: index - value_start,
                },
                quote,
            },
        );
        index += 1;
    }
    Some((
        name,
        attributes,
        source.get(end.wrapping_sub(1)) == Some(&b'/'),
    ))
}

fn skip_space(source: &[u8], index: &mut usize, end: usize) {
    while *index < end && source[*index].is_ascii_whitespace() {
        *index += 1;
    }
}

fn utf8_boundary(source: &[u8], index: usize) -> bool {
    index == 0 || index == source.len() || source[index] & 0b1100_0000 != 0b1000_0000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patcher_preserves_unedited_bytes_and_rejects_overlap() {
        let source = b"a\xC3\xA4bc";
        assert_eq!(
            apply_span_edits(
                source,
                &[SpanEdit {
                    span: SourceSpan {
                        offset: 3,
                        length: 1
                    },
                    replacement: b"X".to_vec()
                }]
            )
            .unwrap(),
            b"a\xC3\xA4Xc"
        );
        assert!(matches!(
            apply_span_edits(
                b"abcd",
                &[
                    SpanEdit {
                        span: SourceSpan {
                            offset: 1,
                            length: 2
                        },
                        replacement: vec![]
                    },
                    SpanEdit {
                        span: SourceSpan {
                            offset: 2,
                            length: 1
                        },
                        replacement: vec![]
                    }
                ]
            ),
            Err(VsdxError::InvalidSpan)
        ));
        assert!(matches!(
            apply_span_edits(
                b"abcd",
                &[SpanEdit {
                    span: SourceSpan {
                        offset: 4,
                        length: 1
                    },
                    replacement: vec![]
                }]
            ),
            Err(VsdxError::InvalidSpan)
        ));
    }
}
