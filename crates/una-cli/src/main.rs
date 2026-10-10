//! The `una` command line tool (`spec/13 §5`).
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::io::IsTerminal;
use std::process::ExitCode;

fn main() -> ExitCode {
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    // `--color auto` follows stderr, where human diagnostics go (ADR 0015 D1).
    let stderr_is_terminal = stderr.is_terminal();
    let status = una_cli::run(
        std::env::args_os(),
        &mut stdout.lock(),
        &mut stderr.lock(),
        stderr_is_terminal,
        &environment,
    );
    ExitCode::from(status.code())
}

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
