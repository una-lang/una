//! The command line syntax of `spec/13 §5`, for the commands that exist so far.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// Una, a verification-first programming language for machine-generated software.
#[derive(Debug, Parser)]
#[command(name = "una", disable_help_subcommand = true)]
pub(crate) struct Cli {
    /// Print machine output as JSON on stdout, and nothing else on stdout
    #[arg(long, global = true)]
    pub(crate) json: bool,

    /// When to use colors in human output
    #[arg(
        long,
        global = true,
        value_enum,
        value_name = "WHEN",
        default_value_t = ColorChoice::Auto
    )]
    pub(crate) color: ColorChoice,

    #[command(subcommand)]
    pub(crate) command: Command,
}

/// The values of `--color`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ColorChoice {
    /// Never use colors.
    Never,
    /// Use colors when the output is a terminal.
    Auto,
    /// Always use colors.
    Always,
}

/// The commands of `spec/13 §5` implemented so far.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Check a package: tiers 0 and 1
    Check {
        /// The package directory, or a single file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// The tier to check: 0 skips the solver, 1 also proves every obligation
        #[arg(long, value_enum, value_name = "TIER", default_value_t = TierArg::One)]
        tier: TierArg,

        /// Print the Core IR of this module on stdout
        #[arg(long, value_name = "MODULE")]
        dump_core: Option<String>,

        /// Stop a solver query after this many seconds; the obligation is then unknown
        #[arg(long, value_name = "SECONDS", default_value_t = 60)]
        solver_wallclock: u64,
    },
    /// List the functions derived from a requirement, their contracts and their obligations
    Trace {
        /// The requirement's identifier, for example EVENTS_3
        requirement: String,

        /// Stop a solver query after this many seconds; the obligation is then unknown
        #[arg(long, value_name = "SECONDS", default_value_t = 60)]
        solver_wallclock: u64,
    },
    /// Print the build report: proofs, authority, information flow, trust and coverage
    Report {
        /// The package directory, or a single file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Stop a solver query after this many seconds; the obligation is then unknown
        #[arg(long, value_name = "SECONDS", default_value_t = 60)]
        solver_wallclock: u64,
    },
    /// List every proof obligation with its status
    Obligations {
        /// The package directory, or a single file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Stop a solver query after this many seconds; the obligation is then unknown
        #[arg(long, value_name = "SECONDS", default_value_t = 60)]
        solver_wallclock: u64,
    },
    /// Rewrite files in canonical form, or report where they differ from it
    Fmt {
        /// The package directory, or a single file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Report non-canonical code (U7001) without writing any file
        #[arg(long)]
        check: bool,
    },
    /// Run `main` of the `[run] entry` module
    Run {
        /// The package directory, or a single file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Grant a host capability: `console`, `clock`, `env`, `keys:<file>` or `files:<path>:<access>`
        #[arg(long = "grant", value_name = "SPEC")]
        grants: Vec<String>,

        /// The maximum call depth
        #[arg(long, value_name = "N", default_value_t = una_interp::DEFAULT_MAX_DEPTH)]
        max_depth: usize,

        /// The memory a run may hold, in MiB
        #[arg(long, value_name = "MIB", default_value_t = una_interp::DEFAULT_MAX_MEMORY, value_parser = memory_budget)]
        max_memory: usize,

        /// Stop a solver query after this many seconds; the obligation is then unknown
        #[arg(long, value_name = "SECONDS", default_value_t = 60)]
        solver_wallclock: u64,
    },
    /// Compile `main` of the `[run] entry` module into a native executable
    Build {
        /// The package directory, or a single file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Write the executable to this file instead of `target/una/<name>`
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,

        /// Stop a solver query after this many seconds; the obligation is then unknown
        #[arg(long, value_name = "SECONDS", default_value_t = 60)]
        solver_wallclock: u64,
    },
    /// Run every example
    Test {
        /// The package directory, or a single file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// The maximum call depth
        #[arg(long, value_name = "N", default_value_t = una_interp::DEFAULT_MAX_DEPTH)]
        max_depth: usize,

        /// The memory a run may hold, in MiB
        #[arg(long, value_name = "MIB", default_value_t = una_interp::DEFAULT_MAX_MEMORY, value_parser = memory_budget)]
        max_memory: usize,

        /// Stop a solver query after this many seconds; the obligation is then unknown
        #[arg(long, value_name = "SECONDS", default_value_t = 60)]
        solver_wallclock: u64,
    },
    /// Print the full header of a function, declared or inferred, with doc comments and
    /// requirements
    Contract {
        /// The module path followed by the function's name, for example events.checkin.check_in
        name: String,
    },
    /// Explain a diagnostic code
    Explain {
        /// A diagnostic code, for example U0001
        code: String,
    },
    /// Serve the language server on stdin and stdout, for editors (ADR 0063)
    Lsp,
}

/// The values of `--tier` (`spec/12 §1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum TierArg {
    /// Tier 0: every check except proof obligations.
    #[value(name = "0")]
    Zero,
    /// Tier 1: tier 0 and every proof obligation.
    #[value(name = "1")]
    One,
}

/// A value of `--max-memory`: a whole number of MiB, at least 1, since no run fits in 0 MiB
/// (#185, ADR 0061).
fn memory_budget(text: &str) -> Result<usize, String> {
    match text.parse::<usize>() {
        Ok(0) => Err("0 MiB holds nothing; give at least 1 MiB".to_owned()),
        Ok(mib) => Ok(mib),
        Err(error) => Err(error.to_string()),
    }
}
