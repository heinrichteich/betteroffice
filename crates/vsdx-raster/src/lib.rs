//! Raster backend: paints a VSDX display-list page to PNG via tiny-skia.
//!
//! Twin of the browser canvas painter; text is always set in the Carlito
//! Regular vendored by `betteroffice-xlsx-raster`, so output is identical on
//! every machine with no system font access.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::LazyLock;

use ooxml_drawingml::GeometryPathCommand;
use rustybuzz::ttf_parser::{self, OutlineBuilder};
use tiny_skia::{
    Color, FillRule, FilterQuality, GradientStop, IntSize, LinearGradient, Paint, Path, PathBuilder,
    Pixmap, PixmapPaint, Point, Rect, SpreadMode, Stroke, StrokeDash, Transform,
};
use vsdx_parse::VsdxPackage;
use vsdx_render::{
    Affine, Paint as VsPaint, Primitive, Renderer, Stroke as VsStroke, VsdxDisplayList,
    text_fragments,
};

/// Carlito Regular shared with the XLSX raster backend (OFL, metric-compatible
/// with Calibri, the usual Visio body font).
const FONT_BYTES: &[u8] = include_bytes!("../../xlsx-raster/assets/Carlito-Regular.ttf");

static FACE: LazyLock<rustybuzz::Face<'static>> = LazyLock::new(|| {
    rustybuzz::Face::from_slice(FONT_BYTES, 0).expect("vendored carlito is a valid font")
});

/// One rendered page's longest side.
pub const MAX_PAGE_DIM: u32 = 16_384;
/// One rendered page's surface.
pub const MAX_PAGE_PIXELS: u64 = 16_777_216;
/// One decoded image.
pub const MAX_IMAGE_PIXELS: u64 = 33_554_432;
/// Synthetic bold's second pass, in device pixels at 12 pt.
const BOLD_OFFSET_PX: f32 = 0.35;
/// Synthetic italic shear, scale-independent.
const ITALIC_SHEAR: f32 = 0.21;
/// Glyphs above this device size are skipped rather than rasterized.
const MAX_GLYPH_PX: f32 = 8192.0;

/// PNG bytes plus what the render could not draw.
pub struct RenderedPage {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Images missing, undecodable, over budget, or in an unsupported format.
    pub skipped_images: usize,
}

/// Renders one diagram page to PNG at `scale` times the 96 dpi display list.
pub fn render_page(
    renderer: &Renderer,
    package: &VsdxPackage,
    page_index: usize,
    scale: f32,
) -> Result<RenderedPage, String> {
    let part = package
        .page_part_paths
        .get(page_index)
        .ok_or_else(|| format!("page index {page_index} is out of range"))?;
    let list = renderer
        .layout_page(package, part)
        .map_err(|error| error.to_string())?;
    let mut assets = Vec::new();
    for primitive in &list.primitives {
        collect_images(primitive, &mut assets);
    }
    let images: HashMap<&str, &[u8]> = assets
        .into_iter()
        .filter_map(|asset_id| {
            package
                .part_bytes(asset_id)
                .map(|bytes| (asset_id, bytes))
        })
        .collect();
    render_list(&list, &images, scale)
}

fn collect_images<'a>(primitive: &'a Primitive, out: &mut Vec<&'a str>) {
    match primitive {
        Primitive::Image { asset_id, .. } => out.push(asset_id),
        Primitive::Group { primitives, .. } => {
            for child in primitives {
                collect_images(child, out);
            }
        }
        _ => {}
    }
}

