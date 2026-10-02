//! The Una Abstract Machine (`spec/11`): it evaluates Core IR, performs host operations through
//! a [`World`], records traces, and runs the examples of a package.
//!
//! Nothing here prints or touches the machine it runs on: `una-cli` implements [`World`] with
//! real input and output.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod harness;
mod host;
mod machine;
mod trace;
mod value;

pub use harness::{Completion, ExampleFailure, ExampleRun, Host, Run, run_examples, run_main};
pub use host::{FileFault, Grant, World, confine, hex, hmac_sha256, parse_keys};
pub use machine::{Abort, Event, EventOutcome};
pub use trace::{EventDoc, OutcomeDoc, Pairs, trace_json};

/// The default maximum call depth (`spec/11 §7`).
pub const DEFAULT_MAX_DEPTH: usize = 10_000;

/// The stack of the thread that evaluates calls up to `max_depth` deep, or `None` when it does
/// not fit in the address space. The evaluator recurses once per call in progress; unoptimized
/// builds use about 13 KiB of stack per call, so 32 KiB leaves room for nested expressions.
/// Evaluation never uses more than this, since it aborts with U6003 when the stack runs short.
pub fn stack_bytes(max_depth: usize) -> Option<usize> {
    const BASE: usize = 16 * 1024 * 1024;
    const PER_CALL: usize = 32 * 1024;
    max_depth.checked_mul(PER_CALL)?.checked_add(BASE)
}
