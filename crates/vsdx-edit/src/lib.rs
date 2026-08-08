//! Collaborative yrs-backed VSDX diagram model.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use yrs::updates::decoder::{Decode, Decoder, DecoderV1};
use yrs::updates::encoder::Encode;
use yrs::{
    ClientID, Doc, OffsetKind, Options, ReadTxn, StateVector, Subscription, Transact, Update,
    WriteTxn,
};

mod diagram;
mod model;
mod undo;

pub use model::*;
pub use undo::DiagramUndoManager;

pub(crate) const META: &str = "vsdx:meta";
pub(crate) const PAGE_ORDER: &str = "vsdx:page-order";
pub(crate) const PAGES: &str = "vsdx:pages";
pub(crate) const SHEETS: &str = "vsdx:sheets";
pub(crate) const STORIES: &str = "vsdx:stories";
pub(crate) const REMOTE_ORIGIN: &str = "vsdx:remote";
pub(crate) const HYDRATE_ORIGIN: &str = "vsdx:hydrate";
const BOOTSTRAP_CLIENT_ID: u64 = (1_u64 << 53) - 1;
pub const MAX_SAFE_CLIENT_ID: u64 = BOOTSTRAP_CLIENT_ID - 1;
pub const MAX_UPDATE_BYTES: usize = 64 * 1024 * 1024;
const MAX_STATE_VECTOR_ENTRIES: u32 = 65_536;

pub struct DiagramSession {
    pub(crate) doc: Doc,
    client_id: u64,
    id_counter: AtomicU64,
    undo: RefCell<DiagramUndoManager>,
}

impl DiagramSession {
    pub fn open(bytes: &[u8], client_id: u64) -> EditResult<Self> {
        let package =
            vsdx_parse::parse_vsdx(bytes).map_err(|error| EditError::Parse(error.to_string()))?;
        Self::from_package_with_fingerprint(
            package,
            format!("{:x}", Sha256::digest(bytes)),
            client_id,
        )
    }

    pub fn from_package(package: vsdx_parse::VsdxPackage, client_id: u64) -> EditResult<Self> {
        let json =
            serde_json::to_vec(&package).map_err(|error| EditError::Json(error.to_string()))?;
        Self::from_package_with_fingerprint(
            package,
            format!("{:x}", Sha256::digest(json)),
            client_id,
        )
    }

    fn from_package_with_fingerprint(
        package: vsdx_parse::VsdxPackage,
        fingerprint: String,
        client_id: u64,
    ) -> EditResult<Self> {
        validate_client_id(client_id)?;
        let bootstrap = doc_with_client_id(BOOTSTRAP_CLIENT_ID);
        diagram::seed_doc(&bootstrap, &package, &fingerprint)?;
        let baseline = bootstrap
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let doc = doc_with_client_id(client_id);
        hydrate_doc(&doc, &baseline)?;
        diagram::validate_doc(&doc)?;
        let undo = DiagramUndoManager::new(&doc, client_id)?;
        Ok(Self {
            doc,
            client_id,
            id_counter: AtomicU64::new(0),
            undo: RefCell::new(undo),
        })
    }

    pub fn open_from_update(update: &[u8], client_id: u64) -> EditResult<Self> {
        validate_client_id(client_id)?;
        if update.len() > MAX_UPDATE_BYTES {
            return Err(EditError::InvalidUpdate(format!(
                "update exceeds {MAX_UPDATE_BYTES} bytes"
            )));
        }
        let doc = doc_with_client_id(client_id);
        hydrate_doc(&doc, update)?;
        diagram::validate_doc(&doc)?;
        let undo = DiagramUndoManager::new(&doc, client_id)?;
        Ok(Self {
            doc,
            client_id,
            id_counter: AtomicU64::new(0),
            undo: RefCell::new(undo),
        })
    }

    pub fn client_id(&self) -> u64 {
        self.client_id
    }
    pub fn yrs_doc(&self) -> &Doc {
        &self.doc
    }
    pub fn encode_state_vector_v1(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }
    pub fn encode_state_as_update_v1(&self) -> Vec<u8> {
        self.doc
            .transact()
            .encode_state_as_update_v1(&StateVector::default())
    }
    pub fn encode_diff_v1(&self, remote: &[u8]) -> EditResult<Vec<u8>> {
        let vector = decode_state_vector_v1(remote).map_err(EditError::InvalidStateVector)?;
        Ok(self.doc.transact().encode_diff_v1(&vector))
    }

    pub fn apply_update_v1(&self, bytes: &[u8]) -> EditResult<DiagramSnapshot> {
        if bytes.len() > MAX_UPDATE_BYTES {
            return Err(EditError::InvalidUpdate(format!(
                "update exceeds {MAX_UPDATE_BYTES} bytes"
            )));
        }
        let incoming = decode_update_v1(bytes).map_err(EditError::InvalidUpdate)?;
        let staged = doc_with_client_id(self.client_id);
        hydrate_doc(&staged, &self.encode_state_as_update_v1())?;
        staged
            .transact_mut_with(REMOTE_ORIGIN)
            .apply_update(incoming)
            .map_err(|error| EditError::InvalidUpdate(error.to_string()))?;
        diagram::validate_doc(&staged)?;
        self.doc
            .transact_mut_with(REMOTE_ORIGIN)
            .apply_update(decode_update_v1(bytes).map_err(EditError::InvalidUpdate)?)
            .map_err(|error| EditError::InvalidUpdate(error.to_string()))?;
        self.snapshot()
    }

