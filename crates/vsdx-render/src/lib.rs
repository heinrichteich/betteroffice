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
        if !page_height.is_sign_positive()
            || !page_width.is_sign_positive()
            || !(page_height as f32).is_finite()
            || !(page_width as f32).is_finite()
            || !(page_height as f32 * PIXELS_PER_INCH).is_finite()
            || !(page_width as f32 * PIXELS_PER_INCH).is_finite()
        {
            return Err(RenderError::PageDimensions(
                "dimensions must be positive finite canvas values".into(),
            ));
        }
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
        if paint::number(&resolved, "NoShow").is_some_and(|value| value != 0.0) {
            return Ok(());
        }
        let Some(bounds) = bounds(package, references, &resolved, shape.id) else {
            return self.placeholder(shape, state, "unresolvable transform");
        };
        if !bounds_finite(bounds) {
            return self.placeholder_at(
                id,
                z_order,
                Bounds::default(),
                state,
                "overflowing transform",
            );
        }
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
                transform: Affine::identity(),
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
                if package.part_bytes(asset_id).is_none() {
                    return self.placeholder_at(
                        id,
                        z_order,
                        bounds,
                        state,
                        "dangling ForeignData image target",
                    );
                }
                state.primitives.push(Primitive::Image {
                    id,
                    z_order,
                    asset_id: asset_id.into(),
                    x: bounds.x as f32,
                    y: bounds.y as f32,
                    width: bounds.width as f32,
                    height: bounds.height as f32,
                    transform: bounds_affine(bounds),
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
            .map(|mut command| {
                transform_command(&mut command, bounds);
                command
            })
            .collect::<Vec<_>>();
        let (fill, stroke) = match paint::paint(package, references, &resolved, shape.id) {
            Ok(paint) => paint,
            Err(reason) => {
                return self.placeholder_at(
                    id,
                    z_order,
                    bounds,
                    state,
                    &format!("unresolvable colour: {reason}"),
                );
            }
        };
        state.primitives.push(Primitive::Shape {
            id: id.clone(),
            z_order,
            path,
            fill,
            stroke,
            transform: Affine::identity(),
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
        let size = paint::number(resolved, "Char.Size").unwrap_or(0.166_666_67) as f32;
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
                y: bounds.y as f32,
            })
            .chain(std::iter::once(CaretStop {
                position: text.len() as u32,
                x: bounds.x as f32 + width,
                y: bounds.y as f32,
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
    let matrix = bounds_affine(group);
    match primitive {
        Primitive::Shape {
            path, transform, ..
        } => {
            for command in path {
                transform_affine(command, matrix);
            }
            *transform = Affine::identity();
        }
        Primitive::Image { transform, .. } => *transform = matrix.compose(*transform),
        Primitive::TextBox {
            x,
            y,
            width,
            height,
            lines,
            ..
        } => {
            transform_rect(x, y, width, height, matrix);
            for line in lines {
                (line.x, line.y) = matrix.apply_point(line.x, line.y);
                for stop in &mut line.caret_stops {
                    (stop.x, stop.y) = matrix.apply_point(stop.x, stop.y);
                }
            }
        }
        Primitive::Placeholder {
            x,
            y,
            width,
            height,
            ..
        } => transform_rect(x, y, width, height, matrix),
        Primitive::Group {
            primitives,
            transform,
            ..
        } => {
            for child in primitives {
                bake_group_transform(child, group);
            }
            *transform = Affine::identity();
        }
    }
}
fn transform_rect(x: &mut f32, y: &mut f32, width: &mut f32, height: &mut f32, matrix: Affine) {
    let corners = [
        matrix.apply_point(*x, *y),
        matrix.apply_point(*x + *width, *y),
        matrix.apply_point(*x, *y + *height),
        matrix.apply_point(*x + *width, *y + *height),
    ];
    let (min_x, max_x) = corners
        .iter()
        .map(|(x, _)| *x)
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        });
    let (min_y, max_y) = corners
        .iter()
        .map(|(_, y)| *y)
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        });
    *x = min_x;
    *y = min_y;
    *width = max_x - min_x;
    *height = max_y - min_y;
}
fn transform_command(command: &mut ooxml_drawingml::GeometryPathCommand, group: Bounds) {
    transform_affine(command, bounds_affine(group));
}
fn transform_affine(command: &mut ooxml_drawingml::GeometryPathCommand, matrix: Affine) {
    use ooxml_drawingml::GeometryPathCommand::*;
    let point = |x: &mut f64, y: &mut f64| {
        let (transformed_x, transformed_y) = matrix.apply_point(*x as f32, *y as f32);
        (*x, *y) = (transformed_x as f64, transformed_y as f64);
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
fn bounds_affine(group: Bounds) -> Affine {
    let loc_x = group.loc_pin_x;
    let loc_y = group.loc_pin_y;
    let pin_x = group.x + group.loc_pin_x;
    let pin_y = group.y + group.loc_pin_y;
    let (sin, cos) = group.angle.sin_cos();
    let sx = if group.flip_x { -1.0 } else { 1.0 };
    let sy = if group.flip_y { -1.0 } else { 1.0 };
    Affine {
        a: (cos * sx) as f32,
        b: (sin * sx) as f32,
        c: (-sin * sy) as f32,
        d: (cos * sy) as f32,
        e: (pin_x - cos * sx * loc_x + sin * sy * loc_y) as f32,
        f: (pin_y - sin * sx * loc_x - cos * sy * loc_y) as f32,
    }
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
fn bounds_finite(bounds: Bounds) -> bool {
    [
        bounds.x,
        bounds.y,
        bounds.width,
        bounds.height,
        bounds.loc_pin_x,
        bounds.loc_pin_y,
        bounds.angle,
    ]
    .into_iter()
    .all(|value| value.is_finite() && (value as f32).is_finite())
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
            path,
            transform,
            fill,
            stroke,
            ..
        } => {
            transform.is_finite()
                && path.iter().all(command_finite)
                && paint_finite(fill)
                && stroke
                    .as_ref()
                    .is_none_or(|stroke| stroke.width.is_finite())
        }
        Primitive::Image {
            x,
            y,
            width,
            height,
            transform,
            ..
        } => [*x, *y, *width, *height].into_iter().all(f32::is_finite) && transform.is_finite(),
        Primitive::TextBox {
            x,
            y,
            width,
            height,
            paragraphs,
            lines,
            ..
        } => {
            [*x, *y, *width, *height].into_iter().all(f32::is_finite)
                && lines.iter().all(|line| {
                    [line.x, line.y, line.width, line.height]
                        .into_iter()
                        .all(f32::is_finite)
                        && line
                            .caret_stops
                            .iter()
                            .all(|stop| stop.x.is_finite() && stop.y.is_finite())
                })
                && paragraphs
                    .iter()
                    .all(|paragraph| paragraph.runs.iter().all(|run| run.size_px.is_finite()))
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
        } => transform.is_finite() && primitives_finite(primitives),
    })
}
fn paint_finite(paint: &Option<Paint>) -> bool {
    match paint {
        Some(Paint::Gradient { stops }) => stops.iter().all(|stop| stop.position.is_finite()),
        _ => true,
    }
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
    let inverse = Affine {
        a: list.paint_transform.a,
        b: list.paint_transform.b,
        c: list.paint_transform.c,
        d: list.paint_transform.d,
        e: list.paint_transform.e,
        f: list.paint_transform.f,
    }
    .invert()?;
    let (x, y) = inverse.apply_point(x, y);
    fn collect<'a>(primitives: &'a [Primitive], output: &mut Vec<&'a Primitive>) {
        for primitive in primitives {
            output.push(primitive);
            if let Primitive::Group { primitives, .. } = primitive {
                collect(primitives, output);
            }
        }
    }
    let mut primitives = Vec::new();
    collect(&list.primitives, &mut primitives);
    primitives.sort_by_key(|primitive| std::cmp::Reverse(z_order(primitive)));
    for primitive in primitives {
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
                fill,
                stroke,
                transform,
                ..
            } if path_hit(
                path,
                *transform,
                fill.is_some(),
                stroke.as_ref().map_or(0.0, |stroke| stroke.width),
                x,
                y,
            ) =>
            {
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
            _ => {}
        }
    }
    None
}

