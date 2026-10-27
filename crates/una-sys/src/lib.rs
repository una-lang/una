//! The code with `unsafe` that the native runtime (`una-rt`) and the interpreter (`una-interp`)
//! share (ADR 0116 rule 18, ADR 0058 amendments, ADR 0117 amendment of 2026-10-19): the handlers
//! of SIGINT and SIGTERM, the attention words, coroutines, and a write-readiness check.
//!
//! The crate stays under 1,200 lines, with one assembly instruction per line (ADR 0116 amendment
//! of 2026-10-19), and depends on no crate of the repository. Every change to
//! it gets a security review before it merges. Only `signals`, `coroutine` and `io` hold `unsafe`
//! code, each block with a `// SAFETY:` comment. The context switch is Rust `naked_asm!` only,
//! assembled by the toolchain: no `.s` or `.S` file, no build script, and no C is compiled.
//! It assumes no x86 shadow stacks (CET) and no AArch64 branch targets (BTI) are enforced, as for
//! Rust binaries on these targets today; if either is turned on, the switch must be revisited.

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("una-sys supports macOS and Linux only (ADR 0116 rule 18)");

pub mod attention;
pub mod coroutine;
pub mod io;
pub mod signals;
