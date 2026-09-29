//! The `una` command line tool (`spec/13 §5`).
#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::io::IsTerminal;
use std::process::ExitCode;

fn main() -> ExitCode {
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    // DECISION D-012: `--color auto` follows stderr, where human diagnostics go (ADR 0015 D1).
    let stderr_is_terminal = stderr.is_terminal();
    let status = una_cli::run(
        std::env::args_os(),
        &mut stdout.lock(),
        &mut stderr.lock(),
        stderr_is_terminal,
    );
    ExitCode::from(status.code())
}
