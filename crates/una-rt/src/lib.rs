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

pub mod deadline;
pub mod decimal;
#[allow(unsafe_code)]
pub mod ffi;
pub mod generic;
pub mod host;
mod json;
pub mod render;
pub mod runtime;
mod sched;
pub mod tasks;
mod timer;
pub mod values;
pub mod wheel;

/// The allocator of the executable, which counts the bytes it holds and fails past a ceiling,
/// for the memory limit of runs (ADR 0061). The `cap` crate wraps the system allocator, so the
/// runtime writes no `unsafe` for it.
#[cfg(not(debug_assertions))]
#[global_allocator]
static ALLOCATOR: Cap<System> = Cap::new(System, usize::MAX);

/// The allocator of a debug build: the same `Cap<System>`, wrapped to keep the high-water mark
/// of what the process holds and each thread's allocations (ADR 0113, I3).
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: ffi::Peak = ffi::Peak::new(Cap::new(System, usize::MAX));

/// The `Cap<System>` that counts and limits the process's heap, in either build.
fn cap() -> &'static Cap<System> {
    #[cfg(debug_assertions)]
    {
        ALLOCATOR.inner()
    }
    #[cfg(not(debug_assertions))]
    {
        &ALLOCATOR
    }
}

/// The bytes the process holds on the heap.
pub(crate) fn held() -> usize {
    cap().allocated()
}

/// The most bytes the process held at any point so far, in a debug build's tests (ADR 0113, I3).
#[cfg(all(debug_assertions, test))]
pub(crate) fn peak() -> usize {
    ALLOCATOR.peak()
}

/// Sets the ceiling past which an allocation fails; `usize::MAX` lifts it. A ceiling below what
/// is held already is refused, and the next check reports the run.
pub(crate) fn ceiling(bytes: usize) {
    let _ = cap().set_limit(bytes);
}
