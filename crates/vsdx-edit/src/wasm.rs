use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use yrs::Subscription;

use crate::{
    DiagramSession, DiagramSnapshot, EditCtx, MAX_SAFE_CLIENT_ID, ShapeDraft, UpdateEvent,
    UpdateOrigin,
};
use vsdx_parse::{CellLocator, CellRow, CellSheet};

#[wasm_bindgen]
pub struct VsdxDocument {
    session: DiagramSession,
    update_observer: Option<UpdateObserver>,
}

struct UpdateObserver {
    pending: Arc<Mutex<VecDeque<UpdateEvent>>>,
    _subscription: Subscription,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CellLocatorArgs {
    section: Option<String>,
    row_index: Option<u32>,
    row_name: Option<String>,
    cell_name: String,
}

impl TryFrom<CellLocatorArgs> for CellLocator {
    type Error = &'static str;

    fn try_from(value: CellLocatorArgs) -> Result<Self, Self::Error> {
        let row = match (value.row_index, value.row_name) {
            (Some(_), Some(_)) => {
                return Err("cell locator cannot contain both rowIndex and rowName");
            }
            (Some(index), None) => Some(CellRow::Index(index)),
            (None, Some(name)) => Some(CellRow::Name(name)),
            (None, None) => None,
        };
        Ok(Self {
            sheet: CellSheet::Page(0),
            shape_id: None,
            section: value.section,
            row,
            cell_name: value.cell_name,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetCellFormulaArgs {
    page_id: String,
    shape_id: String,
    locator: CellLocatorArgs,
    formula: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveShapeArgs {
    page_id: String,
    shape_id: String,
    x_formula: String,
    y_formula: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResizeShapeArgs {
    page_id: String,
    shape_id: String,
    width_formula: String,
    height_formula: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReorderShapeArgs {
    page_id: String,
    shape_id: String,
    to_index: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReorderPageArgs {
    page_id: String,
    to_index: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddShapeArgs {
    page_id: String,
    draft: ShapeDraft,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryResult {
    applied: bool,
    snapshot: DiagramSnapshot,
}

#[wasm_bindgen]
impl VsdxDocument {
    #[wasm_bindgen(js_name = openCollaborative)]
    pub fn open_collaborative(bytes: &[u8], client_id: f64) -> Result<VsdxDocument, JsValue> {
        DiagramSession::open(bytes, parse_client_id(client_id)?)
            .map(|session| Self {
                session,
                update_observer: None,
            })
            .map_err(js_error)
    }

    #[wasm_bindgen(js_name = openCollaborativeFromUpdate)]
    pub fn open_collaborative_from_update(
        update: &[u8],
        client_id: f64,
    ) -> Result<VsdxDocument, JsValue> {
        DiagramSession::open_from_update(update, parse_client_id(client_id)?)
            .map(|session| Self {
                session,
                update_observer: None,
            })
            .map_err(js_error)
    }

    #[wasm_bindgen(getter, js_name = clientId)]
    pub fn client_id(&self) -> f64 {
        self.session.client_id() as f64
    }

    #[wasm_bindgen(js_name = snapshotJson)]
    pub fn snapshot_json(&self) -> Result<String, JsValue> {
        json(self.session.snapshot().map_err(js_error)?)
    }

    #[wasm_bindgen(js_name = encodeStateVector)]
    pub fn encode_state_vector(&self) -> Vec<u8> {
        self.session.encode_state_vector_v1()
    }

    #[wasm_bindgen(js_name = encodeStateAsUpdate)]
    pub fn encode_state_as_update(&self) -> Vec<u8> {
        self.session.encode_state_as_update_v1()
    }

    #[wasm_bindgen(js_name = encodeDiff)]
    pub fn encode_diff(&self, remote_state_vector: &[u8]) -> Result<Vec<u8>, JsValue> {
        self.session
            .encode_diff_v1(remote_state_vector)
            .map_err(js_error)
    }

    #[wasm_bindgen(js_name = applyUpdateJson)]
    pub fn apply_update_json(&self, update: &[u8]) -> Result<String, JsValue> {
        json(self.session.apply_update_v1(update).map_err(js_error)?)
    }

    #[wasm_bindgen(js_name = startUpdateObservation)]
    pub fn start_update_observation(&mut self) -> Result<(), JsValue> {
        if self.update_observer.is_some() {
            return Ok(());
        }
        let pending = Arc::new(Mutex::new(VecDeque::new()));
        let observed = Arc::clone(&pending);
        let subscription = self
            .session
            .observe_update_v1(move |event| {
                observed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push_back(event);
            })
            .map_err(js_error)?;
        self.update_observer = Some(UpdateObserver {
            pending,
            _subscription: subscription,
        });
        Ok(())
    }

    #[wasm_bindgen(js_name = clearUpdateObservation)]
    pub fn clear_update_observation(&mut self) {
        self.update_observer = None;
    }

    #[wasm_bindgen(js_name = drainUpdateEvent)]
    pub fn drain_update_event(&self) -> Vec<u8> {
        let Some(observer) = &self.update_observer else {
            return Vec::new();
        };
        let Some(event) = observer
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
        else {
            return Vec::new();
        };
        let mut encoded = Vec::with_capacity(event.update.len() + 1);
        encoded.push(match event.origin {
            UpdateOrigin::Local => 0,
            UpdateOrigin::Remote => 1,
        });
        encoded.extend_from_slice(&event.update);
        encoded
    }

    #[wasm_bindgen(js_name = setCellFormulaJson)]
    pub fn set_cell_formula_json(&self, args: &str) -> Result<String, JsValue> {
        let args: SetCellFormulaArgs = parse_args(args)?;
        let locator = args.locator.try_into().map_err(JsValue::from_str)?;
        json(
            self.session
                .set_cell_formula_at(
                    &local_context(),
                    &args.page_id,
                    &args.shape_id,
                    locator,
                    args.formula,
                )
                .map_err(js_error)?,
        )
    }

    #[wasm_bindgen(js_name = moveShapeJson)]
    pub fn move_shape_json(&self, args: &str) -> Result<String, JsValue> {
        let args: MoveShapeArgs = parse_args(args)?;
        json(
            self.session
                .move_shape(
                    &local_context(),
                    &args.page_id,
                    &args.shape_id,
                    args.x_formula,
                    args.y_formula,
                )
                .map_err(js_error)?,
        )
    }

    #[wasm_bindgen(js_name = resizeShapeJson)]
    pub fn resize_shape_json(&self, args: &str) -> Result<String, JsValue> {
        let args: ResizeShapeArgs = parse_args(args)?;
        json(
            self.session
                .resize_shape(
                    &local_context(),
                    &args.page_id,
                    &args.shape_id,
                    args.width_formula,
                    args.height_formula,
                )
                .map_err(js_error)?,
        )
    }

    #[wasm_bindgen(js_name = reorderShapeJson)]
    pub fn reorder_shape_json(&self, args: &str) -> Result<String, JsValue> {
        let args: ReorderShapeArgs = parse_args(args)?;
        json(
            self.session
                .reorder_shape(
                    &local_context(),
                    &args.page_id,
                    &args.shape_id,
                    args.to_index,
                )
                .map_err(js_error)?,
        )
    }

    #[wasm_bindgen(js_name = reorderPageJson)]
    pub fn reorder_page_json(&self, args: &str) -> Result<String, JsValue> {
        let args: ReorderPageArgs = parse_args(args)?;
        json(
            self.session
                .reorder_page(&local_context(), &args.page_id, args.to_index)
                .map_err(js_error)?,
        )
    }

    #[wasm_bindgen(js_name = addShapeJson)]
    pub fn add_shape_json(&self, args: &str) -> Result<String, JsValue> {
        let args: AddShapeArgs = parse_args(args)?;
        json(
            self.session
                .add_shape(&local_context(), &args.page_id, &args.draft)
                .map_err(js_error)?,
        )
    }

    #[wasm_bindgen(js_name = undoJson)]
    pub fn undo_json(&self) -> Result<String, JsValue> {
        json(HistoryResult {
            applied: self.session.undo(),
            snapshot: self.session.snapshot().map_err(js_error)?,
        })
    }

    #[wasm_bindgen(js_name = redoJson)]
    pub fn redo_json(&self) -> Result<String, JsValue> {
        json(HistoryResult {
            applied: self.session.redo(),
            snapshot: self.session.snapshot().map_err(js_error)?,
        })
    }

    #[wasm_bindgen(js_name = canUndo)]
    pub fn can_undo(&self) -> bool {
        self.session.can_undo()
    }

    #[wasm_bindgen(js_name = canRedo)]
    pub fn can_redo(&self) -> bool {
        self.session.can_redo()
    }

    pub fn version() -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }
}

impl VsdxDocument {
    pub fn session(&self) -> &DiagramSession {
        &self.session
    }
}

fn local_context() -> EditCtx {
    EditCtx::local("wasm")
}

fn parse_args<T: serde::de::DeserializeOwned>(args: &str) -> Result<T, JsValue> {
    serde_json::from_str(args).map_err(js_error)
}

fn json(value: impl Serialize) -> Result<String, JsValue> {
    serde_json::to_string(&value).map_err(js_error)
}

fn parse_client_id(client_id: f64) -> Result<u64, JsValue> {
    parse_client_id_raw(client_id).map_err(JsValue::from_str)
}

fn parse_client_id_raw(client_id: f64) -> Result<u64, &'static str> {
    if !client_id.is_finite()
        || client_id.fract() != 0.0
        || client_id < 1.0
        || client_id > MAX_SAFE_CLIENT_ID as f64
    {
        return Err("client ID must be a positive safe integer below Number.MAX_SAFE_INTEGER");
    }
    Ok(client_id as u64)
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_client_id_raw;
    use crate::MAX_SAFE_CLIENT_ID;

    #[test]
    fn client_ids_must_be_positive_safe_integers() {
        for client_id in [-1.0, 1.5, (MAX_SAFE_CLIENT_ID + 1) as f64] {
            assert!(parse_client_id_raw(client_id).is_err());
        }
    }
}
