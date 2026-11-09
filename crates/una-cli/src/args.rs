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

        /// The memory limit of each solver query in MiB, from 256 to 1048576 (ADR 0124)
        #[arg(long, value_name = "MIB", default_value_t = 4096, value_parser = clap::value_parser!(u64).range(256..=1_048_576))]
        solver_memory: u64,
    },
    /// List the functions derived from a requirement, their contracts and their obligations
    Trace {
        /// The requirement's identifier, for example EVENTS_3
        requirement: String,

        /// Stop a solver query after this many seconds; the obligation is then unknown
        #[arg(long, value_name = "SECONDS", default_value_t = 60)]
        solver_wallclock: u64,

        /// The memory limit of each solver query in MiB, from 256 to 1048576 (ADR 0124)
        #[arg(long, value_name = "MIB", default_value_t = 4096, value_parser = clap::value_parser!(u64).range(256..=1_048_576))]
        solver_memory: u64,
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

        /// The memory limit of each solver query in MiB, from 256 to 1048576 (ADR 0124)
        #[arg(long, value_name = "MIB", default_value_t = 4096, value_parser = clap::value_parser!(u64).range(256..=1_048_576))]
        solver_memory: u64,
    },
    /// List every proof obligation with its status
    Obligations {
        /// The package directory, or a single file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Stop a solver query after this many seconds; the obligation is then unknown
        #[arg(long, value_name = "SECONDS", default_value_t = 60)]
        solver_wallclock: u64,

        /// The memory limit of each solver query in MiB, from 256 to 1048576 (ADR 0124)
        #[arg(long, value_name = "MIB", default_value_t = 4096, value_parser = clap::value_parser!(u64).range(256..=1_048_576))]
        solver_memory: u64,
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

        /// The memory limit of each solver query in MiB, from 256 to 1048576 (ADR 0124)
        #[arg(long, value_name = "MIB", default_value_t = 4096, value_parser = clap::value_parser!(u64).range(256..=1_048_576))]
        solver_memory: u64,
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

        /// The memory a run may hold, in MiB: 4096 by default, or the package's declared
        /// max_memory, which it may lower and never raise
        #[arg(long, value_name = "MIB", value_parser = memory_budget)]
        max_memory: Option<usize>,

        /// The largest HTTP response body, in MiB (`spec/24 §4`)
        #[arg(long, value_name = "MIB", default_value_t = 16, value_parser = at_least_one)]
        max_response: u64,

        /// The total time of each HTTP request, in seconds (`spec/24 §4`)
        #[arg(long, value_name = "SECONDS", default_value_t = 30, value_parser = http_timeout)]
        http_timeout: u64,

        /// The threads that run tasks, from 1 to 1024; by default one per core (ADR 0117)
        #[arg(long, value_name = "N", value_parser = task_number)]
        workers: Option<usize>,

        /// The depth limit of the tasks of `first of` and `start`, from 1 to --max-depth; by default 1000 (ADR 0117)
        #[arg(long, value_name = "N", value_parser = task_number)]
        task_depth: Option<usize>,

        /// The helper threads of `Http`, from 1 to 4096; by default 64 (ADR 0117). A run granted `Http` starts 2 x N + max(2, N / 4) threads for its requests before `main`, besides --workers and --file-threads, so a large N can pass the system's limit on a process's threads, and the run then stops with U6003
        #[arg(long, value_name = "N", value_parser = task_number)]
        io_threads: Option<usize>,

        /// The helper threads of `Files` and `Console`, from 1 to 256; by default 4 (ADR 0117)
        #[arg(long, value_name = "N", value_parser = task_number)]
        file_threads: Option<usize>,

        /// The threads that serve the connections of `serve:` grants, from 1 to 256; by default one per core (ADR 0123). Accepted and unused by a run that does not serve
        #[arg(long, value_name = "N", value_parser = task_number)]
        serve_threads: Option<usize>,

        /// Stop a solver query after this many seconds; the obligation is then unknown
        #[arg(long, value_name = "SECONDS", default_value_t = 60)]
        solver_wallclock: u64,

        /// The memory limit of each solver query in MiB, from 256 to 1048576 (ADR 0124)
        #[arg(long, value_name = "MIB", default_value_t = 4096, value_parser = clap::value_parser!(u64).range(256..=1_048_576))]
        solver_memory: u64,

        /// A budget of this many iterations for each `for ever` loop, the driver's loop included:
        /// each loop instance ends at the start of its iteration N + 1; no host stop request
        /// (ADR 0116)
        #[arg(long, value_name = "N", value_parser = iterations)]
        stop_after_iterations: Option<u64>,

        /// Run on a virtual clock that starts at 0 and never waits (ADR 0116)
        #[arg(long)]
        virtual_time: bool,
        /// With --virtual-time: each check point (call, operation, loop iteration) moves the
        /// virtual clock by one millisecond, so time follows the run's work; the security hunt's
        /// cost-clock oracle (ADR 0117, `spec/27 §8`)
        #[arg(long, requires = "virtual_time")]
        cost_clock: bool,
        /// Request a host stop after this many seconds, from 1 to 31536000 (ADR 0116)
        #[arg(long, value_name = "SECONDS", value_parser = deadline)]
        deadline: Option<u64>,

        /// The seconds from the first host stop request to the hard stop, from 1 to 86400
        /// (ADR 0116)
        #[arg(long, value_name = "SECONDS", default_value_t = una_interp::stopping::DEFAULT_GRACE_SECS, value_parser = stop_grace)]
        stop_grace: u64,

        /// Write the trace to this file as a JSON array, each event as it happens (ADR 0117)
        #[arg(long, value_name = "FILE")]
        trace: Option<PathBuf>,

        /// Replace the listeners of the `serve:` grants with the connections this file describes, one JSON object per line; needs --virtual-time (ADR 0123)
        #[arg(long, value_name = "FILE")]
        serve_script: Option<PathBuf>,

        /// Write what each scripted connection received to this file; needs --serve-script (ADR 0123)
        #[arg(long, value_name = "FILE")]
        serve_out: Option<PathBuf>,
    },
    /// Compile `main` of the `[run] entry` module into a native executable
    Build {
        /// The package directory, or a single file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Write the executable to this file instead of `target/una/<name>`
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,

        /// Let a debugger or profiler attach to the executable (macOS: the get-task-allow entitlement)
        #[arg(long)]
        debuggable: bool,

        /// Stop a solver query after this many seconds; the obligation is then unknown
        #[arg(long, value_name = "SECONDS", default_value_t = 60)]
        solver_wallclock: u64,

        /// The memory limit of each solver query in MiB, from 256 to 1048576 (ADR 0124)
        #[arg(long, value_name = "MIB", default_value_t = 4096, value_parser = clap::value_parser!(u64).range(256..=1_048_576))]
        solver_memory: u64,
    },
    /// Run every example
    Test {
        /// The package directory, or a single file
        #[arg(default_value = ".")]
        path: PathBuf,

        /// The maximum call depth
        #[arg(long, value_name = "N", default_value_t = una_interp::DEFAULT_MAX_DEPTH)]
        max_depth: usize,

        /// The memory a run may hold, in MiB: 4096 by default, or the package's declared
        /// max_memory, which it may lower and never raise
        #[arg(long, value_name = "MIB", value_parser = memory_budget)]
        max_memory: Option<usize>,

        /// The threads that run tasks, from 1 to 1024; by default one per core (ADR 0117)
        #[arg(long, value_name = "N", value_parser = task_number)]
        workers: Option<usize>,

        /// The depth limit of the tasks of `first of` and `start`, from 1 to --max-depth; by default 1000 (ADR 0117)
        #[arg(long, value_name = "N", value_parser = task_number)]
        task_depth: Option<usize>,

        /// The helper threads of `Http`, from 1 to 4096; by default 64 (ADR 0117). A run granted `Http` starts 2 x N + max(2, N / 4) threads for its requests before `main`, besides --workers and --file-threads, so a large N can pass the system's limit on a process's threads, and the run then stops with U6003
        #[arg(long, value_name = "N", value_parser = task_number)]
        io_threads: Option<usize>,

        /// The helper threads of `Files` and `Console`, from 1 to 256; by default 4 (ADR 0117)
        #[arg(long, value_name = "N", value_parser = task_number)]
        file_threads: Option<usize>,

        /// Stop a solver query after this many seconds; the obligation is then unknown
        #[arg(long, value_name = "SECONDS", default_value_t = 60)]
        solver_wallclock: u64,

        /// The memory limit of each solver query in MiB, from 256 to 1048576 (ADR 0124)
        #[arg(long, value_name = "MIB", default_value_t = 4096, value_parser = clap::value_parser!(u64).range(256..=1_048_576))]
        solver_memory: u64,
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
    /// Remove a need from ita.una, then write the lock
    Remove {
        /// The package's name, as the need writes it
        name: String,

        /// The package directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Print the program's packages, with their ceilings, authority and trust
    Tree {
        /// The package directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Write `ita.lock` for the package, or check it with --check
    Lock {
        /// The package directory
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Report where the lock is missing or does not meet `ita.una` (U7009), and write
        /// nothing
        #[arg(long)]
        check: bool,

        /// With --check: also read a checkpoint less than 24 hours old from a place or a
        /// mirror, and fail on a package whose owner's chain froze since the lock was written
        #[arg(long, requires = "check")]
        fresh: bool,

        /// Record the ceiling of this package wider than the one the lock records: an
        /// authority change; repeat it for each package
        #[arg(long, value_name = "PACKAGE")]
        accept_authority: Vec<String>,

        /// Record consent to the trusted modules of this package for the content of this
        /// digest, which the review prints; repeat it for each package
        #[arg(long, value_name = "PACKAGE@DIGEST")]
        accept_trust: Vec<String>,

        /// Record a change of the keys of this package's owner: a key added or revoked, a
        /// threshold, a recovery or a dispute, or another signing key
        #[arg(long, value_name = "PACKAGE")]
        accept_key: Vec<String>,

        /// Print the review of the changes, and write nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Raise the named needs, or all, and write the lock, for the package in the current
    /// directory
    Update {
        /// The packages to raise, as the needs write them; all when none is named
        names: Vec<String>,

        /// Take only releases logged at least this many hours before the resolution point
        #[arg(long, value_name = "HOURS", conflicts_with = "now")]
        cooldown: Option<u64>,

        /// Take releases with no cooldown
        #[arg(long)]
        now: bool,

        /// Record the ceiling of this package wider than the one the lock records: an
        /// authority change; repeat it for each package
        #[arg(long, value_name = "PACKAGE")]
        accept_authority: Vec<String>,

        /// Record consent to the trusted modules of this package for the content of this
        /// digest, which the review prints; repeat it for each package
        #[arg(long, value_name = "PACKAGE@DIGEST")]
        accept_trust: Vec<String>,

        /// Record a change of the keys of this package's owner: a key added or revoked, a
        /// threshold, a recovery or a dispute, or another signing key
        #[arg(long, value_name = "PACKAGE")]
        accept_key: Vec<String>,

        /// Print the review of the changes, and write nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Add a need to ita.una and write the lock, for the package in the current directory
    Add {
        /// The package's name, `name` or `owner/name`
        name: String,

        /// The least version the need admits, as `2.1` or `2.1.0`
        version: Option<String>,

        /// The need's local name
        #[arg(long = "as", value_name = "LOCAL")]
        local: Option<String>,

        /// The source: `itaty` by default, or a source of ita.una's `sources`
        #[arg(long, value_name = "SOURCE")]
        from: Option<String>,

        /// Take a release with no cooldown
        #[arg(long)]
        now: bool,

        /// Print the review of the changes, and write nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Fetch each locked registry package into Itaoca, verified, with its provenance
    Fetch {
        /// The package directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Copy each locked registry package from Itaoca into vendor/ita/, with its provenance
    Vendor {
        /// The package directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Verify each locked registry package again, in vendor/ita/ and in Itaoca
    Verify {
        /// The package directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Check, build, sign and send a release of the package to the registry it names
    Publish {
        /// The package directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Yank a release of the package: new resolutions skip it, and existing locks keep it
    Yank {
        /// The version to yank, a full version
        version: String,

        /// The package directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Check each locked registry package for yanks, advisories and key changes (U7014)
    Audit {
        /// The package directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Print the new entries of an owner's chains, and any promise its registry broke
    Watch {
        /// The owner's name
        owner: String,

        /// More package names to follow besides those the registry lists for the owner
        names: Vec<String>,

        /// The source the owner belongs to
        #[arg(long, value_name = "SOURCE", default_value = "itaty")]
        from: String,
    },
    /// Write each locked registry package, with its provenance, into one file
    Bundle {
        /// The file to write
        file: PathBuf,

        /// The package directory
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Verify a bundle against the lock, and store its packages in Itaoca
    Unbundle {
        /// The bundle to read
        file: PathBuf,

        /// The package directory
        #[arg(default_value = ".")]
        path: PathBuf,
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

/// A value of `--stop-after-iterations`: from 1 to 2^63 - 1 (ADR 0116 rule 19).
fn iterations(text: &str) -> Result<u64, String> {
    let n = at_least_one(text)?;
    if n > i64::MAX.unsigned_abs() {
        return Err(format!("{n} is too many; give at most {}", i64::MAX));
    }
    Ok(n)
}

/// A value of `--deadline`: whole seconds, from 1 to a year of seconds (ADR 0116).
fn deadline(text: &str) -> Result<u64, String> {
    let n = at_least_one(text)?;
    if n > una_interp::stopping::MAX_DEADLINE_SECS {
        return Err(format!(
            "{n} is longer than a year; give at most {}",
            una_interp::stopping::MAX_DEADLINE_SECS
        ));
    }
    Ok(n)
}

/// A value of `--stop-grace`: whole seconds, from 1 to a day (ADR 0116).
fn stop_grace(text: &str) -> Result<u64, String> {
    let n = at_least_one(text)?;
    if n > una_interp::stopping::MAX_GRACE_SECS {
        return Err(format!(
            "{n} is longer than a day; give at most {}",
            una_interp::stopping::MAX_GRACE_SECS
        ));
    }
    Ok(n)
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

/// A value of `--workers`, `--task-depth`, `--io-threads` or `--file-threads`: a whole number;
/// its range is checked with the other flags (`una_interp::Concurrency`, ADR 0117).
fn task_number(text: &str) -> Result<usize, String> {
    text.parse::<usize>().map_err(|error| error.to_string())
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
