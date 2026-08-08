//! VSDX display-list compilation. Scene data remains in Visio inches until paint.

mod display_list;
mod layout;
mod paint;

pub use display_list::*;
pub use layout::{PIXELS_PER_INCH, final_paint_transform, to_canvas};

use std::collections::BTreeMap;

use thiserror::Error;
use vsdx_eval::{Evaluation, PageShapeReferences, Value, evaluate_cell_with_shape_package_theme};
use vsdx_parse::{ParseLimits, Shape, VsdxPackage};
use vsdx_resolve::{Lookup, ResolvedShape, Resolver, realize_geometry};

const MAX_RECURSION_DEPTH: usize = 64;

#[derive(Clone, Debug)]
pub struct RenderLimits {
    pub max_shapes: usize,
    pub max_text_bytes: usize,
    pub max_text_paragraphs: usize,
    pub max_text_lines: usize,
    pub max_text_runs: usize,
    pub max_fonts: usize,
    pub max_font_bytes: usize,
}
impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            max_shapes: 100_000,
            max_text_bytes: 16 * 1024 * 1024,
            max_text_paragraphs: 100_000,
            max_text_lines: 1_000_000,
            max_text_runs: 1_000_000,
            max_fonts: 256,
            max_font_bytes: 64 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Error)]
