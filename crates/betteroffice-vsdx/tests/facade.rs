use betteroffice_vsdx::{CellLocator, CellSheet, Diagram, SemanticCellEdit};

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
            formula: None,
            value: Some("42".to_owned()),
        }])
        .unwrap();
    let saved = Diagram::open(&saved).unwrap();
    let page = saved.pages().next().unwrap();
    assert!(page.shapes().any(|shape| {
        shape
            .model()
            .cells()
            .any(|cell| cell.name == "Both" && cell.value.as_deref() == Some("42"))
    }));
}
