use serde::{Deserialize, Serialize};

use crate::VsdxError;
use crate::xml::{ParseBudget, XmlElement, parse_xml};

pub mod relationship_types {
    pub const DOCUMENT: &str = "http://schemas.microsoft.com/visio/2010/relationships/document";
    pub const PAGES: &str = "/pages";
    pub const MASTERS: &str = "/masters";
    pub const THEME: &str = "/theme";
    pub const WINDOWS: &str = "/windows";
    pub const PAGE: &str = "/page";
    pub const MASTER: &str = "/master";
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetMode {
    #[default]
    Internal,
    External,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Relationship {
    pub id: String,
    pub relationship_type: String,
    pub target: String,
    pub target_mode: TargetMode,
    pub resolved_target: Option<String>,
}
impl Relationship {
    pub fn has_type(&self, suffix: &str) -> bool {
        self.relationship_type.ends_with(suffix)
    }
}

pub(crate) fn parse_relationships(
    xml: &[u8],
    relationship_part: &str,
    source: &str,
    budget: &mut ParseBudget<'_>,
) -> Result<Vec<Relationship>, VsdxError> {
    let root = parse_xml(xml, relationship_part, budget)?;
    root.children_named("Relationship")
        .map(|element| {
            budget.charge_relationship(relationship_part)?;
            parse_relationship(element, source)
        })
        .collect()
}

fn parse_relationship(element: &XmlElement, source: &str) -> Result<Relationship, VsdxError> {
    let target = element.attribute("Target").unwrap_or_default().to_owned();
    let target_mode = if element
        .attribute("TargetMode")
        .is_some_and(|value| value.eq_ignore_ascii_case("External"))
    {
        TargetMode::External
    } else {
        TargetMode::Internal
    };
    let resolved_target = match target_mode {
        TargetMode::External => None,
        TargetMode::Internal => Some(resolve_target(source, &target)?),
    };
    Ok(Relationship {
        id: element.attribute("Id").unwrap_or_default().to_owned(),
        relationship_type: element.attribute("Type").unwrap_or_default().to_owned(),
        target,
        target_mode,
        resolved_target,
    })
}

pub(crate) fn resolve_target(source: &str, target: &str) -> Result<String, VsdxError> {
    let lower = target.to_ascii_lowercase();
    if lower.contains("://") || lower.starts_with("mailto:") || lower.starts_with("data:") {
        return Err(invalid(source, target));
    }
    let target = target
        .split(['#', '?'])
        .next()
        .unwrap_or_default()
        .replace('\\', "/");
    let mut segments = Vec::new();
    if !target.starts_with('/')
        && let Some((directory, _)) = source.rsplit_once('/')
    {
        segments.extend(directory.split('/').filter(|segment| !segment.is_empty()));
    }
    for segment in target.trim_start_matches('/').split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err(invalid(source, &target));
                }
            }
            other => segments.push(other),
        }
    }
    if segments.is_empty() {
        return Err(invalid(source, &target));
    }
    Ok(segments.join("/"))
}
fn invalid(source: &str, target: &str) -> VsdxError {
    VsdxError::InvalidRelationship {
        source_part: source.to_owned(),
        target: target.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_targets() {
        for target in ["https://x", "mailto:a@b", "data:x", "../../x"] {
            assert!(resolve_target("visio/document.xml", target).is_err());
        }
    }
}
