use super::*;
use vsdx_parse::{ThemeEffectColor, ThemeEffectStyle, ThemeEffects, ThemeOuterShadow};

fn shadow_shape(id: u32, cells: Vec<Cell>) -> Shape {
    let mut shape = shape(id, 1.0, 1.0);
    shape
        .children
        .extend(cells.into_iter().map(ShapeChild::Cell));
    shape
}

fn guarded(name: &str, formula: &str, value: &str) -> Cell {
    Cell {
        name: name.into(),
        formula: Some(formula.into()),
        value: Some(value.into()),
        unit: None,
        del: false,
        other_attrs: vec![],
    }
}

fn shape_shadow(list: &VsdxDisplayList) -> Option<Shadow> {
    match list.primitives.as_slice() {
        [Primitive::Shape { shadow, .. }] => shadow.clone(),
        _ => panic!("expected a single shape primitive"),
    }
}

fn group_shape(id: u32, cells: Vec<Cell>, children: Vec<Shape>) -> Shape {
    let mut group = shape(id, 5.0, 5.0);
    group.shape_type = Some("Group".into());
    group
        .children
        .extend(cells.into_iter().map(ShapeChild::Cell));
    group.children.push(ShapeChild::Shapes(
        children.into_iter().map(ShapesChild::Shape).collect(),
    ));
    group
}

fn group_shadow(list: &VsdxDisplayList, child: &str) -> Option<Shadow> {
    let Primitive::Group { primitives, .. } = &list.primitives[0] else {
        panic!("expected a group primitive");
    };
    match primitives.as_slice() {
        [Primitive::Shape { id, shadow, .. }] if id.ends_with(child) => shadow.clone(),
        _ => panic!("expected a single grouped shape"),
    }
}

fn explicit_cells() -> Vec<Cell> {
    vec![
        cell("ShapeShdwShow", "2"),
        cell("ShapeShdwType", "1"),
        cell("ShapeShdwOffsetX", "0.125"),
        cell("ShapeShdwOffsetY", "-0.125"),
        cell("ShapeShdwBlur", "36"),
        cell("ShdwForegnd", "#112233"),
        cell("ShdwForegndTrans", "0.5"),
    ]
}

#[test]
fn explicit_shadow_cells_reach_the_display_list() {
    let list = render(vec![shadow_shape(1, explicit_cells())]);
    let shadow = shape_shadow(&list).expect("shadow");
    assert_eq!(shadow.color, "#11223380");
    assert!((shadow.blur_in - 0.5).abs() < 1e-6);
    assert!((shadow.offset_x_in - 0.125).abs() < 1e-9);
    assert!((shadow.offset_y_in + 0.125).abs() < 1e-9);
}

#[test]
fn guarded_shadow_offsets_evaluate_through_formulas() {
    let list = render(vec![shadow_shape(
        1,
        vec![
            cell("ShapeShdwShow", "2"),
            cell("ShapeShdwType", "1"),
            guarded("ShapeShdwOffsetX", "GUARD(0.125)", "0.125"),
            guarded("ShapeShdwOffsetY", "GUARD(-0.125)", "-0.125"),
            guarded("ShdwForegnd", "THEMEGUARD(RGB(17,34,51))", "#112233"),
        ],
    )]);
    let shadow = shape_shadow(&list).expect("shadow");
    assert_eq!(shadow.color, "#112233");
    assert!((shadow.offset_x_in - 0.125).abs() < 1e-9);
}

#[test]
fn default_shapes_cast_no_shadow() {
    let list = render(vec![shape(1, 1.0, 1.0)]);
    assert_eq!(shape_shadow(&list), None);
}

#[test]
fn opted_in_shapes_fall_back_to_page_offsets() {
    let mut package = package(vec![shadow_shape(1, vec![cell("ShapeShdwShow", "1")])]);
    let sheet = package.page_sheets.get_mut(&1).unwrap();
    sheet.children.extend([
        SheetChild::Cell(cell("ShdwOffsetX", "0.125")),
        SheetChild::Cell(cell("ShdwOffsetY", "-0.125")),
    ]);
    let list = Renderer::default().layout_page(&package, "page").unwrap();
    let shadow = shape_shadow(&list).expect("shadow");
    assert!((shadow.offset_x_in - 0.125).abs() < 1e-9);
    assert!((shadow.offset_y_in + 0.125).abs() < 1e-9);
}

#[test]
fn zero_offsets_with_no_blur_cast_no_shadow() {
    let list = render(vec![shadow_shape(
        1,
        vec![
            cell("ShapeShdwShow", "2"),
            cell("ShapeShdwType", "1"),
            cell("ShapeShdwOffsetX", "0"),
            cell("ShapeShdwOffsetY", "0"),
            cell("ShdwForegnd", "#112233"),
        ],
    )]);
    assert_eq!(shape_shadow(&list), None);
}

