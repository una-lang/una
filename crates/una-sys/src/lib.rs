//! The code with `unsafe` that the native runtime (`una-rt`) and the interpreter (`una-interp`)
//! share (ADR 0116 rule 18, and the amendments of ADR 0058): the handlers of SIGINT and SIGTERM,
//! the attention words that check points read, and the system calls that the host of `serve:`
//! grants makes before `main` (ADR 0123 rules 9 and 19), which `una-http` calls for both
//! engines.
//!
//! The crate stays under 800 lines and depends on no crate of the repository. Every change to
//! it gets a security review before it merges. Only `signals` and `server` hold `unsafe` code,
//! each block with a `// SAFETY:` comment; the coroutines of ADR 0117 rule 2 will join it in a
//! module of their own.

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("una-sys supports macOS and Linux only (ADR 0116 rule 18)");

pub mod attention;
pub mod server;
pub mod signals;
