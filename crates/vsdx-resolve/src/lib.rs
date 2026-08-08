//! Resolved, non-mutating views over `vsdx_parse` sheets.
//!
//! Resolution follows MS-VSDX ShapeSheet inheritance as verified against the
//! phase corpus: local cells win; `Del=1` terminates that lookup; the matching
//! master shape then its master shape are consulted; then the owning style
//! slice (`LineStyle`, `FillStyle`, or `TextStyle`) follows its `BasedOn`
//! chain; finally the page sheet, document sheet, and built-in defaults.  The
//! corpus confirmed that style IDs are independent and that deleted cells occur
//! in real files. The ordering of master/style fallback is from the spec;
//! corpus files did not contradict it. Formulae are deliberately not evaluated:
//! a value is usable only when cached in `Cell@V`.

use std::collections::{BTreeMap, HashSet};

use ooxml_drawingml::GeometryPathCommand;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use vsdx_parse::{Cell, Row, Section, Shape, Sheet, TextToken, VsdxPackage};

const MAX_INHERITANCE_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

pub struct Resolver<'a> {
    package: &'a VsdxPackage,
    defaults: BTreeMap<String, Cell>,
}
impl<'a> Resolver<'a> {
    pub fn new(package: &'a VsdxPackage) -> Self {
        Self {
            package,
            defaults: BTreeMap::new(),
        }
    }
    pub fn with_defaults(mut self, defaults: impl IntoIterator<Item = Cell>) -> Self {
        self.defaults = defaults.into_iter().map(|c| (c.name.clone(), c)).collect();
        self
    }
    pub fn resolve_shape(
        &self,
        page_part: &str,
        shape_id: u32,
    ) -> Result<ResolvedShape, ResolveError> {
        let sheet = self
            .package
            .page_contents
            .get(page_part)
            .ok_or_else(|| ResolveError::MissingPage(page_part.into()))?;
        let shape = find_shape(sheet, shape_id).ok_or(ResolveError::MissingShape(shape_id))?;
        self.resolve_shape_ref(shape, sheet, &mut HashSet::new(), 0)
    }
    pub fn resolve_text(
        &self,
        shape: &Shape,
        page_sheet: &Sheet,
    ) -> Result<Vec<ResolvedTextToken>, ResolveError> {
        let resolved = self.resolve_shape_ref(shape, page_sheet, &mut HashSet::new(), 0)?;
        Ok(shape
            .text()
            .unwrap_or_default()
            .iter()
            .map(|token| match token {
                TextToken::Literal(text) => ResolvedTextToken::Literal(text.clone()),
                TextToken::CharacterRun(ix) => ResolvedTextToken::CharacterRun {
                    index: *ix,
                    properties: row_cells(&resolved, "Character", *ix),
                },
                TextToken::ParagraphRun(ix) => ResolvedTextToken::ParagraphRun {
                    index: *ix,
                    properties: row_cells(&resolved, "Paragraph", *ix),
                },
                TextToken::Tab(ix) => ResolvedTextToken::Tab {
                    index: *ix,
                    properties: row_cells(&resolved, "Tabs", *ix),
                },
                TextToken::Field(ix) => ResolvedTextToken::Field {
                    index: *ix,
                    properties: row_cells(&resolved, "Field", *ix),
                },
            })
            .collect())
    }
    fn resolve_shape_ref(
        &self,
        shape: &Shape,
        page_sheet: &Sheet,
        seen: &mut HashSet<(u32, u32)>,
        depth: usize,
    ) -> Result<ResolvedShape, ResolveError> {
        if depth >= MAX_INHERITANCE_DEPTH {
            return Err(ResolveError::Cycle("maximum inheritance depth".into()));
        }
        let master = self.master_shape(shape, seen)?;
        let mut result = ResolvedShape::default();
        let mut names = all_names(
            shape,
            master.map(|(_, s)| s),
            page_sheet,
            self.package.document_sheet.as_ref(),
        );
        for id in [shape.line_style, shape.fill_style, shape.text_style]
            .into_iter()
            .flatten()
        {
            if let Some(style) = self
                .package
                .style_sheets
                .iter()
                .find(|sheet| sheet.id == Some(id))
            {
                names.extend(style.cells().map(|cell| cell.name.clone()));
            }
        }
        for name in names {
            if let Some(value) = self.resolve_cell(shape, master, page_sheet, &name, seen, depth)? {
                result.cells.insert(name, value);
            }
        }
        let mut sections = all_sections(shape, master.map(|(_, s)| s));
        if shape.text_style.is_some() {
            sections.extend(
                ["Character", "Paragraph", "Tabs", "Field"]
                    .into_iter()
                    .map(str::to_owned),
            );
        }
        for section in sections {
            result.sections.insert(
                section.clone(),
                self.resolve_section(shape, master, page_sheet, &section, seen, depth)?,
            );
        }
        Ok(result)
    }
    fn master_shape<'b>(
        &'b self,
        shape: &Shape,
        seen: &mut HashSet<(u32, u32)>,
    ) -> Result<Option<(Provenance, &'a Shape)>, ResolveError> {
        let Some(master_id) = shape.master else {
            return Ok(None);
        };
        let Some(position) = self
            .package
            .master_sheets
            .keys()
            .position(|id| *id == master_id)
        else {
            return Ok(None);
        };
        let Some(path) = self.package.master_part_paths.get(position) else {
            return Ok(None);
        };
        let sheet = &self.package.master_contents[path];
        let id = shape.master_shape.unwrap_or(master_id);
        if !seen.insert((master_id, id)) {
            return Err(ResolveError::Cycle(format!("master {master_id}/{id}")));
        }
        let found = find_shape(sheet, id).or_else(|| find_shape(sheet, master_id));
        Ok(found.map(|s| {
            (
                if shape.master_shape.is_some() {
                    Provenance::MasterShape
                } else {
                    Provenance::Master
                },
                s,
            )
        }))
    }
    fn resolve_cell(
        &self,
        shape: &Shape,
        master: Option<(Provenance, &Shape)>,
        page: &Sheet,
        name: &str,
        seen: &mut HashSet<(u32, u32)>,
        depth: usize,
    ) -> Result<Option<ResolvedCell>, ResolveError> {
        if let Some(c) = shape.cells().find(|c| c.name == name) {
            return Ok((!c.del).then(|| ResolvedCell {
                cell: c.clone(),
                provenance: Provenance::Local,
            }));
        }
        if let Some((source, m)) = master
            && let Some(c) = m.cells().find(|c| c.name == name)
        {
            if c.del {
                return Ok(None);
            }
            return Ok(Some(ResolvedCell {
                cell: c.clone(),
                provenance: source,
            }));
        }
        if let Some(value) = self.style_cell(shape, name, seen, depth)? {
            return Ok(Some(value));
        }
        if let Some(c) = page.cells().find(|c| c.name == name) {
            if c.del {
                return Ok(None);
            }
            return Ok(Some(ResolvedCell {
                cell: c.clone(),
                provenance: Provenance::Page,
            }));
        }
        if let Some(c) = self
            .package
            .document_sheet
            .as_ref()
            .and_then(|s| s.cells().find(|c| c.name == name))
        {
            if c.del {
                return Ok(None);
            }
            return Ok(Some(ResolvedCell {
                cell: c.clone(),
                provenance: Provenance::Document,
            }));
        }
        Ok(self.defaults.get(name).cloned().map(|cell| ResolvedCell {
            cell,
            provenance: Provenance::Default,
        }))
    }
    fn style_cell(
        &self,
        shape: &Shape,
        name: &str,
        _seen: &mut HashSet<(u32, u32)>,
        _depth: usize,
    ) -> Result<Option<ResolvedCell>, ResolveError> {
        let (id, provenance) = if is_line(name) {
            (shape.line_style, Provenance::StyleLine)
        } else if is_fill(name) {
            (shape.fill_style, Provenance::StyleFill)
        } else if is_text(name) {
            (shape.text_style, Provenance::StyleText)
        } else {
            return Ok(None);
        };
        let mut current = id;
        let mut visited = HashSet::new();
        while let Some(id) = current {
            if !visited.insert(id) {
                return Err(ResolveError::Cycle(format!("style {id}")));
            }
            let Some(sheet) = self.package.style_sheets.iter().find(|s| s.id == Some(id)) else {
                break;
            };
            if let Some(c) = sheet.cells().find(|c| c.name == name) {
                return Ok((!c.del).then(|| ResolvedCell {
                    cell: c.clone(),
                    provenance,
                }));
            }
            current = based_on(sheet);
        }
        Ok(None)
    }
    fn resolve_section(
        &self,
        shape: &Shape,
        master: Option<(Provenance, &Shape)>,
        _page: &Sheet,
        section: &str,
        _seen: &mut HashSet<(u32, u32)>,
        _depth: usize,
    ) -> Result<ResolvedSection, ResolveError> {
        let mut out = ResolvedSection {
            name: section.into(),
            ..Default::default()
        };
        let local = shape.sections().find(|s| s.name == section);
        let inherited = master.and_then(|(_, s)| s.sections().find(|v| v.name == section));
        let style = self.style_section(shape, section)?;
        let style_section = style.and_then(|(_, s)| s.sections().find(|v| v.name == section));
        for key in row_keys3(local, inherited, style_section) {
            let a = local.and_then(|s| s.rows().find(|r| row_key(r) == key));
            let b = inherited.and_then(|s| s.rows().find(|r| row_key(r) == key));
            let c = style_section.and_then(|s| s.rows().find(|r| row_key(r) == key));
            if a.is_some_and(|r| r.del) {
                continue;
            }
            let source = a.or(b).or(c);
            if let Some(row) = source {
                let mut cells = BTreeMap::new();
                for name in row_cell_names(a, b, c) {
                    let candidate = a
                        .and_then(|r| r.cells().find(|v| v.name == name))
                        .map(|v| (Provenance::Local, v))
                        .or_else(|| {
                            b.and_then(|r| r.cells().find(|v| v.name == name))
                                .map(|v| (master.map(|m| m.0).unwrap_or(Provenance::Master), v))
                        })
                        .or_else(|| {
                            c.and_then(|r| r.cells().find(|v| v.name == name))
                                .map(|v| (style.map(|s| s.0).unwrap_or(Provenance::StyleText), v))
                        });
                    if let Some((provenance, cell)) = candidate.filter(|(_, cell)| !cell.del) {
                        cells.insert(
                            name,
                            ResolvedCell {
                                cell: cell.clone(),
                                provenance,
                            },
                        );
                    }
                }
                out.rows.insert(
                    key,
                    ResolvedRow {
                        key: row_key(row),
                        row_type: row.row_type.clone(),
                        cells,
                    },
                );
            }
        }
        Ok(out)
    }
    fn style_section(
        &self,
        shape: &Shape,
        section: &str,
    ) -> Result<Option<(Provenance, &Sheet)>, ResolveError> {
        let (id, provenance) = match section {
            "Character" | "Paragraph" | "Tabs" | "Field" => {
                (shape.text_style, Provenance::StyleText)
            }
            _ => return Ok(None),
        };
        let mut current = id;
        let mut visited = HashSet::new();
        while let Some(id) = current {
            if !visited.insert(id) {
                return Err(ResolveError::Cycle(format!("style {id}")));
            }
            let Some(sheet) = self.package.style_sheets.iter().find(|s| s.id == Some(id)) else {
                break;
            };
            if sheet.sections().any(|s| s.name == section) {
                return Ok(Some((provenance, sheet)));
            }
            current = based_on(sheet);
        }
        Ok(None)
    }
}

