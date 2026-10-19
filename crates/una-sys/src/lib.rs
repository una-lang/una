//! The code with `unsafe` that the native runtime (`una-rt`) and the interpreter (`una-interp`)
//! share (ADR 0116 rule 18, and the amendment of ADR 0058 in its Appendix A): the handlers of
//! SIGINT and SIGTERM, the attention words that check points read, and the coroutines of ADR 0117
//! rule 2.
//!
//! The crate stays under 800 lines and depends on no crate of the repository. Every change to
//! it gets a security review before it merges. Only `signals`, `coroutine` and `io` hold `unsafe`
//! code, each block with a `// SAFETY:` comment.

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("una-sys supports macOS and Linux only (ADR 0116 rule 18)");

pub mod attention;
pub mod coroutine;
pub mod io;
pub mod signals;