/// Renders an already laid-out page; `images` maps asset ids to file bytes.
pub fn render_list(
    list: &VsdxDisplayList,
    images: &HashMap<&str, &[u8]>,
    scale: f32,
) -> Result<RenderedPage, String> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err("png scale must be a finite positive number".to_owned());
    }
    let width = (list.width * scale).ceil().max(1.0) as u32;
    let height = (list.height * scale).ceil().max(1.0) as u32;
    if width > MAX_PAGE_DIM || height > MAX_PAGE_DIM {
        return Err("png page dimensions exceed the raster budget".to_owned());
    }
    if u64::from(width) * u64::from(height) > MAX_PAGE_PIXELS {
        return Err("png page surface exceeds the raster budget".to_owned());
    }
    let mut pixmap = Pixmap::new(width, height).ok_or_else(|| "invalid pixmap size".to_owned())?;
    pixmap.fill(Color::WHITE);
    let paint = list.paint_transform;
    let base = Affine {
        a: scale,
        b: 0.0,
        c: 0.0,
        d: scale,
        e: 0.0,
        f: 0.0,
    }
    .compose(Affine {
        a: paint.a,
        b: paint.b,
        c: paint.c,
        d: paint.d,
        e: paint.e,
        f: paint.f,
    });
    let skipped = {
        let mut painter = Painter {
            pixmap: &mut pixmap,
            images,
            skipped_images: 0,
        };
        let mut top: Vec<&Primitive> = list.primitives.iter().collect();
        top.sort_by_key(|primitive| z_order(primitive));
        for primitive in top {
            painter.paint(primitive, base);
        }
        painter.skipped_images
    };
    let bytes = encode_png(pixmap, width, height)?;
    Ok(RenderedPage {
        bytes,
        width,
        height,
        skipped_images: skipped,
    })
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

fn tiny(transform: Affine) -> Transform {
    Transform::from_row(
        transform.a,
        transform.b,
        transform.c,
        transform.d,
        transform.e,
        transform.f,
    )
}

fn parse_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range)?, 16).ok();
    Some(Color::from_rgba8(
        channel(0..2)?,
        channel(2..4)?,
        channel(4..6)?,
        255,
    ))
}

struct Painter<'a, 'b> {
    pixmap: &'a mut Pixmap,
    images: &'b HashMap<&'b str, &'b [u8]>,
    skipped_images: usize,
}

impl Painter<'_, '_> {
    fn paint(&mut self, primitive: &Primitive, outer: Affine) {
        match primitive {
            Primitive::Shape {
                path,
                fill,
                stroke,
                transform,
                ..
            } => {
                let Some(shape) = build_path(path) else {
                    return;
                };
                let composed = tiny(outer.compose(*transform));
                if let Some(paint) = fill_paint(fill, path) {
                    self.pixmap
                        .fill_path(&shape, &paint, FillRule::Winding, composed, None);
                }
                if let Some((paint, stroke)) = stroke_paint(stroke) {
                    self.pixmap.stroke_path(&shape, &paint, &stroke, composed, None);
                }
            }
            Primitive::TextBox {
                paragraphs,
                lines,
                transform,
                ..
            } => {
                let composed = outer.compose(*transform);
                for fragment in text_fragments(paragraphs, lines) {
                    self.text(&fragment, composed);
                }
            }
            Primitive::Image {
                asset_id,
                x,
                y,
                width,
                height,
                transform,
                ..
            } => {
                let composed = outer.compose(*transform);
                match self.decode(asset_id) {
                    Some((source, bitmap)) => {
                        let fit = Affine {
                            a: width / bitmap.0,
                            b: 0.0,
                            c: 0.0,
                            d: -height / bitmap.1,
                            e: *x,
                            f: *y + height,
                        };
                        self.pixmap.draw_pixmap(
                            0,
                            0,
                            source.as_ref(),
                            &PixmapPaint {
                                quality: FilterQuality::Bilinear,
                                ..PixmapPaint::default()
                            },
                            tiny(composed.compose(fit)),
                            None,
                        );
                    }
                    None => {
                        self.skipped_images += 1;
                        self.placeholder_rect(*x, *y, *width, *height, composed);
                    }
                }
            }
            Primitive::Placeholder {
                x,
                y,
                width,
                height,
                reason,
                ..
            } => {
                self.placeholder_rect(*x, *y, *width, *height, outer);
                if !reason.is_empty() {
                    self.label(reason, *x, *y, outer);
                }
            }
            Primitive::Group {
                primitives,
                transform,
                ..
            } => {
                let composed = outer.compose(*transform);
                let mut children: Vec<&Primitive> = primitives.iter().collect();
                children.sort_by_key(|primitive| z_order(primitive));
                for child in children {
                    self.paint(child, composed);
                }
            }
        }
    }

