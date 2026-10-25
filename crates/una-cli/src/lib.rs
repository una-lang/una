//! Command implementations of the `una` binary (`spec/13 §5`), the only crate that does I/O to
//! the terminal.
//!
//! The binary hands its arguments, standard streams and environment variables to [`run`];
//! nothing here reads the process environment, so every command also runs in-process.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod args;
mod contract;
mod package;
mod proof_key;
mod report;
mod runtime;
mod trace;
mod verification;

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Parser;
use una_check::{
    Header, ModuleInput, PackageInput, PendingObligation, RunInput, check_names, check_types,
};
use una_diag::{Code, Diagnostic, render_human, render_json};
use una_syntax::ast::File;
use una_syntax::{FileId, Interner, Lexed, SourceDb, lex, parse};

use crate::args::{Cli, ColorChoice, Command, TierArg};
pub use crate::package::{LoadError, Package, load};
use crate::verification::Solving;
pub use crate::verification::{DEFAULT_WALLCLOCK, Verification};

/// The exit status of a `una` command (`spec/13 §5`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitStatus {
    /// `0`: success.
    Success,
    /// `1`: errors in the program or failed tests.
    Errors,
    /// `2`: usage or configuration error.
    Usage,
    /// `3`: internal compiler error.
    Internal,
}

impl ExitStatus {
    /// The process exit code.
    pub fn code(self) -> u8 {
        match self {
            ExitStatus::Success => 0,
            ExitStatus::Errors => 1,
            ExitStatus::Usage => 2,
            ExitStatus::Internal => 3,
        }
    }
}

/// The tiers of `una check` (`spec/12 §1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Every check except proof obligations.
    Zero,
    /// Tier 0 and every proof obligation.
    One,
}

/// The outcome of [`check`]: its diagnostics, the sources they point into, the Core IR of
/// the declarations without diagnostics, and at tier 1 the obligations.
#[derive(Debug)]
pub struct Checked {
    /// The diagnostics, in reporting order.
    pub diagnostics: Vec<Diagnostic>,
    /// The loaded sources.
    pub sources: SourceDb,
    /// The Core IR (`spec/09`).
    pub program: una_core::Program,
    /// The obligations and their outcomes, when tier 1 ran.
    pub verification: Option<Verification>,
    /// The obligation sites that the type checker found, which the verifier generates over
    /// Core IR (`spec/12 §3`).
    pub pending: Vec<PendingObligation>,
}

/// Loads and checks the file or package at `path` (`spec/13 §5`, `spec/12 §1`), with the
/// default wall-clock guard of the solver and no proof cache.
pub fn check(path: &Path, tier: Tier) -> Result<Checked, LoadError> {
    check_with(path, tier, DEFAULT_WALLCLOCK)
}

/// Loads and checks the file or package at `path`: tier 0, then, at tier 1 and when tier 0
/// reports nothing, every proof obligation, solved with the guard `wallclock`. No proof cache
/// is read or written: only the command line has the user's key that authenticates one (#1164).
pub fn check_with(path: &Path, tier: Tier, wallclock: Duration) -> Result<Checked, LoadError> {
    check_solving(path, tier, Solving::uncached(wallclock))
}

/// [`check_with`] with the proof cache of `solving`, when it has a key.
fn check_solving(path: &Path, tier: Tier, solving: Solving<'_>) -> Result<Checked, LoadError> {
    let package = load(path)?;
    let (mut diagnostics, program, pending) = on_large_stack(|| {
        let analysis = analyze(&package, None);
        (analysis.diagnostics, analysis.program, analysis.pending)
    });
    let mut verification = None;
    if tier == Tier::One && diagnostics.is_empty() {
        let found = verification::tier_one(&package, &program, solving);
        diagnostics.extend(found.diagnostics);
        verification = found.verification;
    }
    Ok(Checked {
        diagnostics,
        sources: package.sources,
        program,
        verification,
        pending,
    })
}

/// What checking a package finds: its diagnostics, the tree of each module, the names of the
/// trees, the headers of its functions after inference, its Core IR, and the module that
/// `una run` runs.
struct Analysis {
    diagnostics: Vec<Diagnostic>,
    trees: Vec<(FileId, File)>,
    interner: Interner,
    headers: Vec<Header>,
    program: una_core::Program,
    entry: Option<String>,
    pending: Vec<PendingObligation>,
}