#[test]
fn grouped_shadows_follow_show_cells() {
    let child = || {
        shadow_shape(
            2,
            vec![
                cell("ShapeShdwType", "1"),
                cell("ShapeShdwOffsetX", "0.125"),
                cell("ShapeShdwOffsetY", "-0.125"),
                cell("ShdwForegnd", "#112233"),
            ],
        )
    };
    let mut suppressed = child();
    suppressed
        .children
        .push(ShapeChild::Cell(cell("ShapeShdwShow", "2")));
    let list = render(vec![group_shape(1, vec![], vec![suppressed])]);
    assert_eq!(group_shadow(&list, ":2"), None);
    let mut kept = child();
    kept.children
        .push(ShapeChild::Cell(cell("ShapeShdwShow", "2")));
    let list = render(vec![group_shape(
        1,
        vec![cell("ShapeShdwShow", "2")],
        vec![kept],
    )]);
    group_shadow(&list, ":2").expect("shadow");
    let mut invited = child();
    invited
        .children
        .push(ShapeChild::Cell(cell("ShapeShdwShow", "0")));
    let list = render(vec![group_shape(
        1,
        vec![cell("ShapeShdwShow", "1")],
        vec![invited],
    )]);
    group_shadow(&list, ":2").expect("shadow");
}

#[test]
fn oblique_shadows_fall_back_to_simple_offsets() {
    let list = render(vec![shadow_shape(
        1,
        vec![
            cell("ShapeShdwShow", "2"),
            cell("ShapeShdwType", "2"),
            cell("ShapeShdwOffsetX", "0.125"),
            cell("ShapeShdwOffsetY", "-0.125"),
            cell("ShdwForegnd", "#112233"),
        ],
    )]);
    let shadow = shape_shadow(&list).expect("shadow");
    assert!((shadow.offset_x_in - 0.125).abs() < 1e-9);
    match &list.primitives[0] {
        Primitive::Shape { diagnostics, .. } => assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "unsupported-shadow-oblique")
        ),
        _ => unreachable!(),
    }
}

#[test]
fn legacy_pattern_shadows_use_page_offsets() {
    let mut package = package(vec![shadow_shape(
        1,
        vec![cell("ShdwPattern", "1"), cell("ShdwForegnd", "#445566")],
    )]);
    let sheet = package.page_sheets.get_mut(&1).unwrap();
    sheet.children.extend([
        SheetChild::Cell(cell("ShdwOffsetX", "0.125")),
        SheetChild::Cell(cell("ShdwOffsetY", "-0.125")),
    ]);
    let list = Renderer::default().layout_page(&package, "page").unwrap();
    let shadow = shape_shadow(&list).expect("shadow");
    assert_eq!(shadow.color, "#445566");
    assert!((shadow.offset_x_in - 0.125).abs() < 1e-9);
}

fn themed_package(shapes: Vec<Shape>) -> VsdxPackage {
    let mut package = package(shapes);
    package.theme_effects.insert(
        1,
        ThemeEffects {
            scheme_id: Some(34),
            effect_styles: vec![
                ThemeEffectStyle::default(),
                ThemeEffectStyle {
                    outer_shadow: Some(ThemeOuterShadow {
                        blur_emu: 0,
                        dist_emu: 914400,
                        direction_60k: 0,
                        color: ThemeEffectColor::Srgb("FF0000".into()),
                        alpha_1000pct: Some(50000),
                    }),
                    has_bevel: false,
                },
            ],
            variation_schemes: vec![],
            variation_colors: vec![],
        },
    );
    package
}

#[test]
fn themed_shadows_resolve_through_effect_styles() {
    let mut package = themed_package(vec![shadow_shape(
        1,
        vec![
            cell("ShapeShdwShow", "2"),
            cell("QuickStyleEffectsMatrix", "2"),
        ],
    )]);
    package.themes.insert(1, ooxml_drawingml::Theme::default());
    let list = Renderer::default().layout_page(&package, "page").unwrap();
    let shadow = shape_shadow(&list).expect("shadow");
    assert_eq!(shadow.color, "#FF000080");
    assert!((shadow.offset_x_in - 1.0).abs() < 1e-6);
    assert!(shadow.offset_y_in.abs() < 1e-6);
}

#[test]
fn empty_theme_effects_cast_no_shadow() {
    let package = themed_package(vec![shadow_shape(1, vec![cell("ShapeShdwShow", "0")])]);
    let list = Renderer::default().layout_page(&package, "page").unwrap();
    assert_eq!(shape_shadow(&list), None);
}
