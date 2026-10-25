//! The code with `unsafe` that calls the operating system for the rest of the repository
//! (ADR 0116 rule 18, and the amendment of ADR 0058 in its Appendix A): the handlers of SIGINT
//! and SIGTERM and the attention words that check points read, for the native runtime
//! (`una-rt`) and the interpreter (`una-interp`); and directory handles with creation, renaming
//! and removal relative to them, for the writes of vendoring and the Itaoca store in
//! `una-project`, and through it `una-check` (#1094).
//!
//! The crate stays under 800 lines and depends on no crate of the repository. Every change to
//! it gets a security review before it merges. Only `signals` and `dir` hold `unsafe` code, each
//! block with a `// SAFETY:` comment; the coroutines of ADR 0117 rule 2 will join them in a
//! module of their own.

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("una-sys supports macOS and Linux only (ADR 0116 rule 18)");

pub mod attention;
pub mod dir;
pub mod signals;