/// Checks `package`; with `run_grants`, for `una run`, whose grants are final (U3012).
fn analyze(package: &Package, run_grants: Option<&[String]>) -> Analysis {
    let (manifest, mut diagnostics) = package::project(package);
    let mut interner = Interner::new();
    let mut trees = Vec::new();
    for checked in check_files(&package.sources, &package.modules, &mut interner) {
        let broken = !checked.syntax.is_empty();
        diagnostics.extend(checked.syntax);
        if let Some(canonical) = checked.canonical {
            diagnostics.extend(una_fmt::differences(
                &package.sources,
                checked.id,
                &canonical,
            ));
        }
        trees.push((checked.id, checked.tree, broken));
    }
    // The entry is `[run] entry`, or the module of a single file.
    let entry = if package.single {
        trees.first().and_then(|(_, tree, _)| {
            let header = tree.module.as_ref()?;
            let path: Vec<&str> = header
                .path
                .segments
                .iter()
                .map(|segment| interner.resolve(segment.name))
                .collect();
            Some(path.join("."))
        })
    } else {
        manifest.entry.as_ref().map(|entry| entry.value.clone())
    };
    let run = match (run_grants, entry.as_deref()) {
        (Some(grants), Some(entry)) => Some(RunInput { entry, grants }),
        _ => None,
    };
    let input = PackageInput {
        sources: &package.sources,
        interner: &interner,
        single: package.single,
        modules: trees
            .iter()
            .map(|(id, tree, broken)| ModuleInput {
                id: *id,
                tree,
                broken: *broken,
            })
            .collect(),
        manifest: package.manifest,
        settings: &manifest,
        run,
    };
    diagnostics.extend(check_names(&input));
    let types = check_types(&input);
    diagnostics.extend(types.diagnostics);
    Analysis {
        diagnostics,
        trees: trees.into_iter().map(|(id, tree, _)| (id, tree)).collect(),
        interner,
        headers: types.headers,
        program: types.program,
        entry,
        pending: types.obligations,
    }
}

/// The syntax diagnostics of one file, its tree, and its canonical form when it has no
/// syntax error.
struct FileCheck {
    id: FileId,
    syntax: Vec<Diagnostic>,
    tree: File,
    canonical: Option<String>,
}

/// Lexes and parses the files `modules` of a package, then prints each one that parsed in its
/// canonical form, knowing the constructors that every such file declares (ADR 0020 rule 6).
fn check_files(sources: &SourceDb, modules: &[FileId], interner: &mut Interner) -> Vec<FileCheck> {
    let parsed: Vec<(FileId, Lexed, File, Vec<Diagnostic>)> = modules
        .iter()
        .map(|&id| {
            let lexed = lex(sources.file(id), id, interner);
            let parsed = parse(&lexed, id, interner);
            let syntax: Vec<Diagnostic> = lexed
                .errors
                .iter()
                .chain(&parsed.errors)
                .map(|error| error.to_diagnostic(sources))
                .collect();
            (id, lexed, parsed.file, syntax)
        })
        .collect();
    let constructors = una_fmt::Constructors::new(
        parsed
            .iter()
            .filter(|(_, _, _, syntax)| syntax.is_empty())
            .map(|(_, _, tree, _)| tree),
    );
    parsed
        .into_iter()
        .map(|(id, lexed, tree, syntax)| {
            // A file with syntax errors has no canonical form.
            let canonical = syntax.is_empty().then(|| {
                una_fmt::format_with(sources.file(id), &lexed, &tree, interner, &constructors)
            });
            FileCheck {
                id,
                syntax,
                tree,
                canonical,
            }
        })
        .collect()
}

/// The stack of the thread that checks a package. The passes recurse over the syntax tree,
/// whose depth the parser bounds (`una_syntax::MAX_NESTING`); unoptimized builds use up to
/// about 40 KiB of stack per level.
const CHECK_STACK_BYTES: usize = 64 * 1024 * 1024;

/// Runs `work` on a thread with [`CHECK_STACK_BYTES`] of stack, or on the current thread when
/// the operating system cannot start one.
fn on_large_stack<T: Send>(work: impl Fn() -> T + Sync) -> T {
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .name("una-check".to_owned())
            .stack_size(CHECK_STACK_BYTES)
            .spawn_scoped(scope, &work);
        match spawned {
            Ok(handle) => handle
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload)),
            Err(_) => work(),
        }
    })
}

/// The environment variables of the process, by name, which `una run` gives to programs that
/// use `Env.get` (`spec/14 §3`). The binary passes the real ones; the library never reads them
/// (`compiler/conventions.md §1`). A value that is not valid UTF-8 is [`una_interp::NotText`]:
/// `Env.get` aborts on it (S-16, ADR 0086).
pub type Environment<'a> = &'a (dyn Fn(&str) -> Result<Option<String>, una_interp::NotText> + Sync);

