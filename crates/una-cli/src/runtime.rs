//! `una run` and `una test` (`spec/13 §5`, `spec/11`): they check the package at tier 1
//! (`spec/12 §1`), then evaluate it on the Una Abstract Machine.
//!
//! The human output of both commands, and their JSON
//! documents `una.run/1` and `una.test/1`.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use una_core::{Base, Decl, Fun, Program, Type};
use una_diag::{Code, Diagnostic, Location, in_output_order, render_human};
use una_interp::{
    Abort, Completion, EventDoc, ExampleRun, FileFault, Grant, Host, NotText, OutcomeDoc, Pairs,
    World,
};
use una_syntax::ast::Convention;
use una_syntax::{SourceDb, Span};

use crate::contract::Modules;
use crate::verification::tier_one;
use crate::{Analysis, Environment, ExitStatus, Package, analyze, load, on_large_stack};

/// The exit status of a package that the checks reject: 3 for an internal error, 1 otherwise.
pub(crate) fn rejected_status(diagnostics: &[Diagnostic]) -> ExitStatus {
    if diagnostics.iter().any(|d| d.code == Code::U8001) {
        ExitStatus::Internal
    } else {
        ExitStatus::Errors
    }
}

/// The JSON schema of `una run --json` (ADR 0032).
const RUN_SCHEMA: &str = "una.run/1";

/// The JSON schema of `una test --json` (ADR 0032).
const TEST_SCHEMA: &str = "una.test/1";

/// The tier that `una run` and `una test` check before evaluating: tier 1, since M8 ended the
/// interim tier 0 of ADR 0027 (rule 4).
const TIER: u8 = 1;

/// What `una run` needs besides the package.
#[derive(Clone)]
pub(crate) struct RunOptions<'a> {
    /// The specs of `--grant` flags, in order.
    pub(crate) grants: &'a [String],
    /// `--max-depth`.
    pub(crate) max_depth: usize,
    /// `--workers`, `--task-depth`, `--io-threads` and `--file-threads`, as given.
    pub(crate) tasks: TaskFlags,
    /// `--max-memory`, in MiB.
    pub(crate) max_memory: usize,
    /// `--max-response` and `--http-timeout` (`spec/24 §4`).
    pub(crate) http: una_http::client::Settings,
    /// The heap of the process, as the binary counts it.
    pub(crate) heap: crate::Heap<'a>,
    /// `--json`.
    pub(crate) json: bool,
    /// Whether human diagnostics use color.
    pub(crate) color: bool,
    /// The environment variables for `Env.get`.
    pub(crate) environment: Environment<'a>,
    /// How tier 1 runs the solver (ADR 0064).
    pub(crate) solving: crate::verification::Solving,
    /// `--stop-after-iterations`: the host's stop after this many iterations have started
    /// (ADR 0116 rule 19).
    pub(crate) stop_after: Option<u64>,
    /// `--virtual-time`: the run's clock is virtual (ADR 0116 rule 19, `spec/26 §8`). A test's
    /// clock is virtual whatever this says.
    pub(crate) virtual_time: bool,
    /// `--deadline`: a host stop request after this many seconds (ADR 0116 rule 14).
    pub(crate) deadline: Option<u64>,
    /// `--stop-grace`: the seconds from the first host stop request to the hard stop (rule 16).
    pub(crate) stop_grace: u64,
}

/// The flags of tasks as given on the command line, each `None` when absent (ADR 0117).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TaskFlags {
    pub(crate) workers: Option<usize>,
    pub(crate) task_depth: Option<usize>,
    pub(crate) io_threads: Option<usize>,
    pub(crate) file_threads: Option<usize>,
}

impl TaskFlags {
    /// The checked settings for a run of `max_depth`, or the message of the usage error. The
    /// interpreter runs no coroutine before `first of` and `start` (ADR 0117 rule 11), so it
    /// only checks them, and refuses what the executable refuses.
    fn check(self, max_depth: usize) -> Result<una_interp::Concurrency, String> {
        una_interp::Concurrency::from_flags(
            max_depth,
            self.workers,
            self.task_depth,
            self.io_threads,
            self.file_threads,
        )
    }
}

/// The world of `una run`: printed lines go to the thread that writes stdout, environment
/// variables come from the binary, and the other host operations reach the machine directly.
struct HostWorld<'a> {
    lines: Sender<String>,
    environment: Environment<'a>,
}

