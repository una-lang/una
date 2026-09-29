//! Command implementations of the `una` binary (`spec/13 §5`), the only crate that does I/O to
//! the terminal.
//!
//! The binary hands its arguments and standard streams to [`run`]; nothing here reads the
//! process environment, so every command also runs in-process.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod args;
mod contract;
mod package;

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;

use clap::Parser;
use una_check::{Header, ModuleInput, PackageInput, check_names, check_types};
use una_diag::{Code, Diagnostic, render_human, render_json};
use una_syntax::ast::File;
use una_syntax::{FileId, Interner, SourceDb, SourceFile, lex, parse};

use crate::args::{Cli, ColorChoice, Command, TierArg};
pub use crate::package::{LoadError, Package, load};

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

/// The outcome of [`check`]: its diagnostics and the sources they point into.
#[derive(Debug)]
pub struct Checked {
    /// The diagnostics, in reporting order.
    pub diagnostics: Vec<Diagnostic>,
    /// The loaded sources.
    pub sources: SourceDb,
}

/// Loads and checks the file or package at `path` (`spec/13 §5`).
///
/// Loading follows ADR 0015 D3 until M3. Each file is lexed and parsed (`spec/01`, `spec/02`);
/// tier 1 differs from tier 0 once proof obligations exist (M8).
pub fn check(path: &Path, _tier: Tier) -> Result<Checked, LoadError> {
    let package = load(path)?;
    let diagnostics = on_large_stack(|| analyze(&package).diagnostics);
    Ok(Checked {
        diagnostics,
        sources: package.sources,
    })
}

/// What checking a package finds: its diagnostics, the tree of each module, the names of the
/// trees, and the headers of its functions after inference.
struct Analysis {
    diagnostics: Vec<Diagnostic>,
    trees: Vec<(FileId, File)>,
    interner: Interner,
    headers: Vec<Header>,
}

fn analyze(package: &Package) -> Analysis {
    let (manifest, mut diagnostics) = package::project(package);
    let mut interner = Interner::new();
    let mut trees = Vec::new();
    for &id in &package.modules {
        let file = package.sources.file(id);
        let checked = check_file(&package.sources, id, file, &mut interner);
        let broken = !checked.syntax.is_empty();
        diagnostics.extend(checked.syntax);
        if let Some(canonical) = checked.canonical {
            diagnostics.extend(una_fmt::differences(&package.sources, id, &canonical));
        }
        trees.push((id, checked.tree, broken));
    }
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
    };
    diagnostics.extend(check_names(&input));
    let types = check_types(&input);
    diagnostics.extend(types.diagnostics);
    Analysis {
        diagnostics,
        trees: trees.into_iter().map(|(id, tree, _)| (id, tree)).collect(),
        interner,
        headers: types.headers,
    }
}

/// The syntax diagnostics of one file, its tree, and its canonical form when it has no
/// syntax error.
struct FileCheck {
    syntax: Vec<Diagnostic>,
    tree: File,
    canonical: Option<String>,
}

fn check_file(
    sources: &SourceDb,
    id: FileId,
    file: &SourceFile,
    interner: &mut Interner,
) -> FileCheck {
    let lexed = lex(file, id, interner);
    let parsed = parse(&lexed, id, interner);
    let syntax: Vec<Diagnostic> = lexed
        .errors
        .iter()
        .chain(&parsed.errors)
        .map(|error| error.to_diagnostic(sources))
        .collect();
    // DECISION D-027: a file with syntax errors has no canonical form.
    let canonical = syntax
        .is_empty()
        .then(|| una_fmt::format(file, &lexed, &parsed.file, interner));
    FileCheck {
        syntax,
        tree: parsed.file,
        canonical,
    }
}

/// The stack of the thread that checks a package. The passes recurse over the syntax tree,
/// whose depth the parser bounds (`una_syntax::MAX_NESTING`); unoptimized builds use up to
/// about 40 KiB of stack per level (DECISION D-022).
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

/// Runs the `una` command line. `args` starts with the program name; `stderr_is_terminal`
/// decides `--color auto`, since human diagnostics go to stderr (ADR 0015 D1).
///
/// A failure to write the output is reported as a usage error (exit code 2, ADR 0015 D1).
pub fn run<I, T>(
    args: I,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    stderr_is_terminal: bool,
) -> ExitStatus
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let status = match Cli::try_parse_from(args) {
        Ok(cli) => match cli.command {
            Command::Check { path, tier } => {
                let options = CheckOptions {
                    tier: match tier {
                        TierArg::Zero => Tier::Zero,
                        TierArg::One => Tier::One,
                    },
                    json: cli.json,
                    color: use_color(cli.color, stderr_is_terminal),
                };
                check_command(&path, options, stdout, stderr)
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
struct CheckOptions {
    tier: Tier,
    json: bool,
    color: bool,
}

/// `una check [path]`: prints the diagnostics of [`check`], as JSON on stdout (ADR 0012) or
/// for people on stderr (ADR 0015 D1).
fn check_command(
    path: &Path,
    options: CheckOptions,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    let checked = match check(path, options.tier) {
        Ok(checked) => checked,
        // DECISION D-010: a usage error goes to stderr even with --json; it happens before the
        // command runs, like the usage errors clap reports.
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
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
    let status = if diagnostics.is_empty() {
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
/// DECISION D-029: files with syntax errors are reported and left untouched; the others are
/// rewritten when they differ from their canonical form, each through a temporary file and a
/// rename, and nothing is printed on success. `--check` reports U7001 instead of writing.
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
        package
            .modules
            .iter()
            .map(|&id| (id, check_file(sources, id, sources.file(id), &mut interner)))
            .collect::<Vec<_>>()
    });
    let mut diagnostics = Vec::new();
    for (id, checked) in checks {
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
        if let Err(error) = write_through_rename(target, &canonical) {
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

/// Replaces the file `target` with `text`, writing a hidden temporary file next to it first so
/// that an interrupted write never leaves a truncated source.
fn write_through_rename(target: &Path, text: &str) -> io::Result<()> {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = target.with_file_name(format!(".{name}.fmt-tmp"));
    std::fs::write(&temporary, text)?;
    if let Ok(metadata) = std::fs::metadata(target) {
        std::fs::set_permissions(&temporary, metadata.permissions())?;
    }
    std::fs::rename(&temporary, target)
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
    let analysis = on_large_stack(|| analyze(&package));
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

/// `una explain <code>`: prints the explanation of a registered code (`spec/13 §4`).
fn explain_command(
    code: &str,
    json: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    if json {
        writeln!(stderr, "error: una explain has no --json output")?;
        return Ok(ExitStatus::Usage);
    }
    match code.parse::<Code>() {
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