/// The variable `name` for the toolchain's own use: unset when its value is not valid UTF-8.
pub(crate) fn variable(environment: Environment<'_>, name: &str) -> Option<String> {
    environment(name).ok().flatten()
}

/// Runs the `una` command line. `args` starts with the program name; `stderr_is_terminal`
/// decides `--color auto`, since human diagnostics go to stderr (ADR 0015 D1).
///
/// A failure to write the output is reported as a usage error (exit code 2, ADR 0015 D1).
pub fn run<I, T>(
    args: I,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    stderr_is_terminal: bool,
    environment: Environment<'_>,
) -> ExitStatus
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    // The key of the proof cache is resolved when a command verifies, and not before: `una fmt`
    // never makes one (#1164).
    let resolve_key = || proof_key::resolve(environment);
    let status = match Cli::try_parse_from(args) {
        Ok(cli) => match cli.command {
            Command::Check {
                path,
                tier,
                dump_core,
                solver_wallclock,
            } => {
                // Tier 0 never verifies, so it needs no key.
                let key = if matches!(tier, TierArg::One) {
                    resolve_key()
                } else {
                    None
                };
                let options = CheckOptions {
                    tier: match tier {
                        TierArg::Zero => Tier::Zero,
                        TierArg::One => Tier::One,
                    },
                    json: cli.json,
                    color: use_color(cli.color, stderr_is_terminal),
                    solving: Solving {
                        wallclock: Duration::from_secs(solver_wallclock),
                        proof_key: key.as_ref(),
                    },
                };
                check_command(&path, options, dump_core.as_deref(), stdout, stderr)
            }
            Command::Trace {
                requirement,
                solver_wallclock,
            } => {
                let key = resolve_key();
                let options = trace::TraceOptions {
                    json: cli.json,
                    color: use_color(cli.color, stderr_is_terminal),
                    solving: Solving {
                        wallclock: Duration::from_secs(solver_wallclock),
                        proof_key: key.as_ref(),
                    },
                };
                trace::trace_command(&requirement, options, stdout, stderr)
            }
            Command::Report {
                path,
                solver_wallclock,
            } => {
                let key = resolve_key();
                let options = report::ReportOptions {
                    json: cli.json,
                    color: use_color(cli.color, stderr_is_terminal),
                    solving: Solving {
                        wallclock: Duration::from_secs(solver_wallclock),
                        proof_key: key.as_ref(),
                    },
                };
                report::report_command(&path, options, stdout, stderr)
            }
            Command::Obligations {
                path,
                solver_wallclock,
            } => {
                let key = resolve_key();
                let options = verification::ObligationsOptions {
                    json: cli.json,
                    color: use_color(cli.color, stderr_is_terminal),
                    solving: Solving {
                        wallclock: Duration::from_secs(solver_wallclock),
                        proof_key: key.as_ref(),
                    },
                };
                verification::obligations_command(&path, options, stdout, stderr)
            }
            Command::Fmt { path, check } => {
                let options = FmtOptions {
                    check,
                    json: cli.json,
                    color: use_color(cli.color, stderr_is_terminal),
                };
                fmt_command(&path, options, stdout, stderr)
            }
            Command::Contract { name } => {
                let color = use_color(cli.color, stderr_is_terminal);
                contract_command(&name, cli.json, color, stdout, stderr)
            }
            Command::Run {
                path,
                grants,
                max_depth,
                solver_wallclock,
            } => {
                let key = resolve_key();
                let options = runtime::RunOptions {
                    grants: &grants,
                    max_depth,
                    json: cli.json,
                    color: use_color(cli.color, stderr_is_terminal),
                    environment,
                    solving: Solving {
                        wallclock: Duration::from_secs(solver_wallclock),
                        proof_key: key.as_ref(),
                    },
                };
                runtime::run_command(&path, options, stdout, stderr)
            }
            Command::Test {
                path,
                max_depth,
                solver_wallclock,
            } => {
                let key = resolve_key();
                let options = runtime::RunOptions {
                    grants: &[],
                    max_depth,
                    json: cli.json,
                    color: use_color(cli.color, stderr_is_terminal),
                    environment,
                    solving: Solving {
                        wallclock: Duration::from_secs(solver_wallclock),
                        proof_key: key.as_ref(),
                    },
                };
                runtime::test_command(&path, options, stdout, stderr)
            }
            Command::Explain { code } => explain_command(&code, cli.json, stdout, stderr),
        },
        // clap reports `--help` as an error that belongs on stdout with exit code 0.
        Err(error) => {
            let status = if error.exit_code() == 0 {
                ExitStatus::Success
            } else {
                ExitStatus::Usage
            };
            let rendered = error.render();
            let written = if error.use_stderr() {
                write!(stderr, "{rendered}")
            } else {
                write!(stdout, "{rendered}")
            };
            written.map(|()| status)
        }
    };
    status.unwrap_or(ExitStatus::Usage)
}

