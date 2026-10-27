//! The verification of a registry's documents (ADR 0122 rules 35 to 39): the Merkle tree of the
//! transparency log, strict Ed25519, signed notes, checkpoints and cosignatures, and witness
//! time, owners' keys, and the log's tiles. Each part is a pure function of the bytes it is
//! given; fetching is the client's ([`crate::client`]).

pub mod ed25519;
pub mod map;
pub mod merkle;
pub mod note;
pub mod owner;
pub mod publish;
pub mod release;
pub mod root;
pub mod state;
pub mod tile;
pub mod witness;
