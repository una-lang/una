//! The `una` command line tool (`spec/13 §5`).
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::alloc::System;
use std::io::IsTerminal;
use std::process::ExitCode;

use cap::Cap;

/// The allocator of the process, which counts the bytes it holds and fails past a ceiling, for
/// the memory limit of runs (ADR 0061).
#[global_allocator]
static ALLOCATOR: Cap<System> = Cap::new(System, usize::MAX);

fn main() -> ExitCode {
    // The solver child (ADR 0124 rule 3): `una __solver <limits...>`, before anything else
    // reads the environment or the package; it exits with the front end's own status.
    // Read as OS strings, since an argument of another command need not be UTF-8.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|first| first == "__solver")
    {
        let args: Vec<String> = std::env::args_os()
            .skip(2)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        return ExitCode::from(una_cli::solver_child::serve(&args));
    }
    // The command runs on a thread of `MAIN_STACK_BYTES`, never on the main thread, whose
    // stack the system fixes (8 MiB on macOS and Linux): the passes over Core IR recurse once
    // per nested expression, and the checker lets a declaration nest up to
    // `una_core::depth::MAX_DEPTH` deep (#2264). The stack is reserved, not committed.
    let command = || {
        let stdout = std::io::stdout();
        let stderr = std::io::stderr();
        // `--color auto` follows stderr, where human diagnostics go (ADR 0015 D1).
        let stderr_is_terminal = stderr.is_terminal();
        una_cli::run(
            std::env::args_os(),
            &mut stdout.lock(),
            &mut stderr.lock(),
            stderr_is_terminal,
            &environment,
            una_cli::Heap {
                held: &|| ALLOCATOR.allocated(),
                ceiling: &|bytes| {
                    // A ceiling below what is held fails; the next check reports the run then.
                    let _ = ALLOCATOR.set_limit(bytes);
                },
            },
        )
    };
    let status = std::thread::scope(|scope| {
        match std::thread::Builder::new()
            .name("una".to_owned())
            .stack_size(MAIN_STACK_BYTES)
            .spawn_scoped(scope, command)
        {
            Ok(handle) => handle
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload)),
            Err(_) => command(),
        }
    });
    ExitCode::from(status.code())
}

/// The stack of the thread that runs a command (#2264): room for the passes over Core IR
/// that run outside the checker's own thread (`una core`, the Core export, the report, the
/// native backend, the copies and drops of the program) at the deepest Core IR the checker
/// accepts.
const MAIN_STACK_BYTES: usize = una_core::depth::STACK_BYTES;

/// A variable of the process environment, for programs that `una run` grants `env`
/// (`spec/14 §3`). The binary reads it, so that the library never does
/// (`compiler/conventions.md §1`).
/// A value that is not valid UTF-8 is `NotText`, which `Env.get` aborts on (S-16, ADR 0086).
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
