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
        let page_sheet = self
            .package
            .page_part_ids
            .get(page_part)
            .and_then(|id| self.package.page_sheets.get(id))
            .unwrap_or(sheet);
        self.resolve_shape_ref(shape, page_sheet, &mut HashSet::new(), 0)
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
        let Some(path) = self
            .package
            .master_part_ids
            .iter()
            .find_map(|(path, id)| (*id == master_id).then_some(path))
        else {
            return Ok(None);
        };
        let Some(sheet) = self.package.master_contents.get(path) else {
            return Ok(None);
        };
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
        let mut required = |names: &[&str]| -> Option<Vec<f64>> {
            let mut values = Vec::with_capacity(names.len());
            for name in names {
                match value(name) {
                    Some(value) => values.push(value),
                    None if row.cells.contains_key(*name) => {
                        out.issues.push(GeometryIssue::UnevaluatedCell {
                            row_type: ty.into(),
                            cell: (*name).into(),
                        })
                    }
                    None => out.issues.push(GeometryIssue::MissingCell {
                        row_type: ty.into(),
                        cell: (*name).into(),
                    }),
                }
            }
            (values.len() == names.len()).then_some(values)
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
            "ArcTo" => {
                let Some(values) = required(&["A"]) else {
                    continue;
                };
                cubic_arc(&mut out.commands, current, xy, values[0]);
                current = xy;
            }
            "EllipticalArcTo" => {
                let Some(values) = required(&["A", "B", "C", "D"]) else {
                    continue;
                };
                cubic_elliptical_arc(
                    &mut out.commands,
                    current,
                    xy,
                    values[0],
                    values[1],
                    values[2],
                    values[3],
                );
                current = xy;
            }
            "Ellipse" => {
                let Some(values) = required(&["A", "B", "C", "D"]) else {
                    continue;
                };
                cubic_ellipse(
                    &mut out.commands,
                    xy,
                    values[0],
                    values[1],
                    values[2],
                    values[3],
                );
                current = xy;
            }
            _ => out
                .issues
                .push(GeometryIssue::UnsupportedRowType(ty.into())),
        }
    }
    out
}

fn cubic_arc(
    commands: &mut Vec<GeometryPathCommand>,
    start: (f64, f64),
    end: (f64, f64),
    bow: f64,
) {
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let chord = dx.hypot(dy);
    if chord == 0.0 || bow == 0.0 {
        commands.push(GeometryPathCommand::Line { x: end.0, y: end.1 });
        return;
    }
    let radius = chord * chord / (8.0 * bow.abs()) + bow.abs() / 2.0;
    let midpoint = ((start.0 + end.0) / 2.0, (start.1 + end.1) / 2.0);
    let normal = (-dy / chord * bow.signum(), dx / chord * bow.signum());
    let center = (
        midpoint.0 - normal.0 * (radius - bow.abs()),
        midpoint.1 - normal.1 * (radius - bow.abs()),
    );
    let a0 = (start.1 - center.1).atan2(start.0 - center.0);
    let a1 = (end.1 - center.1).atan2(end.0 - center.0);
    let mut sweep = a1 - a0;
    if bow > 0.0 && sweep < 0.0 {
        sweep += std::f64::consts::TAU;
    }
    if bow < 0.0 && sweep > 0.0 {
        sweep -= std::f64::consts::TAU;
    }
    cubic_arc_segment(commands, center, radius, radius, 0.0, a0, sweep);
}

fn cubic_elliptical_arc(
    commands: &mut Vec<GeometryPathCommand>,
    start: (f64, f64),
    end: (f64, f64),
    a: f64,
    b: f64,
    angle: f64,
    eccentricity: f64,
) {
    let rx = a.abs();
    let ry = (a.abs() * eccentricity.abs()).max(f64::EPSILON);
    if rx <= f64::EPSILON || b.abs() <= f64::EPSILON {
        commands.push(GeometryPathCommand::Line { x: end.0, y: end.1 });
        return;
    }
    let center = (
        start.0 + a * angle.cos() - b * angle.sin(),
        start.1 + a * angle.sin() + b * angle.cos(),
    );
    let start_angle = ((start.1 - center.1) / ry).atan2((start.0 - center.0) / rx);
    let end_angle = ((end.1 - center.1) / ry).atan2((end.0 - center.0) / rx);
    cubic_arc_segment(
        commands,
        center,
        rx,
        ry,
        angle,
        start_angle,
        end_angle - start_angle,
    );
}

fn cubic_ellipse(
    commands: &mut Vec<GeometryPathCommand>,
    _xy: (f64, f64),
    left: f64,
    bottom: f64,
    right: f64,
    top: f64,
) {
    let center = ((left + right) / 2.0, (bottom + top) / 2.0);
    let rx = (right - left).abs() / 2.0;
    let ry = (top - bottom).abs() / 2.0;
    let start = (center.0 + rx, center.1);
    commands.push(GeometryPathCommand::Move {
        x: start.0,
        y: start.1,
    });
    for quarter in 0..4 {
        cubic_arc_segment(
            commands,
            center,
            rx,
            ry,
            0.0,
            quarter as f64 * std::f64::consts::FRAC_PI_2,
            std::f64::consts::FRAC_PI_2,
        );
    }
}

