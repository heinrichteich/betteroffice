//! Headless VSDX export to PowerPoint and Word.
//!
//! Each page renders through `vsdx-render` into its display list, then maps
//! onto native DrawingML shapes so output stays editable in its host.

mod docx;
mod geom;
mod metadata;
mod pptx;

use vsdx_parse::VsdxPackage;
use vsdx_render::{Renderer, VsdxDisplayList};

pub use metadata::{ShapeDatum, shape_data};

#[derive(Debug)]
pub enum ExportError {
    NoPages,
    Render(String),
    Resolve(String),
    Package(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPages => write!(f, "diagram has no pages"),
            Self::Render(reason) => write!(f, "render failed: {reason}"),
            Self::Resolve(reason) => write!(f, "resolve failed: {reason}"),
            Self::Package(reason) => write!(f, "package failed: {reason}"),
        }
    }
}

impl std::error::Error for ExportError {}

/// Exports every page as one slide carrying native shapes.
pub fn export_pptx(package: &VsdxPackage) -> Result<Vec<u8>, ExportError> {
    let pages = display_lists(package)?;
    pptx::build(&pages, package)
}

/// Exports every page as shapes plus a shape-data table.
pub fn export_docx(package: &VsdxPackage) -> Result<Vec<u8>, ExportError> {
    let pages = display_lists(package)?;
    docx::build(&pages, package)
}

pub(crate) struct Page {
    pub name: String,
    pub width_in: f64,
    pub height_in: f64,
    pub list: VsdxDisplayList,
}

fn display_lists(package: &VsdxPackage) -> Result<Vec<Page>, ExportError> {
    if package.page_part_paths.is_empty() {
        return Err(ExportError::NoPages);
    }
    let renderer = Renderer::default();
    let mut pages = Vec::with_capacity(package.page_part_paths.len());
    for part in &package.page_part_paths {
        let list = renderer
            .layout_page(package, part)
            .map_err(|error| ExportError::Render(error.to_string()))?;
        let width_in = f64::from(list.width) / 96.0;
        let height_in = f64::from(list.height) / 96.0;
        pages.push(Page {
            name: metadata::page_name_for(package, part),
            width_in,
            height_in,
            list,
        });
    }
    Ok(pages)
}
