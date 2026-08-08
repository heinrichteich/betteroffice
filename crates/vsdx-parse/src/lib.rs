//! Bounded Visio XML parsing and part-preserving package writes.

mod error;
mod model;
mod package;
mod patch;
mod relationships;
mod sheet;
mod xml;

pub use error::VsdxError;
pub use model::*;
pub use package::{parse_vsdx, parse_vsdx_with_limits, save_cell_edits, write_vsdx};
pub use patch::{
    CellAttribute, CellEdit, MAX_PATCH_BYTES, MAX_PATCH_EDITS, SourceSpan, SpanEdit,
    apply_span_edits,
};
pub use relationships::{Relationship, TargetMode, relationship_types};
pub use sheet::*;
pub use xml::ParseLimits;
