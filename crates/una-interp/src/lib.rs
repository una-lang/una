//! The Una Abstract Machine (`spec/11`): it evaluates Core IR, performs host operations through
//! a [`World`], records traces, and runs the examples of a package.
//!
//! Nothing here prints or touches the machine it runs on: `una-cli` implements [`World`] with
//! real input and output.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod clock;
pub mod concurrency;
mod env;
mod harness;
mod host;
mod json;
mod machine;
pub mod serve_trace;
pub mod stopping;
mod tasks;
mod trace;
mod value;

pub use clock::{Clock, NotVirtual, VirtualClock};
pub use concurrency::Concurrency;
pub use harness::{
    Completion, DriveShape, ExampleFailure, ExampleRun, Host, Run, run_drive, run_examples,
    run_main, run_serve,
};
pub use host::{
    Absence, Confined, FileFault, Grant, LimitLabel, NotText, World, check_limited_flags, confine,
    disk_absent, disk_step, hashed_equal, hex, hmac_sha256, invalid_grant, parse_keys,
};
pub use machine::{Abort, Drive, Event, EventOutcome, Limits, fixed_method};
pub use trace::{EventDoc, OutcomeDoc, Pairs, event_json, trace_json};

/// The default maximum call depth (`spec/11 §7`).
pub const DEFAULT_MAX_DEPTH: usize = 10_000;

/// The default memory budget of a run, in MiB (`spec/11 §7`, ADR 0061).
pub const DEFAULT_MAX_MEMORY: usize = 4096;

/// The usage error of a run of a package with a `serve` section whose module is `module` and
/// that holds no `serve:` grant (`spec/31 §7` rule 1), in `una run` and in an executable alike.
pub fn unserved_section(module: &str) -> String {
    format!(
        "the `serve` section's module `{module}` answers requests, but neither `run grants` of ita.una nor a `--grant` flag grants an address to serve on; grant one, as `serve \"http://127.0.0.1:8080\"` in `run grants` or `--grant serve:http://127.0.0.1:8080`"
    )
}

/// The usage error of a run under `--virtual-time` that holds a `serve:` grant (ADR 0123, S4
/// amendment C), the same in `una run` and in executables: live peers send in real time, which
/// virtual time cannot place.
pub const VIRTUAL_SERVE: &str = "`--virtual-time` cannot run with a `serve:` grant: live peers send in real time, which virtual time cannot place";

/// The most stack a run reserves, whatever its maximum depth (ADR 0061 rule 1).
const STACK_CEILING: usize = 1024 * 1024 * 1024;

/// The stack of the thread that evaluates calls up to `max_depth` deep, capped at 1 GiB
/// (ADR 0061 rule 1); `None` only when the system cannot address it. The evaluator recurses
/// once per call in progress; unoptimized builds use about 13 KiB of stack per call, so 32 KiB
/// leaves room for nested expressions. Evaluation never uses more than this, since it aborts
/// with U6003 when the stack runs short (ADR 0032), before the depth when the cap is lower.
pub fn stack_bytes(max_depth: usize) -> Option<usize> {
    const BASE: usize = 16 * 1024 * 1024;
    const PER_CALL: usize = 32 * 1024;
    let wanted = max_depth.saturating_mul(PER_CALL).saturating_add(BASE);
    Some(wanted.min(STACK_CEILING))
}