impl World for HostWorld<'_> {
    fn print(&mut self, line: &str) {
        // The receiver only goes away once stdout has failed, and that is reported then.
        let _ = self.lines.send(line.to_owned());
    }

    fn now(&mut self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            })
    }

    fn env_var(&mut self, name: &str) -> Result<Option<String>, NotText> {
        if name.is_empty() || name.contains(['=', '\0']) {
            return Ok(None);
        }
        (self.environment)(name)
    }

    fn read_file(&mut self, path: &Path) -> Result<String, FileFault> {
        // Content that is not valid UTF-8 is `NotText`, which aborts the run, not `Io`, which
        // the program could branch on (S-16, ADR 0086).
        let bytes = std::fs::read(path).map_err(file_fault)?;
        String::from_utf8(bytes).map_err(|_| FileFault::NotText)
    }

    fn write_file(&mut self, path: &Path, content: &str) -> Result<(), FileFault> {
        // `spec/14 §3`, ADR 0038 D9 rule 3: a file that does not exist is created with
        // `create_new` (`O_CREAT | O_EXCL`), which fails on a symbolic link, dangling or not,
        // so a write never creates a file through a link. An existing file is truncated and
        // never created.
        let exists = std::fs::symlink_metadata(path).is_ok();
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(exists)
            .create_new(!exists)
            .open(path)
            .map_err(file_fault)?;
        file.write_all(content.as_bytes()).map_err(file_fault)
    }

    fn canonical(&mut self, path: &Path) -> Option<PathBuf> {
        std::fs::canonicalize(path).ok()
    }

    fn is_link(&mut self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
    }
}

fn file_fault(error: io::Error) -> FileFault {
    if error.kind() == io::ErrorKind::NotFound {
        FileFault::NotFound
    } else {
        FileFault::Io
    }
}

/// The limits of a run (`spec/11 §7`, ADR 0061).
fn limits<'a>(
    options: &RunOptions<'a>,
    stop: Option<&'a std::sync::Arc<una_interp::stopping::RunStop>>,
) -> una_interp::Limits<'a> {
    una_interp::Limits {
        max_depth: options.max_depth,
        max_memory: options.max_memory,
        held: options.heap.held,
        http: options.http,
        stop_after: options.stop_after,
        // Checked before the run starts, as a usage error.
        task_depth: options
            .tasks
            .check(options.max_depth)
            .map_or(options.max_depth, |settings| settings.task_depth),
        stop,
    }
}

/// The HTTP settings of `--max-response <MiB>` and `--http-timeout <seconds>` (`spec/24 §4`).
pub(crate) fn http_settings(max_response: u64, http_timeout: u64) -> una_http::client::Settings {
    let defaults = una_http::client::Settings::default();
    una_http::client::Settings {
        limits: una_http::parse::Limits {
            max_body: usize::try_from(max_response.saturating_mul(1024 * 1024))
                .unwrap_or(usize::MAX),
            ..defaults.limits
        },
        timeout: std::time::Duration::from_secs(http_timeout),
    }
}

/// The hard ceiling of the heap while a run evaluates (ADR 0061 rule 3): twice the budget past
/// what the process holds when the run starts. One step between two checks can allocate beyond
/// the budget; past the ceiling, allocation fails and the process aborts. Dropping it lifts the
/// ceiling again.
struct Ceiling<'a> {
    heap: crate::Heap<'a>,
}

impl<'a> Ceiling<'a> {
    fn raise(options: &RunOptions<'a>) -> Ceiling<'a> {
        let budget = options.max_memory.saturating_mul(1024 * 1024);
        let held = (options.heap.held)();
        (options.heap.ceiling)(held.saturating_add(budget.saturating_mul(2)));
        Ceiling { heap: options.heap }
    }
}

impl Drop for Ceiling<'_> {
    fn drop(&mut self) {
        (self.heap.ceiling)(usize::MAX);
    }
}

