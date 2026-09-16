use ooxml_drawingml::GeometryPathCommand;
use vsdx_render::{Affine, Paint, Primitive, Stroke, TextParagraph, TextRun};

pub const EMU_PER_INCH: f64 = 914400.0;
const EMU_PER_DEGREE: f64 = 60000.0;

pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn srgb(color: &str) -> Option<&str> {
    let hex = color.strip_prefix('#').unwrap_or(color);
    (hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(hex)
}

pub fn emu(inches: f64) -> i64 {
    (inches * EMU_PER_INCH).round() as i64
}

pub struct Placed {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub rot: i64,
    pub flip_h: bool,
    pub flip_v: bool,
}

pub fn place_rect(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    transform: Affine,
    page_height: f64,
) -> Option<Placed> {
    let corners = [
        transform.apply_point(x, y),
        transform.apply_point(x + width, y),
        transform.apply_point(x, y + height),
        transform.apply_point(x + width, y + height),
    ];
    if corners
        .iter()
        .any(|(px, py)| !px.is_finite() || !py.is_finite())
    {
        return None;
    }
    let min_x = corners
        .iter()
        .map(|corner| corner.0)
        .fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|corner| corner.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = corners
        .iter()
        .map(|corner| corner.1)
        .fold(f32::INFINITY, f32::min);
    let max_y = corners
        .iter()
        .map(|corner| corner.1)
        .fold(f32::NEG_INFINITY, f32::max);
    let (rotation, flip_h, flip_v) = affine_placement(transform)?;
    Some(Placed {
        x: emu(f64::from(min_x)),
        y: emu(page_height - f64::from(max_y)),
        w: emu(f64::from(max_x - min_x)).max(1),
        h: emu(f64::from(max_y - min_y)).max(1),
        rot: rotation,
        flip_h,
        flip_v,
    })
}

fn affine_placement(transform: Affine) -> Option<(i64, bool, bool)> {
    const EPSILON: f32 = 1e-6;
    if transform.b.abs() <= EPSILON && transform.c.abs() <= EPSILON {
        return Some((0, transform.a < -EPSILON, transform.d < -EPSILON));
    }
    let scale = (f64::from(transform.a) * f64::from(transform.a)
        + f64::from(transform.b) * f64::from(transform.b))
    .sqrt();
    if scale <= EPSILON as f64 {
        return None;
    }
    let cos = f64::from(transform.a) / scale;
    let sin = f64::from(transform.b) / scale;
    let matches = (f64::from(transform.c) + sin * scale).abs() <= f64::from(EPSILON) * scale
        && (f64::from(transform.d) - cos * scale).abs() <= f64::from(EPSILON) * scale;
    if !matches {
        return None;
    }
    Some((
        (-sin.atan2(cos).to_degrees() * EMU_PER_DEGREE).round() as i64,
        transform.a * transform.d - transform.b * transform.c < -EPSILON,
        false,
    ))
}

pub fn flat(primitives: &[Primitive]) -> Vec<Primitive> {
    let mut out = Vec::new();
    flat_with_transform(primitives, Affine::identity(), &mut out);
    out
}

fn flat_with_transform(primitives: &[Primitive], parent: Affine, out: &mut Vec<Primitive>) {
    let mut ordered: Vec<&Primitive> = primitives.iter().collect();
    ordered.sort_by_key(z_order);
    for primitive in ordered {
        let mut primitive = primitive.clone();
        match &mut primitive {
            Primitive::Group {
                primitives,
                transform,
                ..
            } => flat_with_transform(primitives, parent.compose(*transform), out),
            _ => {
                bake_transform(&mut primitive, parent);
                out.push(primitive);
            }
        }
    }
}

fn z_order(primitive: &&Primitive) -> u32 {
    match primitive {
        Primitive::Shape { z_order, .. }
        | Primitive::Image { z_order, .. }
        | Primitive::TextBox { z_order, .. }
        | Primitive::Placeholder { z_order, .. }
        | Primitive::Group { z_order, .. } => *z_order,
    }
}

fn bake_transform(primitive: &mut Primitive, matrix: Affine) {
    match primitive {
        Primitive::Shape {
            path, transform, ..
        } => {
            for command in path {
                transform_command(command, matrix);
            }
            *transform = Affine::identity();
        }
        Primitive::Image { transform, .. } | Primitive::TextBox { transform, .. } => {
            *transform = matrix.compose(*transform);
        }
        Primitive::Placeholder {
            x,
            y,
            width,
            height,
            ..
        } => transform_rect(x, y, width, height, matrix),
        Primitive::Group { .. } => {}
    }
}

fn transform_command(command: &mut GeometryPathCommand, matrix: Affine) {
    match command {
        GeometryPathCommand::Move { x, y } | GeometryPathCommand::Line { x, y } => {
            let (px, py) = matrix.apply_point(*x as f32, *y as f32);
            (*x, *y) = (f64::from(px), f64::from(py));
        }
        GeometryPathCommand::Quad { cpx, cpy, x, y } => {
            let (px, py) = matrix.apply_point(*cpx as f32, *cpy as f32);
            (*cpx, *cpy) = (f64::from(px), f64::from(py));
            let (px, py) = matrix.apply_point(*x as f32, *y as f32);
            (*x, *y) = (f64::from(px), f64::from(py));
        }
        GeometryPathCommand::Cubic {
            cp1x,
            cp1y,
            cp2x,
            cp2y,
            x,
            y,
        } => {
            let (px, py) = matrix.apply_point(*cp1x as f32, *cp1y as f32);
            (*cp1x, *cp1y) = (f64::from(px), f64::from(py));
            let (px, py) = matrix.apply_point(*cp2x as f32, *cp2y as f32);
            (*cp2x, *cp2y) = (f64::from(px), f64::from(py));
            let (px, py) = matrix.apply_point(*x as f32, *y as f32);
            (*x, *y) = (f64::from(px), f64::from(py));
        }
        GeometryPathCommand::Close => {}
    }
}

fn transform_rect(x: &mut f32, y: &mut f32, width: &mut f32, height: &mut f32, matrix: Affine) {
    let corners = [
        matrix.apply_point(*x, *y),
        matrix.apply_point(*x + *width, *y),
        matrix.apply_point(*x, *y + *height),
        matrix.apply_point(*x + *width, *y + *height),
    ];
    let min_x = corners
        .iter()
        .map(|(x, _)| *x)
        .fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|(x, _)| *x)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y = corners
        .iter()
        .map(|(_, y)| *y)
        .fold(f32::INFINITY, f32::min);
    let max_y = corners
        .iter()
        .map(|(_, y)| *y)
        .fold(f32::NEG_INFINITY, f32::max);
    *x = min_x;
    *y = min_y;
    *width = max_x - min_x;
    *height = max_y - min_y;
}

