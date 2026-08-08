use std::sync::Arc;

use yrs::{
    Any, Array, ArrayPrelim, ArrayRef, Doc, Map, MapPrelim, MapRef, Out, ReadTxn, Transact,
    TransactionMut, WriteTxn,
};

use crate::{
    CellFormulaReceipt, CellSnapshot, DiagramSession, DiagramSnapshot, EditCtx, EditError,
    EditResult, META, PAGE_ORDER, PAGES, PageSnapshot, SHEETS, STORIES, ShapeDraft, ShapeReceipt,
    ShapeSnapshot,
};

const SCHEMA_VERSION: f64 = 1.0;

pub(crate) fn seed_doc(
    doc: &Doc,
    package: &vsdx_parse::VsdxPackage,
    fingerprint: &str,
) -> EditResult<()> {
    let package_json =
        serde_json::to_vec(package).map_err(|error| EditError::Json(error.to_string()))?;
    let mut txn = doc.transact_mut_with("vsdx:bootstrap");
    let meta = txn.get_or_insert_map(META);
    meta.insert(&mut txn, "schemaVersion", SCHEMA_VERSION);
    meta.insert(&mut txn, "fingerprint", fingerprint);
    meta.insert(
        &mut txn,
        "packageJson",
        Any::Buffer(Arc::from(package_json)),
    );
    meta.insert(&mut txn, "pageWidth", 0.0);
    meta.insert(&mut txn, "pageHeight", 0.0);
    let order = txn.get_or_insert_array(PAGE_ORDER);
    let pages = txn.get_or_insert_map(PAGES);
    let sheets = txn.get_or_insert_map(SHEETS);
    txn.get_or_insert_map(STORIES);
    for path in &package.page_part_paths {
        let Some(page_id) = package.page_part_ids.get(path) else {
            continue;
        };
        let id = format!("page:{page_id}");
        order.push_back(&mut txn, id.as_str());
        let page = pages.insert(&mut txn, id.as_str(), MapPrelim::default());
        page.insert(&mut txn, "id", id.as_str());
        page.insert(&mut txn, "sourcePartPath", path.as_str());
        let shape_order = page.insert(&mut txn, "shapes", ArrayPrelim::default());
        if let Some(sheet) = package.page_contents.get(path) {
            for shape in sheet.shapes() {
                let shape_id = format!("{id}:shape:{}", shape.id);
                shape_order.push_back(&mut txn, shape_id.as_str());
                seed_shape(&sheets, &mut txn, &shape_id, shape)?;
            }
        }
    }
    Ok(())
}

fn seed_shape(
    sheets: &MapRef,
    txn: &mut TransactionMut<'_>,
    id: &str,
    shape: &vsdx_parse::Shape,
) -> EditResult<()> {
    let map = sheets.insert(txn, id, MapPrelim::default());
    map.insert(txn, "id", id);
    map.insert(txn, "sourceId", shape.id as f64);
    if let Some(name) = &shape.name {
        map.insert(txn, "name", name.as_str());
    }
    let cells = map.insert(txn, "cells", MapPrelim::default());
    for cell in shape.cells() {
        seed_cell(
            &cells,
            txn,
            &cell.name,
            cell.formula.as_deref(),
            cell.value.as_deref(),
        );
    }
    Ok(())
}

fn seed_cell(
    cells: &MapRef,
    txn: &mut TransactionMut<'_>,
    name: &str,
    formula: Option<&str>,
    value: Option<&str>,
) {
    let cell = cells.insert(txn, name, MapPrelim::default());
    cell.insert(txn, "name", name);
    if let Some(formula) = formula {
        cell.insert(txn, "formula", formula);
    }
    if let Some(value) = value {
        cell.insert(txn, "value", value);
    }
}

impl DiagramSession {
    pub fn snapshot(&self) -> EditResult<DiagramSnapshot> {
        snapshot_doc(&self.doc)
    }