pub enum RenderError {
    #[error("page not found: {0}")]
    MissingPage(String),
    #[error("render budget exceeded: {0}")]
    Budget(&'static str),
    #[error("invalid font: {0}")]
    Font(String),
    #[error("resolve failed: {0}")]
    Resolve(#[from] vsdx_resolve::ResolveError),
}

pub struct Renderer {
    limits: RenderLimits,
    fonts: ooxml_text::FontStore,
    font_bytes: usize,
    registered_fonts: BTreeMap<(String, bool, bool), ooxml_text::FontId>,
}
impl Default for Renderer {
    fn default() -> Self {
        Self::new(RenderLimits::default())
    }
}
impl Renderer {
    pub fn new(limits: RenderLimits) -> Self {
        Self {
            limits,
            fonts: ooxml_text::FontStore::new(),
            font_bytes: 0,
            registered_fonts: BTreeMap::new(),
        }
    }
    pub fn register_font(
        &mut self,
        family: impl Into<String>,
        bold: bool,
        italic: bool,
        bytes: Vec<u8>,
    ) -> Result<(), RenderError> {
        if self.registered_fonts.len() >= self.limits.max_fonts {
            return Err(RenderError::Budget("fonts"));
        }
        self.font_bytes = self
            .font_bytes
            .checked_add(bytes.len())
            .ok_or(RenderError::Budget("font bytes"))?;
        if self.font_bytes > self.limits.max_font_bytes {
            return Err(RenderError::Budget("font bytes"));
        }
        let key = (family.into(), bold, italic);
        let id = self
            .fonts
            .register(bytes)
            .map_err(|error| RenderError::Font(error.to_string()))?;
        self.registered_fonts.insert(key, id);
        Ok(())
    }
    pub fn layout_page(
        &self,
        package: &VsdxPackage,
        page_part: &str,
    ) -> Result<VsdxDisplayList, RenderError> {
        let page = package
            .page_contents
            .get(page_part)
            .ok_or_else(|| RenderError::MissingPage(page_part.into()))?;
        let resolver = Resolver::new(package);
        let references = PageShapeReferences::new(&resolver, page_part).ok();
        let page_height =
            page_dimension(&resolver, package, page_part, "PageHeight").unwrap_or(11.0);
        let page_width = page_dimension(&resolver, package, page_part, "PageWidth").unwrap_or(8.5);
        let mut state = State {
            count: 0,
            z_order: 0,
            text_bytes: 0,
            primitives: Vec::new(),
        };
        for shape in page.shapes() {
            self.layout_shape(
                package,
                &resolver,
                references.as_ref(),
                page_part,
                shape,
                0,
                &mut state,
            )?;
        }
        Ok(VsdxDisplayList {
            contract_version: CONTRACT_VERSION,
            width: page_width as f32 * PIXELS_PER_INCH,
            height: page_height as f32 * PIXELS_PER_INCH,
            paint_transform: final_paint_transform(page_height as f32),
            primitives: state.primitives,
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn layout_shape(
        &self,
        package: &VsdxPackage,
        resolver: &Resolver<'_>,
        references: Option<&PageShapeReferences>,
        page_part: &str,
        shape: &Shape,
        depth: usize,
        state: &mut State,
    ) -> Result<(), RenderError> {
        if depth >= MAX_RECURSION_DEPTH {
            return self.placeholder(shape, state, "group nesting depth exceeded");
        }
        state.count += 1;
        if state.count > self.limits.max_shapes {
            return Err(RenderError::Budget("shapes"));
        }
        let z_order = state.next_z();
        let id = format!("{page_part}:{}", shape.id);
        let resolved = resolver.resolve_shape(page_part, shape.id)?;
        if resolved.deleted {
            return Ok(());
        }
        let Some(bounds) = bounds(package, references, &resolved, shape.id) else {
            return self.placeholder(shape, state, "unresolvable transform");
        };
        let geometry = resolved.sections.get("Geometry").map(realize_geometry);
        let child_shapes = shape.shapes().collect::<Vec<_>>();
        if !child_shapes.is_empty() {
            let start = state.primitives.len();
            for child in child_shapes {
                self.layout_shape(
                    package,
                    resolver,
                    references,
                    page_part,
                    child,
                    depth + 1,
                    state,
                )?;
            }
            let children = state.primitives.split_off(start);
            state.primitives.push(Primitive::Group {
                id,
                z_order,
                primitives: children,
                transform: Transform::default(),
            });
            return Ok(());
        }
        let Some(geometry) = geometry else {
            return self.placeholder_at(
                id,
                z_order,
                bounds,
                state,
                "shape has no Geometry section",
            );
        };
        if !geometry.issues.is_empty() || geometry.commands.is_empty() {
            return self.placeholder_at(
                id,
                z_order,
                bounds,
                state,
                &format!("unsupported geometry: {:?}", geometry.issues),
            );
        }
        let path = geometry
            .commands
            .into_iter()
            .map(|command| place_command(command, bounds))
            .collect();
        let (fill, stroke) = paint::paint(&resolved);
        state.primitives.push(Primitive::Shape {
            id: id.clone(),
            z_order,
            path,
            fill,
            stroke,
            transform: Transform {
                rotation_deg: bounds.angle.to_degrees() as f32,
                flip_x: bounds.flip_x,
                flip_y: bounds.flip_y,
            },
        });
        self.text(shape, &resolved, id, z_order, bounds, state)?;
        Ok(())
    }
    fn text(
        &self,
        shape: &Shape,
        resolved: &ResolvedShape,
        id: String,
        z_order: u32,
        bounds: Bounds,
        state: &mut State,
    ) -> Result<(), RenderError> {
        let Some(tokens) = shape.text() else {
            return Ok(());
        };
        let text = tokens
            .iter()
            .filter_map(|token| {
                if let vsdx_parse::TextToken::Literal(value) = token {
                    Some(value.as_str())
                } else {
                    None
                }
            })
            .collect::<String>();
        if text.is_empty() {
            return Ok(());
        }
        state.text_bytes += text.len();
        if state.text_bytes > self.limits.max_text_bytes {
            return Err(RenderError::Budget("text bytes"));
        }
        let size =
            paint::number(resolved, "Char.Size").unwrap_or(0.166_666_67) as f32 * PIXELS_PER_INCH;
        let family = "Arial".to_owned();
        let width = self
            .registered_fonts
            .get(&(family.clone(), false, false))
            .and_then(|font| ooxml_text::shape(&self.fonts, *font, &text, size, &[]).ok())
            .map(|glyphs| glyphs.iter().map(|glyph| glyph.x_advance).sum())
            .unwrap_or_else(|| text.chars().count() as f32 * size * 0.5);
        let _breaks = ooxml_text::break_opportunities(&text);
        let stops = text
            .char_indices()
            .map(|(position, _)| CaretStop {
                position: position as u32,
                x: bounds.x as f32 + position as f32 * size * 0.5,
            })
            .chain(std::iter::once(CaretStop {
                position: text.len() as u32,
                x: bounds.x as f32 + width,
            }))
            .collect();
        state.primitives.push(Primitive::TextBox {
            id,
            z_order,
            x: bounds.x as f32,
            y: bounds.y as f32,
            width: bounds.width as f32,
            height: bounds.height as f32,
            paragraphs: vec![TextParagraph {
                runs: vec![TextRun {
                    text: text.clone(),
                    family,
                    size_px: size,
                    bold: false,
                    italic: false,
                    color: "#000000".into(),
                }],
            }],
            lines: vec![PositionedLine {
                x: bounds.x as f32,
                y: bounds.y as f32,
                width,
                height: size * 1.2,
                start: 0,
                end: text.len() as u32,
                caret_stops: stops,
            }],
        });
        Ok(())
    }
    fn placeholder(
        &self,
        shape: &Shape,
        state: &mut State,
        reason: &str,
    ) -> Result<(), RenderError> {
        self.placeholder_at(
            format!("shape:{}", shape.id),
            state.next_z(),
            Bounds::default(),
            state,
            reason,
        )
    }
    fn placeholder_at(
        &self,
        id: String,
        z_order: u32,
        bounds: Bounds,
        state: &mut State,
        reason: &str,
    ) -> Result<(), RenderError> {
        state.primitives.push(Primitive::Placeholder {
            id,
            z_order,
            x: bounds.x as f32,
            y: bounds.y as f32,
            width: bounds.width as f32,
            height: bounds.height as f32,
            reason: reason.into(),
        });
        Ok(())
    }
}
struct State {
    count: usize,
    z_order: u32,
    text_bytes: usize,
    primitives: Vec<Primitive>,
}
impl State {
    fn next_z(&mut self) -> u32 {
        let z = self.z_order;
        self.z_order += 1;
        z
    }
}
#[derive(Clone, Copy, Default)]
struct Bounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    angle: f64,
    flip_x: bool,
    flip_y: bool,
}
fn page_dimension(
    resolver: &Resolver<'_>,
    package: &VsdxPackage,
    page: &str,
    name: &str,
) -> Option<f64> {
    let page_id = package.page_part_ids.get(page)?;
    let sheet = package.page_sheets.get(page_id)?;
    paint::number(&resolver.resolve_sheet(sheet).ok()?, name)
}
fn bounds(
    package: &VsdxPackage,
    references: Option<&PageShapeReferences>,
    shape: &ResolvedShape,
    shape_id: u32,
) -> Option<Bounds> {
    let value = |name| evaluated(package, references, shape, shape_id, name);
    let width = value("Width")?;
    let height = value("Height")?;
    let pin_x = value("PinX")?;
    let pin_y = value("PinY")?;
    Some(Bounds {
        x: pin_x - value("LocPinX").unwrap_or(width / 2.0),
        y: pin_y - value("LocPinY").unwrap_or(height / 2.0),
        width,
        height,
        angle: value("Angle").unwrap_or(0.0),
        flip_x: value("FlipX").unwrap_or(0.0) != 0.0,
        flip_y: value("FlipY").unwrap_or(0.0) != 0.0,
    })
}
fn evaluated(
    package: &VsdxPackage,
    references: Option<&PageShapeReferences>,
    shape: &ResolvedShape,
    shape_id: u32,
    name: &str,
) -> Option<f64> {
    let Lookup::Found(cell) = shape.cell(name)? else {
        return None;
    };
    if let (Some(formula), Some(references)) = (cell.cell.formula.as_deref(), references)
        && let Evaluation::Evaluated(result) = evaluate_cell_with_shape_package_theme(
            name,
            formula,
            &references.for_shape(shape_id),
            &ParseLimits::default(),
            shape,
            package,
        )
        && let Value::Number(number) = result.value
        && number.number.is_finite()
    {
        return Some(number.number);
    }
    cell.cell
        .value
        .as_deref()?
        .parse()
        .ok()
        .filter(|value: &f64| value.is_finite())
}
fn place_command(
    command: ooxml_drawingml::GeometryPathCommand,
    bounds: Bounds,
) -> ooxml_drawingml::GeometryPathCommand {
    use ooxml_drawingml::GeometryPathCommand::*;
    let point = |x: f64, y: f64| (x + bounds.x, y + bounds.y);
    match command {
        Move { x, y } => {
            let (x, y) = point(x, y);
            Move { x, y }
        }
        Line { x, y } => {
            let (x, y) = point(x, y);
            Line { x, y }
        }
        Quad { cpx, cpy, x, y } => {
            let (cpx, cpy) = point(cpx, cpy);
            let (x, y) = point(x, y);
            Quad { cpx, cpy, x, y }
        }
        Cubic {
            cp1x,
            cp1y,
            cp2x,
            cp2y,
            x,
            y,
        } => {
            let (cp1x, cp1y) = point(cp1x, cp1y);
            let (cp2x, cp2y) = point(cp2x, cp2y);
            let (x, y) = point(x, y);
            Cubic {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            }
        }
        Close => Close,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HitTestResult {
    Shape { shape_id: String },
    Text { shape_id: String, position: u32 },
}
pub fn hit_test(list: &VsdxDisplayList, x: f32, y: f32) -> Option<HitTestResult> {
    let (x, y) = (
        (x - list.paint_transform.e) / list.paint_transform.a,
        (y - list.paint_transform.f) / list.paint_transform.d,
    );
    fn visit(primitives: &[Primitive], x: f32, y: f32) -> Option<HitTestResult> {
        for primitive in primitives.iter().rev() {
            match primitive {
                Primitive::TextBox {
                    id,
                    x: left,
                    y: top,
                    width,
                    height,
                    lines,
                    ..
                } if x >= *left && x <= left + width && y >= *top && y <= top + height => {
                    let line = lines
                        .iter()
                        .min_by(|a, b| (a.y - y).abs().total_cmp(&(b.y - y).abs()))?;
                    let stop = line
                        .caret_stops
                        .iter()
                        .min_by(|a, b| (a.x - x).abs().total_cmp(&(b.x - x).abs()))?;
                    return Some(HitTestResult::Text {
                        shape_id: id.clone(),
                        position: stop.position,
                    });
                }
                Primitive::Shape { id, .. }
                | Primitive::Image { id, .. }
                | Primitive::Placeholder { id, .. } => {
                    return Some(HitTestResult::Shape {
                        shape_id: id.clone(),
                    });
                }
                Primitive::Group { primitives, .. } => {
                    if let Some(hit) = visit(primitives, x, y) {
                        return Some(hit);
                    }
                }
                _ => {}
            }
        }
        None
    }
    visit(&list.primitives, x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flip_is_only_final_paint_transform() {
        let transform = final_paint_transform(10.0);
        assert_eq!(to_canvas(transform, 0.0, 0.0), (0.0, 960.0));
        assert_eq!(to_canvas(transform, 0.0, 10.0), (0.0, 0.0));
    }
    #[test]
    fn rejects_unknown_contract() {
        let list = VsdxDisplayList {
            contract_version: 2,
            width: 0.0,
            height: 0.0,
            paint_transform: final_paint_transform(0.0),
            primitives: vec![],
        };
        assert!(list.validate().is_err());
    }

    #[test]
    fn foundation_display_list_matches_golden_contract() {
        let package = vsdx_parse::parse_vsdx(include_bytes!(
            "../../vsdx-parse/tests/fixtures/foundation.vsdx"
        ))
        .unwrap();
        let list = Renderer::default()
            .layout_page(&package, &package.page_part_paths[0])
            .unwrap();
        let actual = serde_json::to_string(&list).unwrap();
        assert_eq!(
            actual,
            include_str!("../tests/golden/foundation.json").trim()
        );
    }

    #[test]
    fn corpus_smoke_reports_painted_and_placeholdered_shapes() {
        let Ok(directory) = std::env::var("VSDX_CORPUS_DIR") else {
            eprintln!(
                "warning: skipping VSDX corpus renderer smoke test; VSDX_CORPUS_DIR is unset"
            );
            return;
        };
        let mut painted = 0usize;
        let mut placeholders = 0usize;
        let mut reasons = BTreeMap::new();
        for file in ["lichtsysteme.vsdx", "soundplan.vsdx"] {
            let path = std::path::Path::new(&directory).join(file);
            let package = vsdx_parse::parse_vsdx(&std::fs::read(path).unwrap()).unwrap();
            for page in &package.page_part_paths {
                let list = Renderer::default().layout_page(&package, page).unwrap();
                let json = serde_json::to_string(&list).unwrap();
                assert!(!json.contains("NaN") && !json.contains("Infinity"));
                count_primitives(
                    &list.primitives,
                    &mut painted,
                    &mut placeholders,
                    &mut reasons,
                );
            }
        }
        eprintln!(
            "VSDX corpus render: painted={painted} placeholdered={placeholders} top reasons={reasons:?}"
        );
    }

    fn count_primitives(
        primitives: &[Primitive],
        painted: &mut usize,
        placeholders: &mut usize,
        reasons: &mut BTreeMap<String, usize>,
    ) {
        for primitive in primitives {
            match primitive {
                Primitive::Shape { .. } | Primitive::Image { .. } => *painted += 1,
                Primitive::Placeholder { reason, .. } => {
                    *placeholders += 1;
                    *reasons.entry(reason.clone()).or_default() += 1;
                }
                Primitive::Group { primitives, .. } => {
                    count_primitives(primitives, painted, placeholders, reasons)
                }
                Primitive::TextBox { .. } => {}
            }
        }
    }
}
