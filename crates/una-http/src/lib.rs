//! The HTTP/1.1 client of the host capability `Http` (`spec/24`, ADR 0074), shared by the
//! interpreter and the native runtime, so both engines send the same requests and read the same
//! responses, and the strict framing of the server of `HttpServer` (`spec/31`, ADR 0123).

pub mod client;
pub mod limit;
pub mod parse;
pub mod reply;
pub mod request;
mod resolve;
pub mod strict;

/// URLs and grants, which `una-core` holds for every crate that reads grants.
pub use una_core::http as grant;