/// `una run [path]`: checks the package at tier 1 with the grants of `[run] grants` and
/// `--grant` (U3012), then runs `main` of the entry module (`spec/04 §8`).
pub(crate) fn run_command(
    path: &Path,
    options: RunOptions<'_>,
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
    // `[run] grants` resolve against the package root, `--grant` flags against
    // the current directory, and they add up.
    let (manifest, _) = crate::package::project(&package);
    let root = package
        .manifest
        .and_then(|id| package.disk_paths.get(&id))
        .and_then(|path| path.parent())
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut specs = Vec::new();
    let mut grants = Vec::new();
    for spec in &manifest.grants {
        if let Some(grant) = Grant::parse(&spec.value, &root) {
            specs.push(spec.value.clone());
            grants.push(grant);
        }
    }
    for spec in options.grants {
        let Some(grant) = Grant::parse(spec, Path::new("")) else {
            writeln!(
                stderr,
                "error: `{spec}` is not a grant; grants are console, clock, env, keys:<file>, files:<path>:<access> and http:<origin>[:<methods>]"
            )?;
            return Ok(ExitStatus::Usage);
        };
        specs.push(spec.clone());
        grants.push(grant);
    }
    let mut analysis = on_large_stack(|| analyze(&package, Some(&specs)));
    if analysis.diagnostics.is_empty() {
        let found = tier_one(&package, &analysis.program, &options.solving);
        analysis.diagnostics = found.diagnostics;
    }
    if !analysis.diagnostics.is_empty() {
        let document = RunDocument::rejected(&analysis.diagnostics);
        let status = rejected_status(&analysis.diagnostics);
        return finish(
            &document,
            &analysis.diagnostics,
            &package.sources,
            options,
            (stdout, stderr),
            status,
        );
    }
    // The module the host drives (ADR 0116 rule 3), whose `init` starts the run; or the entry,
    // `[run] entry` or the module of a single file, whose `main` runs.
    let drive = manifest.drive.as_ref().map(|drive| una_interp::Drive {
        ticks: drive.ticks,
        budget: drive.budget,
    });
    let (entry, start) = if let Some(driven) = &manifest.drive {
        let module = driven.module.value.clone();
        match driven_init(&analysis, &package.sources, &module) {
            Ok(init) => (module, init),
            Err(message) => {
                writeln!(stderr, "error: {message}")?;
                return Ok(ExitStatus::Usage);
            }
        }
    } else {
        let Some(entry) = analysis.entry.clone() else {
            writeln!(
                stderr,
                "error: una.toml has no `[run] entry`; name the module whose `main` runs"
            )?;
            return Ok(ExitStatus::Usage);
        };
        let Some(main) = entry_main(&analysis, &package.sources, &entry) else {
            writeln!(
                stderr,
                "error: the module `{entry}` has no `pub main() -> Unit` to run"
            )?;
            return Ok(ExitStatus::Usage);
        };
        (entry, main)
    };
    let keys = match load_keys(&grants) {
        Ok(keys) => keys,
        Err(message) => {
            writeln!(stderr, "error: {message}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let host = Host {
        grants,
        keys,
        virtual_time: options.virtual_time,
    };
    let Some(stack) = una_interp::stack_bytes(options.max_depth) else {
        writeln!(
            stderr,
            "error: `--max-depth {}` needs more stack than the system can address",
            options.max_depth
        )?;
        return Ok(ExitStatus::Usage);
    };
    if let Err(message) = options.tasks.check(options.max_depth) {
        writeln!(stderr, "error: {message}")?;
        return Ok(ExitStatus::Usage);
    }
    // ADR 0116 rule 18: a run that can reach a `for ever` or runs a driver installs the
    // handlers of SIGINT and SIGTERM; any other keeps their default actions.
    // The run's host stop state is made first, so a signal that comes while the handlers are
    // installed counts for the run.
    let stops = takes_stops(&analysis, &package.sources, &entry, drive.is_some());
    let stop = una_interp::stopping::RunStop::new(options.stop_grace);
    if stops && let Err(una_sys::signals::InstallError(message)) = una_sys::signals::install() {
        writeln!(stderr, "internal compiler error (U8001): {message}")?;
        return Ok(ExitStatus::Internal);
    }
    // The run's watcher when a request can come: a signal, or the deadline (rules 14 and 16).
    // Its guard ends the watch on every way out, a panic's included.
    let watching = if stops || options.deadline.is_some() {
        match stop.watch(options.deadline.map(std::time::Duration::from_secs)) {
            Ok(watching) => Some(watching),
            Err(error) => {
                writeln!(
                    stderr,
                    "internal compiler error (U8001): cannot start the watcher of host stops: {error}"
                )?;
                return Ok(ExitStatus::Internal);
            }
        }
    } else {
        None
    };
    let (sender, receiver) = channel::<String>();
    let program = &analysis.program;
    let entry_path = entry.as_str();
    let host = &host;
    let max_depth = options.max_depth;
    let limits = limits(&options, Some(&stop));
    let environment = options.environment;
    let mut output = Vec::new();
    let mut write_error = None;
    let ceiling = Ceiling::raise(&options);
    let evaluated = std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("una-run".to_owned())
            .stack_size(stack)
            .spawn_scoped(scope, move || {
                let mut world = HostWorld {
                    lines: sender,
                    environment,
                };
                match drive {
                    Some(drive) => {
                        una_interp::run_drive(program, entry_path, &mut world, host, limits, drive)
                            .ok()
                    }
                    None => una_interp::run_main(program, entry_path, &mut world, host, limits),
                }
            })?;
        for line in receiver {
            if options.json {
                output.push(line);
            } else if write_error.is_none()
                && let Err(error) = writeln!(stdout, "{line}").and_then(|()| stdout.flush())
            {
                write_error = Some(error);
            }
        }
        Ok::<_, io::Error>(
            worker
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload)),
        )
    });
    drop(ceiling);
    drop(watching);
    let run = match evaluated {
        Ok(Some(run)) => run,
        Ok(None) => {
            writeln!(stderr, "error: the module `{entry}` has no `main` to run")?;
            return Ok(ExitStatus::Usage);
        }
        Err(error) => {
            writeln!(
                stderr,
                "error: cannot start the evaluator with the stack that `--max-depth {max_depth}` needs: {error}"
            )?;
            return Ok(ExitStatus::Usage);
        }
    };
    let root_location = root_location(&package);
    let diagnostics: Vec<Diagnostic> = completion_diagnostic(
        &run.completion,
        start.span,
        &package.sources,
        &root_location,
    )
    .into_iter()
    .collect();
    // The program ran to its end after stdout failed. Its failure or abort is still reported,
    // then the failed write, with exit code 2, as the native executable does (#199).
    if let Some(error) = write_error {
        if !diagnostics.is_empty() {
            write!(
                stderr,
                "{}",
                render_human(&diagnostics, &package.sources.texts(), options.color)
            )?;
        }
        writeln!(stderr, "error: cannot write the output: {error}")?;
        stderr.flush()?;
        return Ok(ExitStatus::Usage);
    }
    let status = match &run.completion {
        Completion::Ok(_) => ExitStatus::Success,
        Completion::Failed { .. }
        | Completion::Aborted(
            Abort::Depth { .. }
            | Abort::Memory { .. }
            | Abort::HeaderValue { .. }
            | Abort::NotText { .. }
            | Abort::HostStop { .. },
        ) => ExitStatus::Errors,
        Completion::Aborted(Abort::Fault { .. }) => ExitStatus::Internal,
    };
    let document = RunDocument {
        schema: RUN_SCHEMA,
        tier: TIER,
        status: match &run.completion {
            Completion::Ok(_) => "ok",
            Completion::Failed { .. } => "failed",
            Completion::Aborted(_) => "aborted",
        },
        output,
        failure: match &run.completion {
            Completion::Failed {
                variant, public, ..
            } => Some(FailureDoc {
                variant,
                fields: Pairs(public),
            }),
            _ => None,
        },
        diagnostics: in_output_order(&diagnostics),
        trace: run.trace.iter().map(EventDoc::from).collect(),
    };
    finish(
        &document,
        &diagnostics,
        &package.sources,
        options,
        (stdout, stderr),
        status,
    )
}

