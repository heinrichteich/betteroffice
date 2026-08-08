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
    pub max_display_list_bytes: usize,
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
            max_display_list_bytes: 64 * 1024 * 1024,
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
    #[error("invalid page dimensions: {0}")]
    PageDimensions(String),
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
        let key = (family.into(), bold, italic);
        if !self.registered_fonts.contains_key(&key)
            && self.registered_fonts.len() >= self.limits.max_fonts
        {
            return Err(RenderError::Budget("fonts"));
        }
        let font_bytes = self
            .font_bytes
            .checked_add(bytes.len())
            .ok_or(RenderError::Budget("font bytes"))?;
        if font_bytes > self.limits.max_font_bytes {
            return Err(RenderError::Budget("font bytes"));
        }
        let id = self
            .fonts
            .register(bytes)
            .map_err(|error| RenderError::Font(error.to_string()))?;
        self.font_bytes = font_bytes;
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
        let page_height = page_dimension(&resolver, package, page_part, "PageHeight")
            .ok_or_else(|| RenderError::PageDimensions("PageHeight is unavailable".into()))?;
        let page_width = page_dimension(&resolver, package, page_part, "PageWidth")
            .ok_or_else(|| RenderError::PageDimensions("PageWidth is unavailable".into()))?;
        let mut state = State {
            count: 0,
            z_order: 0,
            text_bytes: 0,
            text_paragraphs: 0,
            text_lines: 0,
            text_runs: 0,
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
        let list = VsdxDisplayList {
            contract_version: CONTRACT_VERSION,
            width: page_width as f32 * PIXELS_PER_INCH,
            height: page_height as f32 * PIXELS_PER_INCH,
            paint_transform: final_paint_transform(page_height as f32),
            primitives: state.primitives,
        };
        if !display_list_finite(&list) {
            return Err(RenderError::PageDimensions(
                "non-finite display list".into(),
            ));
        }
        if serde_json::to_vec(&list).map_or(true, |bytes| {
            bytes.len() > self.limits.max_display_list_bytes
        }) {
            return Err(RenderError::Budget("display-list bytes"));
        }
        Ok(list)
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
            let mut children = state.primitives.split_off(start);
            for primitive in &mut children {
                bake_group_transform(primitive, bounds);
            }
            let z_order = state.next_z();
            state.primitives.push(Primitive::Group {
                id,
                z_order,
                primitives: children,
                transform: Transform::default(),
            });
            return Ok(());
        }
        if let Some(data) = shape.foreign_data() {
            let asset_id = data.relationship_id.as_deref().and_then(|relationship_id| {
                package
                    .relationships
                    .get(page_part)?
                    .iter()
                    .find_map(|relationship| {
                        (relationship.id == relationship_id)
                            .then_some(relationship.resolved_target.as_deref())
                            .flatten()
                    })
            });
            if let Some(asset_id) = asset_id {
                state.primitives.push(Primitive::Image {
                    id,
                    z_order,
                    asset_id: asset_id.into(),
                    x: bounds.x as f32,
                    y: bounds.y as f32,
                    width: bounds.width as f32,
                    height: bounds.height as f32,
                    transform: Transform {
                        rotation_deg: bounds.angle.to_degrees() as f32,
                        flip_x: bounds.flip_x,
                        flip_y: bounds.flip_y,
                    },
                });
                return Ok(());
            }
            return self.placeholder_at(
                id,
                z_order,
                bounds,
                state,
                "unsupported ForeignData image",
            );
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
        let Ok((fill, stroke)) = paint::paint(package, references, &resolved, shape.id) else {
            return self.placeholder_at(id, z_order, bounds, state, "unresolvable colour");
        };
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
        self.text(shape, &resolved, id, bounds, state)?;
        Ok(())
    }
    fn text(
        &self,
        shape: &Shape,
        resolved: &ResolvedShape,
        id: String,
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
        let text_bytes = state
            .text_bytes
            .checked_add(text.len())
            .ok_or(RenderError::Budget("text bytes"))?;
        if text_bytes > self.limits.max_text_bytes {
            return Err(RenderError::Budget("text bytes"));
        }
        if state.text_paragraphs >= self.limits.max_text_paragraphs {
            return Err(RenderError::Budget("text paragraphs"));
        }
        if state.text_lines >= self.limits.max_text_lines {
            return Err(RenderError::Budget("text lines"));
        }
        if state.text_runs >= self.limits.max_text_runs {
            return Err(RenderError::Budget("text runs"));
        }
        state.text_bytes = text_bytes;
        state.text_paragraphs += 1;
        state.text_lines += 1;
        state.text_runs += 1;
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
        let z_order = state.next_z();
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
fn bake_group_transform(primitive: &mut Primitive, group: Bounds) {
    match primitive {
        Primitive::Shape {
            path, transform, ..
        } => {
            for command in path {
                transform_command(command, group);
            }
            *transform = Transform::default();
        }
        Primitive::Image {
            x,
            y,
            width,
            height,
            ..
        }
        | Primitive::TextBox {
            x,
            y,
            width,
            height,
            ..
        }
        | Primitive::Placeholder {
            x,
            y,
            width,
            height,
            ..
        } => {
            let (x0, y0) = group_point(*x as f64, *y as f64, group);
            let (x1, y1) = group_point((*x + *width) as f64, (*y + *height) as f64, group);
            *x = x0.min(x1) as f32;
            *y = y0.min(y1) as f32;
            *width = (x1 - x0).abs() as f32;
            *height = (y1 - y0).abs() as f32;
            if let Primitive::Image { transform, .. } = primitive {
                *transform = Transform::default();
            }
        }
        Primitive::Group {
            primitives,
            transform,
            ..
        } => {
            for child in primitives {
                bake_group_transform(child, group);
            }
            *transform = Transform::default();
        }
    }
}
fn transform_command(command: &mut ooxml_drawingml::GeometryPathCommand, group: Bounds) {
    use ooxml_drawingml::GeometryPathCommand::*;
    let point = |x: &mut f64, y: &mut f64| {
        (*x, *y) = group_point(*x, *y, group);
    };
    match command {
        Move { x, y } | Line { x, y } => point(x, y),
        Quad { cpx, cpy, x, y } => {
            point(cpx, cpy);
            point(x, y);
        }
        Cubic {
            cp1x,
            cp1y,
            cp2x,
            cp2y,
            x,
            y,
        } => {
            point(cp1x, cp1y);
            point(cp2x, cp2y);
            point(x, y);
        }
        Close => {}
    }
}
fn group_point(x: f64, y: f64, group: Bounds) -> (f64, f64) {
    let loc_x = group.loc_pin_x;
    let loc_y = group.loc_pin_y;
    let pin_x = group.x + group.loc_pin_x;
    let pin_y = group.y + group.loc_pin_y;
    let mut x = x - loc_x;
    let mut y = y - loc_y;
    if group.flip_x {
        x = -x;
    }
    if group.flip_y {
        y = -y;
    }
    let (sin, cos) = group.angle.sin_cos();
    (pin_x + x * cos - y * sin, pin_y + x * sin + y * cos)
}
struct State {
    count: usize,
    z_order: u32,
    text_bytes: usize,
    text_paragraphs: usize,
    text_lines: usize,
    text_runs: usize,
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
    loc_pin_x: f64,
    loc_pin_y: f64,
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
    let resolved = resolver.resolve_sheet(sheet).ok()?;
    let Lookup::Found(cell) = resolved.cell(name)? else {
        return None;
    };
    if let Some(formula) = cell.cell.formula.as_deref()
        && let Evaluation::Evaluated(result) = evaluate_cell_with_shape_package_theme(
            name,
            formula,
            &resolved,
            &ParseLimits::default(),
            &resolved,
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
    let loc_pin_x = value("LocPinX").unwrap_or(width / 2.0);
    let loc_pin_y = value("LocPinY").unwrap_or(height / 2.0);
    Some(Bounds {
        x: pin_x - loc_pin_x,
        y: pin_y - loc_pin_y,
        width,
        height,
        loc_pin_x,
        loc_pin_y,
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

fn display_list_finite(list: &VsdxDisplayList) -> bool {
    list.width.is_finite()
        && list.height.is_finite()
        && [
            list.paint_transform.a,
            list.paint_transform.b,
            list.paint_transform.c,
            list.paint_transform.d,
            list.paint_transform.e,
            list.paint_transform.f,
        ]
        .into_iter()
        .all(f32::is_finite)
        && primitives_finite(&list.primitives)
}
fn primitives_finite(primitives: &[Primitive]) -> bool {
    primitives.iter().all(|primitive| match primitive {
        Primitive::Shape {
            path, transform, ..
        } => transform.rotation_deg.is_finite() && path.iter().all(command_finite),
        Primitive::Image {
            x,
            y,
            width,
            height,
            transform,
            ..
        } => [*x, *y, *width, *height, transform.rotation_deg]
            .into_iter()
            .all(f32::is_finite),
        Primitive::TextBox {
            x,
            y,
            width,
            height,
            lines,
            ..
        } => {
            [*x, *y, *width, *height].into_iter().all(f32::is_finite)
                && lines.iter().all(|line| {
                    [line.x, line.y, line.width, line.height]
                        .into_iter()
                        .all(f32::is_finite)
                        && line.caret_stops.iter().all(|stop| stop.x.is_finite())
                })
        }
        Primitive::Placeholder {
            x,
            y,
            width,
            height,
            ..
        } => [*x, *y, *width, *height].into_iter().all(f32::is_finite),
        Primitive::Group {
            transform,
            primitives,
            ..
        } => transform.rotation_deg.is_finite() && primitives_finite(primitives),
    })
}
fn command_finite(command: &ooxml_drawingml::GeometryPathCommand) -> bool {
    use ooxml_drawingml::GeometryPathCommand::*;
    match command {
        Move { x, y } | Line { x, y } => x.is_finite() && y.is_finite(),
        Quad { cpx, cpy, x, y } => {
            cpx.is_finite() && cpy.is_finite() && x.is_finite() && y.is_finite()
        }
        Cubic {
            cp1x,
            cp1y,
            cp2x,
            cp2y,
            x,
            y,
        } => [*cp1x, *cp1y, *cp2x, *cp2y, *x, *y]
            .into_iter()
            .all(f64::is_finite),
        Close => true,
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
                Primitive::Shape {
                    id,
                    path,
                    transform,
                    ..
                } if path_hit(path, *transform, x as f64, y as f64) => {
                    return Some(HitTestResult::Shape {
                        shape_id: id.clone(),
                    });
                }
                Primitive::Image {
                    id,
                    x: left,
                    y: top,
                    width,
                    height,
                    transform,
                    ..
                } if point_in_transformed_rect(*left, *top, *width, *height, *transform, x, y) => {
                    return Some(HitTestResult::Shape {
                        shape_id: id.clone(),
                    });
                }
                Primitive::Placeholder {
                    id,
                    x: left,
                    y: top,
                    width,
                    height,
                    ..
                } if x >= *left && x <= left + width && y >= *top && y <= top + height => {
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

fn point_in_transformed_rect(
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    transform: Transform,
    x: f32,
    y: f32,
) -> bool {
    let (x, y) = inverse_transform(
        x as f64,
        y as f64,
        left as f64 + width as f64 / 2.0,
        top as f64 + height as f64 / 2.0,
        transform,
    );
    x >= left as f64 && x <= (left + width) as f64 && y >= top as f64 && y <= (top + height) as f64
}
fn path_hit(
    path: &[ooxml_drawingml::GeometryPathCommand],
    transform: Transform,
    x: f64,
    y: f64,
) -> bool {
    let points = path
        .iter()
        .filter_map(|command| match command {
            ooxml_drawingml::GeometryPathCommand::Move { x, y }
            | ooxml_drawingml::GeometryPathCommand::Line { x, y } => Some((*x, *y)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let Some((&min_x, &max_x, &min_y, &max_y)) = points
        .iter()
        .map(|(x, _)| x)
        .min_by(|a, b| a.total_cmp(b))
        .zip(points.iter().map(|(x, _)| x).max_by(|a, b| a.total_cmp(b)))
        .zip(points.iter().map(|(_, y)| y).min_by(|a, b| a.total_cmp(b)))
        .zip(points.iter().map(|(_, y)| y).max_by(|a, b| a.total_cmp(b)))
        .map(|(((a, b), c), d)| (a, b, c, d))
    else {
        return false;
    };
    let (x, y) = inverse_transform(
        x,
        y,
        (min_x + max_x) / 2.0,
        (min_y + max_y) / 2.0,
        transform,
    );
    let mut inside = false;
    for index in 0..points.len() {
        let (ax, ay) = points[index];
        let (bx, by) = points[(index + 1) % points.len()];
        if (ay > y) != (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax {
            inside = !inside;
        }
    }
    inside
}
fn inverse_transform(x: f64, y: f64, cx: f64, cy: f64, transform: Transform) -> (f64, f64) {
    let radians = -(transform.rotation_deg as f64).to_radians();
    let (sin, cos) = radians.sin_cos();
    let x = x - cx;
    let y = y - cy;
    let x = x * cos - y * sin;
    let y = x * sin + y * cos;
    (
        if transform.flip_x { -x } else { x } + cx,
        if transform.flip_y { -y } else { y } + cy,
    )
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
