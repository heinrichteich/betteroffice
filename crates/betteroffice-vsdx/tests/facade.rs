use betteroffice_vsdx::{CellLocator, CellSheet, Diagram, MutationGesture, SemanticCellEdit};
use ooxml_opc::{rezip_parts, unzip_parts};

fn diagram_with_page(xml: &str) -> (Vec<u8>, Diagram, u32) {
    let source = include_bytes!("../../vsdx-parse/tests/fixtures/foundation.vsdx");
    let package = vsdx_parse::parse_vsdx(source).unwrap();
    let path = package.page_part_paths[0].clone();
    let page_id = package.page_part_ids[&path];
    let mut parts = unzip_parts(source).unwrap();
    parts
        .iter_mut()
        .find(|(candidate, _)| candidate == &path)
        .unwrap()
        .1 = xml.as_bytes().to_vec();
    let source = rezip_parts(&parts).unwrap();
    let diagram = Diagram::open(&source).unwrap();
    (source, diagram, page_id)
}

fn edit(
    page_id: u32,
    shape_id: u32,
    name: &str,
    formula: &str,
    gesture: MutationGesture,
) -> SemanticCellEdit {
    SemanticCellEdit {
        locator: CellLocator {
            sheet: CellSheet::Page(page_id),
            shape_id: Some(shape_id),
            section: None,
            row: None,
            cell_name: name.to_owned(),
        },
        gesture,
        formula: Some(formula.to_owned()),
        value: None,
    }
}

#[test]
fn opens_pages_and_inspects_shapes() {
    let diagram = Diagram::open(include_bytes!(
        "../../vsdx-parse/tests/fixtures/foundation.vsdx"
    ))
    .unwrap();
    let page = diagram.pages().next().unwrap();
    let shape = page.shapes().next().unwrap();
    assert!(!shape.resolved().unwrap().cells.is_empty());
}

#[test]
fn guarded_edit_refuses_without_writing() {
    let (_, diagram, page_id) = diagram_with_page(
        "<PageContents><Shapes><Shape ID='1'><Cell N='Width' F='GUARD(1)' V='1'/></Shape></Shapes></PageContents>",
    );
    let page_path = diagram.package().page_part_paths[0].clone();
    let before = diagram.package().part_bytes(&page_path).unwrap().to_vec();
    assert!(
        diagram
            .save_cell_edits(&[edit(page_id, 1, "Width", "2", MutationGesture::ResizeWidth)])
            .is_err()
    );
    assert_eq!(diagram.package().part_bytes(&page_path).unwrap(), before);
}

#[test]
fn setatref_redirects_to_the_referenced_cell() {
    let (_, diagram, page_id) = diagram_with_page(
        "<PageContents><Shapes><Shape ID='1'><Cell N='Width' F='SETATREF(Target)' V='1'/><Cell N='Target' V='1'/></Shape></Shapes></PageContents>",
    );
    let saved = diagram
        .save_cell_edits(&[edit(page_id, 1, "Width", "7", MutationGesture::ResizeWidth)])
        .unwrap();
    let reopened = Diagram::open(&saved).unwrap();
    let page = reopened.pages().next().unwrap();
    let shape = page.shapes().next().unwrap();
    assert_eq!(
        shape
            .model()
            .cells()
            .find(|cell| cell.name == "Width")
            .unwrap()
            .formula
            .as_deref(),
        Some("SETATREF(Target)")
    );
    let target = shape
        .model()
        .cells()
        .find(|cell| cell.name == "Target")
        .unwrap();
    assert_eq!(target.formula.as_deref(), Some("7"));
    assert_eq!(target.value.as_deref(), Some("7"));
}

#[test]
fn setatref_resolves_a_sheet_reference() {
    let (_, diagram, page_id) = diagram_with_page(
        "<PageContents><Shapes><Shape ID='1'><Cell N='Width' F='SETATREF(Sheet.2!Target)' V='1'/></Shape><Shape ID='2'><Cell N='Target' V='3'/></Shape></Shapes></PageContents>",
    );
    let saved = diagram
        .save_cell_edits(&[edit(page_id, 1, "Width", "9", MutationGesture::ResizeWidth)])
        .unwrap();
    let reopened = Diagram::open(&saved).unwrap();
    let page = reopened.pages().next().unwrap();
    let target = page.shapes().find(|shape| shape.model().id == 2).unwrap();
    assert_eq!(
        target.model().cells().next().unwrap().formula.as_deref(),
        Some("9")
    );
}