/// Prints `document` as JSON on stdout, or `diagnostics` for people on stderr, and returns
/// `status`.
fn finish(
    document: &impl Serialize,
    diagnostics: &[Diagnostic],
    sources: &SourceDb,
    options: RunOptions<'_>,
    (stdout, stderr): (&mut dyn Write, &mut dyn Write),
    status: ExitStatus,
) -> io::Result<ExitStatus> {
    if options.json {
        match serde_json::to_string(document) {
            Ok(text) => writeln!(stdout, "{text}")?,
            Err(error) => {
                writeln!(
                    stderr,
                    "internal compiler error (U8001): rendering the output as JSON failed: {error}"
                )?;
                return Ok(ExitStatus::Internal);
            }
        }
        stdout.flush()?;
    } else if !diagnostics.is_empty() {
        write!(
            stderr,
            "{}",
            render_human(diagnostics, &sources.texts(), options.color)
        )?;
        stderr.flush()?;
    }
    Ok(status)
}

/// `main` of `entry` when it is `pub main() -> Unit` (`spec/04 §8`).
pub(crate) fn entry_main<'a>(
    analysis: &'a Analysis,
    sources: &SourceDb,
    entry: &str,
) -> Option<&'a Fun> {
    let modules = Modules {
        sources,
        interner: &analysis.interner,
        trees: &analysis.trees,
        mounts: &analysis.mounts,
    };
    let (_, _, function) = modules.function(&format!("{entry}.main"))?;
    if !function.public || !function.params.is_empty() || !function.type_params.is_empty() {
        return None;
    }
    let main = main_of(&analysis.program, entry)?;
    matches!(
        main.result,
        Type::Base {
            base: Base::Unit,
            ..
        }
    )
    .then_some(main)
}

