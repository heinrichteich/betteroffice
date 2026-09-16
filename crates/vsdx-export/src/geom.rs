use ooxml_drawingml::GeometryPathCommand;
use vsdx_render::{Affine, Paint, Stroke, TextParagraph, TextRun};

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
    let rotation = affine_rotation(transform)?;
    Some(Placed {
        x: emu(f64::from(min_x)),
        y: emu(page_height - f64::from(max_y)),
        w: emu(f64::from(max_x - min_x)).max(1),
        h: emu(f64::from(max_y - min_y)).max(1),
        rot: rotation,
    })
}

fn affine_rotation(transform: Affine) -> Option<i64> {
    const EPSILON: f32 = 1e-6;
    if transform.b.abs() <= EPSILON && transform.c.abs() <= EPSILON {
        return Some(0);
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
    Some((-sin.atan2(cos).to_degrees() * EMU_PER_DEGREE).round() as i64)
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

pub fn sniff_image(asset_id: &str, bytes: &[u8]) -> (&'static str, &'static str) {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n']) {
        ("png", "image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ("jpg", "image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        ("gif", "image/gif")
    } else if bytes.starts_with(b"BM") {
        ("bmp", "image/bmp")
    } else if asset_id.ends_with(".png") {
        ("png", "image/png")
    } else if asset_id.ends_with(".jpg") || asset_id.ends_with(".jpeg") {
        ("jpg", "image/jpeg")
    } else {
        ("bin", "application/octet-stream")
    }
}
