//! The HTTP/1.1 client of the host capability `Http` (`spec/24`, ADR 0074) without its TLS
//! stack: requests, redirects, deadlines and the response parser, over TCP, and over the TLS
//! client that a process installs with [`client::install_tls`]. `una-http` holds that TLS
//! client, over `rustls`, and installs it.
//!
//! The interpreter and the runtime library `libuna_rt.a` depend on this crate alone, so an
//! executable that never uses `Http` links no TLS code (ADR 0074 H7, #1102). An executable that
//! uses `Http` links `libuna_rt_http.a`, which installs the TLS client of `una-http`.

pub mod client;
pub mod parse;

/// URLs and grants, which `una-core` holds for every crate that reads grants.
pub use una_core::http as grant;