    pub fn observe_update_v1<F>(&self, callback: F) -> EditResult<Subscription>
    where
        F: Fn(UpdateEvent) + 'static,
    {
        self.doc
            .observe_update_v1(move |txn, event| {
                let origin = if txn
                    .origin()
                    .is_some_and(|origin| origin.as_ref() == REMOTE_ORIGIN.as_bytes())
                {
                    UpdateOrigin::Remote
                } else {
                    UpdateOrigin::Local
                };
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    callback(UpdateEvent {
                        update: event.update.clone(),
                        origin,
                    })
                }));
            })
            .map_err(|error| EditError::Observer(error.to_string()))
    }

    pub fn undo(&self) -> bool {
        self.undo.borrow_mut().undo()
    }
    pub fn redo(&self) -> bool {
        self.undo.borrow_mut().redo()
    }
    pub fn can_undo(&self) -> bool {
        self.undo.borrow().can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.undo.borrow().can_redo()
    }
    pub fn add_undo_barrier(&self) {
        self.undo.borrow_mut().add_undo_barrier()
    }
    pub(crate) fn transact_for(&self, context: &EditCtx) -> yrs::TransactionMut<'_> {
        match context.origin {
            EditOrigin::Local => self.doc.transact_mut_with(self.client_id),
            EditOrigin::Agent => self.doc.transact_mut_with("vsdx:agent"),
            EditOrigin::Remote => self.doc.transact_mut_with(REMOTE_ORIGIN),
            EditOrigin::System => self.doc.transact_mut_with("vsdx:system"),
        }
    }
    pub(crate) fn next_id(&self, prefix: &str) -> String {
        format!(
            "{prefix}:{}:{}",
            self.client_id,
            self.id_counter.fetch_add(1, Ordering::Relaxed)
        )
    }
}

fn doc_with_client_id(client_id: u64) -> Doc {
    let mut options = Options::with_client_id(ClientID::new(client_id));
    options.offset_kind = OffsetKind::Utf16;
    Doc::with_options(options)
}
fn validate_client_id(client_id: u64) -> EditResult<()> {
    if client_id == 0 || client_id > MAX_SAFE_CLIENT_ID {
        Err(EditError::InvalidClientId(client_id))
    } else {
        Ok(())
    }
}
fn hydrate_doc(doc: &Doc, bytes: &[u8]) -> EditResult<()> {
    let update = decode_update_v1(bytes).map_err(EditError::InvalidUpdate)?;
    let mut txn = doc.transact_mut_with(HYDRATE_ORIGIN);
    txn.apply_update(update)
        .map_err(|error| EditError::InvalidUpdate(error.to_string()))?;
    txn.get_or_insert_array(PAGE_ORDER);
    for root in [META, PAGES, SHEETS, STORIES] {
        txn.get_or_insert_map(root);
    }
    Ok(())
}
fn decode_update_v1(bytes: &[u8]) -> Result<Update, String> {
    let mut decoder = DecoderV1::from(bytes);
    let update = Update::decode(&mut decoder).map_err(|error| error.to_string())?;
    if !decoder
        .read_to_end()
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Err("update contains trailing bytes".to_owned());
    }
    Ok(update)
}
fn decode_state_vector_v1(bytes: &[u8]) -> Result<StateVector, String> {
    validate_state_vector_entry_count(bytes)?;
    let mut decoder = DecoderV1::from(bytes);
    let vector = StateVector::decode(&mut decoder).map_err(|error| error.to_string())?;
    if !decoder
        .read_to_end()
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Err("state vector contains trailing bytes".to_owned());
    }
    Ok(vector)
}
fn validate_state_vector_entry_count(bytes: &[u8]) -> Result<(), String> {
    let Some((&first, _)) = bytes.split_first() else {
        return Err("state vector is empty".to_owned());
    };
    let mut value = u32::from(first & 0x7f);
    let mut shift = 7;
    let mut used = 1;
    let mut byte = first;
    while byte & 0x80 != 0 {
        if used == 5 || used >= bytes.len() {
            return Err("invalid state vector entry count".to_owned());
        }
        byte = bytes[used];
        if used == 4 && byte > 0x0f {
            return Err("invalid state vector entry count".to_owned());
        }
        value |= u32::from(byte & 0x7f) << shift;
        shift += 7;
        used += 1;
    }
    if value > MAX_STATE_VECTOR_ENTRIES {
        return Err(format!(
            "state vector contains {value} entries, exceeds the {MAX_STATE_VECTOR_ENTRIES}-entry limit"
        ));
    }
    if value as usize > bytes.len().saturating_sub(used) / 2 {
        return Err("state vector entry count exceeds its payload".to_owned());
    }
    Ok(())
}
