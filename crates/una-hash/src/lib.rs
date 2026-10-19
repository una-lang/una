//! Keyed hashing for Una's hash tables (ADR 0119 rules 9 and 19), one module that both engines
//! use: SipHash-2-4, the hash encoding of an `Eq` value, the process key and the key of each
//! table, and the ordered hash table with its own index.
//!
//! No operation of the language returns a hash or a key, and none appears in a trace, the report,
//! the Core export or a message: this crate gives the engines hashes to place keys, and nothing
//! else.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod encode;
pub mod key;
pub mod siphash;
pub mod table;

#[cfg(feature = "test-hooks")]
pub mod hooks;

pub use encode::{Encoder, Sink};
pub use key::{KeyError, ProcessKey, table_key};
pub use siphash::{Sip24, SipKey, sip24};
pub use table::{OrderedTable, index_capacity};

/// The hash of a key under its table's key: SipHash-2-4 of the key's hash encoding, which
/// `encode` writes, in O(|encoding|).
pub fn hash(table: SipKey, encode: impl FnOnce(&mut Encoder<'_, Sip24>)) -> u64 {
    #[cfg(feature = "test-hooks")]
    if hooks::constant_hash() {
        return 0;
    }
    let mut state = Sip24::new(table);
    encode(&mut Encoder::new(&mut state));
    state.finish()
}
