//! The code with `unsafe` that the native runtime (`una-rt`) and the interpreter (`una-interp`)
//! share (ADR 0116 rule 18, ADR 0058 amendments, ADR 0117 amendment of 2026-10-19): the handlers
//! of SIGINT and SIGTERM, the attention words, coroutines, a write-readiness check, and the
//! `kqueue` or `epoll` instance of the reactor (ADR 0117 slice S7).
//!
//! The base code stays under its cap of 1,751 lines (ADR 0058, amendment of 2026-11-19), with one
//! assembly instruction per line, and depends on no crate of the repository. Every change to
//! it gets a security review before it merges. Only `signals`, `coroutine`, `io`, `poll`,
//! `server` (`serve:` grants) and, behind the feature of that name, `solver` hold `unsafe` code,
//! each block with a `// SAFETY:` comment; `solver` holds the solver child's confinement, limits,
//! spawn attributes and footprint reading, and the record of its confinement (ADR 0124 rules 5 to
//! 7). The context switch is Rust `naked_asm!` only,
//! assembled by the toolchain: no `.s` or `.S` file, no build script, and no C is compiled.
//! It assumes no x86 shadow stacks (CET) and no AArch64 branch targets (BTI) are enforced, as for
//! Rust binaries on these targets today; if either is turned on, the switch must be revisited.

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("una-sys supports macOS and Linux only (ADR 0116 rule 18)");

pub mod attention;
pub mod coroutine;
pub mod io;
pub mod poll;
pub mod server;
pub mod signals;
#[cfg(feature = "solver")]
pub mod solver;