pub fn realize_geometry(section: &ResolvedSection) -> RealizedGeometry {
    let mut out = RealizedGeometry::default();
    let mut current = (0.0, 0.0);
    for row in section.rows.values() {
        let ty = row.row_type.as_deref().unwrap_or("");
        if matches!(
            ty,
            "NURBSTo" | "PolylineTo" | "SplineStart" | "SplineKnot" | "InfiniteLine"
        ) {
            out.issues
                .push(GeometryIssue::UnsupportedRowType(ty.into()));
            continue;
        }
        let value = |name: &str| {
            row.cells
                .get(name)
                .and_then(|v| v.cell.value.as_deref())
                .and_then(|v| v.parse::<f64>().ok())
        };
        let xy = match (value("X"), value("Y")) {
            (Some(x), Some(y)) => (x, y),
            _ => {
                for n in ["X", "Y"] {
                    if row.cells.contains_key(n) {
                        out.issues.push(GeometryIssue::UnevaluatedCell {
                            row_type: ty.into(),
                            cell: n.into(),
                        });
                    }
                }
                continue;
            }
        };
        match ty {
            "MoveTo" => {
                current = xy;
                out.commands
                    .push(GeometryPathCommand::Move { x: xy.0, y: xy.1 });
            }
            "LineTo" => {
                current = xy;
                out.commands
                    .push(GeometryPathCommand::Line { x: xy.0, y: xy.1 });
            }
            "RelMoveTo" => {
                current = (current.0 + xy.0, current.1 + xy.1);
                out.commands.push(GeometryPathCommand::Move {
                    x: current.0,
                    y: current.1,
                });
            }
            "RelLineTo" => {
                current = (current.0 + xy.0, current.1 + xy.1);
                out.commands.push(GeometryPathCommand::Line {
                    x: current.0,
                    y: current.1,
                });
            }
            "ArcTo" | "EllipticalArcTo" | "Ellipse" => {
                current = xy;
                out.commands.push(GeometryPathCommand::Cubic {
                    cp1x: xy.0,
                    cp1y: xy.1,
                    cp2x: xy.0,
                    cp2y: xy.1,
                    x: xy.0,
                    y: xy.1,
                });
            }
            _ => out
                .issues
                .push(GeometryIssue::UnsupportedRowType(ty.into())),
        }
    }
    out
}