/// The message of U6007, which says why the hard stop came (ADR 0116 rule 16): the same in
/// both engines.
pub(crate) fn host_stop_message(hard: una_interp::stopping::HardStop) -> String {
    una_interp::stopping::hard_stop_message(hard)
}

/// Whether a run of the package can take a host stop, so that it installs the handlers of
/// SIGINT and SIGTERM (ADR 0116 rule 18): it runs a driver, or its `main` says `diverges`,
/// the only way a run reaches a `for ever` (U3027, rule 8).
pub(crate) fn takes_stops(
    analysis: &Analysis,
    sources: &SourceDb,
    entry: &str,
    driven: bool,
) -> bool {
    let modules = Modules {
        sources,
        interner: &analysis.interner,
        trees: &analysis.trees,
        mounts: &analysis.mounts,
    };
    driven
        || modules
            .function(&format!("{entry}.main"))
            .is_some_and(|(_, _, main)| {
                main.clauses
                    .iter()
                    .any(|clause| matches!(clause.kind, una_syntax::ast::ClauseKind::Diverges))
            })
}

/// `init` of the driven module `module`, when the module has the shapes of ADR 0116 rule 2:
/// `pub init() -> S`, `pub step(edit state: S, input: Tick) -> Unit` and, when declared,
/// `pub on_stop(state: S) -> Unit`, with `Tick` of `std.drive`. Otherwise the message of the
/// usage error.
pub(crate) fn driven_init<'a>(
    analysis: &'a Analysis,
    sources: &SourceDb,
    module: &str,
) -> Result<&'a Fun, String> {
    let modules = Modules {
        sources,
        interner: &analysis.interner,
        trees: &analysis.trees,
        mounts: &analysis.mounts,
    };
    let header = |name: &str| {
        modules
            .function(&format!("{module}.{name}"))
            .map(|(_, _, function)| function)
            .filter(|function| function.public && function.type_params.is_empty())
    };
    let core = |name: &str| {
        let found = analysis.program.modules.iter().find(|m| m.path == module)?;
        found.decls.iter().find_map(|decl| match decl {
            Decl::Fun(fun) if fun.name == name => Some(fun.as_ref()),
            _ => None,
        })
    };
    let init = header("init")
        .filter(|function| function.params.is_empty())
        .and(core("init"))
        .ok_or_else(|| format!("the driven module `{module}` has no `pub init() -> S` to start"))?;
    let state = &init.result;
    let step = header("step").is_some_and(|function| {
        matches!(function.params.as_slice(), [held, input]
            if held.convention == Some(Convention::Edit) && input.convention.is_none())
    }) && core("step").is_some_and(|step| {
        // `step(edit state: S, input)` returns `(S, Unit)` in Core (`spec/17 §3`).
        matches!(step.params.as_slice(), [(_, held), (_, input)]
            if same_type(held, state) && is_tick(input))
            && matches!(&step.result, Type::Base { base: Base::Tuple(parts), .. }
                if matches!(parts.as_slice(), [held, done] if same_type(held, state) && is_unit(done)))
    });
    if !step {
        return Err(format!(
            "the driven module `{module}` has no `pub step(edit state: S, input: Tick) -> Unit`, with S the result of `init`"
        ));
    }
    let on_stop = match modules.function(&format!("{module}.on_stop")) {
        None => true,
        Some(_) => header("on_stop").is_some_and(
            |function| matches!(function.params.as_slice(), [held] if held.convention.is_none()),
        ) && core("on_stop").is_some_and(|on_stop| {
            matches!(on_stop.params.as_slice(), [(_, held)] if same_type(held, state))
                && is_unit(&on_stop.result)
        }),
    };
    if !on_stop {
        return Err(format!(
            "`on_stop` of the driven module `{module}` is not `pub on_stop(state: S) -> Unit`, with S the result of `init`"
        ));
    }
    Ok(init)
}

