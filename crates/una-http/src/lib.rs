//! The HTTP/1.1 client of the host capability `Http` (`spec/24`, ADR 0074), shared by the
//! interpreter and the native runtime, so both engines send the same requests and read the same
//! responses, and the strict framing of the server of `HttpServer` (`spec/31`, ADR 0123).
//!
//! The client without TLS is `una-http-core`'s, which the interpreter and the runtime library
//! `libuna_rt.a` depend on. This crate holds its TLS client, over `rustls`, the only crate that
//! links the TLS stack: [`install_tls`] installs it, [`client::send`] does first, and so do
//! `una` at start and the runtime library `libuna_rt_http.a` of executables that use `Http`
//! (ADR 0074 H7, #1102).

pub mod reply;
pub mod request;
pub mod strict;
mod tls;

pub use tls::install_tls;
/// URLs and grants, which `una-core` holds for every crate that reads grants.
pub use una_http_core::grant;
/// The response parser of the client.
pub use una_http_core::parse;

/// Sending a request, over TCP or the TLS client of this crate (`spec/24 §3`, §4).
pub mod client {
    pub use una_http_core::client::{Error, MAX_TIMEOUT_SECS, Request, Response, Settings};

    #[cfg(debug_assertions)]
    pub use crate::tls::trust_test_roots;

    /// Sends `request` where `grants` allow, within `settings` (`spec/24`), with the TLS client
    /// of this crate installed.
    pub fn send(
        request: &Request<'_>,
        grants: &[crate::grant::Grant],
        settings: &Settings,
    ) -> Result<Response, Error> {
        crate::install_tls();
        una_http_core::client::send(request, grants, settings)
    }
}