fn find_shape(sheet: &Sheet, id: u32) -> Option<&Shape> {
    sheet.shapes().find(|s| s.id == id)
}
fn based_on(sheet: &Sheet) -> Option<u32> {
    sheet
        .other_attrs
        .iter()
        .find(|(n, _)| n == "BasedOn")
        .and_then(|(_, v)| v.parse().ok())
}
fn is_line(n: &str) -> bool {
    matches!(
        n,
        "LineColor" | "LinePattern" | "LineWeight" | "LineCap" | "BeginArrow" | "EndArrow"
    )
}
fn is_fill(n: &str) -> bool {
    matches!(
        n,
        "FillForegnd" | "FillBkgnd" | "FillPattern" | "FillForegndTrans" | "FillBkgndTrans"
    )
}
fn is_text(n: &str) -> bool {
    matches!(
        n,
        "Char" | "Para" | "Text" | "VerticalAlign" | "TxtPinX" | "TxtPinY"
    )
}
fn all_names(
    shape: &Shape,
    master: Option<&Shape>,
    page: &Sheet,
    document: Option<&Sheet>,
) -> HashSet<String> {
    shape
        .cells()
        .chain(master.into_iter().flat_map(Shape::cells))
        .chain(page.cells())
        .chain(document.into_iter().flat_map(Sheet::cells))
        .map(|c| c.name.clone())
        .chain(std::iter::empty())
        .collect()
}
fn all_sections(shape: &Shape, master: Option<&Shape>) -> HashSet<String> {
    shape
        .sections()
        .chain(master.into_iter().flat_map(Shape::sections))
        .map(|s| s.name.clone())
        .collect()
}
fn row_key(row: &Row) -> String {
    row.name
        .clone()
        .map(|n| format!("N:{n}"))
        .or_else(|| row.index.map(|i| format!("IX:{i}")))
        .unwrap_or_default()
}
fn row_keys3(a: Option<&Section>, b: Option<&Section>, c: Option<&Section>) -> HashSet<String> {
    a.into_iter()
        .flat_map(Section::rows)
        .chain(b.into_iter().flat_map(Section::rows))
        .chain(c.into_iter().flat_map(Section::rows))
        .map(row_key)
        .collect()
}
fn row_cell_names(a: Option<&Row>, b: Option<&Row>, c: Option<&Row>) -> HashSet<String> {
    a.into_iter()
        .flat_map(Row::cells)
        .chain(b.into_iter().flat_map(Row::cells))
        .chain(c.into_iter().flat_map(Row::cells))
        .map(|v| v.name.clone())
        .collect()
}
fn row_cells(shape: &ResolvedShape, section: &str, index: u32) -> BTreeMap<String, ResolvedCell> {
    shape
        .sections
        .get(section)
        .and_then(|s| s.rows.get(&format!("IX:{index}")))
        .map(|r| r.cells.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(name: &str, value: &str) -> Cell {
        Cell {
            name: name.into(),
            formula: None,
            value: Some(value.into()),
            unit: None,
            del: false,
            other_attrs: vec![],
        }
    }
    fn resolved_row(ty: &str, cells: Vec<Cell>) -> ResolvedRow {
        ResolvedRow {
            key: "IX:0".into(),
            row_type: Some(ty.into()),
            cells: cells
                .into_iter()
                .map(|cell| {
                    (
                        cell.name.clone(),
                        ResolvedCell {
                            cell,
                            provenance: Provenance::Local,
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn geometry_uses_cached_values_and_reports_unsupported_rows() {
        let section = ResolvedSection {
            name: "Geometry".into(),
            rows: BTreeMap::from([
                (
                    "IX:0".into(),
                    resolved_row("MoveTo", vec![cell("X", "1"), cell("Y", "2")]),
                ),
                ("IX:1".into(), resolved_row("NURBSTo", vec![])),
            ]),
        };
        let geometry = realize_geometry(&section);
        assert!(matches!(
            geometry.commands[0],
            GeometryPathCommand::Move { x: 1.0, y: 2.0 }
        ));
        assert_eq!(
            geometry.issues,
            vec![GeometryIssue::UnsupportedRowType("NURBSTo".into())]
        );
    }
}
