use std::collections::BTreeMap;

use ooxml_drawingml::GeometryPathCommand;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use vsdx_parse::Cell;

#[serde(rename_all = "kebab-case")]
pub enum Provenance {
    Local,
    MasterShape,
    Master,
    StyleLine,
    StyleFill,
    StyleText,
    Page,
    Document,
    Default,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedCell {
    pub cell: Cell,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRow {
    pub key: String,
    pub row_type: Option<String>,
    pub cells: BTreeMap<String, ResolvedCell>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSection {
    pub name: String,
    pub rows: BTreeMap<String, ResolvedRow>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedShape {
    pub cells: BTreeMap<String, ResolvedCell>,
    pub sections: BTreeMap<String, ResolvedSection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolvedTextToken {
    Literal(String),
    CharacterRun {
        index: u32,
        properties: BTreeMap<String, ResolvedCell>,
    },
    ParagraphRun {
        index: u32,
        properties: BTreeMap<String, ResolvedCell>,
    },
    Tab {
        index: u32,
        properties: BTreeMap<String, ResolvedCell>,
    },
    Field {
        index: u32,
        properties: BTreeMap<String, ResolvedCell>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GeometryIssue {
    UnsupportedRowType(String),
    UnevaluatedCell { row_type: String, cell: String },
    MissingCell { row_type: String, cell: String },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RealizedGeometry {
    pub commands: Vec<GeometryPathCommand>,
    pub issues: Vec<GeometryIssue>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResolveError {
    #[error("page content not found: {0}")]
    MissingPage(String),
    #[error("shape not found: {0}")]
    MissingShape(u32),
    #[error("inheritance cycle: {0}")]
    Cycle(String),
}
