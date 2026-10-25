//! The model of signatures that this crate's unit tests may stand in for Ed25519 in the
//! owner-chain replay (the parent module's `verified`). The module exists only in test builds,
//! so no other build can reach it, and a test turns the model on for its own thread only.

use std::cell::Cell;

use crate::tlog::note::{Key, Note};

/// The bytes a modeled signature line carries after its key hash.
pub(super) const MARK: [u8; 64] = [0x6d; 64];

thread_local! {
    /// Whether this thread's signature checks are the model's.
    static MODELED: Cell<bool> = const { Cell::new(false) };
}

/// The model's answer to whether `key` signs `note`, when this thread turned the model on: a
/// signature line of the key's name and key hash that carries [`MARK`]. `None` when it is off,
/// and then Ed25519 decides.
pub(super) fn modeled(note: &Note, key: &Key) -> Option<bool> {
    MODELED.with(Cell::get).then(|| {
        let id = key.id();
        note.signatures
            .iter()
            .any(|line| line.name == key.name && line.id == id && line.bytes == MARK)
    })
}

/// Turns the model of signatures on or off for this thread.
pub(super) fn set_modeled(on: bool) {
    MODELED.with(|cell| cell.set(on));
}
