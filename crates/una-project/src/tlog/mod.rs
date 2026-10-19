//! The verification of a registry's documents (ADR 0122 rules 35 to 39): the Merkle tree of the
//! transparency log, strict Ed25519, signed notes, checkpoints and cosignatures, and witness
//! time. Each part is a pure function of the bytes it is given; fetching is the client's.

pub mod ed25519;
pub mod merkle;
pub mod note;
pub mod release;
pub mod witness;