    pub fn set_cell_formula(
        &self,
        context: &EditCtx,
        page_id: &str,
        shape_id: &str,
        cell_name: &str,
        formula: impl Into<String>,
    ) -> EditResult<CellFormulaReceipt> {
        let formula = formula.into();
        let mut txn = self.transact_for(context);
        let cell = cell_map(&mut txn, page_id, shape_id, cell_name)?;
        let before = map_string(&cell, &txn, "formula");
        if before
            .as_deref()
            .is_some_and(|current| current.trim_start().starts_with("GUARD("))
        {
            return Err(EditError::InvalidState(format!(
                "cell {cell_name:?} is guarded"
            )));
        }
        cell.insert(&mut txn, "formula", formula.as_str());
        Ok(CellFormulaReceipt {
            page_id: page_id.to_owned(),
            shape_id: shape_id.to_owned(),
            cell_name: cell_name.to_owned(),
            before,
            after: formula,
        })
    }

    pub fn move_shape(
        &self,
        context: &EditCtx,
        page_id: &str,
        shape_id: &str,
        x_formula: impl Into<String>,
        y_formula: impl Into<String>,
    ) -> EditResult<[CellFormulaReceipt; 2]> {
        Ok([
            self.set_cell_formula(context, page_id, shape_id, "PinX", x_formula)?,
            self.set_cell_formula(context, page_id, shape_id, "PinY", y_formula)?,
        ])
    }

    pub fn resize_shape(
        &self,
        context: &EditCtx,
        page_id: &str,
        shape_id: &str,
        width_formula: impl Into<String>,
        height_formula: impl Into<String>,
    ) -> EditResult<[CellFormulaReceipt; 2]> {
        Ok([
            self.set_cell_formula(context, page_id, shape_id, "Width", width_formula)?,
            self.set_cell_formula(context, page_id, shape_id, "Height", height_formula)?,
        ])
    }

    pub fn reorder_shape(
        &self,
        context: &EditCtx,
        page_id: &str,
        shape_id: &str,
        to_index: u32,
    ) -> EditResult<ShapeReceipt> {
        reorder(&self.doc, context, page_id, shape_id, to_index)
    }
    pub fn reorder_page(
        &self,
        context: &EditCtx,
        page_id: &str,
        to_index: u32,
    ) -> EditResult<ShapeReceipt> {
        reorder_page(&self.doc, context, page_id, to_index)
    }

    pub fn add_shape(
        &self,
        context: &EditCtx,
        page_id: &str,
        draft: &ShapeDraft,
    ) -> EditResult<ShapeReceipt> {
        let mut txn = self.transact_for(context);
        let pages = txn
            .get_map(PAGES)
            .ok_or_else(|| EditError::InvalidState("missing pages map".to_owned()))?;
        let page = map_ref(&pages, &txn, page_id)?;
        let order = map_array(&page, &txn, "shapes")?;
        let index = order.len(&txn);
        let id = self.next_id(&format!("{page_id}:shape"));
        let sheets = txn
            .get_map(SHEETS)
            .ok_or_else(|| EditError::InvalidState("missing sheets map".to_owned()))?;
        let shape = sheets.insert(&mut txn, id.as_str(), MapPrelim::default());
        shape.insert(&mut txn, "id", id.as_str());
        shape.insert(&mut txn, "sourceId", draft.source_id as f64);
        if let Some(name) = &draft.name {
            shape.insert(&mut txn, "name", name.as_str());
        }
        let cells = shape.insert(&mut txn, "cells", MapPrelim::default());
        for cell in &draft.cells {
            seed_cell(
                &cells,
                &mut txn,
                &cell.name,
                cell.formula.as_deref(),
                cell.value.as_deref(),
            );
        }
        order.push_back(&mut txn, id.as_str());
        Ok(ShapeReceipt {
            page_id: page_id.to_owned(),
            shape_id: id,
            from_index: None,
            to_index: Some(index),
        })
    }
}

pub(crate) fn validate_doc(doc: &Doc) -> EditResult<()> {
    let txn = doc.transact();
    let meta = required_map(&txn, META)?;
    if map_number(&meta, &txn, "schemaVersion") != Some(SCHEMA_VERSION) {
        return Err(EditError::InvalidState(
            "unsupported diagram schema version".to_owned(),
        ));
    }
    for key in ["fingerprint", "packageJson"] {
        if meta.get(&txn, key).is_none() {
            return Err(EditError::InvalidState(format!(
                "missing diagram metadata {key}"
            )));
        }
    }
    required_array(&txn, PAGE_ORDER)?;
    for root in [PAGES, SHEETS, STORIES] {
        required_map(&txn, root)?;
    }
    Ok(())
}

