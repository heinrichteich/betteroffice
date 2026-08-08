use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EditOrigin {
    #[default]
    Local,
    Agent,
    Remote,
    System,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditCtx {
    pub origin: EditOrigin,
    pub author: String,
}

impl EditCtx {
    pub fn local(author: impl Into<String>) -> Self {
        Self {
            origin: EditOrigin::Local,
            author: author.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellSnapshot {
    pub name: String,
    pub formula: Option<String>,
    pub value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeSnapshot {
    pub id: String,
    pub source_id: u32,
    pub name: Option<String>,
    pub cells: Vec<CellSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageSnapshot {
    pub id: String,
    pub source_part_path: String,
    pub name: Option<String>,
    pub shapes: Vec<ShapeSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramSnapshot {
    pub pages: Vec<PageSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellFormulaReceipt {
    pub page_id: String,
    pub shape_id: String,
    pub cell_name: String,
    pub before: Option<String>,
    pub after: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeReceipt {
    pub page_id: String,
    pub shape_id: String,
    pub from_index: Option<u32>,
    pub to_index: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeDraft {
    pub source_id: u32,
    pub name: Option<String>,
    pub cells: Vec<CellSnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateOrigin {
    Local,
    Remote,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateEvent {
    pub update: Vec<u8>,
    pub origin: UpdateOrigin,
}

#[derive(Debug, Error)]
pub enum EditError {
    #[error("invalid client ID {0}")]
    InvalidClientId(u64),
    #[error("could not parse VSDX: {0}")]
    Parse(String),
    #[error("invalid diagram state: {0}")]
    InvalidState(String),
    #[error("invalid yrs update: {0}")]
    InvalidUpdate(String),
    #[error("invalid yrs state vector: {0}")]
    InvalidStateVector(String),
    #[error("page {0:?} was not found")]
    PageNotFound(String),
    #[error("shape {0:?} was not found")]
    ShapeNotFound(String),
    #[error("cell {0:?} was not found")]
    CellNotFound(String),
    #[error("index {index} is outside length {length}")]
    OutOfBounds { index: u32, length: u32 },
    #[error("update observer failed: {0}")]
    Observer(String),
    #[error("JSON boundary error: {0}")]
    Json(String),
}

pub type EditResult<T> = Result<T, EditError>;
