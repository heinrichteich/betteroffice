use base64::Engine as _;
use ooxml_drawingml::GeometryPathCommand;
use vsdx_parse::VsdxPackage;

use crate::display_list::{Affine, Primitive, VsdxDisplayList};
use crate::vector::{
    collect_ordered, escape, fragments, jpeg_dimensions, matrix, num, png_dimensions,
    solid_color, z_order,
};
use crate::{RenderError, Renderer};

struct Emitter<'a> {
    package: &'a VsdxPackage,
    out: String,
}

fn path_data(path: &[GeometryPathCommand]) -> String {
    let mut data = String::new();
    for command in path {
        match command {
            GeometryPathCommand::Move { x, y } => {
                data.push_str(&format!("M{} {} ", num(*x), num(*y)));
            }
            GeometryPathCommand::Line { x, y } => {
                data.push_str(&format!("L{} {} ", num(*x), num(*y)));
            }
            GeometryPathCommand::Quad { cpx, cpy, x, y } => {
                data.push_str(&format!(
                    "Q{} {} {} {} ",
                    num(*cpx),
                    num(*cpy),
                    num(*x),
                    num(*y)
                ));
            }
            GeometryPathCommand::Cubic {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                data.push_str(&format!(
                    "C{} {} {} {} {} {} ",
                    num(*cp1x),
                    num(*cp1y),
                    num(*cp2x),
                    num(*cp2y),
                    num(*x),
                    num(*y)
                ));
            }
            GeometryPathCommand::Close => data.push_str("Z "),
        }
    }
    data.trim_end().to_owned()
}

fn paint_attributes(
    fill: &Option<crate::display_list::Paint>,
    stroke: &Option<crate::display_list::Stroke>,
) -> String {
    let mut attributes = String::new();
    match solid_color(fill) {
        Some(color) => attributes.push_str(&format!(" fill=\"{}\"", escape(color))),
        None => attributes.push_str(" fill=\"none\""),
    }
    if let Some(stroke) = stroke {
        if crate::vector::rgb(&stroke.color).is_some() {
            attributes.push_str(&format!(
                " stroke=\"{}\" stroke-width=\"{}",
                escape(&stroke.color),
                num(f64::from(stroke.width).max(0.0))
            ));
            if stroke.dashed {
                let step = num(f64::from(stroke.width).max(0.0) * 2.0);
                attributes.push_str(&format!("\" stroke-dasharray=\"{step} {step}"));
            }
            attributes.push('"');
        }
    }
    attributes
}

fn translate(x: f32, y: f32) -> Affine {
    Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: x,
        f: y,
    }
}

const FLIP_Y: Affine = Affine {
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: -1.0,
    e: 0.0,
    f: 0.0,
};

impl<'a> Emitter<'a> {
    fn primitive(&mut self, primitive: &Primitive, outer: Affine) {
        let mut ordered = Vec::new();
        collect_ordered(primitive, &mut ordered);
        for (item, transform) in ordered {
            let composed = outer.compose(transform);
            match item {
                Primitive::Shape {
                    path, fill, stroke, ..
                } => {
                    let data = path_data(path);
                    if data.is_empty() {
                        continue;
                    }
                    let mut element = String::from("<path d=\"");
                    element.push_str(&escape(&data));
                    element.push('"');
                    element.push_str(&paint_attributes(fill, stroke));
                    element.push_str("/>");
                    self.wrapped(&element, composed);
                }
                Primitive::TextBox {
                    paragraphs, lines, ..
                } => {
                    for run in fragments(paragraphs, lines) {
                        let anchor = composed
                            .compose(translate(run.x, run.line_y))
                            .compose(FLIP_Y);
                        self.out.push_str(&format!(
                            "<text transform=\"{}\" x=\"0\" y=\"0\" dominant-baseline=\"text-before-edge\" font-family=\"'{}', {}\" font-size=\"{}\" fill=\"{}\"",
                            matrix(anchor),
                            escape(&run.family),
                            run.generic,
                            num(f64::from(run.size_in)),
                            escape(&run.color)
                        ));
                        if run.bold {
                            self.out.push_str(" font-weight=\"bold\"");
                        }
                        if run.italic {
                            self.out.push_str(" font-style=\"italic\"");
                        }
                        if run.underline {
                            self.out.push_str(" text-decoration=\"underline\"");
                        }
                        if run.letter_spacing != 0.0 && run.letter_spacing.is_finite() {
                            self.out.push_str(&format!(
                                " letter-spacing=\"{}\"",
                                num(f64::from(run.letter_spacing))
                            ));
                        }
                        self.out.push('>');
                        self.out.push_str(&escape(&run.text));
                        self.out.push_str("</text>");
                    }
                }
                Primitive::Image {
                    asset_id,
                    x,
                    y,
                    width,
                    height,
                    ..
                } => self.image(asset_id, *x, *y, *width, *height, composed),
                Primitive::Placeholder {
                    x,
                    y,
                    width,
                    height,
                    reason,
                    ..
                } => self.placeholder(*x, *y, *width, *height, reason, composed),
                Primitive::Group { .. } => {}
            }
        }
    }