/// The options of `una check`.
#[derive(Clone, Copy, Debug)]
struct CheckOptions<'a> {
    tier: Tier,
    json: bool,
    color: bool,
    solving: Solving<'a>,
}

/// `una check [path]`: prints the diagnostics of [`check`], as JSON on stdout (ADR 0012) or
/// for people on stderr (ADR 0015 D1).
fn check_command(
    path: &Path,
    options: CheckOptions<'_>,
    dump_core: Option<&str>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    let checked = match check_solving(path, options.tier, options.solving) {
        Ok(checked) => checked,
        // A usage error goes to stderr even with --json; it happens before the
        // command runs, like the usage errors clap reports.
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    // The Core IR of the module's declarations without diagnostics, on stdout.
    if let Some(module) = dump_core {
        match checked.program.modules.iter().find(|m| m.path == module) {
            Some(core) => write!(stdout, "{}", una_core::print_module(core))?,
            None => {
                writeln!(stderr, "error: the package has no module `{module}`")?;
                return Ok(ExitStatus::Usage);
            }
        }
    }
    report(
        &checked.diagnostics,
        &checked.sources,
        options.json,
        options.color,
        stdout,
        stderr,
    )
}

/// Prints diagnostics as JSON on stdout (ADR 0012) or for people on stderr (ADR 0015 D1), and
/// returns the exit status they imply.
fn report(
    diagnostics: &[Diagnostic],
    sources: &SourceDb,
    json: bool,
    color: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    // `spec/13 §5`: an internal compiler error (U8001) exits with 3, whatever else is reported.
    let status = if diagnostics.iter().any(|d| d.code == Code::U8001) {
        ExitStatus::Internal
    } else if diagnostics.is_empty() {
        ExitStatus::Success
    } else {
        ExitStatus::Errors
    };
    if json {
        match render_json(diagnostics) {
            Ok(document) => writeln!(stdout, "{document}")?,
            Err(error) => {
                writeln!(
                    stderr,
                    "internal compiler error (U8001): rendering diagnostics as JSON failed: {error}"
                )?;
                return Ok(ExitStatus::Internal);
            }
        }
        stdout.flush()?;
    } else {
        write!(
            stderr,
            "{}",
            render_human(diagnostics, &sources.texts(), color)
        )?;
        stderr.flush()?;
    }
    Ok(status)
}

/// The options of `una fmt`.
#[derive(Clone, Copy, Debug)]
struct FmtOptions {
    check: bool,
    json: bool,
    color: bool,
}

/// `una fmt [path]` and `una fmt --check [path]` (`spec/13 §5`, ADR 0020).
///
/// Files with syntax errors are reported and left untouched; the others are
/// rewritten when they differ from their canonical form, each through a fresh temporary file
/// and a rename onto the file itself or, for a symbolic link that resolves inside the package,
/// onto its resolved target (a link leading outside it is refused), and nothing is printed on
/// success. `--check` reports U7001 instead of writing.
fn fmt_command(
    path: &Path,
    options: FmtOptions,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    let package = match load(path) {
        Ok(package) => package,
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let sources = &package.sources;
    let checks = on_large_stack(|| {
        let mut interner = Interner::new();
        check_files(sources, &package.modules, &mut interner)
    });
    let mut diagnostics = Vec::new();
    for checked in checks {
        let id = checked.id;
        diagnostics.extend(checked.syntax);
        let Some(canonical) = checked.canonical else {
            continue;
        };
        if options.check {
            diagnostics.extend(una_fmt::differences(sources, id, &canonical));
            continue;
        }
        let file = sources.file(id);
        if file.text() == canonical && file.first_crlf().is_none() {
            continue;
        }
        let Some(target) = package.disk_paths.get(&id) else {
            continue;
        };
        if let Err(error) = write_through_rename(&write_root(path), target, &canonical) {
            writeln!(
                stderr,
                "error: cannot write `{}`: {error}",
                target.display()
            )?;
            return Ok(ExitStatus::Usage);
        }
    }
    report(
        &diagnostics,
        sources,
        options.json,
        options.color,
        stdout,
        stderr,
    )
}

/// Replaces the file `target` with `text` through a fresh temporary file and a rename, so an
/// interrupted write never leaves a truncated file ([`una_diag::safe_write::write_atomically`]).
///
/// The temporary file is created exclusively under a name unique to the write, so anything
/// already at that name, a planted link included, is never written through (#1160).
///
/// A symbolic link at `target` is written through onto the file it resolves to (review finding
/// R-024) only when that file, and the directory it lives in, resolve inside `root`. Otherwise
/// the write is refused, naming the link and where it leads: a cloned repository can choose a
/// target link as easily as a temporary one.
fn write_through_rename(root: &Path, target: &Path, text: &str) -> io::Result<()> {
    una_diag::safe_write::write_atomically(root, target, text.as_bytes(), false, true)
}

/// The package directory a command given `path` writes in: `path` itself, or the directory of a
/// single file.
fn write_root(path: &Path) -> PathBuf {
    if path.is_dir() {
        return path.to_path_buf();
    }
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// `una contract <qualified.name>` (`spec/13 §5`, ADR 0029): checks the package in the current
/// directory at tier 0 and prints the contract of the function, or the diagnostics as
/// `una check` does when there are any. An unknown name is a usage error.
fn contract_command(
    name: &str,
    json: bool,
    color: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    let package = match load(Path::new(".")) {
        Ok(package) => package,
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let analysis = on_large_stack(|| analyze(&package, None));
    // R-027, ADR 0036: the package's diagnostics, those of its project files included, come
    // before the name is resolved, so that an unknown name never hides them.
    if !analysis.diagnostics.is_empty() {
        return report(
            &analysis.diagnostics,
            &package.sources,
            json,
            color,
            stdout,
            stderr,
        );
    }
    let modules = contract::Modules {
        sources: &package.sources,
        interner: &analysis.interner,
        trees: &analysis.trees,
    };
    let Some((module, decl, function)) = modules.function(name) else {
        writeln!(
            stderr,
            "error: the package has no function `{name}`; name it by module path and name, as in `events.checkin.check_in`"
        )?;
        return Ok(ExitStatus::Usage);
    };
    let function_name = analysis.interner.resolve(function.name.name);
    let header = analysis
        .headers
        .iter()
        .find(|h| h.module == module && h.name == function_name);
    let contract = contract::contract(&modules, &module, decl, function, header);
    if json {
        match serde_json::to_string(&contract) {
            Ok(document) => writeln!(stdout, "{document}")?,
            Err(error) => {
                writeln!(
                    stderr,
                    "internal compiler error (U8001): rendering the contract as JSON failed: {error}"
                )?;
                return Ok(ExitStatus::Internal);
            }
        }
    } else {
        write!(stdout, "{}", contract.human())?;
    }
    stdout.flush()?;
    Ok(ExitStatus::Success)
}

/// `una.explain/1`, with keys in this order (ADR 0034 rule 4).
#[derive(serde::Serialize)]
struct ExplainDocument {
    schema: &'static str,
    code: &'static str,
    domain: &'static str,
    meaning: &'static str,
    text: &'static str,
}

/// `una explain <code>`: prints the explanation of a registered code (`spec/13 §4`), or with
/// `--json` the document `una.explain/1`.
fn explain_command(
    code: &str,
    json: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    match code.parse::<Code>() {
        // `una.explain/1` with `--json`, as every command has a document.
        Ok(code) if json => {
            let document = ExplainDocument {
                schema: "una.explain/1",
                code: code.as_str(),
                domain: code.domain().as_str(),
                meaning: code.meaning(),
                text: code.explain_text(),
            };
            match serde_json::to_string(&document) {
                Ok(text) => writeln!(stdout, "{text}")?,
                Err(error) => {
                    writeln!(
                        stderr,
                        "internal compiler error (U8001): rendering the explanation as JSON failed: {error}"
                    )?;
                    return Ok(ExitStatus::Internal);
                }
            }
            stdout.flush()?;
            Ok(ExitStatus::Success)
        }
        Ok(code) => {
            write!(stdout, "{}", code.explain_text())?;
            stdout.flush()?;
            Ok(ExitStatus::Success)
        }
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            Ok(ExitStatus::Usage)
        }
    }
}

/// Whether human output uses color, for a `--color` choice (`spec/13 §5`).
fn use_color(choice: ColorChoice, is_terminal: bool) -> bool {
    match choice {
        ColorChoice::Never => false,
        ColorChoice::Auto => is_terminal,
        ColorChoice::Always => true,
    }
}
