use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use yrs::Subscription;

use crate::{
    CellSnapshot, DiagramSession, DiagramSnapshot, EditCtx, MAX_SAFE_CLIENT_ID, ShapeDraft,
    UpdateEvent, UpdateOrigin,
};
use vsdx_parse::{CellLocator, CellRow, CellSheet};

#[wasm_bindgen]
pub struct VsdxDocument {
    session: DiagramSession,
    update_observer: Option<UpdateObserver>,
}

struct UpdateObserver {
    pending: Arc<Mutex<PendingUpdates>>,
    _subscription: Subscription,
}

struct PendingUpdates {
    events: VecDeque<UpdateEvent>,
    resync_required: bool,
}

const MAX_PENDING_UPDATE_EVENTS: usize = 1024;

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
    draft: FormulaShapeDraft,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FormulaShapeDraft {
    source_id: u32,
    name: Option<String>,
    cells: Vec<serde_json::Value>,
}

impl TryFrom<FormulaShapeDraft> for ShapeDraft {
    type Error = &'static str;

    fn try_from(value: FormulaShapeDraft) -> Result<Self, Self::Error> {
        let mut cells = Vec::with_capacity(value.cells.len());
        for cell in value.cells {
            if cell.get("value").is_some() {
                return Err("shape draft cells must not contain value");
            }
            cells.push(
                serde_json::from_value::<CellSnapshot>(cell)
                    .map_err(|_| "invalid shape draft cell")?,
            );
        }
        Ok(Self {
            source_id: value.source_id,
            name: value.name,
            cells,
        })
    }
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

    #[wasm_bindgen(js_name = mediaBytes)]
    pub fn media_bytes(&self, part_path: &str) -> Result<Vec<u8>, JsValue> {
        self.media_bytes_result(part_path).map_err(js_error)
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
        json(self.apply_update(update).map_err(js_error)?)
    }