fn cubic_arc_segment(
    commands: &mut Vec<GeometryPathCommand>,
    center: (f64, f64),
    rx: f64,
    ry: f64,
    rotation: f64,
    start: f64,
    sweep: f64,
) {
    let count = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = sweep / count as f64;
    for index in 0..count {
        let t0 = start + step * index as f64;
        let t1 = t0 + step;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let point = |t: f64| {
            (
                center.0 + rx * t.cos() * rotation.cos() - ry * t.sin() * rotation.sin(),
                center.1 + rx * t.cos() * rotation.sin() + ry * t.sin() * rotation.cos(),
            )
        };
        let tangent = |t: f64| {
            (
                -rx * t.sin() * rotation.cos() - ry * t.cos() * rotation.sin(),
                -rx * t.sin() * rotation.sin() + ry * t.cos() * rotation.cos(),
            )
        };
        let p0 = point(t0);
        let p1 = point(t1);
        let d0 = tangent(t0);
        let d1 = tangent(t1);
        commands.push(GeometryPathCommand::Cubic {
            cp1x: p0.0 + k * d0.0,
            cp1y: p0.1 + k * d0.1,
            cp2x: p1.0 - k * d1.0,
            cp2y: p1.1 - k * d1.1,
            x: p1.0,
            y: p1.1,
        });
    }
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

    #[test]
    fn arc_to_uses_its_bow_for_cubic_controls() {
        let section = ResolvedSection {
            name: "Geometry".into(),
            rows: BTreeMap::from([
                (
                    "IX:0".into(),
                    resolved_row("MoveTo", vec![cell("X", "0"), cell("Y", "0")]),
                ),
                (
                    "IX:1".into(),
                    resolved_row(
                        "ArcTo",
                        vec![cell("X", "2"), cell("Y", "0"), cell("A", "1")],
                    ),
                ),
            ]),
        };
        let geometry = realize_geometry(&section);
        let GeometryPathCommand::Cubic {
            cp1x,
            cp1y,
            cp2x,
            cp2y,
            x,
            y,
        } = geometry.commands[1]
        else {
            panic!("expected cubic")
        };
        assert!((cp1x - 0.0).abs() < 1e-12 && (cp1y + 0.5522847498307933).abs() < 1e-12);
        assert!((cp2x - 0.44771525016920655).abs() < 1e-12 && (cp2y + 1.0).abs() < 1e-12);
        assert!((x - 1.0).abs() < 1e-12 && (y + 1.0).abs() < 1e-12);
    }

    #[test]
    fn ellipse_uses_its_bounds_for_cubic_controls() {
        let section = ResolvedSection {
            name: "Geometry".into(),
            rows: BTreeMap::from([(
                "IX:0".into(),
                resolved_row(
                    "Ellipse",
                    vec![
                        cell("X", "0"),
                        cell("Y", "0"),
                        cell("A", "-1"),
                        cell("B", "-1"),
                        cell("C", "1"),
                        cell("D", "1"),
                    ],
                ),
            )]),
        };
        let geometry = realize_geometry(&section);
        assert!(matches!(
            geometry.commands[0],
            GeometryPathCommand::Move { x: 1.0, y: 0.0 }
        ));
        let GeometryPathCommand::Cubic {
            cp1x,
            cp1y,
            cp2x,
            cp2y,
            x,
            y,
        } = geometry.commands[1]
        else {
            panic!("expected cubic")
        };
        assert!((cp1x - 1.0).abs() < 1e-12 && (cp1y - 0.5522847498307933).abs() < 1e-12);
        assert!((cp2x - 0.5522847498307935).abs() < 1e-12 && (cp2y - 1.0).abs() < 1e-12);
        assert!(x.abs() < 1e-12 && (y - 1.0).abs() < 1e-12);
    }

    #[test]
    fn elliptical_arc_requires_all_cached_schema_cells() {
        let section = ResolvedSection {
            name: "Geometry".into(),
            rows: BTreeMap::from([(
                "IX:0".into(),
                resolved_row(
                    "EllipticalArcTo",
                    vec![
                        cell("X", "1"),
                        cell("Y", "1"),
                        cell("A", "1"),
                        cell("B", "1"),
                        cell("C", "0"),
                    ],
                ),
            )]),
        };
        let geometry = realize_geometry(&section);
        assert!(geometry.commands.is_empty());
        assert_eq!(
            geometry.issues,
            vec![GeometryIssue::MissingCell {
                row_type: "EllipticalArcTo".into(),
                cell: "D".into()
            }]
        );
    }
}
