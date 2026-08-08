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

struct RichParagraph {
    runs: Vec<TextRun>,
    align: i32,
    before: f32,
    after: f32,
}

fn rich_paragraphs(
    renderer: &Renderer,
    package: &VsdxPackage,
    tokens: &[vsdx_resolve::ResolvedTextToken],
) -> Vec<RichParagraph> {
    let mut character = TextRun {
        text: String::new(),
        family: "Arial".into(),
        size_px: 1.0 / 6.0,
        bold: false,
        italic: false,
        color: "#000000".into(),
        diagnostic: None,
    };
    let mut paragraphs = vec![RichParagraph {
        runs: Vec::new(),
        align: 0,
        before: 0.0,
        after: 0.0,
    }];
    for token in tokens {
        match token {
            vsdx_resolve::ResolvedTextToken::Literal(value) => {
                append_run(&mut paragraphs, &character, value.clone())
            }
            vsdx_resolve::ResolvedTextToken::CharacterRun { properties, .. } => {
                character = character_run(renderer, package, properties, &character)
            }
            vsdx_resolve::ResolvedTextToken::ParagraphRun { properties, .. } => {
                let mut paragraph = RichParagraph {
                    runs: Vec::new(),
                    align: property_number(properties, "HorzAlign").unwrap_or(0.0) as i32,
                    before: property_number(properties, "SpBefore").unwrap_or(0.0) as f32,
                    after: property_number(properties, "SpAfter").unwrap_or(0.0) as f32,
                };
                if let Some(indent) = property_number(properties, "IndLeft") {
                    paragraph.before += indent as f32 * 0.0;
                }
                paragraphs.push(paragraph);
            }
            vsdx_resolve::ResolvedTextToken::Tab { properties, .. } => {
                let width = property_number(properties, "Position").unwrap_or(0.5);
                append_run(&mut paragraphs, &character, "\t".into());
                if let Some(run) = paragraphs
                    .last_mut()
                    .and_then(|paragraph| paragraph.runs.last_mut())
                {
                    run.diagnostic = Some(format!("tab stop {width}"));
                }
            }
            vsdx_resolve::ResolvedTextToken::Field { properties, .. } => {
                let value = property_value(properties, "Value")
                    .or_else(|| property_value(properties, "Format"));
                let mut run = character.clone();
                match value {
                    Some(value) if !value.is_empty() => run.text = value.into(),
                    _ => {
                        run.text = "[unresolved field]".into();
                        run.diagnostic = Some("unresolvable field".into());
                    }
                }
                paragraphs
                    .last_mut()
                    .expect("paragraph exists")
                    .runs
                    .push(run);
            }
        }
    }
    paragraphs
}

fn append_run(paragraphs: &mut [RichParagraph], character: &TextRun, text: String) {
    let mut run = character.clone();
    run.text = text;
    paragraphs
        .last_mut()
        .expect("paragraph exists")
        .runs
        .push(run);
}

fn character_run(
    renderer: &Renderer,
    package: &VsdxPackage,
    properties: &std::collections::BTreeMap<String, Lookup>,
    current: &TextRun,
) -> TextRun {
    let mut run = current.clone();
    if let Some(font) = property_value(properties, "Font") {
        run.family = package
            .face_names
            .iter()
            .find(|face| {
                face.attributes
                    .iter()
                    .any(|(key, value)| key == "ID" && value == font)
            })
            .and_then(|face| {
                face.attributes
                    .iter()
                    .find(|(key, _)| key == "Name")
                    .map(|(_, value)| value.clone())
            })
            .unwrap_or_else(|| font.into());
    }
    if let Some(size) = property_number(properties, "Size") {
        run.size_px = size as f32;
    }
    if let Some(color) = property_value(properties, "Color") {
        run.color = if color.starts_with('#') {
            color.into()
        } else {
            format!("#{color}")
        };
    }
    if let Some(style) = property_number(properties, "Style") {
        let style = style as i32;
        run.bold = style & 1 != 0;
        run.italic = style & 2 != 0;
    }
    if property_value(properties, "Font").is_some() {
        run.diagnostic = renderer
            .font_for(&run.family, run.bold, run.italic)
            .is_none()
            .then(|| format!("unregistered font '{}'", run.family));
    }
    run
}

fn property_value<'a>(
    properties: &'a std::collections::BTreeMap<String, Lookup>,
    name: &str,
) -> Option<&'a str> {
    match properties.get(name)? {
        Lookup::Found(cell) => cell.cell.value.as_deref(),
        Lookup::Deleted | Lookup::Absent => None,
    }
}