    fn wrapped(&mut self, element: &str, transform: Affine) {
        if transform.is_identity() {
            self.out.push_str(element);
            return;
        }
        self.out.push_str(&format!(
            "<g transform=\"{}\">{element}</g>",
            matrix(transform)
        ));
    }

    fn image(
        &mut self,
        asset_id: &str,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        transform: Affine,
    ) {
        let uri = self.package.part_bytes(asset_id).and_then(|bytes| {
            if jpeg_dimensions(bytes).is_some() {
                Some((bytes, "image/jpeg"))
            } else if png_dimensions(bytes).is_some() {
                Some((bytes, "image/png"))
            } else {
                None
            }
        });
        let Some((bytes, mime)) = uri else {
            self.placeholder(x, y, width, height, "", transform);
            return;
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let anchor = transform
            .compose(translate(0.0, y * 2.0 + height))
            .compose(FLIP_Y);
        let element = format!(
            "<image transform=\"{}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"data:{mime};base64,{encoded}\"/>",
            matrix(anchor),
            num(f64::from(x)),
            num(f64::from(y)),
            num(f64::from(width).max(0.0)),
            num(f64::from(height).max(0.0))
        );
        self.out.push_str(&element);
    }

    fn placeholder(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        reason: &str,
        transform: Affine,
    ) {
        let mut element = format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"#8a94a6\" stroke-width=\"{}\" stroke-dasharray=\"{} {}\"/>",
            num(f64::from(x)),
            num(f64::from(y)),
            num(f64::from(width).max(0.0)),
            num(f64::from(height).max(0.0)),
            num(1.0 / 96.0),
            num(5.0 / 96.0),
            num(4.0 / 96.0)
        );
        if !reason.is_empty() {
            let anchor = transform.compose(translate(x, y)).compose(FLIP_Y);
            element.push_str(&format!(
                "<text transform=\"{}\" x=\"0\" y=\"0\" dominant-baseline=\"text-before-edge\" font-family=\"'sans-serif'\" font-size=\"{}\" fill=\"#5d6675\">{}</text>",
                matrix(anchor),
                num(10.0 / 72.0),
                escape(reason)
            ));
        }
        self.wrapped(&element, transform);
    }
}

fn emit_page(list: &VsdxDisplayList, package: &VsdxPackage) -> String {
    let mut primitives: Vec<&Primitive> = list.primitives.iter().collect();
    primitives.sort_by_key(|primitive| z_order(primitive));
    let mut emitter = Emitter {
        package,
        out: String::new(),
    };
    for primitive in primitives {
        emitter.primitive(primitive, Affine::identity());
    }
    let paint = list.paint_transform;
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"><g transform=\"{transform}\">{body}</g></svg>",
        width = num(f64::from(list.width)),
        height = num(f64::from(list.height)),
        transform = matrix(Affine {
            a: paint.a,
            b: paint.b,
            c: paint.c,
            d: paint.d,
            e: paint.e,
            f: paint.f,
        }),
        body = emitter.out
    )
}

