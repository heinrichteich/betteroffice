//! VSDX display-list wasm boundary.

use wasm_bindgen::prelude::*;

pub use vsdx_edit::wasm::VsdxDocument;

#[wasm_bindgen]
pub struct VsdxRenderer {
    renderer: vsdx_render::Renderer,
    rendered: Option<vsdx_render::VsdxDisplayList>,
    font_count: u32,
}

#[wasm_bindgen]
impl VsdxRenderer {
    #[wasm_bindgen(constructor)]
    pub fn new() -> VsdxRenderer {
        Self {
            renderer: vsdx_render::Renderer::default(),
            rendered: None,
            font_count: 0,
        }
    }

    #[wasm_bindgen(js_name = registerFont)]
    pub fn register_font(
        &mut self,
        family: &str,
        bold: bool,
        italic: bool,
        bytes: &[u8],
    ) -> Result<u32, JsValue> {
        let handle = self.font_count;
        let next_font_count = self
            .font_count
            .checked_add(1)
            .ok_or_else(|| JsValue::from_str("font handle limit exceeded"))?;
        self.renderer
            .register_font(family, bold, italic, bytes.to_vec())
            .map_err(js_error)?;
        self.font_count = next_font_count;
        Ok(handle)
    }

    #[wasm_bindgen(js_name = layoutPageJson)]
    pub fn layout_page_json(
        &mut self,
        document: &VsdxDocument,
        page_index: u32,
    ) -> Result<String, JsValue> {
        let package = document.session().package().map_err(js_error)?;
        let page_part = package
            .page_part_paths
            .get(page_index as usize)
            .ok_or_else(|| JsValue::from_str("page index is outside the document"))?;
        let rendered = self
            .renderer
            .layout_page(&package, page_part)
            .map_err(js_error)?;
        let json = serde_json::to_string(&rendered).map_err(js_error)?;
        self.rendered = Some(rendered);
        Ok(json)
    }

    #[wasm_bindgen(js_name = hitTestJson)]
    pub fn hit_test_json(&self, x: f32, y: f32) -> Result<String, JsValue> {
        let result = self
            .rendered
            .as_ref()
            .and_then(|rendered| vsdx_render::hit_test(rendered, x, y));
        let result = match result {
            Some(vsdx_render::HitTestResult::Shape { shape_id }) => {
                serde_json::json!({ "kind": "shape", "shapeId": shape_id })
            }
            Some(vsdx_render::HitTestResult::Text { shape_id, position }) => {
                serde_json::json!({ "kind": "text", "shapeId": shape_id, "position": position })
            }
            None => serde_json::Value::Null,
        };
        serde_json::to_string(&result).map_err(js_error)
    }
}

impl Default for VsdxRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen(js_name = parseVsdxJson)]
pub fn parse_vsdx_json(data: &[u8]) -> Result<String, JsValue> {
    let package = vsdx_parse::parse_vsdx(data).map_err(js_error)?;
    serde_json::to_string(&package).map_err(js_error)
}

#[wasm_bindgen(js_name = rendererVersion)]
pub fn renderer_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{VsdxDocument, VsdxRenderer, parse_vsdx_json};
    use vsdx_edit::EditCtx;
    use yrs::{Map, MapPrelim, Out, ReadTxn, Transact};

    fn add_formula(document: &VsdxDocument, name: &str, formula: &str) {
        let mut txn = document.session().yrs_doc().transact_mut();
        let sheets = txn.get_map("vsdx:sheets").unwrap();
        let shape = match sheets.get(&txn, "page:1:shape:1") {
            Some(Out::YMap(shape)) => shape,
            _ => unreachable!(),
        };
        let cells = match shape.get(&txn, "cells") {
            Some(Out::YMap(cells)) => cells,
            _ => unreachable!(),
        };
        let cell = cells.insert(&mut txn, name, MapPrelim::default());
        cell.insert(&mut txn, "name", name);
        cell.insert(&mut txn, "formula", formula);
    }

    #[test]
    fn parse_vsdx_json_round_trips_a_fixture() {
        let json = parse_vsdx_json(include_bytes!(
            "../../vsdx-parse/tests/fixtures/foundation.vsdx"
        ))
        .unwrap();
        assert!(json.contains("pagePartPaths"));
    }

    #[test]
    fn layout_page_json_uses_the_current_collaborative_state() {
        let document = VsdxDocument::open_collaborative(
            include_bytes!("../../vsdx-parse/tests/fixtures/foundation.vsdx"),
            1.0,
        )
        .unwrap();
        add_formula(&document, "PinX", "1");
        add_formula(&document, "PinY", "1");
        add_formula(&document, "Width", "1");
        add_formula(&document, "Height", "1");
        let mut renderer = VsdxRenderer::new();
        let before = renderer.layout_page_json(&document, 0).unwrap();
        document
            .session()
            .set_cell_formula(
                &EditCtx::local("test"),
                "page:1",
                "page:1:shape:1",
                "Width",
                "10",
            )
            .unwrap();
        let after = renderer.layout_page_json(&document, 0).unwrap();
        assert_ne!(before, after);
    }
}