    #[wasm_bindgen(js_name = startUpdateObservation)]
    pub fn start_update_observation(&mut self) -> Result<(), JsValue> {
        if self.update_observer.is_some() {
            return Ok(());
        }
        let pending = Arc::new(Mutex::new(PendingUpdates {
            events: VecDeque::new(),
            resync_required: false,
        }));
        let observed = Arc::clone(&pending);
        let subscription = self
            .session
            .observe_update_v1(move |event| {
                let mut pending = observed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if pending.events.len() == MAX_PENDING_UPDATE_EVENTS {
                    pending.events.clear();
                    pending.resync_required = true;
                }
                if !pending.resync_required {
                    pending.events.push_back(event);
                }
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
        let mut pending = observer
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pending.resync_required {
            pending.resync_required = false;
            return vec![2];
        }
        let Some(event) = pending.events.pop_front() else {
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
        json(self.set_cell_formula(args).map_err(js_error)?)
    }

    #[wasm_bindgen(js_name = moveShapeJson)]
    pub fn move_shape_json(&self, args: &str) -> Result<String, JsValue> {
        let args: MoveShapeArgs = parse_args(args)?;
        json(self.move_shape(args).map_err(js_error)?)
    }

    #[wasm_bindgen(js_name = resizeShapeJson)]
    pub fn resize_shape_json(&self, args: &str) -> Result<String, JsValue> {
        let args: ResizeShapeArgs = parse_args(args)?;
        json(self.resize_shape(args).map_err(js_error)?)
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
        let draft = args.draft.try_into().map_err(JsValue::from_str)?;
        json(
            self.session
                .add_shape(&local_context(), &args.page_id, &draft)
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

    fn apply_update(&self, update: &[u8]) -> crate::EditResult<DiagramSnapshot> {
        self.session.apply_update_v1(update)
    }

    fn media_bytes_result(&self, part_path: &str) -> crate::EditResult<Vec<u8>> {
        self.session
            .package()?
            .part_bytes(part_path)
            .map(ToOwned::to_owned)
            .ok_or_else(|| crate::EditError::InvalidState("media part was not found".to_owned()))
    }

    fn set_cell_formula(
        &self,
        args: SetCellFormulaArgs,
    ) -> crate::EditResult<crate::CellFormulaReceipt> {
        let locator = CellLocator::try_from(args.locator)
            .map_err(|error| crate::EditError::InvalidState(error.to_owned()))?;
        self.session.set_cell_formula_at(
            &local_context(),
            &args.page_id,
            &args.shape_id,
            locator,
            args.formula,
        )
    }

    fn move_shape(&self, args: MoveShapeArgs) -> crate::EditResult<[crate::CellFormulaReceipt; 2]> {
        self.session.move_shape(
            &local_context(),
            &args.page_id,
            &args.shape_id,
            args.x_formula,
            args.y_formula,
        )
    }

    fn resize_shape(
        &self,
        args: ResizeShapeArgs,
    ) -> crate::EditResult<[crate::CellFormulaReceipt; 2]> {
        self.session.resize_shape(
            &local_context(),
            &args.page_id,
            &args.shape_id,
            args.width_formula,
            args.height_formula,
        )
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
    use super::{
        CellLocatorArgs, MoveShapeArgs, ResizeShapeArgs, SetCellFormulaArgs, VsdxDocument,
        parse_client_id_raw,
    };
    use crate::{DiagramSession, MAX_SAFE_CLIENT_ID, SHEETS};
    use yrs::{Map, MapPrelim, Out, ReadTxn, Transact};

    fn document() -> VsdxDocument {
        VsdxDocument::open_collaborative(
            include_bytes!("../../vsdx-parse/tests/fixtures/foundation.vsdx"),
            1.0,
        )
        .unwrap()
    }

    fn add_cell(document: &VsdxDocument, key: &str, name: &str, formula: &str) {
        let mut txn = document.session().yrs_doc().transact_mut();
        let sheets = txn.get_map(SHEETS).unwrap();
        let shape = match sheets.get(&txn, "page:1:shape:1") {
            Some(Out::YMap(shape)) => shape,
            _ => unreachable!(),
        };
        let cells = match shape.get(&txn, "cells") {
            Some(Out::YMap(cells)) => cells,
            _ => unreachable!(),
        };
        let cell = cells.insert(&mut txn, key, MapPrelim::default());
        cell.insert(&mut txn, "name", name);
        cell.insert(&mut txn, "formula", formula);
        if name == "X" {
            cell.insert(&mut txn, "section", "Geometry");
            cell.insert(&mut txn, "rowIndex", 0.0);
        }
    }

    #[test]
    fn client_ids_must_be_positive_safe_integers() {
        for client_id in [-1.0, 1.5, (MAX_SAFE_CLIENT_ID + 1) as f64] {
            assert!(parse_client_id_raw(client_id).is_err());
        }
    }

    #[test]
    fn wasm_refuses_guarded_section_row_cell_edits() {
        let document = document();
        add_cell(&document, "Geometry\u{1f}IX:0\u{1f}X", "X", "GUARD(1)");
        let result = document.set_cell_formula(SetCellFormulaArgs {
            page_id: "page:1".to_owned(),
            shape_id: "page:1:shape:1".to_owned(),
            locator: CellLocatorArgs {
                section: Some("Geometry".to_owned()),
                row_index: Some(0),
                row_name: None,
                cell_name: "X".to_owned(),
            },
            formula: "2".to_owned(),
        });
        assert!(result.is_err());
    }

    #[test]
    fn wasm_locks_refuse_atomic_move_and_resize() {
        let document = document();
        add_cell(&document, "PinX", "PinX", "1");
        add_cell(&document, "PinY", "PinY", "1");
        add_cell(&document, "LockMoveY", "LockMoveY", "1");
        assert!(
            document
                .move_shape(MoveShapeArgs {
                    page_id: "page:1".to_owned(),
                    shape_id: "page:1:shape:1".to_owned(),
                    x_formula: "2".to_owned(),
                    y_formula: "3".to_owned(),
                })
                .is_err()
        );
        let snapshot = document.snapshot_json().unwrap();
        assert!(snapshot.contains(r#""name":"PinX","formula":"1""#));
        assert!(snapshot.contains(r#""name":"PinY","formula":"1""#));

        add_cell(&document, "Width", "Width", "1");
        add_cell(&document, "Height", "Height", "1");
        add_cell(&document, "LockHeight", "LockHeight", "1");
        assert!(
            document
                .resize_shape(ResizeShapeArgs {
                    page_id: "page:1".to_owned(),
                    shape_id: "page:1:shape:1".to_owned(),
                    width_formula: "2".to_owned(),
                    height_formula: "3".to_owned(),
                })
                .is_err()
        );
        let snapshot = document.snapshot_json().unwrap();
        assert!(snapshot.contains(r#""name":"Width","formula":"1""#));
        assert!(snapshot.contains(r#""name":"Height","formula":"1""#));
    }

    #[test]
    fn wasm_setatref_redirects_and_reports_the_target() {
        let document = document();
        add_cell(&document, "Width", "Width", "SETATREF(Target)");
        add_cell(&document, "Target", "Target", "1");
        let receipt = document
            .set_cell_formula(SetCellFormulaArgs {
                page_id: "page:1".to_owned(),
                shape_id: "page:1:shape:1".to_owned(),
                locator: CellLocatorArgs {
                    section: None,
                    row_index: None,
                    row_name: None,
                    cell_name: "Width".to_owned(),
                },
                formula: "2".to_owned(),
            })
            .unwrap();
        assert_eq!(receipt.cell_name, "Target");
        let snapshot = document.snapshot_json().unwrap();
        assert!(snapshot.contains(r#""name":"Width","formula":"SETATREF(Target)""#));
        assert!(snapshot.contains(r#""name":"Target","formula":"2""#));
    }

    #[test]
    fn wasm_rejects_malicious_setatref_updates_without_changing_the_document() {
        let document = document();
        add_cell(&document, "Width", "Width", "SETATREF(Target)");
        let before = document.encode_state_as_update();
        let attacker = DiagramSession::open_from_update(&before, 2).unwrap();
        let mut txn = attacker.yrs_doc().transact_mut();
        let sheets = txn.get_map(SHEETS).unwrap();
        let shape = match sheets.get(&txn, "page:1:shape:1") {
            Some(Out::YMap(shape)) => shape,
            _ => unreachable!(),
        };
        let cells = match shape.get(&txn, "cells") {
            Some(Out::YMap(cells)) => cells,
            _ => unreachable!(),
        };
        let width = match cells.get(&txn, "Width") {
            Some(Out::YMap(cell)) => cell,
            _ => unreachable!(),
        };
        width.insert(&mut txn, "formula", "2");
        drop(txn);
        let update = attacker
            .encode_diff_v1(&document.encode_state_vector())
            .unwrap();
        assert!(document.apply_update(&update).is_err());
        assert_eq!(before, document.encode_state_as_update());
    }

    #[test]
    fn wasm_media_bytes_returns_known_part_and_rejects_unknown_part() {
        let document = document();
        let path = document.session().package().unwrap().document_part_path;
        assert!(!document.media_bytes_result(&path).unwrap().is_empty());
        assert!(
            document
                .media_bytes_result("visio/media/missing.png")
                .is_err()
        );
    }
}