impl Renderer {
    /// Renders every diagram page to one SVG string per page, sized from the PageSheet.
    pub fn export_svg(&self, package: &VsdxPackage) -> Result<Vec<String>, RenderError> {
        let mut pages = Vec::with_capacity(package.page_part_paths.len().max(1));
        for part in &package.page_part_paths.clone() {
            pages.push(emit_page(&self.layout_page(package, part)?, package));
        }
        Ok(pages)
    }

    /// Renders one diagram page to an SVG string sized from the PageSheet.
    pub fn export_svg_page(
        &self,
        package: &VsdxPackage,
        page_index: usize,
    ) -> Result<String, RenderError> {
        let part = package
            .page_part_paths
            .get(page_index)
            .ok_or_else(|| RenderError::MissingPage(format!("page index {page_index}")))?;
        Ok(emit_page(&self.layout_page(package, part)?, package))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package() -> VsdxPackage {
        let source = include_bytes!("../../vsdx-parse/tests/fixtures/foundation.vsdx");
        vsdx_parse::parse_vsdx(source).unwrap()
    }

    #[test]
    fn exports_one_svg_per_diagram_page() {
        let package = package();
        let pages = Renderer::default().export_svg(&package).unwrap();
        assert_eq!(pages.len(), package.page_part_paths.len());
        assert!(!pages.is_empty());
        for page in &pages {
            assert!(page.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
            assert!(page.ends_with("</svg>"));
        }
    }

    #[test]
    fn sizes_pages_from_the_display_list_rather_than_a_hardcoded_format() {
        let package = package();
        let list = Renderer::default()
            .layout_page(&package, &package.page_part_paths[0])
            .unwrap();
        let expected = format!(
            "width=\"{}\" height=\"{}\" viewBox=\"0 0 {0} {1}\"",
            num(f64::from(list.width)),
            num(f64::from(list.height))
        );
        let pages = Renderer::default().export_svg(&package).unwrap();
        assert!(pages[0].contains(&expected), "missing {expected}");
        assert!(!pages[0].contains("viewBox=\"0 0 595") && !pages[0].contains("viewBox=\"0 0 842"));
    }

    #[test]
    fn keeps_text_selectable_as_text_elements() {
        let source = include_bytes!("../../vsdx-parse/tests/fixtures/text-accounting.vsdx");
        let package = vsdx_parse::parse_vsdx(source).unwrap();
        let pages = Renderer::default().export_svg(&package).unwrap();
        let body = pages.join("");
        assert!(body.contains("<text") && body.contains("</text>"));
        assert!(!body.contains("<text ") || body.contains("font-size="));
    }

    #[test]
    fn emits_shape_geometry_as_path_data() {
        let source = include_bytes!("../../vsdx-parse/tests/fixtures/indexed-geometry.vsdx");
        let package = vsdx_parse::parse_vsdx(source).unwrap();
        let pages = Renderer::default().export_svg(&package).unwrap();
        let body = pages.join("");
        assert!(body.contains("<path d=\"M"));
    }

    #[test]
    fn references_fonts_by_name_without_embedding() {
        let source = include_bytes!("../../vsdx-parse/tests/fixtures/text-accounting.vsdx");
        let package = vsdx_parse::parse_vsdx(source).unwrap();
        let pages = Renderer::default().export_svg(&package).unwrap();
        let body = pages.join("");
        assert!(body.contains("font-family="));
        assert!(!body.contains("@font-face") && !body.contains("data:font"));
    }

    #[test]
    fn groups_balance() {
        let source = include_bytes!("../../vsdx-parse/tests/fixtures/text-accounting.vsdx");
        let package = vsdx_parse::parse_vsdx(source).unwrap();
        let pages = Renderer::default().export_svg(&package).unwrap();
        let body = pages.join("");
        assert_eq!(
            body.match_indices("<g ").count(),
            body.match_indices("</g>").count()
        );
    }

    #[test]
    fn rejects_an_unknown_page_index() {
        let package = package();
        assert!(Renderer::default().export_svg_page(&package, 99).is_err());
    }
}
