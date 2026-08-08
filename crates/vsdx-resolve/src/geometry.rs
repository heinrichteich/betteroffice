use ooxml_drawingml::GeometryPathCommand;
use vsdx_parse::Cell;

use crate::{GeometryIssue, RealizedGeometry, ResolvedRow, ResolvedSection};

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

#[cfg(test)]
mod tests {
    use crate::*;

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
