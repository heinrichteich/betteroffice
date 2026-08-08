use crate::display_list::{Paint, Stroke};
use vsdx_resolve::{Lookup, ResolvedShape};

pub fn paint(shape: &ResolvedShape) -> (Option<Paint>, Option<Stroke>) {
    let fill = value(shape, "FillPattern")
        .filter(|v| *v != "0")
        .and_then(|_| colour(shape, "FillForegnd"))
        .map(|color| Paint::Solid { color });
    let stroke = value(shape, "LinePattern")
        .filter(|v| *v != "0")
        .and_then(|_| colour(shape, "LineColor"))
        .map(|color| Stroke {
            color,
            width: number(shape, "LineWeight").unwrap_or(0.01) as f32,
            dashed: value(shape, "LinePattern").is_some_and(|v| v != "1"),
        });
    (fill, stroke)
}
pub fn number(shape: &ResolvedShape, name: &str) -> Option<f64> {
    value(shape, name)?
        .parse()
        .ok()
        .filter(|n: &f64| n.is_finite())
}
fn value<'a>(shape: &'a ResolvedShape, name: &str) -> Option<&'a str> {
    match shape.cell(name)? {
        Lookup::Found(cell) => cell.cell.value.as_deref(),
        Lookup::Deleted | Lookup::Absent => None,
    }
}
fn colour(shape: &ResolvedShape, name: &str) -> Option<String> {
    let value = value(shape, name)?;
    if value.starts_with('#') {
        return Some(value.to_owned());
    }
    let value = value.trim_start_matches("RGB(").trim_end_matches(')');
    let channels = value
        .split(',')
        .map(str::trim)
        .map(str::parse::<u8>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (channels.len() == 3)
        .then(|| format!("#{:02X}{:02X}{:02X}", channels[0], channels[1], channels[2]))
}
