//! The code with `unsafe` that the native runtime (`una-rt`) and the interpreter (`una-interp`)
//! share (ADR 0116 rule 18, and the amendment of ADR 0058 in its Appendix A): the handlers of
//! SIGINT and SIGTERM, and the attention words that check points read.
//!
//! The crate stays under 800 lines and depends on no crate of the repository. Every change to
//! it gets a security review before it merges. Only `signals` holds `unsafe` code, each block
//! with a `// SAFETY:` comment; the coroutines of ADR 0117 rule 2 will join it in a module of
//! their own.

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("una-sys supports macOS and Linux only (ADR 0116 rule 18)");

pub mod attention;
pub mod signals;