    fn text(&mut self, fragment: &vsdx_render::TextFragment, outer: Affine) {
        if fragment.size_in <= 0.0 || !fragment.size_in.is_finite() {
            return;
        }
        let size_px = fragment.size_in * device_scale(outer);
        if !size_px.is_finite() || size_px <= 0.0 || size_px > MAX_GLYPH_PX {
            return;
        }
        let Some(color) = parse_color(&fragment.color) else {
            return;
        };
        let (anchor_x, anchor_y) = outer.apply_point(fragment.x, fragment.line_y);
        let baseline = anchor_y + size_px * 0.8;
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        let shear = if fragment.italic { ITALIC_SHEAR } else { 0.0 };
        let spacing = if fragment.letter_spacing.is_finite() {
            fragment.letter_spacing * device_scale(outer)
        } else {
            0.0
        };
        let bold_shift = fragment.bold.then(|| BOLD_OFFSET_PX * size_px / 12.0);
        for extra in [0.0].into_iter().chain(bold_shift) {
            self.glyphs(
                &fragment.text,
                anchor_x + extra,
                baseline,
                size_px,
                shear,
                spacing,
                &paint,
            );
        }
        if fragment.underline {
            let face = &*FACE;
            let scale = size_px / face.units_per_em() as f32;
            let width = glyph_advance(&fragment.text) * scale
                + spacing * fragment.text.chars().count().max(1) as f32;
            self.underline(width, anchor_x, baseline, scale, &paint);
        }
    }

    fn glyphs(
        &mut self,
        text: &str,
        anchor_x: f32,
        baseline: f32,
        size_px: f32,
        shear: f32,
        spacing: f32,
        paint: &Paint,
    ) {
        let face = &*FACE;
        let scale = size_px / face.units_per_em() as f32;
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        let shaped = rustybuzz::shape(face, &[], buffer);
        let mut pen = anchor_x;
        for (info, position) in shaped
            .glyph_infos()
            .iter()
            .zip(shaped.glyph_positions())
        {
            let tx = pen + position.x_offset as f32 * scale;
            let ty = baseline - position.y_offset as f32 * scale;
            if let Some(path) = glyph_path(face, info.glyph_id as u16) {
                self.pixmap.fill_path(
                    &path,
                    paint,
                    FillRule::Winding,
                    Transform::from_row(scale, 0.0, shear * scale, -scale, tx, ty),
                    None,
                );
            }
            pen += position.x_advance as f32 * scale + spacing;
        }
    }

    fn underline(&mut self, width: f32, x: f32, baseline: f32, scale: f32, paint: &Paint) {
        let face = &*FACE;
        let em = face.units_per_em() as f32;
        let metrics = face.underline_metrics();
        let position = metrics.map(|m| m.position as f32).unwrap_or(-0.1 * em);
        let thickness = metrics.map(|m| m.thickness as f32).unwrap_or(0.05 * em);
        let height = (thickness * scale).max(0.5);
        let cy = baseline - position * scale;
        if width > 0.0
            && let Some(rect) = Rect::from_xywh(x, cy - height / 2.0, width, height)
        {
            self.pixmap
                .fill_rect(rect, paint, Transform::identity(), None);
        }
    }

    fn label(&mut self, reason: &str, x: f32, y: f32, outer: Affine) {
        let (anchor_x, anchor_y) = outer.apply_point(x, y);
        let size_px = 10.0 / 72.0 * device_scale(outer);
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(0x5d, 0x66, 0x75, 255));
        paint.anti_alias = true;
        self.glyphs(reason, anchor_x, anchor_y, size_px, 0.0, 0.0, &paint);
    }

    fn placeholder_rect(&mut self, x: f32, y: f32, width: f32, height: f32, outer: Affine) {
        let (xa, ya) = outer.apply_point(x, y);
        let (xb, yb) = outer.apply_point(x + width, y + height);
        let (x0, x1) = (xa.min(xb), xa.max(xb));
        let (y0, y1) = (ya.min(yb), ya.max(yb));
        let Some(rect) = Rect::from_xywh(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)) else {
            return;
        };
        let unit = device_scale(outer).max(1.0);
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(0x8a, 0x94, 0xa6, 255));
        paint.anti_alias = true;
        let stroke = Stroke {
            width: unit,
            dash: StrokeDash::new(vec![5.0 * unit, 4.0 * unit], 0.0),
            ..Stroke::default()
        };
        self.pixmap.stroke_path(
            &PathBuilder::from_rect(rect),
            &paint,
            &stroke,
            Transform::identity(),
            None,
        );
    }

    fn decode(&mut self, asset_id: &str) -> Option<(Pixmap, (f32, f32))> {
        use image::ImageDecoder as _;
        let bytes = self.images.get(asset_id)?;
        let decoder = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .ok()?
            .into_decoder()
            .ok()?;
        let (declared_width, declared_height) = decoder.dimensions();
        let declared = u64::from(declared_width) * u64::from(declared_height);
        if declared == 0 || declared > MAX_IMAGE_PIXELS {
            return None;
        }
        let decoded = image::load_from_memory(bytes).ok()?.into_rgba8();
        let (width, height) = (decoded.width(), decoded.height());
        let size = IntSize::from_wh(width, height)?;
        let mut data = decoded.into_raw();
        for pixel in data.chunks_exact_mut(4) {
            let color = tiny_skia::ColorU8::from_rgba(pixel[0], pixel[1], pixel[2], pixel[3])
                .premultiply();
            pixel.copy_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
        }
        Pixmap::from_vec(data, size).map(|pixmap| (pixmap, (width as f32, height as f32)))
    }
}

