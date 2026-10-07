//! The HTTP/1.1 client of the host capability `Http` (`spec/24`, ADR 0074), shared by the
//! interpreter and the native runtime, so both engines send the same requests and read the same
//! responses.

pub mod client;
pub mod parse;

/// URLs and grants, which `una-core` holds for every crate that reads grants.
pub use una_core::http as grant;
