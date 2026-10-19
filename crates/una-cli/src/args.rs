//! The command line syntax of `spec/13 §5`, for the commands that exist so far.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// Una, a verification-first programming language for machine-generated software.
#[derive(Debug, Parser)]
#[command(
    name = "una",
    disable_help_subcommand = true,
    disable_version_flag = true,
    override_usage = "una [OPTIONS] <COMMAND>\n       una --version [--json]"
)]
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

    /// Print the version, the editions and the pinned Z3 version, and exit
    #[arg(long)]
    pub(crate) version: bool,

    #[command(subcommand)]
    pub(crate) command: Option<Command>,
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
    /// Print the package's Core IR; with --json, as `una.core/1` (ADR 0083)
    Core {
        /// The package directory, or a single file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// With --json, also export the declarations rejected for information flow, under
        /// `rejected`, apart from Core (ADR 0091)
        #[arg(long)]
        with_rejected: bool,
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
    /// Apply the mechanical repairs of `una check`, then format, and report what remains
    Fix {
        /// The package directory, or a single file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// The tier of the final check: 0 skips the solver, 1 also proves every obligation
        #[arg(long, value_enum, value_name = "TIER", default_value_t = TierArg::One)]
        tier: TierArg,

        /// Report what would be applied, and write no file
        #[arg(long)]
        dry_run: bool,

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
    /// The package manager of edition 0.4: dependencies, `ita.una` and `ita.lock`
    Ita {
        #[command(subcommand)]
        command: ItaCommand,
    },
    /// Move a package from una.toml to ita.una and ita.lock, edition 0.4
    Migrate {
        /// The package directory
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Print the files and the diagnostics, and write nothing
        #[arg(long)]
        dry_run: bool,

        /// The package's name in ita.una, when its una.toml name is not one
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
    /// Run `main` of the `[run] entry` module
    Run {
        /// The package directory, or a single file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Grant a host capability: `console`, `clock`, `env`, `keys:<file>`, `files:<path>:<access>` or `http:<origin>[:<methods>]`
        #[arg(long = "grant", value_name = "SPEC")]
        grants: Vec<String>,

        /// The maximum call depth
        #[arg(long, value_name = "N", default_value_t = una_interp::DEFAULT_MAX_DEPTH)]
        max_depth: usize,

        /// The memory a run may hold, in MiB
        #[arg(long, value_name = "MIB", default_value_t = una_interp::DEFAULT_MAX_MEMORY, value_parser = memory_budget)]
        max_memory: usize,

        /// The largest HTTP response body, in MiB (`spec/24 §4`)
        #[arg(long, value_name = "MIB", default_value_t = 16, value_parser = at_least_one)]
        max_response: u64,

        /// The total time of each HTTP request, in seconds (`spec/24 §4`)
        #[arg(long, value_name = "SECONDS", default_value_t = 30, value_parser = http_timeout)]
        http_timeout: u64,

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

/// The commands of `una ita` implemented so far (ADR 0122 rule 57).
#[derive(Debug, Subcommand)]
pub(crate) enum ItaCommand {
    /// Write `ita.lock` for the package, or check it with --check
    Lock {
        /// The package directory
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Report where the lock is missing or does not meet `ita.una` (U7009), and write
        /// nothing
        #[arg(long)]
        check: bool,

        /// Record a ceiling wider than the one the lock records: an authority change
        #[arg(long)]
        accept_authority: bool,
    },
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

/// A value of `--max-response` or `--http-timeout`: a whole number, at least 1 (`spec/24 §4`).
fn at_least_one(text: &str) -> Result<u64, String> {
    match text.parse::<u64>() {
        Ok(0) => Err("0 allows nothing; give at least 1".to_owned()),
        Ok(n) => Ok(n),
        Err(error) => Err(error.to_string()),
    }
}

/// A value of `--http-timeout`: a whole number of seconds, from 1 to a day (`spec/24 §4`).
fn http_timeout(text: &str) -> Result<u64, String> {
    let n = at_least_one(text)?;
    if n > una_http::client::MAX_TIMEOUT_SECS {
        return Err(format!(
            "{n} is longer than a day; give at most {}",
            una_http::client::MAX_TIMEOUT_SECS
        ));
    }
    Ok(n)
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