#[test]
fn locks_refuse_only_their_matching_gesture() {
    let (_, diagram, page_id) = diagram_with_page(
        "<PageContents><Shapes><Shape ID='1'><Cell N='LockWidth' V='1'/><Cell N='LockMoveX' V='1'/><Cell N='Width' V='1'/><Cell N='PinX' V='1'/><Cell N='PinY' V='1'/></Shape></Shapes></PageContents>",
    );
    assert!(
        diagram
            .save_cell_edits(&[edit(page_id, 1, "Width", "2", MutationGesture::ResizeWidth)])
            .is_err()
    );
    assert!(
        diagram
            .save_cell_edits(&[edit(page_id, 1, "PinX", "2", MutationGesture::MoveX)])
            .is_err()
    );
    assert!(
        diagram
            .save_cell_edits(&[edit(page_id, 1, "PinY", "2", MutationGesture::MoveY)])
            .is_ok()
    );
}

#[test]
fn a_refused_edit_aborts_the_entire_set() {
    let (_, diagram, page_id) = diagram_with_page(
        "<PageContents><Shapes><Shape ID='1'><Cell N='Width' V='1'/><Cell N='Height' F='GUARD(1)' V='1'/></Shape></Shapes></PageContents>",
    );
    let page_path = diagram.package().page_part_paths[0].clone();
    let before = diagram.package().part_bytes(&page_path).unwrap().to_vec();
    assert!(
        diagram
            .save_cell_edits(&[
                edit(page_id, 1, "Width", "2", MutationGesture::ResizeWidth),
                edit(page_id, 1, "Height", "2", MutationGesture::ResizeHeight),
            ])
            .is_err()
    );
    assert_eq!(diagram.package().part_bytes(&page_path).unwrap(), before);
}

#[test]
fn writing_an_inherited_cell_creates_a_local_formula_override() {
    let (_, diagram, page_id) = diagram_with_page(
        "<PageContents><Shapes><Shape ID='1'><Cell N='Width' F='Inh' V='1'/></Shape></Shapes></PageContents>",
    );
    let saved = diagram
        .save_cell_edits(&[edit(page_id, 1, "Width", "4", MutationGesture::ResizeWidth)])
        .unwrap();
    let reopened = Diagram::open(&saved).unwrap();
    let page = reopened.pages().next().unwrap();
    let shape = page.shapes().next().unwrap();
    let cell = shape.resolved().unwrap().cells["Width"].clone();
    assert!(
        matches!(cell, vsdx_resolve::Lookup::Found(ref cell) if cell.provenance == vsdx_resolve::Provenance::Local && cell.cell.formula.as_deref() == Some("4"))
    );
}

#[test]
fn saves_a_semantic_cell_edit_without_source_spans() {
    let source = include_bytes!("../../vsdx-parse/tests/fixtures/foundation.vsdx");
    let diagram = Diagram::open(source).unwrap();
    let page_id = *diagram.package().page_part_ids.values().next().unwrap();
    let saved = diagram
        .save_cell_edits(&[SemanticCellEdit {
            locator: CellLocator {
                sheet: CellSheet::Page(page_id),
                shape_id: Some(1),
                section: None,
                row: None,
                cell_name: "Both".to_owned(),
            },
            gesture: MutationGesture::CellEdit,
            formula: Some("42".to_owned()),
            value: None,
        }])
        .unwrap();
    let saved = Diagram::open(&saved).unwrap();
    let page = saved.pages().next().unwrap();
    assert!(page.shapes().any(|shape| {
        shape.model().cells().any(|cell| {
            cell.name == "Both"
                && cell.formula.as_deref() == Some("42")
                && cell.value.as_deref() == Some("42")
        })
    }));
}