fn device_scale(transform: Affine) -> f32 {
    transform.a.hypot(transform.b).max(0.0)
}

fn glyph_advance(text: &str) -> f32 {
    let face = &*FACE;
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    rustybuzz::shape(face, &[], buffer)
        .glyph_positions()
        .iter()
        .map(|position| position.x_advance as f32)
        .sum()
}

fn build_path(path: &[GeometryPathCommand]) -> Option<Path> {
    let mut builder = PathBuilder::new();
    for command in path {
        match command {
            GeometryPathCommand::Move { x, y } => builder.move_to(*x as f32, *y as f32),
            GeometryPathCommand::Line { x, y } => builder.line_to(*x as f32, *y as f32),
            GeometryPathCommand::Quad { cpx, cpy, x, y } => {
                builder.quad_to(*cpx as f32, *cpy as f32, *x as f32, *y as f32);
            }
            GeometryPathCommand::Cubic {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => builder.cubic_to(
                *cp1x as f32,
                *cp1y as f32,
                *cp2x as f32,
                *cp2y as f32,
                *x as f32,
                *y as f32,
            ),
            GeometryPathCommand::Close => builder.close(),
        }
    }
    builder.finish()
}

fn shape_bounds(path: &[GeometryPathCommand]) -> Option<(f32, f32, f32, f32)> {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut point = |x: f64, y: f64| {
        let (x, y) = (x as f32, y as f32);
        if x.is_finite() && y.is_finite() {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    };
    for command in path {
        match command {
            GeometryPathCommand::Move { x, y } | GeometryPathCommand::Line { x, y } => point(*x, *y),
            GeometryPathCommand::Quad { cpx, cpy, x, y } => {
                point(*cpx, *cpy);
                point(*x, *y);
            }
            GeometryPathCommand::Cubic {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                point(*cp1x, *cp1y);
                point(*cp2x, *cp2y);
                point(*x, *y);
            }
            GeometryPathCommand::Close => {}
        }
    }
    (min_x.is_finite() && min_y.is_finite()).then_some((min_x, min_y, max_x, max_y))
}

fn fill_paint(fill: &Option<VsPaint>, path: &[GeometryPathCommand]) -> Option<Paint<'static>> {
    match fill {
        Some(VsPaint::Solid { color }) => {
            let mut paint = Paint::default();
            paint.set_color(parse_color(color)?);
            paint.anti_alias = true;
            Some(paint)
        }
        Some(VsPaint::Gradient { angle_deg, stops }) => {
            let mut raw: Vec<(f32, Color)> = stops
                .iter()
                .filter_map(|stop| parse_color(&stop.color).map(|color| (stop.position, color)))
                .filter(|(position, _)| position.is_finite())
                .collect();
            if raw.is_empty() {
                return None;
            }
            raw.sort_by(|left, right| {
                left.0
                    .partial_cmp(&right.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let first = raw[0].1;
            if raw.len() == 1 {
                let mut paint = Paint::default();
                paint.set_color(first);
                paint.anti_alias = true;
                return Some(paint);
            }
            let (min_x, min_y, max_x, max_y) = shape_bounds(path)?;
            let center_x = (min_x + max_x) / 2.0;
            let center_y = (min_y + max_y) / 2.0;
            let radius = ((max_x - min_x).powi(2) + (max_y - min_y).powi(2)).sqrt() / 2.0;
            if !radius.is_finite() || radius <= 0.0 {
                let mut paint = Paint::default();
                paint.set_color(first);
                paint.anti_alias = true;
                return Some(paint);
            }
            let radians = angle_deg.unwrap_or(0.0).to_radians();
            let (sin, cos) = radians.sin_cos();
            let mut paint = Paint::default();
            paint.shader = LinearGradient::new(
                Point::from_xy(center_x - cos * radius, center_y - sin * radius),
                Point::from_xy(center_x + cos * radius, center_y + sin * radius),
                raw.into_iter()
                    .map(|(position, color)| GradientStop::new(position, color))
                    .collect(),
                SpreadMode::Pad,
                Transform::identity(),
            )?;
            paint.anti_alias = true;
            Some(paint)
        }
        None => None,
    }
}

fn stroke_paint(stroke: &Option<VsStroke>) -> Option<(Paint<'_>, Stroke)> {
    let stroke = stroke.as_ref()?;
    let mut paint = Paint::default();
    paint.set_color(parse_color(&stroke.color)?);
    paint.anti_alias = true;
    let width = f64::from(stroke.width).max(0.0) as f32;
    if !width.is_finite() {
        return None;
    }
    Some((
        paint,
        Stroke {
            width,
            dash: stroke
                .dashed
                .then(|| StrokeDash::new(vec![width * 2.0, width * 2.0], 0.0))
                .flatten(),
            ..Stroke::default()
        },
    ))
}

fn glyph_path(face: &ttf_parser::Face, glyph_id: u16) -> Option<Path> {
    let mut builder = PathCollector {
        builder: PathBuilder::new(),
    };
    face.outline_glyph(ttf_parser::GlyphId(glyph_id), &mut builder)?;
    builder.builder.finish()
}

struct PathCollector {
    builder: PathBuilder,
}

impl OutlineBuilder for PathCollector {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.builder.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.builder.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.builder.close();
    }
}

fn encode_png(pixmap: Pixmap, width: u32, height: u32) -> Result<Vec<u8>, String> {
    let pixels = pixmap.take_demultiplied();
    let mut data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut data, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(&pixels)
            .map_err(|error| error.to_string())?;
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vsdx_render::{PaintTransform, Primitive, TextParagraph};

    fn package(path: &str) -> VsdxPackage {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let source = std::fs::read(root.join("../vsdx-parse/tests/fixtures").join(path)).unwrap();
        vsdx_parse::parse_vsdx(&source).unwrap()
    }

    fn empty_list(width_in: f32, height_in: f32) -> VsdxDisplayList {
        let (width, height) = (
            width_in * vsdx_render::PIXELS_PER_INCH,
            height_in * vsdx_render::PIXELS_PER_INCH,
        );
        VsdxDisplayList {
            contract_version: 5,
            width,
            height,
            paint_transform: PaintTransform {
                a: 96.0,
                b: 0.0,
                c: 0.0,
                d: -96.0,
                e: 0.0,
                f: height,
            },
            primitives: Vec::new(),
        }
    }

    fn png_size(bytes: &[u8]) -> (u32, u32) {
        assert_eq!(&bytes[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert_eq!(&bytes[12..16], b"IHDR");
        (
            u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]),
            u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
        )
    }

    #[test]
    fn renders_white_pages_at_display_list_size_times_scale() {
        let images = HashMap::new();
        let first = render_list(&empty_list(8.5, 11.0), &images, 1.0).unwrap();
        assert_eq!((first.width, first.height), (816, 1056));
        assert_eq!(png_size(&first.bytes), (816, 1056));
        let second = render_list(&empty_list(8.5, 11.0), &images, 2.0).unwrap();
        assert_eq!((second.width, second.height), (1632, 2112));
        assert_eq!(png_size(&second.bytes), (1632, 2112));
    }

    #[test]
    fn rejects_bad_scales_and_pages() {
        let package = package("foundation.vsdx");
        let renderer = Renderer::default();
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(render_page(&renderer, &package, 0, scale).is_err());
        }
        assert!(render_page(&renderer, &package, 99, 1.0).is_err());
        assert!(render_page(&renderer, &package, 0, 64.0).is_err());
    }

    #[test]
    fn renders_are_deterministic() {
        let package = package("text-accounting.vsdx");
        let renderer = Renderer::default();
        let first = render_page(&renderer, &package, 0, 1.0).unwrap();
        let second = render_page(&renderer, &package, 0, 1.0).unwrap();
        assert_eq!(first.bytes, second.bytes);
    }

    #[test]
    fn dark_geometry_leaves_marks() {
        let mut list = empty_list(4.0, 4.0);
        list.primitives.push(Primitive::Shape {
            id: "rect".into(),
            z_order: 0,
            path: vec![
                GeometryPathCommand::Move { x: 0.0, y: 0.0 },
                GeometryPathCommand::Line { x: 2.0, y: 0.0 },
                GeometryPathCommand::Line { x: 2.0, y: 1.0 },
                GeometryPathCommand::Line { x: 0.0, y: 1.0 },
                GeometryPathCommand::Close,
            ],
            fill: Some(vsdx_render::Paint::Solid {
                color: "#111111".into(),
            }),
            stroke: Some(vsdx_render::Stroke {
                color: "#222222".into(),
                width: 0.02,
                dashed: false,
            }),
            transform: Affine::identity(),
            diagnostics: Vec::new(),
        });
        let page = render_list(&list, &HashMap::new(), 1.0).unwrap();
        let decoded = image::load_from_memory(&page.bytes).unwrap().into_rgba8();
        let marked = decoded
            .pixels()
            .filter(|pixel| pixel.0 != [255, 255, 255, 255])
            .count();
        assert!(marked > 100, "expected painted pixels, found {marked}");
    }

    #[test]
    fn text_fixture_leaves_marks() {
        let package = package("text-accounting.vsdx");
        let renderer = Renderer::default();
        let page = render_page(&renderer, &package, 0, 1.0).unwrap();
        let decoded = image::load_from_memory(&page.bytes).unwrap().into_rgba8();
        let marked = decoded
            .pixels()
            .filter(|pixel| pixel.0 != [255, 255, 255, 255])
            .count();
        assert!(marked > 100, "expected painted text, found {marked}");
    }

    #[test]
    fn counts_missing_images_without_failing() {
        let mut list = empty_list(4.0, 4.0);
        list.primitives.push(Primitive::Image {
            id: "missing".into(),
            z_order: 0,
            asset_id: "visio/media/gone.png".into(),
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            transform: Affine::identity(),
        });
        let page = render_list(&list, &HashMap::new(), 1.0).unwrap();
        assert_eq!(page.skipped_images, 1);
    }

    #[test]
    fn draws_a_decoded_image() {
        let mut data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut data, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[200u8, 30, 30, 255].repeat(4))
                .unwrap();
        }
        let mut list = empty_list(4.0, 4.0);
        list.primitives.push(Primitive::Image {
            id: "red".into(),
            z_order: 0,
            asset_id: "a".into(),
            x: 0.0,
            y: 0.0,
            width: 2.0,
            height: 2.0,
            transform: Affine::identity(),
        });
        let mut images: HashMap<&str, &[u8]> = HashMap::new();
        images.insert("a", &data);
        let page = render_list(&list, &images, 1.0).unwrap();
        assert_eq!(page.skipped_images, 0);
        let decoded = image::load_from_memory(&page.bytes).unwrap().into_rgba8();
        let red = decoded
            .pixels()
            .filter(|pixel| pixel[0] > 150 && pixel[1] < 100)
            .count();
        assert!(red > 100, "expected red pixels, found {red}");
    }

    #[test]
    fn paragraphs_without_runs_render_nothing() {
        let mut list = empty_list(4.0, 4.0);
        list.primitives.push(Primitive::TextBox {
            id: "empty".into(),
            z_order: 0,
            x: 0.0,
            y: 0.0,
            width: 2.0,
            height: 1.0,
            paragraphs: vec![TextParagraph { runs: Vec::new() }],
            lines: Vec::new(),
            transform: Affine::identity(),
        });
        let page = render_list(&list, &HashMap::new(), 1.0).unwrap();
        assert_eq!(page.skipped_images, 0);
    }
}