fn snapshot_doc(doc: &Doc) -> EditResult<DiagramSnapshot> {
    let txn = doc.transact();
    let order = required_array(&txn, PAGE_ORDER)?;
    let pages = required_map(&txn, PAGES)?;
    let sheets = required_map(&txn, SHEETS)?;
    let mut result = Vec::new();
    for index in 0..order.len(&txn) {
        let id = array_string(&order, &txn, index)
            .ok_or_else(|| EditError::InvalidState("page order contains non-string".to_owned()))?;
        let page = map_ref(&pages, &txn, &id)?;
        let shape_order = map_array(&page, &txn, "shapes")?;
        let mut shapes = Vec::new();
        for shape_index in 0..shape_order.len(&txn) {
            let shape_id = array_string(&shape_order, &txn, shape_index).ok_or_else(|| {
                EditError::InvalidState("shape order contains non-string".to_owned())
            })?;
            let shape = map_ref(&sheets, &txn, &shape_id)?;
            let cells = map_map(&shape, &txn, "cells")?;
            let mut snapshots = Vec::new();
            for (name, value) in cells.iter(&txn) {
                let Out::YMap(cell) = value else {
                    return Err(EditError::InvalidState("cell is not a map".to_owned()));
                };
                snapshots.push(CellSnapshot {
                    name: name.to_string(),
                    formula: map_string(&cell, &txn, "formula"),
                    value: map_string(&cell, &txn, "value"),
                });
            }
            shapes.push(ShapeSnapshot {
                id: shape_id,
                source_id: map_number(&shape, &txn, "sourceId")
                    .ok_or_else(|| EditError::InvalidState("missing source ID".to_owned()))?
                    as u32,
                name: map_string(&shape, &txn, "name"),
                cells: snapshots,
            });
        }
        result.push(PageSnapshot {
            id,
            source_part_path: required_string(&page, &txn, "sourcePartPath")?,
            name: map_string(&page, &txn, "name"),
            shapes,
        });
    }
    Ok(DiagramSnapshot { pages: result })
}