fn z_order(primitive: &Primitive) -> u32 {
    match primitive {
        Primitive::Shape { z_order, .. }
        | Primitive::Image { z_order, .. }
        | Primitive::TextBox { z_order, .. }
        | Primitive::Placeholder { z_order, .. }
        | Primitive::Group { z_order, .. } => *z_order,
    }
}

fn point_in_transformed_rect(
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    transform: Affine,
    x: f32,
    y: f32,
) -> bool {
    let Some(inverse) = transform.invert() else {
        return false;
    };
    let (x, y) = inverse.apply_point(x, y);
    x >= left && x <= left + width && y >= top && y <= top + height
}
fn path_hit(
    path: &[ooxml_drawingml::GeometryPathCommand],
    transform: Affine,
    fill: bool,
    stroke_width: f32,
    x: f32,
    y: f32,
) -> bool {
    let Some(inverse) = transform.invert() else {
        return false;
    };
    let (x, y) = inverse.apply_point(x, y);
    let points = flatten_path(path);
    if points.is_empty() {
        return false;
    }
    let mut inside = false;
    for index in 0..points.len() {
        let (ax, ay) = points[index];
        let (bx, by) = points[(index + 1) % points.len()];
        if (ay > y) != (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax {
            inside = !inside;
        }
    }
    fill && inside
        || stroke_width > 0.0
            && points.windows(2).any(|segment| {
                point_segment_distance(x, y, segment[0], segment[1]) <= stroke_width / 2.0
            })
}
fn point_segment_distance(x: f32, y: f32, (ax, ay): (f32, f32), (bx, by): (f32, f32)) -> f32 {
    let dx = bx - ax;
    let dy = by - ay;
    let length = dx * dx + dy * dy;
    let t = if length == 0.0 {
        0.0
    } else {
        (((x - ax) * dx + (y - ay) * dy) / length).clamp(0.0, 1.0)
    };
    ((x - (ax + t * dx)).powi(2) + (y - (ay + t * dy)).powi(2)).sqrt()
}
fn flatten_path(path: &[ooxml_drawingml::GeometryPathCommand]) -> Vec<(f32, f32)> {
    use ooxml_drawingml::GeometryPathCommand::*;
    let mut points = Vec::new();
    let mut current = (0.0, 0.0);
    for command in path {
        match *command {
            Move { x, y } => {
                current = (x as f32, y as f32);
                points.push(current);
            }
            Line { x, y } => {
                current = (x as f32, y as f32);
                points.push(current);
            }
            Quad { cpx, cpy, x, y } => {
                let start = current;
                for step in 1..=16 {
                    let t = step as f32 / 16.0;
                    let u = 1.0 - t;
                    points.push((
                        u * u * start.0 + 2.0 * u * t * cpx as f32 + t * t * x as f32,
                        u * u * start.1 + 2.0 * u * t * cpy as f32 + t * t * y as f32,
                    ));
                }
                current = (x as f32, y as f32);
            }
            Cubic {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                let start = current;
                for step in 1..=16 {
                    let t = step as f32 / 16.0;
                    let u = 1.0 - t;
                    points.push((
                        u.powi(3) * start.0
                            + 3.0 * u * u * t * cp1x as f32
                            + 3.0 * u * t * t * cp2x as f32
                            + t.powi(3) * x as f32,
                        u.powi(3) * start.1
                            + 3.0 * u * u * t * cp1y as f32
                            + 3.0 * u * t * t * cp2y as f32
                            + t.powi(3) * y as f32,
                    ));
                }
                current = (x as f32, y as f32);
            }
            Close => {}
        }
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_drawingml::GeometryPathCommand;
    use vsdx_parse::{
        Cell, ForeignData, Row, RowChild, Section, SectionChild, ShapeChild, ShapesChild, Sheet,
        SheetChild, TextToken,
    };

    fn package(shapes: Vec<Shape>) -> VsdxPackage {
        let mut package: VsdxPackage = serde_json::from_value(serde_json::json!({
            "documentPartPath": "", "pagesPartPath": null, "mastersPartPath": null,
            "pagePartPaths": ["page"], "masterPartPaths": [], "themePartPaths": [],
            "windowsPartPath": null, "relationships": {}, "documentSheet": null,
            "styleSheets": [], "colors": [], "faceNames": [], "pageSheets": {},
            "masterSheets": {}, "pagePartIds": {"page": 1}, "masterPartIds": {},
            "pageContents": {}, "masterContents": {}
        }))
        .unwrap();
        package.page_sheets.insert(
            1,
            Sheet {
                id: None,
                children: vec![
                    SheetChild::Cell(cell("PageWidth", "10")),
                    SheetChild::Cell(cell("PageHeight", "8")),
                ],
                other_attrs: vec![],
            },
        );
        package.page_contents.insert(
            "page".into(),
            Sheet {
                id: None,
                children: vec![SheetChild::Shapes(
                    shapes.into_iter().map(ShapesChild::Shape).collect(),
                )],
                other_attrs: vec![],
            },
        );
        package
    }

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

    fn formula(name: &str, value: &str) -> Cell {
        Cell {
            name: name.into(),
            formula: Some(value.into()),
            value: None,
            unit: None,
            del: false,
            other_attrs: vec![],
        }
    }

    fn row(index: u32, row_type: &str, cells: Vec<Cell>) -> Row {
        Row {
            index: Some(index),
            name: None,
            local_name: None,
            row_type: Some(row_type.into()),
            del: false,
            children: cells.into_iter().map(RowChild::Cell).collect(),
            other_attrs: vec![],
        }
    }

    fn rectangle() -> Section {
        Section {
            name: "Geometry".into(),
            index: None,
            del: false,
            children: vec![
                row(0, "MoveTo", vec![cell("X", "0"), cell("Y", "0")]),
                row(1, "LineTo", vec![cell("X", "1"), cell("Y", "0")]),
                row(2, "LineTo", vec![cell("X", "1"), cell("Y", "1")]),
                row(3, "LineTo", vec![cell("X", "0"), cell("Y", "1")]),
            ]
            .into_iter()
            .map(SectionChild::Row)
            .collect(),
            other_attrs: vec![],
        }
    }

    fn shape(id: u32, pin_x: f64, pin_y: f64) -> Shape {
        Shape {
            id,
            name: None,
            name_u: None,
            shape_type: None,
            master: None,
            master_shape: None,
            line_style: None,
            fill_style: None,
            text_style: None,
            del: false,
            other_attrs: vec![],
            children: vec![
                ShapeChild::Cell(cell("Width", "1")),
                ShapeChild::Cell(cell("Height", "1")),
                ShapeChild::Cell(cell("PinX", &pin_x.to_string())),
                ShapeChild::Cell(cell("PinY", &pin_y.to_string())),
                ShapeChild::Cell(cell("LocPinX", "0")),
                ShapeChild::Cell(cell("LocPinY", "0")),
                ShapeChild::Cell(cell("FillPattern", "1")),
                ShapeChild::Cell(formula("FillForegnd", "RGB(1,2,3)")),
                ShapeChild::Cell(cell("LinePattern", "1")),
                ShapeChild::Cell(formula("LineColor", "RGB(4,5,6)")),
                ShapeChild::Cell(cell("LineWeight", "0.02")),
                ShapeChild::Section(rectangle()),
            ],
        }
    }

    fn render(shapes: Vec<Shape>) -> VsdxDisplayList {
        let package = package(shapes);
        Renderer::default().layout_page(&package, "page").unwrap()
    }

    fn shape_primitive(list: &VsdxDisplayList, id: u32) -> &Primitive {
        list.primitives.iter().find(|primitive| matches!(primitive, Primitive::Shape { id: actual, .. } if actual == &format!("page:{id}"))).unwrap()
    }
    #[test]
    fn flip_is_only_final_paint_transform() {
        let transform = final_paint_transform(10.0);
        assert_eq!(to_canvas(transform, 0.0, 0.0), (0.0, 960.0));
        assert_eq!(to_canvas(transform, 0.0, 10.0), (0.0, 0.0));
    }

    #[test]
    fn renderer_keeps_inches_until_the_final_canvas_transform() {
        let list = render(vec![shape(1, 2.0, 3.0)]);
        assert_eq!(to_canvas(list.paint_transform, 0.0, 0.0), (0.0, 768.0));
        assert_eq!(to_canvas(list.paint_transform, 0.0, 8.0), (0.0, 0.0));
        let Primitive::Shape { path, .. } = shape_primitive(&list, 1) else {
            unreachable!()
        };
        assert!(matches!(
            path[0],
            GeometryPathCommand::Move { x: 2.0, y: 3.0 }
        ));
    }

    #[test]
    fn shape_transform_rotates_and_flips_about_its_local_pin() {
        let mut rotated = shape(1, 3.0, 4.0);
        rotated.children.push(ShapeChild::Cell(cell(
            "Angle",
            &std::f64::consts::FRAC_PI_2.to_string(),
        )));
        let list = render(vec![rotated]);
        let Primitive::Shape {
            transform, path, ..
        } = shape_primitive(&list, 1)
        else {
            unreachable!()
        };
        assert_eq!(*transform, Affine::identity());
        assert!(
            matches!(path[1], GeometryPathCommand::Line { x, y } if (x - 3.0).abs() < 1e-9 && (y - 5.0).abs() < 1e-9)
        );

        for (id, flip_x, flip_y) in [(2, true, false), (3, false, true)] {
            let mut flipped = shape(id, 3.0, 4.0);
            flipped.children.push(ShapeChild::Cell(cell(
                "FlipX",
                if flip_x { "1" } else { "0" },
            )));
            flipped.children.push(ShapeChild::Cell(cell(
                "FlipY",
                if flip_y { "1" } else { "0" },
            )));
            let list = render(vec![flipped]);
            let Primitive::Shape {
                path, transform, ..
            } = shape_primitive(&list, id)
            else {
                unreachable!()
            };
            assert_eq!(*transform, Affine::identity());
            let expected = if flip_x { (2.0, 5.0) } else { (4.0, 3.0) };
            assert!(
                matches!(path[2], GeometryPathCommand::Line { x, y } if (x - expected.0).abs() < 1e-9 && (y - expected.1).abs() < 1e-9)
            );
        }
    }

    #[test]
    fn group_composes_translation_rotation_and_flips_for_child_geometry() {
        let child = shape(2, 1.0, 0.0);
        let mut group = shape(1, 10.0, 20.0);
        group
            .children
            .retain(|child| !matches!(child, ShapeChild::Section(_)));
        group.children.extend([
            ShapeChild::Cell(cell("Angle", &std::f64::consts::FRAC_PI_2.to_string())),
            ShapeChild::Cell(cell("FlipX", "1")),
            ShapeChild::Shapes(vec![ShapesChild::Shape(child)]),
        ]);
        let list = render(vec![group]);
        let Primitive::Group { primitives, .. } = &list.primitives[0] else {
            unreachable!()
        };
        let Primitive::Shape {
            path, transform, ..
        } = &primitives[0]
        else {
            unreachable!()
        };
        assert_eq!(*transform, Affine::identity());
        assert!(
            matches!(path[0], GeometryPathCommand::Move { x, y } if (x - 10.0).abs() < 1e-9 && (y - 19.0).abs() < 1e-9)
        );
    }

    #[test]
    fn hit_testing_honors_geometry_transform_order_groups_and_empty_canvas() {
        let mut rotated = shape(2, 3.0, 3.0);
        rotated.children.push(ShapeChild::Cell(cell(
            "Angle",
            &std::f64::consts::FRAC_PI_2.to_string(),
        )));
        let group = Shape {
            id: 3,
            name: None,
            name_u: None,
            shape_type: None,
            master: None,
            master_shape: None,
            line_style: None,
            fill_style: None,
            text_style: None,
            del: false,
            other_attrs: vec![],
            children: vec![
                ShapeChild::Cell(cell("Width", "1")),
                ShapeChild::Cell(cell("Height", "1")),
                ShapeChild::Cell(cell("PinX", "6")),
                ShapeChild::Cell(cell("PinY", "1")),
                ShapeChild::Shapes(vec![ShapesChild::Shape(shape(4, 0.0, 0.0))]),
            ],
        };
        let list = render(vec![shape(1, 1.0, 1.0), rotated, shape(5, 1.0, 1.0), group]);
        assert_eq!(
            hit_test(&list, 96.0 * 1.5, 768.0 - 96.0 * 1.5),
            Some(HitTestResult::Shape {
                shape_id: "page:5".into()
            })
        );
        assert_eq!(hit_test(&list, 96.0 * 2.1, 768.0 - 96.0 * 2.1), None);
        assert_eq!(
            hit_test(&list, 96.0 * 5.5, 768.0 - 96.0 * 0.5),
            Some(HitTestResult::Shape {
                shape_id: "page:4".into()
            })
        );
        assert_eq!(hit_test(&list, 1.0, 1.0), None);
    }

    #[test]
    fn paint_images_visibility_and_z_order_follow_the_render_contract() {
        let mut hidden = shape(1, 1.0, 1.0);
        hidden.children.push(ShapeChild::Cell(cell("NoShow", "1")));
        let mut deleted = shape(2, 2.0, 1.0);
        deleted.del = true;
        let mut text = shape(3, 3.0, 1.0);
        text.children
            .push(ShapeChild::Text(vec![TextToken::Literal("text".into())]));
        let mut image = shape(4, 4.0, 1.0);
        image.children.push(ShapeChild::ForeignData(ForeignData {
            foreign_type: Some("Bitmap".into()),
            compression_type: None,
            relationship_id: Some("image".into()),
            other_attrs: vec![],
        }));
        let mut package = package(vec![hidden, deleted, text, image]);
        package.relationships.insert(
            "page".into(),
            vec![vsdx_parse::Relationship {
                id: "image".into(),
                relationship_type: "image".into(),
                target: "media/image1.png".into(),
                target_mode: Default::default(),
                resolved_target: Some("visio/media/image1.png".into()),
            }],
        );
        package.add_part("visio/media/image1.png", vec![0]);
        let list = Renderer::default().layout_page(&package, "page").unwrap();
        assert!(
            matches!(&list.primitives[0], Primitive::Shape { fill: Some(Paint::Solid { color }), stroke: Some(Stroke { color: line, width, dashed: false }), .. } if color == "#010203" && line == "#040506" && *width == 0.02)
        );
        assert!(matches!(&list.primitives[1], Primitive::TextBox { .. }));
        assert!(
            matches!(&list.primitives[2], Primitive::Image { asset_id, .. } if asset_id == "visio/media/image1.png")
        );
        let z_orders = list
            .primitives
            .iter()
            .map(|primitive| match primitive {
                Primitive::Shape { z_order, .. }
                | Primitive::Image { z_order, .. }
                | Primitive::TextBox { z_order, .. }
                | Primitive::Placeholder { z_order, .. }
                | Primitive::Group { z_order, .. } => *z_order,
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(z_orders.len(), 3);
        assert_eq!(z_orders.into_iter().collect::<Vec<_>>(), vec![2, 3, 4]);
    }

    #[test]
    fn unresolved_foreign_data_and_colours_become_placeholders() {
        let mut image = shape(1, 1.0, 1.0);
        image.children.push(ShapeChild::ForeignData(ForeignData {
            foreign_type: None,
            compression_type: None,
            relationship_id: Some("missing".into()),
            other_attrs: vec![],
        }));
        let mut colour = shape(2, 2.0, 1.0);
        colour.children.retain(
            |child| !matches!(child, ShapeChild::Cell(Cell { name, .. }) if name == "FillForegnd"),
        );
        let list = render(vec![image, colour]);
        assert!(
            matches!(&list.primitives[0], Primitive::Placeholder { reason, .. } if reason == "unsupported ForeignData image")
        );
        assert!(
            matches!(&list.primitives[1], Primitive::Placeholder { reason, .. } if reason.starts_with("unresolvable colour:"))
        );
    }

    #[test]
    fn every_renderer_budget_rejects_cleanly_and_font_rejection_is_atomic() {
        let page = package(vec![shape(1, 1.0, 1.0)]);
        let mut limits = RenderLimits {
            max_shapes: 0,
            ..RenderLimits::default()
        };
        assert!(matches!(
            Renderer::new(limits.clone()).layout_page(&page, "page"),
            Err(RenderError::Budget("shapes"))
        ));
        let mut text_shape = shape(1, 1.0, 1.0);
        text_shape
            .children
            .push(ShapeChild::Text(vec![TextToken::Literal("x".into())]));
        let text_page = package(vec![text_shape]);
        for (name, set) in [
            ("text bytes", 0usize),
            ("text paragraphs", 0),
            ("text lines", 0),
            ("text runs", 0),
        ] {
            limits = RenderLimits::default();
            match name {
                "text bytes" => limits.max_text_bytes = set,
                "text paragraphs" => limits.max_text_paragraphs = set,
                "text lines" => limits.max_text_lines = set,
                _ => limits.max_text_runs = set,
            }
            assert!(
                matches!(Renderer::new(limits).layout_page(&text_page, "page"), Err(RenderError::Budget(actual)) if actual == name)
            );
        }
        limits = RenderLimits {
            max_display_list_bytes: 0,
            ..RenderLimits::default()
        };
        assert!(matches!(
            Renderer::new(limits).layout_page(&page, "page"),
            Err(RenderError::Budget("display-list bytes"))
        ));
        for limits in [
            RenderLimits {
                max_fonts: 0,
                ..RenderLimits::default()
            },
            RenderLimits {
                max_font_bytes: 0,
                ..RenderLimits::default()
            },
        ] {
            let mut renderer = Renderer::new(limits);
            assert!(renderer.register_font("x", false, false, vec![1]).is_err());
            assert_eq!(renderer.font_bytes, 0);
        }
    }

    #[test]
    fn pathological_group_nesting_is_placeholdered_at_the_depth_limit() {
        let mut nested = shape(65, 0.0, 0.0);
        for id in (1..65).rev() {
            let mut group = shape(id, 0.0, 0.0);
            group
                .children
                .retain(|child| !matches!(child, ShapeChild::Section(_)));
            group
                .children
                .push(ShapeChild::Shapes(vec![ShapesChild::Shape(nested)]));
            nested = group;
        }
        let list = render(vec![nested]);
        fn reasons(primitives: &[Primitive]) -> Vec<&str> {
            primitives
                .iter()
                .flat_map(|primitive| match primitive {
                    Primitive::Placeholder { reason, .. } => vec![reason.as_str()],
                    Primitive::Group { primitives, .. } => reasons(primitives),
                    _ => vec![],
                })
                .collect()
        }
        assert_eq!(reasons(&list.primitives), ["group nesting depth exceeded"]);
    }

    #[test]
    fn overflowing_coordinates_become_a_finite_placeholder() {
        let mut overflowing = shape(1, 1.0, 1.0);
        overflowing.children.retain(
            |child| !matches!(child, ShapeChild::Cell(Cell { name, .. }) if name == "PinX"),
        );
        overflowing
            .children
            .push(ShapeChild::Cell(cell("PinX", "1e308")));
        let list = render(vec![overflowing]);
        assert!(matches!(
            list.primitives.as_slice(),
            [Primitive::Placeholder { .. }]
        ));
        assert!(display_list_finite(&list));
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
        assert_eq!(list.contract_version, CONTRACT_VERSION);
        assert!(list.width > 0.0 && list.height > 0.0);
        assert!(display_list_finite(&list));
        assert!(!list.primitives.is_empty());
        assert!(
            list.primitives
                .iter()
                .all(|primitive| z_order(primitive) < 100)
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
                let mut page_painted = std::collections::BTreeSet::new();
                let mut page_placeholders = std::collections::BTreeSet::new();
                let total_shapes = package.page_contents[page]
                    .shapes()
                    .map(shape_count)
                    .sum::<usize>();
                count_primitives(
                    &list.primitives,
                    &mut page_painted,
                    &mut page_placeholders,
                    &mut reasons,
                );
                assert_eq!(page_painted.len() + page_placeholders.len(), total_shapes);
                painted += page_painted.len();
                placeholders += page_placeholders.len();
            }
        }
        eprintln!(
            "VSDX corpus render: painted={} placeholdered={} placeholder reasons={reasons:?}",
            painted, placeholders
        );
    }

    fn count_primitives(
        primitives: &[Primitive],
        painted: &mut std::collections::BTreeSet<String>,
        placeholders: &mut std::collections::BTreeSet<String>,
        reasons: &mut BTreeMap<String, usize>,
    ) {
        for primitive in primitives {
            match primitive {
                Primitive::Shape { id, .. }
                | Primitive::Image { id, .. }
                | Primitive::TextBox { id, .. } => {
                    painted.insert(id.clone());
                }
                Primitive::Placeholder { id, reason, .. } => {
                    placeholders.insert(id.clone());
                    *reasons.entry(reason.clone()).or_default() += 1;
                }
                Primitive::Group { primitives, .. } => {
                    count_primitives(primitives, painted, placeholders, reasons)
                }
            }
        }
    }

    fn shape_count(shape: &Shape) -> usize {
        1 + shape.shapes().map(shape_count).sum::<usize>()
    }
}