pub struct CustGeom {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub body: String,
}

pub fn cust_geom(path: &[GeometryPathCommand], page_height: f64) -> Option<CustGeom> {
    if path.is_empty() {
        return None;
    }
    let mut points: Vec<(f64, f64)> = Vec::new();
    for command in path {
        match command {
            GeometryPathCommand::Move { x, y } | GeometryPathCommand::Line { x, y } => {
                points.push((*x, page_height - *y));
            }
            GeometryPathCommand::Quad { cpx, cpy, x, y } => {
                points.push((*cpx, page_height - *cpy));
                points.push((*x, page_height - *y));
            }
            GeometryPathCommand::Cubic {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                points.push((*cp1x, page_height - *cp1y));
                points.push((*cp2x, page_height - *cp2y));
                points.push((*x, page_height - *y));
            }
            GeometryPathCommand::Close => {}
        }
    }
    if points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
        return None;
    }
    let min_x = points
        .iter()
        .map(|point| point.0)
        .fold(f64::INFINITY, f64::min);
    let max_x = points
        .iter()
        .map(|point| point.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = points
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let max_y = points
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let x = emu(min_x);
    let y = emu(min_y);
    let w = emu(max_x - min_x).max(1);
    let h = emu(max_y - min_y).max(1);
    let pt = |x: f64, y: f64| format!("<a:pt x=\"{}\" y=\"{}\"/>", emu(x - min_x), emu(y - min_y));
    let mut body = String::new();
    for command in path {
        match command {
            GeometryPathCommand::Move { x, y } => {
                body.push_str(&format!(
                    "<a:moveTo>{}</a:moveTo>",
                    pt(*x, page_height - *y)
                ));
            }
            GeometryPathCommand::Line { x, y } => {
                body.push_str(&format!("<a:lnTo>{}</a:lnTo>", pt(*x, page_height - *y)));
            }
            GeometryPathCommand::Quad { cpx, cpy, x, y } => {
                body.push_str(&format!(
                    "<a:quadBezTo>{}{}</a:quadBezTo>",
                    pt(*cpx, page_height - *cpy),
                    pt(*x, page_height - *y)
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
                body.push_str(&format!(
                    "<a:cubicBezTo>{}{}{}</a:cubicBezTo>",
                    pt(*cp1x, page_height - *cp1y),
                    pt(*cp2x, page_height - *cp2y),
                    pt(*x, page_height - *y)
                ));
            }
            GeometryPathCommand::Close => body.push_str("<a:close/>"),
        }
    }
    Some(CustGeom { x, y, w, h, body })
}

pub fn fill_xml(fill: &Option<Paint>) -> String {
    match fill {
        None => "<a:noFill/>".to_owned(),
        Some(Paint::Solid { color }) => match srgb(color) {
            Some(hex) => format!("<a:solidFill><a:srgbClr val=\"{hex}\"/></a:solidFill>"),
            None => "<a:noFill/>".to_owned(),
        },
        Some(Paint::Gradient { stops }) => {
            let mut list = String::new();
            for stop in stops {
                let Some(hex) = srgb(&stop.color) else {
                    continue;
                };
                let position = (f64::from(stop.position) * 1000.0)
                    .round()
                    .clamp(0.0, 100000.0);
                list.push_str(&format!(
                    "<a:gs pos=\"{position}\"><a:srgbClr val=\"{hex}\"/></a:gs>"
                ));
            }
            if list.is_empty() {
                return "<a:noFill/>".to_owned();
            }
            format!(
                "<a:gradFill><a:gsLst>{list}</a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill>"
            )
        }
    }
}

pub fn line_xml(stroke: &Option<Stroke>) -> Option<String> {
    let stroke = stroke.as_ref()?;
    let color = srgb(&stroke.color)?;
    let width = emu(f64::from(stroke.width)).max(1);
    let dash = if stroke.dashed {
        "<a:prstDash val=\"dash\"/>"
    } else {
        ""
    };
    Some(format!(
        "<a:ln w=\"{width}\"><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill>{dash}</a:ln>"
    ))
}

pub fn font_points(size_in: f32) -> i64 {
    (f64::from(size_in) * 72.0 * 100.0).round() as i64
}

fn run_props(run: &TextRun) -> String {
    let mut props = format!(" sz=\"{}\"", font_points(run.size_in).max(100));
    if run.bold {
        props.push_str(" b=\"1\"");
    }
    if run.italic {
        props.push_str(" i=\"1\"");
    }
    if run.underline {
        props.push_str(" u=\"sng\"");
    }
    if run.superscript {
        props.push_str(" baseline=\"30000\"");
    }
    if run.subscript {
        props.push_str(" baseline=\"-25000\"");
    }
    if run.small_caps {
        props.push_str(" cap=\"smCaps\"");
    }
    props
}

fn run_color(run: &TextRun) -> String {
    match srgb(&run.color) {
        Some(hex) => format!("<a:solidFill><a:srgbClr val=\"{hex}\"/></a:solidFill>"),
        None => String::new(),
    }
}

fn run_face(run: &TextRun) -> String {
    let family = escape(&run.family);
    format!("<a:latin typeface=\"{family}\"/>")
}

pub fn dml_paragraphs(paragraphs: &[TextParagraph]) -> String {
    let mut out = String::new();
    for paragraph in paragraphs {
        out.push_str("<a:p>");
        for run in &paragraph.runs {
            let text = escape(&run.text);
            let props = run_props(run);
            let color = run_color(run);
            let face = run_face(run);
            out.push_str(&format!(
                "<a:r><a:rPr{props}>{color}{face}</a:rPr><a:t>{text}</a:t></a:r>"
            ));
        }
        out.push_str("<a:endParaRPr/>");
        out.push_str("</a:p>");
    }
    out
}

pub fn has_text(paragraphs: &[TextParagraph]) -> bool {
    paragraphs
        .iter()
        .any(|paragraph| paragraph.runs.iter().any(|run| !run.text.is_empty()))
}

pub fn wml_runs(paragraphs: &[TextParagraph]) -> String {
    let mut out = String::new();
    for paragraph in paragraphs {
        out.push_str("<w:p>");
        for run in &paragraph.runs {
            out.push_str("<w:r><w:rPr>");
            let family = escape(&run.family);
            out.push_str(&format!(
                "<w:rFonts w:ascii=\"{family}\" w:hAnsi=\"{family}\" w:cs=\"{family}\"/>"
            ));
            if run.bold {
                out.push_str("<w:b/><w:bCs/>");
            }
            if run.italic {
                out.push_str("<w:i/><w:iCs/>");
            }
            if run.small_caps {
                out.push_str("<w:smallCaps/>");
            }
            if let Some(hex) = srgb(&run.color) {
                out.push_str(&format!("<w:color w:val=\"{hex}\"/>"));
            }
            let half = (f64::from(run.size_in) * 144.0).round() as i64;
            out.push_str(&format!(
                "<w:sz w:val=\"{half}\"/><w:szCs w:val=\"{half}\"/>",
                half = half.max(2)
            ));
            if run.underline {
                out.push_str("<w:u w:val=\"single\"/>");
            }
            if run.superscript {
                out.push_str("<w:vertAlign w:val=\"superscript\"/>");
            }
            if run.subscript {
                out.push_str("<w:vertAlign w:val=\"subscript\"/>");
            }
            out.push_str("</w:rPr>");
            let text = escape(&run.text);
            out.push_str(&format!("<w:t xml:space=\"preserve\">{text}</w:t></w:r>"));
        }
        out.push_str("</w:p>");
    }
    out
}

pub fn sniff_image(asset_id: &str, bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n']) {
        Some(("png", "image/png"))
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("jpg", "image/jpeg"))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(("gif", "image/gif"))
    } else if bytes.starts_with(b"BM") {
        Some(("bmp", "image/bmp"))
    } else if has_extension(asset_id, "png") {
        Some(("png", "image/png"))
    } else if has_extension(asset_id, "jpg") || has_extension(asset_id, "jpeg") {
        Some(("jpg", "image/jpeg"))
    } else if has_extension(asset_id, "emf") {
        Some(("emf", "image/x-emf"))
    } else if has_extension(asset_id, "wmf") {
        Some(("wmf", "image/x-wmf"))
    } else {
        None
    }
}

