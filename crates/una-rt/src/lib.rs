//! `una-rt`: the runtime library that every native Una executable links (`spec/16`,
//! ADR 0055).
//!
//! The executable's `main` calls [`ffi`]'s `una_rt_start`, which reads the program's embedded
//! description and the command line, checks `main`'s capabilities against the grants
//! (U3012), runs the compiled `main`, and writes the trace when `--trace` asks for it. The
//! compiled code keeps its values in [`values`], performs host operations and records events
//! through [`host`], and reports a failure of `main` (U6001) with [`render`].
//!
//! Host capabilities have the interpreter's semantics and grants, and traces use its JSON
//! form ([`una_interp::trace_json`]), so a native run and `una run` agree (`spec/16 §5`).
//!
//! Only [`ffi`] may use `unsafe`, at the C ABI boundary (ADR 0058). This runtime is the
//! process, so unlike the library crates it writes to stdout and stderr.
#![deny(missing_docs)]

use std::alloc::System;

use cap::Cap;

#[allow(unsafe_code)]
pub mod ffi;
pub mod generic;
pub mod host;
pub mod render;
pub mod runtime;
pub mod tasks;
pub mod values;

/// The allocator of the executable, which counts the bytes it holds and fails past a ceiling,
/// for the memory limit of runs (ADR 0061). The `cap` crate wraps the system allocator, so the
/// runtime writes no `unsafe` for it.
#[global_allocator]
static ALLOCATOR: Cap<System> = Cap::new(System, usize::MAX);

/// The bytes the process holds on the heap.
pub(crate) fn held() -> usize {
    ALLOCATOR.allocated()
}

/// Sets the ceiling past which an allocation fails; `usize::MAX` lifts it. A ceiling below what
/// is held already is refused, and the next check reports the run.
pub(crate) fn ceiling(bytes: usize) {
    let _ = ALLOCATOR.set_limit(bytes);
}
