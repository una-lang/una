//! `una-oracle`: the security hunt's oracle runner, for development only (ADR 0115).
//!
//! It runs a package exactly as `una --json run` does, through the same code, but its document,
//! `una.oracle-run/1`, shows every value, labeled ones included, and records each
//! declassification: the hunt's oracle reads it to tell whether a varied literal reaches what a
//! run does, and what a policy released.
//!
//! **It prints labeled values.** It is only for the hunt's generated programs, whose literals are
//! synthetic, and it refuses to run unless `UNA_HUNT_RUN=1` is set, which the hunt's sandbox sets
//! for every run. That marker guards against using it by mistake, not against an adversary.
//! It is never built for a release: `scripts/release.sh` builds `una` without the `oracle`
//! feature, and `scripts/check-release.sh` fails an archive that holds this binary.
//!
//! Usage: `UNA_HUNT_RUN=1 una-oracle --json run [path] [the options of una run]`
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::alloc::System;
use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use cap::Cap;

/// The allocator of the process, as `una`'s, which counts the bytes it holds and fails past a
/// ceiling, for the memory limit of runs (ADR 0061).
#[global_allocator]
static ALLOCATOR: Cap<System> = Cap::new(System, usize::MAX);

/// The variable the hunt's sandbox sets for every run, and its value.
const MARKER: &str = "UNA_HUNT_RUN";

#[allow(clippy::disallowed_methods)]
fn main() -> ExitCode {
    let stderr = std::io::stderr();
    if std::env::var_os(MARKER).as_deref() != Some(std::ffi::OsStr::new("1")) {
        let _ = writeln!(
            stderr.lock(),
            "error: una-oracle prints labeled values and runs only for the security hunt's \
             oracle, with {MARKER}=1 (ADR 0115)"
        );
        return ExitCode::from(2);
    }
    let stdout = std::io::stdout();
    // The command line of `una`: this binary's name, then `--json run ...` as given.
    let args: Vec<OsString> = std::iter::once(OsString::from("una"))
        .chain(std::env::args_os().skip(1))
        .collect();
    let status = una_cli::run_observed(
        args,
        &mut stdout.lock(),
        &mut stderr.lock(),
        &environment,
        una_cli::Heap {
            held: &|| ALLOCATOR.allocated(),
            ceiling: &|bytes| {
                // A ceiling below what is held fails; the next check reports the run then.
                let _ = ALLOCATOR.set_limit(bytes);
            },
        },
    );
    ExitCode::from(status.code())
}

/// A variable of the process environment, for programs that the run grants `env`, as `una`
/// reads it (`spec/14 §3`). A value that is not valid UTF-8 is `NotText`.
#[allow(clippy::disallowed_methods)]
fn environment(name: &str) -> Result<Option<String>, una_interp::NotText> {
    match std::env::var_os(name) {
        Some(value) => value
            .into_string()
            .map(Some)
            .map_err(|_| una_interp::NotText),
        None => Ok(None),
    }
}