/// Whether two types are the same, labels and refinements aside.
fn same_type(a: &Type, b: &Type) -> bool {
    let all = |xs: &[Type], ys: &[Type]| {
        xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| same_type(x, y))
    };
    match (a, b) {
        (Type::Base { base: x, .. }, Type::Base { base: y, .. }) => match (x, y) {
            (
                Base::Named {
                    module: m,
                    name: n,
                    args: xs,
                },
                Base::Named {
                    module: k,
                    name: o,
                    args: ys,
                },
            ) => m == k && n == o && all(xs, ys),
            (Base::Tuple(xs), Base::Tuple(ys)) => all(xs, ys),
            _ => x == y,
        },
        _ => a == b,
    }
}

/// Whether `ty` is `Tick` of `std.drive`.
fn is_tick(ty: &Type) -> bool {
    matches!(ty, Type::Base { base: Base::Named { module, name, .. }, .. }
        if module == "std.drive" && name == "Tick")
}

/// Whether `ty` is `Unit`.
fn is_unit(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Base {
            base: Base::Unit,
            ..
        }
    )
}

fn main_of<'a>(program: &'a Program, entry: &str) -> Option<&'a Fun> {
    let module = program.modules.iter().find(|m| m.path == entry)?;
    module.decls.iter().find_map(|decl| match decl {
        Decl::Fun(fun) if fun.name == "main" => Some(fun.as_ref()),
        _ => None,
    })
}

/// The keys of the key files of `keys:` grants; the first grant of a name wins.
fn load_keys(grants: &[Grant]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut keys = BTreeMap::new();
    for grant in grants {
        let Grant::Keys { file } = grant else {
            continue;
        };
        let text = std::fs::read_to_string(file)
            .map_err(|error| format!("cannot read the key file `{}`: {error}", file.display()))?;
        let loaded = una_interp::parse_keys(&text)
            .map_err(|reason| format!("the key file `{}` is invalid: {reason}", file.display()))?;
        for (name, key) in loaded {
            keys.entry(name).or_insert(key);
        }
    }
    Ok(keys)
}

/// Where a diagnostic without a source position goes: 1:1 of `una.toml`, or of the file of a
/// single-file package (ADR 0022).
fn root_location(package: &Package) -> Location {
    let file = package
        .manifest
        .or_else(|| package.modules.first().copied());
    match file {
        Some(id) => package.sources.location(Span::at(id, 0)),
        None => Location {
            file: String::new(),
            start: una_diag::Position { line: 1, col: 1 },
            end: una_diag::Position { line: 1, col: 1 },
        },
    }
}

/// U6001 when `main` failed, U6003 when a call went too deep, U6004 for a header value with a
/// control character, U6005 for host text that is not valid UTF-8, U8001 on a fault.
fn completion_diagnostic(
    completion: &Completion,
    main: Span,
    sources: &SourceDb,
    root: &Location,
) -> Option<Diagnostic> {
    match completion {
        Completion::Ok(_) => None,
        Completion::Failed {
            variant, public, ..
        } => {
            // `spec/04 §8`: only the payload fields whose labels are `public` are shown.
            let shown = if public.is_empty() {
                variant.clone()
            } else {
                let fields: Vec<String> = public
                    .iter()
                    .map(|(name, value)| format!("{name}: {value}"))
                    .collect();
                format!("{variant}({})", fields.join(", "))
            };
            Some(Diagnostic::error(
                Code::U6001,
                format!("`main` failed with `{shown}`"),
                sources.location(main),
            ))
        }
        Completion::Aborted(abort) => Some(abort_diagnostic(abort, sources, root)),
    }
}

fn abort_diagnostic(abort: &Abort, sources: &SourceDb, root: &Location) -> Diagnostic {
    match abort {
        Abort::Depth { span, limit } => Diagnostic::error(
            Code::U6003,
            format!("this call exceeds the maximum call depth of {limit}"),
            sources.location(*span),
        ),
        Abort::Memory { span, limit } => Diagnostic::error(
            Code::U6003,
            format!("this call exceeds the memory limit of {limit} MiB"),
            sources.location(*span),
        ),
        Abort::HeaderValue { span } => Diagnostic::error(
            Code::U6004,
            "a header value contains a control character",
            sources.location(*span),
        ),
        Abort::NotText { span, message } => {
            Diagnostic::error(Code::U6005, *message, sources.location(*span))
        }
        Abort::HostStop { span, hard } => Diagnostic::error(
            Code::U6007,
            host_stop_message(*hard),
            sources.location(*span),
        ),
        Abort::Fault { message, span } => Diagnostic::error(
            Code::U8001,
            format!("evaluation stopped: {message}"),
            span.map_or_else(|| root.clone(), |span| sources.location(span)),
        ),
    }
}