fn has_extension(asset_id: &str, extension: &str) -> bool {
    asset_id
        .rsplit_once('.')
        .is_some_and(|(_, actual)| actual.eq_ignore_ascii_case(extension))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_composes_group_transforms() {
        let primitives = vec![Primitive::Group {
            id: "group".into(),
            z_order: 0,
            transform: Affine {
                a: 0.0,
                b: 1.0,
                c: -1.0,
                d: 0.0,
                e: 4.0,
                f: 5.0,
            },
            primitives: vec![Primitive::TextBox {
                id: "text".into(),
                z_order: 0,
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0,
                paragraphs: Vec::new(),
                lines: Vec::new(),
                transform: Affine::identity(),
            }],
        }];
        let flattened = flat(&primitives);
        let [Primitive::TextBox { transform, .. }] = flattened.as_slice() else {
            panic!("text expected");
        };
        assert_eq!(transform.apply_point(1.0, 2.0), (2.0, 6.0));
    }

    #[test]
    fn reflected_rect_emits_flip_flag() {
        let placed = place_rect(
            0.0,
            0.0,
            1.0,
            1.0,
            Affine {
                a: -1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: 1.0,
                f: 0.0,
            },
            1.0,
        )
        .unwrap();
        assert!(placed.flip_h);
        assert!(!placed.flip_v);
    }

    #[test]
    fn emf_uses_renderable_content_type() {
        assert_eq!(
            sniff_image("visio/media/image.emf", &[]),
            Some(("emf", "image/x-emf"))
        );
    }

    #[test]
    fn wmf_uses_renderable_content_type() {
        assert_eq!(
            sniff_image("visio/media/image.WMF", &[]),
            Some(("wmf", "image/x-wmf"))
        );
    }
}
