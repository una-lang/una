//! `una-rt`: the runtime library that every native Una executable links (`spec/16`,
//! ADR 0055).
//!
//! The executable's `main` calls [`ffi`]'s `una_rt_start`, which reads the program's embedded
//! description and the command line, checks `main`'s capabilities against the grants
//! (U3012), runs the compiled `main`, and writes the trace when `--trace` asks for it. The
//! compiled code performs host operations through the other functions of [`ffi`].
//!
//! Host capabilities have the interpreter's semantics and grants, and traces use its JSON
//! form ([`una_interp::trace_json`]), so a native run and `una run` agree (`spec/16 §5`).
//!
//! Only [`ffi`] may use `unsafe`, at the C ABI boundary (ADR 0058). This runtime is the
//! process, so unlike the library crates it writes to stdout and stderr.
#![deny(missing_docs)]

#[allow(unsafe_code)]
pub mod ffi;
pub mod runtime;