/// `una test [path]`: checks the package at tier 1, then runs every example (`spec/08 §2`,
/// `spec/11 §6`).
pub(crate) fn test_command(
    path: &Path,
    options: RunOptions<'_>,
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
    // Edition `0.1` redacts labeled fields in an example's results and reasons; edition `0.2`
    // shows every field, since an example's data come from the source (ADR 0078).
    let redact = crate::package::project(&package).0.edition == una_project::Edition::V0_1;
    let mut analysis = on_large_stack(|| analyze(&package, None));
    if analysis.diagnostics.is_empty() {
        let found = tier_one(&package, &analysis.program, &options.solving);
        analysis.diagnostics = found.diagnostics;
    }
    if !analysis.diagnostics.is_empty() {
        let document = TestDocument {
            schema: TEST_SCHEMA,
            tier: TIER,
            redacted: redact,
            diagnostics: in_output_order(&analysis.diagnostics),
            examples: Vec::new(),
            passed: 0,
            failed: 0,
        };
        let status = rejected_status(&analysis.diagnostics);
        return finish(
            &document,
            &analysis.diagnostics,
            &package.sources,
            options,
            (stdout, stderr),
            status,
        );
    }
    let Some(stack) = una_interp::stack_bytes(options.max_depth) else {
        writeln!(
            stderr,
            "error: `--max-depth {}` needs more stack than the system can address",
            options.max_depth
        )?;
        return Ok(ExitStatus::Usage);
    };
    if let Err(message) = options.tasks.check(options.max_depth) {
        writeln!(stderr, "error: {message}")?;
        return Ok(ExitStatus::Usage);
    }
    // `una test` runs the examples of the root's modules only: a dependency's examples are
    // checked as its own, and never run with the root's authority (ADR 0122 rule 52).
    let mounts = crate::package::project(&package).0.mounts;
    let mut tested = analysis.program.clone();
    for module in &mut tested.modules {
        let dependency = mounts.iter().any(|mount| {
            module
                .path
                .strip_prefix(mount.local.as_str())
                .is_some_and(|rest| rest.starts_with('.'))
        });
        if dependency {
            module.examples.clear();
        }
    }
    let program = &tested;
    let max_depth = options.max_depth;
    let limits = limits(&options, None);
    let ceiling = Ceiling::raise(&options);
    let evaluated = std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("una-test".to_owned())
            .stack_size(stack)
            .spawn_scoped(scope, move || {
                una_interp::run_examples(program, limits, redact)
            })?;
        Ok::<_, io::Error>(
            worker
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload)),
        )
    });
    drop(ceiling);
    let runs = match evaluated {
        Ok(runs) => runs,
        Err(error) => {
            writeln!(
                stderr,
                "error: cannot start the evaluator with the stack that `--max-depth {max_depth}` needs: {error}"
            )?;
            return Ok(ExitStatus::Usage);
        }
    };
    let files = module_files(&analysis, &package.sources);
    let root = root_location(&package);
    let diagnostics: Vec<Diagnostic> = runs
        .iter()
        .filter_map(|run| match &run.completion {
            Completion::Aborted(abort) => Some(abort_diagnostic(abort, &package.sources, &root)),
            _ => None,
        })
        .collect();
    let failed = runs.iter().filter(|run| run.failure.is_some()).count();
    let status = if diagnostics.iter().any(|d| d.code == Code::U8001) {
        ExitStatus::Internal
    } else if failed > 0 {
        ExitStatus::Errors
    } else {
        ExitStatus::Success
    };
    if options.json {
        let document = TestDocument {
            schema: TEST_SCHEMA,
            tier: TIER,
            redacted: redact,
            diagnostics: in_output_order(&diagnostics),
            examples: runs
                .iter()
                .map(|run| ExampleDoc::new(run, &files))
                .collect(),
            passed: runs.len() - failed,
            failed,
        };
        return finish(
            &document,
            &diagnostics,
            &package.sources,
            options,
            (stdout, stderr),
            status,
        );
    }
    for run in &runs {
        let verdict = if run.failure.is_some() {
            "FAILED"
        } else {
            "ok"
        };
        writeln!(
            stdout,
            "example {} {} ... {verdict}",
            run.module,
            una_core::quote(&run.name)
        )?;
        if let Some(failure) = &run.failure {
            if let Some((line, text)) = &failure.expectation {
                let file = files.get(run.module.as_str()).map_or("?", String::as_str);
                writeln!(stdout, "  {file}:{line}: {text}")?;
            }
            writeln!(stdout, "  {}", failure.reason)?;
        }
    }
    let noun = if runs.len() == 1 {
        "example"
    } else {
        "examples"
    };
    writeln!(
        stdout,
        "\n{} {noun}: {} passed, {failed} failed",
        runs.len(),
        runs.len() - failed
    )?;
    stdout.flush()?;
    finish(
        &(),
        &diagnostics,
        &package.sources,
        options,
        (stdout, stderr),
        status,
    )
}