fn cell_map(
    txn: &mut TransactionMut<'_>,
    page_id: &str,
    shape_id: &str,
    name: &str,
) -> EditResult<MapRef> {
    let pages = txn
        .get_map(PAGES)
        .ok_or_else(|| EditError::InvalidState("missing pages map".to_owned()))?;
    map_ref(&pages, txn, page_id)?;
    let sheets = txn
        .get_map(SHEETS)
        .ok_or_else(|| EditError::InvalidState("missing sheets map".to_owned()))?;
    let shape = map_ref(&sheets, txn, shape_id)?;
    let cells = map_map(&shape, txn, "cells")?;
    cells
        .get(txn, name)
        .and_then(|value| {
            if let Out::YMap(map) = value {
                Some(map)
            } else {
                None
            }
        })
        .ok_or_else(|| EditError::CellNotFound(name.to_owned()))
}
fn reorder(
    doc: &Doc,
    context: &EditCtx,
    page_id: &str,
    shape_id: &str,
    to: u32,
) -> EditResult<ShapeReceipt> {
    let mut txn = match context.origin {
        crate::EditOrigin::Local => doc.transact_mut_with(0_u64),
        crate::EditOrigin::Agent => doc.transact_mut_with("vsdx:agent"),
        crate::EditOrigin::Remote => doc.transact_mut_with(crate::REMOTE_ORIGIN),
        crate::EditOrigin::System => doc.transact_mut_with("vsdx:system"),
    };
    let pages = txn
        .get_map(PAGES)
        .ok_or_else(|| EditError::InvalidState("missing pages map".to_owned()))?;
    let page = map_ref(&pages, &txn, page_id)?;
    let order = map_array(&page, &txn, "shapes")?;
    let length = order.len(&txn);
    if to >= length {
        return Err(EditError::OutOfBounds { index: to, length });
    }
    let from = (0..length)
        .find(|index| array_string(&order, &txn, *index).as_deref() == Some(shape_id))
        .ok_or_else(|| EditError::ShapeNotFound(shape_id.to_owned()))?;
    order.remove_range(&mut txn, from, 1);
    order.insert(&mut txn, to, shape_id);
    Ok(ShapeReceipt {
        page_id: page_id.to_owned(),
        shape_id: shape_id.to_owned(),
        from_index: Some(from),
        to_index: Some(to),
    })
}
fn reorder_page(doc: &Doc, context: &EditCtx, page_id: &str, to: u32) -> EditResult<ShapeReceipt> {
    let mut txn = match context.origin {
        crate::EditOrigin::Local => doc.transact_mut_with(0_u64),
        crate::EditOrigin::Agent => doc.transact_mut_with("vsdx:agent"),
        crate::EditOrigin::Remote => doc.transact_mut_with(crate::REMOTE_ORIGIN),
        crate::EditOrigin::System => doc.transact_mut_with("vsdx:system"),
    };
    let order = txn
        .get_array(PAGE_ORDER)
        .ok_or_else(|| EditError::InvalidState("missing page order".to_owned()))?;
    let length = order.len(&txn);
    if to >= length {
        return Err(EditError::OutOfBounds { index: to, length });
    }
    let from = (0..length)
        .find(|index| array_string(&order, &txn, *index).as_deref() == Some(page_id))
        .ok_or_else(|| EditError::PageNotFound(page_id.to_owned()))?;
    order.remove_range(&mut txn, from, 1);
    order.insert(&mut txn, to, page_id);
    Ok(ShapeReceipt {
        page_id: page_id.to_owned(),
        shape_id: page_id.to_owned(),
        from_index: Some(from),
        to_index: Some(to),
    })
}
fn required_map<T: ReadTxn>(txn: &T, name: &str) -> EditResult<MapRef> {
    txn.get_map(name)
        .ok_or_else(|| EditError::InvalidState(format!("missing {name} map")))
}
fn required_array<T: ReadTxn>(txn: &T, name: &str) -> EditResult<ArrayRef> {
    txn.get_array(name)
        .ok_or_else(|| EditError::InvalidState(format!("missing {name} array")))
}
fn map_ref<T: ReadTxn>(map: &MapRef, txn: &T, key: &str) -> EditResult<MapRef> {
    map.get(txn, key)
        .and_then(|value| {
            if let Out::YMap(map) = value {
                Some(map)
            } else {
                None
            }
        })
        .ok_or_else(|| EditError::PageNotFound(key.to_owned()))
}
fn map_map<T: ReadTxn>(map: &MapRef, txn: &T, key: &str) -> EditResult<MapRef> {
    map.get(txn, key)
        .and_then(|value| {
            if let Out::YMap(map) = value {
                Some(map)
            } else {
                None
            }
        })
        .ok_or_else(|| EditError::InvalidState(format!("missing {key} map")))
}
fn map_array<T: ReadTxn>(map: &MapRef, txn: &T, key: &str) -> EditResult<ArrayRef> {
    map.get(txn, key)
        .and_then(|value| {
            if let Out::YArray(array) = value {
                Some(array)
            } else {
                None
            }
        })
        .ok_or_else(|| EditError::InvalidState(format!("missing {key} array")))
}
fn map_string<T: ReadTxn>(map: &MapRef, txn: &T, key: &str) -> Option<String> {
    map.get(txn, key).and_then(|value| match value {
        Out::Any(Any::String(value)) => Some(value.to_string()),
        _ => None,
    })
}
fn required_string<T: ReadTxn>(map: &MapRef, txn: &T, key: &str) -> EditResult<String> {
    map_string(map, txn, key).ok_or_else(|| EditError::InvalidState(format!("missing {key}")))
}
fn map_number<T: ReadTxn>(map: &MapRef, txn: &T, key: &str) -> Option<f64> {
    match map.get(txn, key) {
        Some(Out::Any(Any::Number(number))) => Some(number),
        _ => None,
    }
}
fn array_string<T: ReadTxn>(array: &ArrayRef, txn: &T, index: u32) -> Option<String> {
    array.get(txn, index).and_then(|value| match value {
        Out::Any(Any::String(value)) => Some(value.to_string()),
        _ => None,
    })
}