fn property_number(
    properties: &std::collections::BTreeMap<String, Lookup>,
    name: &str,
) -> Option<f64> {
    property_value(properties, name)?
        .parse()
        .ok()
        .filter(|value: &f64| value.is_finite())
}

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
        if page_height <= 0.0
            || page_width <= 0.0
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
        for primitive in &mut state.primitives {
            bake_group_transform(primitive, Affine::identity());
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
            let group_transform = bounds_affine(
                bounds,
                child_coordinate_extent(
                    package,
                    resolver,
                    references,
                    page_part,
                    child_shapes.iter().copied(),
                ),
            );
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
            let z_order = state.next_z();
            state.primitives.push(Primitive::Group {
                id,
                z_order,
                primitives: children,
                transform: group_transform,
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
                    x: 0.0,
                    y: 0.0,
                    width: bounds.width as f32,
                    height: bounds.height as f32,
                    transform: bounds_affine(bounds, None),
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
        self.text(
            package, resolver, page_part, shape, &resolved, id, bounds, state,
        )?;
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        package: &VsdxPackage,
        resolver: &Resolver<'_>,
        page_part: &str,
        shape: &Shape,
        resolved: &ResolvedShape,
        id: String,
        bounds: Bounds,
        state: &mut State,
    ) -> Result<(), RenderError> {
        if shape.text().is_none() {
            return Ok(());
        }
        if matches!(resolved.cell("Char.Size"), Some(Lookup::Found(cell)) if cell.cell.value.as_deref().and_then(|value| value.parse::<f32>().ok()).is_some_and(|value| !value.is_finite()))
        {
            return Err(RenderError::PageDimensions(
                "non-finite text metrics".into(),
            ));
        }
        let page = package
            .page_contents
            .get(page_part)
            .ok_or_else(|| RenderError::MissingPage(page_part.into()))?;
        let tokens = resolver.resolve_text(shape, page)?;
        let mut paragraphs = rich_paragraphs(self, package, &tokens);
        if paragraphs.iter().all(|paragraph| paragraph.runs.is_empty()) {
            return Ok(());
        }
        for paragraph in &mut paragraphs {
            if paragraph.align == 3
                && let Some(run) = paragraph.runs.first_mut()
            {
                run.diagnostic = Some(match run.diagnostic.take() {
                    Some(diagnostic) => format!("{diagnostic}; justify falls back to left"),
                    None => "justify falls back to left".into(),
                });
            }
        }
        let text = paragraphs
            .iter()
            .flat_map(|paragraph| paragraph.runs.iter())
            .map(|run| run.text.as_str())
            .collect::<String>();
        let text_bytes = state
            .text_bytes
            .checked_add(text.len())
            .ok_or(RenderError::Budget("text bytes"))?;
        if text_bytes > self.limits.max_text_bytes {
            return Err(RenderError::Budget("text bytes"));
        }
        if state
            .text_paragraphs
            .checked_add(paragraphs.len())
            .ok_or(RenderError::Budget("text paragraphs"))?
            > self.limits.max_text_paragraphs
        {
            return Err(RenderError::Budget("text paragraphs"));
        }
        if state.text_lines >= self.limits.max_text_lines {
            return Err(RenderError::Budget("text lines"));
        }
        if state.text_runs >= self.limits.max_text_runs {
            return Err(RenderError::Budget("text runs"));
        }
        state.text_bytes = text_bytes;
        state.text_paragraphs += paragraphs.len();
        state.text_runs += paragraphs
            .iter()
            .map(|paragraph| paragraph.runs.len())
            .sum::<usize>();
        if state.text_runs > self.limits.max_text_runs {
            return Err(RenderError::Budget("text runs"));
        }
        let left = paint::number(resolved, "LeftMargin").unwrap_or(0.0) as f32;
        let right = paint::number(resolved, "RightMargin").unwrap_or(0.0) as f32;
        let top = paint::number(resolved, "TopMargin").unwrap_or(0.0) as f32;
        let bottom = paint::number(resolved, "BottomMargin").unwrap_or(0.0) as f32;
        let block_width = paint::number(resolved, "TxtWidth").unwrap_or(bounds.width) as f32;
        let block_height = paint::number(resolved, "TxtHeight").unwrap_or(bounds.height) as f32;
        let x = bounds.x as f32 + left;
        let y = bounds.y as f32 + top;
        let available_width = (block_width - left - right).max(0.0);
        let mut lines = Vec::new();
        let mut cursor_y = y;
        let mut offset = 0u32;
        for paragraph in &paragraphs {
            if paragraph.runs.is_empty() {
                continue;
            }
            cursor_y += paragraph.before;
            let laid_out = self.wrap_paragraph(paragraph, available_width, x, cursor_y, offset);
            offset += paragraph
                .runs
                .iter()
                .map(|run| run.text.len() as u32)
                .sum::<u32>();
            cursor_y += laid_out.iter().map(|line| line.height).sum::<f32>() + paragraph.after;
            lines.extend(laid_out);
        }
        if state
            .text_lines
            .checked_add(lines.len())
            .ok_or(RenderError::Budget("text lines"))?
            > self.limits.max_text_lines
        {
            return Err(RenderError::Budget("text lines"));
        }
        state.text_lines += lines.len();
        let used_height = cursor_y - y;
        let vertical = paint::number(resolved, "VerticalAlign").unwrap_or(0.0) as i32;
        let dy = match vertical {
            1 => (block_height - top - bottom - used_height) * 0.5,
            2 => block_height - top - bottom - used_height,
            _ => 0.0,
        }
        .max(0.0);
        for line in &mut lines {
            line.y += dy;
            for stop in &mut line.caret_stops {
                stop.y += dy;
            }
        }
        let z_order = state.next_z();
        state.primitives.push(Primitive::TextBox {
            id,
            z_order,
            x,
            y,
            width: available_width,
            height: (block_height - top - bottom).max(0.0),
            paragraphs: paragraphs
                .into_iter()
                .map(|paragraph| TextParagraph {
                    runs: paragraph.runs,
                })
                .collect(),
            lines,
        });
        Ok(())
    }
    fn wrap_paragraph(
        &self,
        paragraph: &RichParagraph,
        available_width: f32,
        x: f32,
        y: f32,
        offset: u32,
    ) -> Vec<PositionedLine> {
        let text = paragraph
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        let breaks = ooxml_text::break_opportunities(&text);
        let mut widths = Vec::new();
        let mut height = 0.0f32;
        for (index, ch) in text.char_indices() {
            let run = paragraph
                .runs
                .iter()
                .scan(0usize, |end, run| {
                    *end += run.text.len();
                    Some((*end > index).then_some(run))
                })
                .find_map(|run| run);
            let (width, line_height) = run
                .map(|run| (self.measure(run, &ch.to_string()), run.size_px * 1.2))
                .unwrap_or((0.0, 0.2));
            widths.push((index, ch.len_utf8(), width));
            height = height.max(line_height);
        }
        let height = height.max(0.2);
        let mut result = Vec::new();
        let mut start = 0usize;
        let mut cursor = 0.0;
        let mut last_break = None;
        let mut mandatory_break = None;
        for (position, length, width) in widths.iter().copied() {
            if mandatory_break == Some(position) {
                result.push(self.line(
                    &text,
                    &widths,
                    start,
                    position,
                    x,
                    y + result.len() as f32 * height,
                    height,
                    offset,
                    paragraph.align,
                    available_width,
                ));
                start = position;
                cursor = 0.0;
                last_break = None;
                mandatory_break = None;
            }
            if position > start && cursor + width > available_width && available_width > 0.0 {
                let end = last_break.unwrap_or(position).max(start + 1);
                result.push(self.line(
                    &text,
                    &widths,
                    start,
                    end,
                    x,
                    y + result.len() as f32 * height,
                    height,
                    offset,
                    paragraph.align,
                    available_width,
                ));
                start = end;
                cursor = widths
                    .iter()
                    .filter(|(p, _, _)| *p >= start && *p < position)
                    .map(|(_, _, w)| *w)
                    .sum();
                last_break = None;
            }
            cursor += width;
            let next = position + length;
            if let Some(opportunity) = breaks.iter().find(|item| item.byte_index == next) {
                if opportunity.mandatory && next < text.len() {
                    mandatory_break = Some(next);
                } else {
                    last_break = Some(next);
                }
            }
        }
        if start < text.len() || result.is_empty() {
            result.push(self.line(
                &text,
                &widths,
                start,
                text.len(),
                x,
                y + result.len() as f32 * height,
                height,
                offset,
                paragraph.align,
                available_width,
            ));
        }
        result
    }
    #[allow(clippy::too_many_arguments)]
    fn line(
        &self,
        _text: &str,
        widths: &[(usize, usize, f32)],
        start: usize,
        end: usize,
        x: f32,
        y: f32,
        height: f32,
        offset: u32,
        align: i32,
        available: f32,
    ) -> PositionedLine {
        let width = widths
            .iter()
            .filter(|(position, _, _)| *position >= start && *position < end)
            .map(|(_, _, width)| *width)
            .sum::<f32>();
        let line_x = x + match align {
            1 => (available - width) * 0.5,
            2 => available - width,
            _ => 0.0,
        };
        let mut advance = 0.0;
        let mut stops = Vec::new();
        for (position, length, char_width) in widths
            .iter()
            .copied()
            .filter(|(position, _, _)| *position >= start && *position < end)
        {
            stops.push(CaretStop {
                position: offset + position as u32,
                x: line_x + advance,
                y,
            });
            advance += char_width;
            if position + length == end {
                stops.push(CaretStop {
                    position: offset + end as u32,
                    x: line_x + advance,
                    y,
                });
            }
        }
        PositionedLine {
            x: line_x,
            y,
            width,
            height,
            start: offset + start as u32,
            end: offset + end as u32,
            caret_stops: stops,
        }
    }
    /// Uses the requested face, then same-family style variants, never another family.
    fn font_for(&self, family: &str, bold: bool, italic: bool) -> Option<ooxml_text::FontId> {
        [
            (bold, italic),
            (bold, false),
            (false, italic),
            (false, false),
        ]
        .into_iter()
        .find_map(|(bold, italic)| self.registered_fonts.get(&(family.into(), bold, italic)))
        .copied()
    }
    fn measure(&self, run: &TextRun, text: &str) -> f32 {
        self.font_for(&run.family, run.bold, run.italic)
            .and_then(|font| ooxml_text::shape(&self.fonts, font, text, run.size_px, &[]).ok())
            .map(|glyphs| glyphs.iter().map(|glyph| glyph.x_advance).sum())
            .unwrap_or(text.chars().count() as f32 * run.size_px * 0.5)
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
fn bake_group_transform(primitive: &mut Primitive, matrix: Affine) {
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
            let matrix = matrix.compose(*transform);
            for child in primitives {
                bake_group_transform(child, matrix);
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
fn transform_command(command: &mut ooxml_drawingml::GeometryPathCommand, bounds: Bounds) {
    transform_affine(command, bounds_affine(bounds, None));
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
fn bounds_affine(group: Bounds, child_extent: Option<ChildCoordinateExtent>) -> Affine {
    let loc_x = group.loc_pin_x;
    let loc_y = group.loc_pin_y;
    let pin_x = group.x + group.loc_pin_x;
    let pin_y = group.y + group.loc_pin_y;
    let (sin, cos) = group.angle.sin_cos();
    // Visio documents the lower-left local origin, but not how to derive a group child scale.
    // Use the conservative selection-extent ratio and translate that documented origin.
    let (origin_x, origin_y, scale_x, scale_y) = child_extent
        .map(|extent| {
            (
                extent.x,
                extent.y,
                group.width / extent.width,
                group.height / extent.height,
            )
        })
        .unwrap_or((0.0, 0.0, 1.0, 1.0));
    let sx = scale_x * if group.flip_x { -1.0 } else { 1.0 };
    let sy = scale_y * if group.flip_y { -1.0 } else { 1.0 };
    Affine {
        a: (cos * sx) as f32,
        b: (sin * sx) as f32,
        c: (-sin * sy) as f32,
        d: (cos * sy) as f32,
        e: (pin_x - cos * sx * (loc_x + origin_x) + sin * sy * (loc_y + origin_y)) as f32,
        f: (pin_y - sin * sx * (loc_x + origin_x) - cos * sy * (loc_y + origin_y)) as f32,
    }
}
struct ChildCoordinateExtent {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
fn child_coordinate_extent<'a>(
    package: &VsdxPackage,
    resolver: &Resolver<'_>,
    references: Option<&PageShapeReferences>,
    page_part: &str,
    shapes: impl Iterator<Item = &'a Shape>,
) -> Option<ChildCoordinateExtent> {
    let bounds = shapes
        .filter_map(|shape| {
            resolver
                .resolve_shape(page_part, shape.id)
                .ok()
                .and_then(|resolved| bounds(package, references, &resolved, shape.id))
        })
        .collect::<Vec<_>>();
    let min_x = bounds.iter().map(|bounds| bounds.x).reduce(f64::min)?;
    let min_y = bounds.iter().map(|bounds| bounds.y).reduce(f64::min)?;
    let max_x = bounds
        .iter()
        .map(|bounds| bounds.x + bounds.width)
        .reduce(f64::max)?;
    let max_y = bounds
        .iter()
        .map(|bounds| bounds.y + bounds.height)
        .reduce(f64::max)?;
    let width = max_x - min_x;
    let height = max_y - min_y;
    (width > 0.0 && height > 0.0).then_some(ChildCoordinateExtent {
        x: min_x,
        y: min_y,
        width,
        height,
    })
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
    let closed = path
        .iter()
        .any(|command| matches!(command, ooxml_drawingml::GeometryPathCommand::Close));
    fill && inside
        || stroke_width > 0.0
            && points.windows(2).any(|segment| {
                point_segment_distance(x, y, segment[0], segment[1]) <= stroke_width / 2.0
            })
        || stroke_width > 0.0
            && closed
            && points.len() > 1
            && point_segment_distance(x, y, points[points.len() - 1], points[0])
                <= stroke_width / 2.0
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

    fn text_shape(tokens: Vec<TextToken>) -> Shape {
        let mut shape = shape(1, 1.0, 1.0);
        shape.children.push(ShapeChild::Text(tokens));
        shape
    }

    fn text_box(list: &VsdxDisplayList) -> &Primitive {
        list.primitives
            .iter()
            .find(|primitive| matches!(primitive, Primitive::TextBox { .. }))
            .unwrap()
    }

    fn shape_primitive(list: &VsdxDisplayList, id: u32) -> &Primitive {
        list.primitives.iter().find(|primitive| matches!(primitive, Primitive::Shape { id: actual, .. } if actual == &format!("page:{id}"))).unwrap()
    }

    fn group(id: u32, pin_x: f64, pin_y: f64, children: Vec<Shape>) -> Shape {
        let mut group = shape(id, pin_x, pin_y);
        group
            .children
            .retain(|child| !matches!(child, ShapeChild::Section(_)));
        group.children.push(ShapeChild::Shapes(
            children.into_iter().map(ShapesChild::Shape).collect(),
        ));
        group
    }

    fn with_cell(shape: &mut Shape, name: &str, value: &str) {
        shape.children.retain(
            |child| !matches!(child, ShapeChild::Cell(Cell { name: actual, .. }) if actual == name),
        );
        shape.children.push(ShapeChild::Cell(cell(name, value)));
    }

    fn assert_point_close(actual: (f32, f32), expected: (f32, f32)) {
        assert!(
            (actual.0 - expected.0).abs() < 1e-5,
            "{actual:?} != {expected:?}"
        );
        assert!(
            (actual.1 - expected.1).abs() < 1e-5,
            "{actual:?} != {expected:?}"
        );
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
    fn word_wraps_at_uax14_opportunities_in_scene_inches() {
        let mut shape = text_shape(vec![TextToken::Literal("a b".into())]);
        with_cell(&mut shape, "TxtWidth", "0.18");
        let list = render(vec![shape]);
        let Primitive::TextBox { lines, .. } = text_box(&list) else {
            unreachable!()
        };
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0].start, lines[0].end), (0, 2));
        assert_eq!((lines[1].start, lines[1].end), (2, 3));
    }

    #[test]
    fn paragraph_alignment_positions_lines_in_scene_inches() {
        for (align, expected_x) in [(0, 1.0), (1, 1.458_333_4), (2, 1.916_666_6)] {
            let mut shape = text_shape(vec![
                TextToken::ParagraphRun(0),
                TextToken::Literal("a".into()),
            ]);
            with_cell(&mut shape, "TxtWidth", "1");
            shape.children.push(ShapeChild::Section(Section {
                name: "Paragraph".into(),
                index: None,
                del: false,
                children: vec![SectionChild::Row(row(
                    0,
                    "",
                    vec![cell("HorzAlign", &align.to_string())],
                ))],
                other_attrs: vec![],
            }));
            let list = render(vec![shape]);
            let Primitive::TextBox { lines, .. } = text_box(&list) else {
                unreachable!()
            };
            assert!((lines[0].x - expected_x).abs() < 1e-5);
        }
    }

    #[test]
    fn vertical_alignment_and_margins_inset_text_in_scene_inches() {
        for (align, expected_y) in [(0, 1.1), (1, 1.4), (2, 1.7)] {
            let mut shape = text_shape(vec![TextToken::Literal("a".into())]);
            let vertical = align.to_string();
            for (name, value) in [
                ("TxtWidth", "1"),
                ("TxtHeight", "1"),
                ("LeftMargin", "0.1"),
                ("RightMargin", "0.2"),
                ("TopMargin", "0.1"),
                ("BottomMargin", "0.1"),
                ("VerticalAlign", vertical.as_str()),
            ] {
                with_cell(&mut shape, name, value);
            }
            let list = render(vec![shape]);
            let Primitive::TextBox {
                x,
                y,
                width,
                height,
                lines,
                ..
            } = text_box(&list)
            else {
                unreachable!()
            };
            assert_point_close((*x, *y), (1.1, 1.1));
            assert_point_close((*width, *height), (0.7, 0.8));
            assert!((lines[0].y - expected_y).abs() < 1e-5);
            assert!((lines[0].height - 0.2).abs() < 1e-5);
        }
    }

    #[test]
    fn unregistered_font_records_a_diagnostic() {
        let mut shape = text_shape(vec![
            TextToken::CharacterRun(0),
            TextToken::Literal("a".into()),
        ]);
        shape.children.push(ShapeChild::Section(Section {
            name: "Character".into(),
            index: None,
            del: false,
            children: vec![SectionChild::Row(row(0, "", vec![cell("Font", "99")]))],
            other_attrs: vec![],
        }));
        let list = render(vec![shape]);
        let Primitive::TextBox { paragraphs, .. } = text_box(&list) else {
            unreachable!()
        };
        assert_eq!(
            paragraphs[0].runs[0].diagnostic.as_deref(),
            Some("unregistered font '99'")
        );
    }

    #[test]
    fn group_child_extent_origin_maps_to_the_group_lower_left_corner() {
        let matrix = bounds_affine(
            Bounds {
                x: 10.0,
                y: 20.0,
                width: 8.0,
                height: 12.0,
                ..Default::default()
            },
            Some(ChildCoordinateExtent {
                x: 2.0,
                y: 3.0,
                width: 4.0,
                height: 6.0,
            }),
        );
        assert_point_close(matrix.apply_point(2.0, 3.0), (10.0, 20.0));
        assert_point_close(matrix.apply_point(6.0, 9.0), (18.0, 32.0));
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
            matches!(path[0], GeometryPathCommand::Move { x, y } if (x - 10.0).abs() < 1e-9 && (y - 20.0).abs() < 1e-9)
        );
    }

    #[test]
    fn forty_five_degree_group_transforms_all_text_corners_lines_and_caret_stops() {
        let mut child = shape(2, 1.0, 2.0);
        child
            .children
            .push(ShapeChild::Text(vec![TextToken::Literal("ab".into())]));
        let mut parent = group(1, 10.0, 20.0, vec![child]);
        with_cell(
            &mut parent,
            "Angle",
            &std::f64::consts::FRAC_PI_4.to_string(),
        );
        let list = render(vec![parent]);
        let Primitive::Group { primitives, .. } = &list.primitives[0] else {
            unreachable!()
        };
        let Primitive::TextBox {
            x,
            y,
            width,
            height,
            lines,
            ..
        } = &primitives[1]
        else {
            unreachable!()
        };
        let matrix = Affine {
            a: std::f32::consts::FRAC_1_SQRT_2,
            b: std::f32::consts::FRAC_1_SQRT_2,
            c: -std::f32::consts::FRAC_1_SQRT_2,
            d: std::f32::consts::FRAC_1_SQRT_2,
            e: 10.707_107,
            f: 17.878_68,
        };
        let corners =
            [(1.0, 2.0), (2.0, 2.0), (1.0, 3.0), (2.0, 3.0)].map(|(x, y)| matrix.apply_point(x, y));
        let min_x = corners
            .iter()
            .map(|point| point.0)
            .fold(f32::INFINITY, f32::min);
        let max_x = corners
            .iter()
            .map(|point| point.0)
            .fold(f32::NEG_INFINITY, f32::max);
        let min_y = corners
            .iter()
            .map(|point| point.1)
            .fold(f32::INFINITY, f32::min);
        let max_y = corners
            .iter()
            .map(|point| point.1)
            .fold(f32::NEG_INFINITY, f32::max);
        assert_point_close((*x, *y), (min_x, min_y));
        assert_point_close((*width, *height), (max_x - min_x, max_y - min_y));
        assert_point_close((lines[0].x, lines[0].y), matrix.apply_point(1.0, 2.0));
        assert_point_close(
            (lines[0].caret_stops[1].x, lines[0].caret_stops[1].y),
            matrix.apply_point(1.0 + 0.166_666_67 * 0.5, 2.0),
        );
    }

    #[test]
    fn forty_five_degree_group_preserves_image_orientation_and_all_corners() {
        let mut image = shape(2, 1.0, 2.0);
        image.children.push(ShapeChild::ForeignData(ForeignData {
            foreign_type: None,
            compression_type: None,
            relationship_id: Some("image".into()),
            other_attrs: vec![],
        }));
        let mut parent = group(1, 10.0, 20.0, vec![image]);
        with_cell(
            &mut parent,
            "Angle",
            &std::f64::consts::FRAC_PI_4.to_string(),
        );
        let mut package = package(vec![parent]);
        package.relationships.insert(
            "page".into(),
            vec![vsdx_parse::Relationship {
                id: "image".into(),
                relationship_type: "image".into(),
                target: "image.png".into(),
                target_mode: Default::default(),
                resolved_target: Some("image.png".into()),
            }],
        );
        package.add_part("image.png", vec![0]);
        let list = Renderer::default().layout_page(&package, "page").unwrap();
        let Primitive::Group { primitives, .. } = &list.primitives[0] else {
            unreachable!()
        };
        let Primitive::Image {
            x,
            y,
            width,
            height,
            transform,
            ..
        } = &primitives[0]
        else {
            unreachable!()
        };
        assert_eq!((*x, *y, *width, *height), (0.0, 0.0, 1.0, 1.0));
        assert!((transform.b - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        let corners = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
            .map(|(x, y)| transform.apply_point(x, y));
        let expected = [(1.0, 2.0), (2.0, 2.0), (1.0, 3.0), (2.0, 3.0)].map(|(x, y)| {
            Affine {
                a: std::f32::consts::FRAC_1_SQRT_2,
                b: std::f32::consts::FRAC_1_SQRT_2,
                c: -std::f32::consts::FRAC_1_SQRT_2,
                d: std::f32::consts::FRAC_1_SQRT_2,
                e: 10.707_107,
                f: 17.878_68,
            }
            .apply_point(x, y)
        });
        for (actual, expected) in corners.into_iter().zip(expected) {
            assert_point_close(actual, expected);
        }
    }

    #[test]
    fn nested_rotated_flipped_groups_compose_by_matrix_multiplication() {
        let child = shape(3, 1.0, 0.0);
        let mut inner = group(2, 2.0, 3.0, vec![child]);
        with_cell(
            &mut inner,
            "Angle",
            &std::f64::consts::FRAC_PI_2.to_string(),
        );
        with_cell(&mut inner, "FlipX", "1");
        let mut outer = group(1, 10.0, 20.0, vec![inner]);
        with_cell(
            &mut outer,
            "Angle",
            &std::f64::consts::FRAC_PI_2.to_string(),
        );
        with_cell(&mut outer, "FlipY", "1");
        let list = render(vec![outer]);
        let Primitive::Group { primitives, .. } = &list.primitives[0] else {
            unreachable!()
        };
        let Primitive::Group { primitives, .. } = &primitives[0] else {
            unreachable!()
        };
        let Primitive::Shape { path, .. } = &primitives[0] else {
            unreachable!()
        };
        let GeometryPathCommand::Move { x, y } = path[0] else {
            unreachable!()
        };
        let outer = Affine {
            a: 0.0,
            b: 1.0,
            c: 1.0,
            d: 0.0,
            e: 7.0,
            f: 18.0,
        };
        let inner = Affine {
            a: 0.0,
            b: -1.0,
            c: -1.0,
            d: 0.0,
            e: 2.0,
            f: 4.0,
        };
        let expected = outer.compose(inner).apply_point(1.0, 0.0);
        assert_point_close((x as f32, y as f32), expected);
    }

    #[test]
    fn group_non_uniformly_scales_children_before_rotation_and_flip() {
        let child = shape(2, 0.0, 0.0);
        let mut group = group(1, 10.0, 20.0, vec![child]);
        with_cell(&mut group, "Width", "4");
        with_cell(&mut group, "Height", "3");
        with_cell(
            &mut group,
            "Angle",
            &std::f64::consts::FRAC_PI_2.to_string(),
        );
        with_cell(&mut group, "FlipX", "1");
        let list = render(vec![group]);
        let Primitive::Group { primitives, .. } = &list.primitives[0] else {
            unreachable!()
        };
        let Primitive::Shape { path, .. } = &primitives[0] else {
            unreachable!()
        };
        let GeometryPathCommand::Line { x, y } = path[1] else {
            unreachable!()
        };
        assert_point_close((x as f32, y as f32), (10.0, 16.0));
    }

    #[test]
    fn affine_inversion_round_trips_and_degenerate_matrices_are_safe() {
        let matrix = Affine {
            a: -1.5,
            b: -2.0,
            c: -0.5,
            d: 3.0,
            e: 12.0,
            f: -7.0,
        };
        let inverse = matrix.invert().unwrap();
        assert_point_close(
            inverse.apply_point(
                matrix.apply_point(3.25, -8.5).0,
                matrix.apply_point(3.25, -8.5).1,
            ),
            (3.25, -8.5),
        );
        let degenerate = Affine {
            a: 0.0,
            ..Affine::identity()
        };
        assert_eq!(degenerate.invert(), None);
        assert!(!point_in_transformed_rect(
            0.0, 0.0, 1.0, 1.0, degenerate, 0.5, 0.5
        ));
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
    fn hit_testing_uses_rotated_quads_curves_strokes_and_z_order() {
        let canvas = VsdxDisplayList {
            contract_version: CONTRACT_VERSION,
            width: 100.0,
            height: 100.0,
            paint_transform: final_paint_transform(1.0),
            primitives: vec![
                Primitive::Shape {
                    id: "bottom".into(),
                    z_order: 10,
                    path: vec![
                        GeometryPathCommand::Move { x: 0.0, y: 0.0 },
                        GeometryPathCommand::Line { x: 1.0, y: 0.0 },
                        GeometryPathCommand::Line { x: 1.0, y: 1.0 },
                        GeometryPathCommand::Line { x: 0.0, y: 1.0 },
                    ],
                    fill: Some(Paint::Solid {
                        color: "#000".into(),
                    }),
                    stroke: None,
                    transform: Affine::identity(),
                },
                Primitive::Shape {
                    id: "top".into(),
                    z_order: 20,
                    path: vec![
                        GeometryPathCommand::Move { x: 0.0, y: 0.0 },
                        GeometryPathCommand::Line { x: 1.0, y: 0.0 },
                        GeometryPathCommand::Line { x: 1.0, y: 1.0 },
                        GeometryPathCommand::Line { x: 0.0, y: 1.0 },
                    ],
                    fill: Some(Paint::Solid {
                        color: "#000".into(),
                    }),
                    stroke: None,
                    transform: Affine::identity(),
                },
                Primitive::Image {
                    id: "rotated-image".into(),
                    z_order: 1,
                    asset_id: "i".into(),
                    x: 0.0,
                    y: 0.0,
                    width: 2.0,
                    height: 2.0,
                    transform: Affine {
                        a: std::f32::consts::FRAC_1_SQRT_2,
                        b: std::f32::consts::FRAC_1_SQRT_2,
                        c: -std::f32::consts::FRAC_1_SQRT_2,
                        d: std::f32::consts::FRAC_1_SQRT_2,
                        e: 5.0,
                        f: 0.0,
                    },
                },
                Primitive::Shape {
                    id: "curve".into(),
                    z_order: 2,
                    path: vec![
                        GeometryPathCommand::Move { x: 7.0, y: 0.0 },
                        GeometryPathCommand::Cubic {
                            cp1x: 9.0,
                            cp1y: 0.0,
                            cp2x: 9.0,
                            cp2y: 2.0,
                            x: 7.0,
                            y: 2.0,
                        },
                        GeometryPathCommand::Cubic {
                            cp1x: 5.0,
                            cp1y: 2.0,
                            cp2x: 5.0,
                            cp2y: 0.0,
                            x: 7.0,
                            y: 0.0,
                        },
                    ],
                    fill: Some(Paint::Solid {
                        color: "#000".into(),
                    }),
                    stroke: None,
                    transform: Affine::identity(),
                },
                Primitive::Shape {
                    id: "stroke".into(),
                    z_order: 3,
                    path: vec![
                        GeometryPathCommand::Move { x: 0.0, y: 3.0 },
                        GeometryPathCommand::Line { x: 2.0, y: 3.0 },
                    ],
                    fill: None,
                    stroke: Some(Stroke {
                        color: "#000".into(),
                        width: 0.2,
                        dashed: false,
                    }),
                    transform: Affine::identity(),
                },
            ],
        };
        let hit = |x: f32, y: f32| hit_test(&canvas, x * 96.0, 96.0 - y * 96.0);
        assert_eq!(
            hit(0.5, 0.5),
            Some(HitTestResult::Shape {
                shape_id: "top".into()
            })
        );
        assert_eq!(
            hit(5.0, 1.0),
            Some(HitTestResult::Shape {
                shape_id: "rotated-image".into()
            })
        );
        assert_eq!(hit(3.7, 0.1), None);
        assert_eq!(
            hit(7.0, 1.0),
            Some(HitTestResult::Shape {
                shape_id: "curve".into()
            })
        );
        assert_eq!(hit(9.5, 1.0), None);
        assert_eq!(
            hit(1.0, 3.08),
            Some(HitTestResult::Shape {
                shape_id: "stroke".into()
            })
        );
    }

    #[test]
    fn hit_testing_returns_a_rotated_group_child() {
        let child = shape(2, 1.0, 0.0);
        let mut parent = group(1, 5.0, 5.0, vec![child]);
        with_cell(
            &mut parent,
            "Angle",
            &std::f64::consts::FRAC_PI_2.to_string(),
        );
        let list = render(vec![parent]);
        assert_eq!(
            hit_test(&list, 96.0 * 4.5, 768.0 - 96.0 * 5.5),
            Some(HitTestResult::Shape {
                shape_id: "page:2".into()
            })
        );
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
    fn dangling_media_and_non_finite_stroke_width_become_placeholders() {
        let mut image = shape(1, 1.0, 1.0);
        image.children.push(ShapeChild::ForeignData(ForeignData {
            foreign_type: None,
            compression_type: None,
            relationship_id: Some("image".into()),
            other_attrs: vec![],
        }));
        let mut line = shape(2, 2.0, 1.0);
        with_cell(&mut line, "LineWeight", "1e100");
        let mut package = package(vec![image, line]);
        package.relationships.insert(
            "page".into(),
            vec![vsdx_parse::Relationship {
                id: "image".into(),
                relationship_type: "image".into(),
                target: "missing.png".into(),
                target_mode: Default::default(),
                resolved_target: Some("missing.png".into()),
            }],
        );
        let list = Renderer::default().layout_page(&package, "page").unwrap();
        assert!(
            matches!(&list.primitives[0], Primitive::Placeholder { reason, .. } if reason.contains("dangling ForeignData image target"))
        );
        assert!(
            matches!(&list.primitives[1], Primitive::Placeholder { reason, .. } if reason.contains("non-finite stroke width"))
        );
    }

    #[test]
    fn unsupported_colour_reason_preserves_evaluator_detail() {
        let mut coloured = shape(1, 1.0, 1.0);
        coloured.children.retain(
            |child| !matches!(child, ShapeChild::Cell(Cell { name, .. }) if name == "FillForegnd"),
        );
        coloured
            .children
            .push(ShapeChild::Cell(formula("FillForegnd", "THEMEVAL(999)")));
        let list = render(vec![coloured]);
        assert!(
            matches!(&list.primitives[0], Primitive::Placeholder { reason, .. } if reason.starts_with("unresolvable colour:") && reason.len() > "unresolvable colour:".len())
        );
    }

    #[test]
    fn invalid_page_dimensions_are_rejected_before_pixel_conversion() {
        for (width, height) in [("0", "8"), ("-1", "8"), ("1e100", "8")] {
            let mut page = package(vec![]);
            let sheet = page.page_sheets.get_mut(&1).unwrap();
            sheet.children = vec![
                SheetChild::Cell(cell("PageWidth", width)),
                SheetChild::Cell(cell("PageHeight", height)),
            ];
            assert!(
                matches!(
                    Renderer::default().layout_page(&page, "page"),
                    Err(RenderError::PageDimensions(_))
                ),
                "{width} x {height}"
            );
        }
    }

    #[test]
    fn non_finite_text_run_size_and_line_height_are_rejected() {
        let mut text = shape(1, 1.0, 1.0);
        with_cell(&mut text, "Char.Size", "1e100");
        text.children
            .push(ShapeChild::Text(vec![TextToken::Literal("x".into())]));
        assert!(matches!(
            Renderer::default().layout_page(&package(vec![text]), "page"),
            Err(RenderError::PageDimensions(_))
        ));
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
    fn closed_path_stroke_hits_its_implicit_closing_segment() {
        let path = vec![
            GeometryPathCommand::Move { x: 0.0, y: 0.0 },
            GeometryPathCommand::Line { x: 1.0, y: 0.0 },
            GeometryPathCommand::Line { x: 1.0, y: 1.0 },
            GeometryPathCommand::Close,
        ];
        assert!(path_hit(&path, Affine::identity(), false, 0.1, 0.5, 0.5));
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
        let actual = serde_json::to_value(list).unwrap();
        let expected: serde_json::Value =
            serde_json::from_str(include_str!("../tests/golden/foundation.json")).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn nested_groups_fixture_preserves_scaled_affine_content_and_replay_order() {
        let package = vsdx_parse::parse_vsdx(include_bytes!(
            "../../vsdx-parse/tests/fixtures/nested-groups.vsdx"
        ))
        .unwrap();
        let list = Renderer::default()
            .layout_page(&package, &package.page_part_paths[0])
            .unwrap();
        let Primitive::Group {
            primitives: outer, ..
        } = &list.primitives[0]
        else {
            unreachable!()
        };
        let Primitive::Group {
            primitives: inner, ..
        } = &outer[0]
        else {
            unreachable!()
        };
        assert!(matches!(&inner[0], Primitive::Shape { id, .. } if id.ends_with(":3")));
        assert!(
            matches!(&inner[1], Primitive::TextBox { id, lines, .. } if id.ends_with(":3") && lines.len() == 2)
        );
        assert!(
            matches!(&inner[2], Primitive::Image { id, asset_id, .. } if id.ends_with(":4") && asset_id == "visio/media/image1.png")
        );
        assert!(
            matches!(&inner[3], Primitive::Placeholder { id, reason, .. } if id.ends_with(":5") && reason.starts_with("unsupported geometry:"))
        );
        let Primitive::Shape { path, .. } = &inner[0] else {
            unreachable!()
        };
        let GeometryPathCommand::Move { x, y } = path[0] else {
            unreachable!()
        };
        // These values were calculated by hand from the composed outer and inner affine,
        // M = [[-0.94190204, 1.8844845, 10.021151], [-1.1970047, 0.27151108, 11.018823]].
        // Each expected AABB is min/max(M(corner)) for the original source rectangle;
        // the image corners and text caret are direct substitutions into that same affine.
        assert_point_close((x as f32, y as f32), (10.021151, 11.018823));
        let Primitive::TextBox {
            x,
            y,
            width,
            height,
            lines,
            ..
        } = &inner[1]
        else {
            unreachable!()
        };
        assert_point_close((*x, *y), (9.079249, 9.821818));
        assert_point_close((*width, *height), (2.8263865, 1.4685154));
        assert_point_close((lines[0].x, lines[0].y), (10.021151, 11.018823));
        assert_point_close(
            (lines[0].caret_stops[1].x, lines[0].caret_stops[1].y),
            (9.942658, 10.919072),
        );
        let Primitive::Image {
            x,
            y,
            width,
            height,
            transform,
            ..
        } = &inner[2]
        else {
            unreachable!()
        };
        let corners = [
            transform.apply_point(*x, *y),
            transform.apply_point(*x + *width, *y),
            transform.apply_point(*x, *y + *height),
            transform.apply_point(*x + *width, *y + *height),
        ];
        assert_point_close(corners[0], (10.0218315, 8.896324));
        assert_point_close(corners[1], (9.079929, 7.6993194));
        assert_point_close(corners[2], (13.7908, 9.439346));
        assert_point_close(corners[3], (12.848899, 8.242341));
        let Primitive::Placeholder {
            x,
            y,
            width,
            height,
            ..
        } = &inner[3]
        else {
            unreachable!()
        };
        assert_point_close((*x, *y), (13.7908, 9.439346));
        assert_point_close((*width, *height), (2.8263874, 1.4685163));
        let z_orders = inner.iter().map(z_order).collect::<Vec<_>>();
        assert_eq!(z_orders, vec![2, 3, 4, 5]);
        assert_eq!(
            hit_test(&list, 10.021151 * 96.0, (11.0 - 11.018823) * 96.0),
            Some(HitTestResult::Text {
                shape_id: "visio/pages/page1.xml:3".into(),
                position: 0,
            })
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
        let mut groups = 0usize;
        let mut reasons = BTreeMap::new();
        for file in ["lichtsysteme.vsdx", "soundplan.vsdx"] {
            let path = std::path::Path::new(&directory).join(file);
            let package = vsdx_parse::parse_vsdx(&std::fs::read(path).unwrap()).unwrap();
            for page in &package.page_part_paths {
                let list = Renderer::default().layout_page(&package, page).unwrap();
                let json = serde_json::to_string(&list).unwrap();
                assert!(!json.contains("NaN") && !json.contains("Infinity"));
                let expected = package.page_contents[page]
                    .shapes()
                    .flat_map(|shape| shape_ids(page, shape))
                    .collect::<std::collections::BTreeSet<_>>();
                let mut page_shapes = std::collections::BTreeSet::new();
                let mut page_text = std::collections::BTreeSet::new();
                let mut page_images = std::collections::BTreeSet::new();
                let mut page_placeholders = std::collections::BTreeSet::new();
                let mut page_groups = std::collections::BTreeSet::new();
                let mut text_shapes = std::collections::BTreeSet::new();
                let mut image_shapes = std::collections::BTreeSet::new();
                expected_subcontent(
                    package.page_contents[page].shapes(),
                    page,
                    &mut text_shapes,
                    &mut image_shapes,
                );
                count_primitives(
                    &list.primitives,
                    &mut page_shapes,
                    &mut page_text,
                    &mut page_images,
                    &mut page_placeholders,
                    &mut page_groups,
                    &mut reasons,
                );
                assert!(page_shapes.is_disjoint(&page_placeholders));
                assert!(page_shapes.is_disjoint(&page_groups));
                assert!(page_placeholders.is_disjoint(&page_groups));
                let actual = page_shapes
                    .union(&page_text)
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>();
                let actual = actual
                    .union(&page_images)
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>();
                let actual = actual
                    .union(&page_placeholders)
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>();
                let actual = actual
                    .union(&page_groups)
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(actual, expected);
                assert_eq!(page_text, text_shapes);
                assert_eq!(page_images, image_shapes);
                painted += page_shapes.len() + page_text.len() + page_images.len();
                placeholders += page_placeholders.len();
                groups += page_groups.len();
            }
        }
        eprintln!(
            "VSDX corpus render: painted={painted} placeholdered={placeholders} group={groups} placeholder reasons={reasons:?}",
        );
    }

    fn count_primitives(
        primitives: &[Primitive],
        shapes: &mut std::collections::BTreeSet<String>,
        text: &mut std::collections::BTreeSet<String>,
        images: &mut std::collections::BTreeSet<String>,
        placeholders: &mut std::collections::BTreeSet<String>,
        groups: &mut std::collections::BTreeSet<String>,
        reasons: &mut BTreeMap<String, usize>,
    ) {
        for primitive in primitives {
            match primitive {
                Primitive::Shape { id, .. } => {
                    shapes.insert(id.clone());
                }
                Primitive::TextBox { id, .. } => {
                    text.insert(id.clone());
                }
                Primitive::Image { id, .. } => {
                    images.insert(id.clone());
                }
                Primitive::Placeholder { id, reason, .. } => {
                    placeholders.insert(id.clone());
                    *reasons.entry(reason.clone()).or_default() += 1;
                }
                Primitive::Group { id, primitives, .. } => {
                    groups.insert(id.clone());
                    count_primitives(
                        primitives,
                        shapes,
                        text,
                        images,
                        placeholders,
                        groups,
                        reasons,
                    )
                }
            }
        }
    }

    fn shape_ids(page: &str, shape: &Shape) -> Vec<String> {
        std::iter::once(format!("{page}:{}", shape.id))
            .chain(shape.shapes().flat_map(|child| shape_ids(page, child)))
            .collect()
    }

    fn expected_subcontent<'a>(
        shapes: impl Iterator<Item = &'a Shape>,
        page: &str,
        text: &mut std::collections::BTreeSet<String>,
        images: &mut std::collections::BTreeSet<String>,
    ) {
        for shape in shapes {
            let id = format!("{page}:{}", shape.id);
            if shape.text().is_some_and(|tokens| {
                tokens
                    .iter()
                    .any(|token| matches!(token, TextToken::Literal(value) if !value.is_empty()))
            }) {
                text.insert(id.clone());
            }
            if shape.foreign_data().is_some() {
                images.insert(id);
            }
            expected_subcontent(shape.shapes(), page, text, images);
        }
    }
}