/// The file of each module, by module path, as diagnostics name it.
fn module_files(analysis: &Analysis, sources: &SourceDb) -> BTreeMap<String, String> {
    analysis
        .trees
        .iter()
        .filter_map(|(id, tree)| {
            let header = tree.module.as_ref()?;
            let path: Vec<&str> = header
                .path
                .segments
                .iter()
                .map(|segment| analysis.interner.resolve(segment.name))
                .collect();
            let file = sources.file(*id).path();
            // A dependency's module `m` is `<local>.m` in the program (ADR 0122 rule 47).
            let path = match analysis
                .mounts
                .iter()
                .find(|mount| file.starts_with(&mount.src))
            {
                Some(mount) => format!("{}.{}", mount.local, path.join(".")),
                None => path.join("."),
            };
            Some((path, file.to_owned()))
        })
        .collect()
}

/// `una.run/1`, with keys in the order of ADR 0032 rule 4.
#[derive(Serialize)]
struct RunDocument<'a> {
    schema: &'static str,
    tier: u8,
    status: &'static str,
    output: Vec<String>,
    failure: Option<FailureDoc<'a>>,
    diagnostics: Vec<&'a Diagnostic>,
    trace: Vec<EventDoc<'a>>,
}

impl<'a> RunDocument<'a> {
    /// The document of a package that the checks reject: nothing runs.
    fn rejected(diagnostics: &'a [Diagnostic]) -> Self {
        RunDocument {
            schema: RUN_SCHEMA,
            tier: TIER,
            status: "rejected",
            output: Vec::new(),
            failure: None,
            diagnostics: in_output_order(diagnostics),
            trace: Vec::new(),
        }
    }
}

/// The failure of `main`: its variant and its `public` fields.
#[derive(Serialize)]
struct FailureDoc<'a> {
    variant: &'a str,
    fields: Pairs<'a>,
}

/// `una.test/1`, with keys in the order of ADR 0032 rule 5.
#[derive(Serialize)]
struct TestDocument<'a> {
    schema: &'static str,
    tier: u8,
    /// Whether results and reasons redact labeled fields: edition `0.1` does, edition `0.2`
    /// does not (ADR 0078). An added key, so the schema stays `una.test/1` (ADR 0076).
    redacted: bool,
    diagnostics: Vec<&'a Diagnostic>,
    examples: Vec<ExampleDoc<'a>>,
    passed: usize,
    failed: usize,
}

/// One example of `una.test/1`.
#[derive(Serialize)]
struct ExampleDoc<'a> {
    module: &'a str,
    name: &'a str,
    outcome: &'static str,
    reason: Option<&'a str>,
    expectation: Option<ExpectationDoc<'a>>,
    result: OutcomeDoc<'a>,
    trace: Vec<EventDoc<'a>>,
}

/// The expectation that does not hold.
#[derive(Serialize)]
struct ExpectationDoc<'a> {
    file: &'a str,
    line: u32,
    text: &'a str,
}

impl<'a> ExampleDoc<'a> {
    fn new(run: &'a ExampleRun, files: &'a BTreeMap<String, String>) -> Self {
        let failure = run.failure.as_ref();
        ExampleDoc {
            module: &run.module,
            name: &run.name,
            outcome: if failure.is_some() { "fail" } else { "pass" },
            reason: failure.map(|f| f.reason.as_str()),
            expectation: failure
                .and_then(|f| f.expectation.as_ref())
                .map(|(line, text)| ExpectationDoc {
                    file: files.get(run.module.as_str()).map_or("", String::as_str),
                    line: *line,
                    text,
                }),
            result: match &run.completion {
                Completion::Ok(value) => OutcomeDoc::Ok { value },
                Completion::Failed { variant, value, .. } => OutcomeDoc::Fail { variant, value },
                Completion::Aborted(_) => OutcomeDoc::Aborted,
            },
            trace: run.trace.iter().map(EventDoc::from).collect(),
        }
    }
}
