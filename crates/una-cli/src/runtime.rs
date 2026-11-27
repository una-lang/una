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
use crate::{Analysis, Environment, ExitStatus, Package, analyze, on_large_stack};

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
/// The schema of `una test --json` for a package of edition 0.3 or later: `una.test/1` with the
/// key `examples` renamed `tests` (ADR 0105 rule 6).
const TEST_SCHEMA_0_3: &str = "una.test/2";

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
    /// The memory budget, in MiB: `--max-memory` or its default, until the package's declared
    /// budget applies ([`with_budget`]).
    pub(crate) max_memory: usize,
    /// Whether `--max-memory` was given (ADR 0109 rule 4).
    pub(crate) max_memory_given: bool,
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
    /// `--stop-after-iterations`: the budget of each loop instance, not a host stop request
    /// (ADR 0116 rule 19, `spec/26 §6` rule 1).
    pub(crate) stop_after: Option<u64>,
    /// `--virtual-time`: the run's clock is virtual (ADR 0116 rule 19, `spec/26 §8`). A test's
    /// clock is virtual whatever this says.
    pub(crate) virtual_time: bool,
    /// `--cost-clock`, with `--virtual-time`: each check point moves the virtual clock by one
    /// tick (`spec/27 §8` rule 6).
    pub(crate) cost_clock: bool,
    /// `--deadline`: a host stop request after this many seconds (ADR 0116 rule 14).
    pub(crate) deadline: Option<u64>,
    /// `--stop-grace`: the seconds from the first host stop request to the hard stop (rule 16).
    pub(crate) stop_grace: u64,
    /// `--trace <file>`: the trace, written to the file as it happens (ADR 0117 rule 10).
    pub(crate) trace: Option<&'a Path>,
    /// `--serve-script <file>`: the connections that replace the listeners of the `serve:`
    /// grants (ADR 0123 rule 21, `spec/31 §8`).
    pub(crate) serve_script: Option<&'a Path>,
    /// `--serve-out <file>`: where what the scripted connections received is written.
    pub(crate) serve_out: Option<&'a Path>,
}

/// The flags of tasks as given on the command line, each `None` when absent (ADR 0117).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TaskFlags {
    pub(crate) workers: Option<usize>,
    pub(crate) task_depth: Option<usize>,
    pub(crate) io_threads: Option<usize>,
    pub(crate) file_threads: Option<usize>,
    /// `--serve-threads` (ADR 0123 rule 20): checked, and unused until a run serves.
    pub(crate) serve_threads: Option<usize>,
}

impl TaskFlags {
    /// The checked settings for a run of `max_depth`, or the message of the usage error. The
    /// interpreter runs no coroutine before `first of` and `start` (ADR 0117 rule 11), so it
    /// only checks them, and refuses what the executable refuses.
    fn check(self, max_depth: usize) -> Result<una_interp::Concurrency, String> {
        una_interp::concurrency::serve_threads(self.serve_threads)?;
        una_interp::Concurrency::from_flags(
            max_depth,
            self.workers,
            self.task_depth,
            self.io_threads,
            self.file_threads,
        )
    }
}

/// A line for the thread that writes the output: to stdout, or to stderr.
enum Line {
    Out(String),
    Err(String),
}

/// The trace file of `--trace`, written as the run goes (ADR 0117 rule 10): `[`, the events
/// separated by `,`, then `]` once the run has ended, so a finished file is one JSON array.
struct TraceFile {
    file: io::BufWriter<std::fs::File>,
    written: usize,
    failed: Option<io::Error>,
}

impl TraceFile {
    fn start(path: &Path) -> io::Result<TraceFile> {
        Ok(TraceFile {
            file: io::BufWriter::new(std::fs::File::create(path)?),
            written: 0,
            failed: None,
        })
    }

    fn event(&mut self, event: &una_interp::Event) {
        if self.failed.is_some() {
            return;
        }
        let separator = if self.written == 0 { "[" } else { "," };
        self.written += 1;
        if let Err(error) = write!(self.file, "{separator}{}", una_interp::event_json(event)) {
            self.failed = Some(error);
        }
    }

    /// Closes the array and flushes the file: the error of the first write that failed.
    fn finish(mut self) -> io::Result<()> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        let opening = if self.written == 0 { "[" } else { "" };
        write!(self.file, "{opening}]")?;
        self.file.flush()
    }
}

/// The world of `una run`: printed lines go to the thread that writes stdout, environment
/// variables come from the binary, and the other host operations reach the machine directly.
struct HostWorld<'a> {
    lines: Sender<Line>,
    environment: Environment<'a>,
    /// The trace file, and whether the run keeps its events too, for `--json`.
    trace: Option<TraceFile>,
    keep: bool,
    /// The sources, for the position of a started task that ends alone.
    sources: &'a SourceDb,
}

impl World for HostWorld<'_> {
    fn print(&mut self, line: &str) {
        // The receiver only goes away once stdout has failed, and that is reported then.
        let _ = self.lines.send(Line::Out(line.to_owned()));
    }

    /// The run keeps an event only for the document of `--json`: without it, the trace goes to
    /// the file of `--trace`, or nowhere, so a long run's trace never grows in memory.
    fn trace_event(&mut self, event: &una_interp::Event) -> bool {
        if let Some(file) = &mut self.trace {
            file.event(event);
        }
        !self.keep
    }

    fn task_aborted(&mut self, path: &str, abort: &Abort) {
        let _ = self
            .lines
            .send(Line::Err(task_aborted_line(path, abort, self.sources)));
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

    /// One `lstat` per component of a confined path (#1343).
    fn step(&mut self, directory: &Path, name: &std::ffi::OsStr) -> Option<PathBuf> {
        una_interp::disk_step(directory, name)
    }

    /// Only a component that does not exist is absence (#1380), which fails the operation
    /// without a path (#1459).
    fn absent(&mut self, path: &Path) -> Option<una_interp::Absence> {
        una_interp::disk_absent(path)
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
        kernels: &una_core::dense::variants::Kernels::BASELINE,
        // A run that serves sets its engine after this, over its listeners or its script.
        serve: None,
        http_places: None,
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

/// The budget of a run of a package that declares one (ADR 0109 rule 4): the declared value,
/// which `--max-memory` (`given`) may lower and never raise; without a declared budget, the
/// flag's value or `default`. `Err` is the usage error when the flag asks for more. O(1).
pub(crate) fn declared_budget(
    given: Option<usize>,
    default: usize,
    declared: Option<u64>,
) -> Result<usize, String> {
    let Some(declared) = declared else {
        return Ok(given.unwrap_or(default));
    };
    let declared_mib = usize::try_from(declared).unwrap_or(usize::MAX);
    match given {
        Some(flag) if flag > declared_mib => Err(format!(
            "--max-memory {flag} is above the package's declared max_memory of {declared} MiB; it may lower the budget, never raise it"
        )),
        Some(flag) => Ok(flag),
        None => Ok(declared_mib),
    }
}

/// `options` with the package's declared budget applied, or the usage error written to
/// `stderr` and exit code 2 (ADR 0109 rule 4).
fn with_budget<'a>(
    mut options: RunOptions<'a>,
    declared: Option<u64>,
    stderr: &mut dyn Write,
) -> io::Result<Result<RunOptions<'a>, ExitStatus>> {
    let given = options.max_memory_given.then_some(options.max_memory);
    match declared_budget(given, options.max_memory, declared) {
        Ok(budget) => {
            options.max_memory = budget;
            Ok(Ok(options))
        }
        Err(message) => {
            writeln!(stderr, "error: {message}")?;
            Ok(Err(ExitStatus::Usage))
        }
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
    // ADR 0123 rule 21: the flags of a script need one another, before the package is read.
    if let Err(message) = una_http::serve::script_file::check_flags(
        options.serve_script.is_some(),
        options.serve_out.is_some(),
        options.virtual_time,
    ) {
        writeln!(stderr, "error: {message}")?;
        return Ok(ExitStatus::Usage);
    }
    let package = match crate::package::load_in(
        path,
        &std::collections::BTreeMap::new(),
        options.solving.itaoca.as_ref(),
    ) {
        Ok(package) => package,
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    // `[run] grants` resolve against the package root, `--grant` flags against
    // the current directory, and they add up.
    let (manifest, _) = crate::package::project(&package);
    // The declared budget applies before anything runs (ADR 0109 rule 4).
    let options = match with_budget(options, manifest.max_memory, stderr)? {
        Ok(options) => options,
        Err(status) => return Ok(status),
    };
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
    // The grants before this index are the package's (`run grants`), the others the flags'.
    let package_grants = grants.len();
    for spec in options.grants {
        let Some(grant) = Grant::parse(spec, Path::new("")) else {
            writeln!(stderr, "error: {}", una_interp::invalid_grant(spec))?;
            return Ok(ExitStatus::Usage);
        };
        specs.push(spec.clone());
        grants.push(grant);
    }
    // A server exists from edition 0.4 (`spec/31 §2` rule 4): a `serve:` flag for a single
    // file or a package of an earlier edition is a usage error.
    if manifest.edition < una_project::Edition::V0_4
        && let Some(spec) = specs
            .iter()
            .find(|spec| spec.starts_with(una_core::http::serve::PREFIX))
    {
        writeln!(
            stderr,
            "error: `{spec}` needs a package of edition 0.4, described by ita.una; this package is of edition {}",
            manifest.edition.name()
        )?;
        return Ok(ExitStatus::Usage);
    }
    let mut analysis = on_large_stack(|| analyze(&package, Some(&specs)));
    if analysis.diagnostics.is_empty() {
        let found = tier_one(&package, &analysis.program, &options.solving);
        analysis.diagnostics = found.diagnostics;
    }
    // A `--grant` with limits needs edition 0.4 and `L_lim` at most `internal` (ADR 0117 IF4,
    // `spec/27 §9` rule 5): a usage error before `main` starts. The manifest's grants are
    // `una check`'s to report (U4001).
    if analysis.diagnostics.is_empty() {
        let label = analysis.limit_label.to_string();
        let limit = (manifest.edition >= una_project::Edition::V0_4).then(|| {
            (
                label.as_str(),
                analysis
                    .limit_label
                    .flows_to(&crate::build::internal_label()),
            )
        });
        if let Err(message) = una_interp::check_limited_flags(options.grants, limit) {
            writeln!(stderr, "error: {message}")?;
            return Ok(ExitStatus::Usage);
        }
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
    // The host-driven form of a server (ADR 0123 rule 16): the module's shapes, then a `serve:`
    // grant, each a usage error; then the driver runs, as a program that serves in the language
    // form below (`spec/31 §1` rule 10, §7 rule 5).
    if let Some(serve) = &manifest.serve {
        let module = serve.module.value.as_str();
        if let Err(message) = served_module(&analysis, &package.sources, module) {
            writeln!(stderr, "error: {message}")?;
            return Ok(ExitStatus::Usage);
        }
        if !specs
            .iter()
            .any(|spec| spec.starts_with(una_core::http::serve::PREFIX))
        {
            writeln!(stderr, "error: {}", una_interp::unserved_section(module))?;
            return Ok(ExitStatus::Usage);
        }
        // `within` is U4003 under the preview's `L_srv` (`spec/31 §7` rule 3 item 7), so a
        // package that checks has none; the driver never runs without the deadline it names.
        if serve.within.is_some() {
            writeln!(
                stderr,
                "error: the `within` of the `serve` section of module `{module}` does not run in this release yet"
            )?;
            return Ok(ExitStatus::Usage);
        }
    }
    let drive = manifest.drive.as_ref().map(|drive| una_interp::Drive {
        ticks: drive.ticks,
        budget: drive.budget,
    });
    let (entry, start) = if let Some(serve) = &manifest.serve {
        // Its shapes were checked above: `handle` is there.
        let module = serve.module.value.clone();
        let Some(handle) = function_of(&analysis.program, &module, "handle") else {
            writeln!(
                stderr,
                "error: the module `{module}` has no `handle` to run"
            )?;
            return Ok(ExitStatus::Usage);
        };
        (module, handle)
    } else if let Some(driven) = &manifest.drive {
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
    let serving = manifest.serve.is_some();
    let started: &[&str] = if serving {
        &["handle", "on_stop"]
    } else if drive.is_some() {
        &["init", "step", "on_stop"]
    } else {
        &["main"]
    };
    // Whether the program serves: its entry functions use the host's `HttpServer`, or it has a
    // `serve` section, whose driver does, so its serve threads start before `main` (S4
    // amendment B.2, P8). `una run` runs it, as `una build` builds it (`spec/31 §1` rule 10).
    let serves = serving || uses_http_server(&analysis, &entry, started);
    let keys = match load_keys(&grants) {
        Ok(keys) => keys,
        Err(message) => {
            writeln!(stderr, "error: {message}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    // A script of connections stands for the peers of the `serve:` grants (ADR 0123 rule 21):
    // it is read and checked in full after the key files, as the executable reads them, and
    // before anything is made or runs. Its connections run only in the toolchain's own test
    // builds (`scripted`); elsewhere they do not run yet (`spec/31 §8`).
    let served_grants: Vec<una_core::http::serve::ServeGrant> = serve_inputs(&grants)
        .iter()
        .map(|input| input.grant.clone())
        .collect();
    // The line of each address a run that serves names on stderr (`spec/31 §6` rule 14).
    let notices: Vec<String> = grants
        .iter()
        .enumerate()
        .filter_map(|(index, grant)| match grant {
            Grant::Serve { grant, .. } => Some(una_http::serve::serving_notice(
                grant,
                index < package_grants,
            )),
            _ => None,
        })
        .collect();
    let script = match options.serve_script {
        Some(script) => {
            let listeners: Vec<String> = served_grants.iter().map(|g| g.listener()).collect();
            match una_http::serve::script_file::load(script, &listeners) {
                Ok(events) => Some(events),
                Err(message) => {
                    writeln!(stderr, "error: {message}")?;
                    return Ok(ExitStatus::Usage);
                }
            }
        }
        None => None,
    };
    let script_runs = script.is_some()
        && cfg!(feature = "test-hooks")
        && crate::variable(options.environment, "UNA_TEST_RUN_SERVER").as_deref() == Some("1");
    if script.is_some() && !script_runs {
        writeln!(stderr, "error: {}", una_http::serve::script_file::PENDING)?;
        return Ok(ExitStatus::Usage);
    }
    // Live peers send in real time, which virtual time cannot place: a run under
    // `--virtual-time` takes no `serve:` grant without `--serve-script` (ADR 0123, S4
    // amendment C).
    if options.virtual_time
        && script.is_none()
        && grants
            .iter()
            .any(|grant| matches!(grant, Grant::Serve { .. }))
    {
        writeln!(stderr, "error: {}", una_interp::VIRTUAL_SERVE)?;
        return Ok(ExitStatus::Usage);
    }
    // ADR 0116 amendment, P8 (#1148): the client's TLS settings are built before `main` when
    // the grants include `Http`, as a native executable builds them, never at a first request.
    if grants
        .iter()
        .any(|grant| matches!(grant, una_interp::Grant::Http(_)))
    {
        una_http::client::prepare();
        // The client's lookup and connect pools, all their threads at once (`spec/24 §4` rule 2).
        let io_threads = options
            .tasks
            .io_threads
            .unwrap_or(una_interp::concurrency::DEFAULT_IO_THREADS);
        if una_http::resolve::start(io_threads).is_err() {
            writeln!(
                stderr,
                "[{}] Error: the lookup and connect threads of this run's requests could not be made",
                Code::U6003
            )?;
            return Ok(ExitStatus::Errors);
        }
    }
    // The `serve:` grants, before `main` (`spec/31 §6` rule 14): the descriptor limit, the TLS
    // files checked on their open descriptors, and the listeners, held until the run ends.
    // Nothing accepts on them until the serve threads of ADR 0123 step 4 take them over.
    // A `--serve-threads` out of range is refused below; here it only sizes the descriptors.
    let serve_threads =
        una_interp::concurrency::serve_threads(options.tasks.serve_threads).unwrap_or(1);
    // A scripted run binds no listener: the script's connections take their place.
    let inputs = if script_runs {
        Vec::new()
    } else {
        serve_inputs(&grants)
    };
    let listeners = match una_http::serve::prepare(
        &inputs,
        serve_threads,
        options
            .tasks
            .file_threads
            .unwrap_or(una_interp::concurrency::DEFAULT_FILE_THREADS),
    ) {
        Ok(prepared) => prepared,
        Err(message) => {
            writeln!(stderr, "error: {message}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let host = Host {
        grants,
        keys,
        virtual_time: options.virtual_time,
        cost_clock: options.cost_clock,
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
    // installed counts for the run. They are installed before the serve threads start and
    // before the lines of `spec/31 §6` rule 14, as the executable installs them (#1920): a
    // peer that stops the run once it names its addresses gets a host stop and its drain,
    // never the default action of SIGTERM. A stop that comes before the serve engine's hook
    // is set reaches it when the hook is set (`RunStop::on_stops`).
    let stops = takes_stops(
        &analysis,
        &package.sources,
        &entry,
        drive.is_some() || serving,
    );
    let stop = una_interp::stopping::RunStop::new(options.stop_grace);
    if stops && let Err(failed) = una_sys::signals::install() {
        // A soft limit of open files too small for the runtime is the run's configuration, a
        // usage error, not an internal one (#1757).
        if failed.usage {
            writeln!(stderr, "error: {}", failed.message)?;
            return Ok(ExitStatus::Usage);
        }
        writeln!(
            stderr,
            "internal compiler error (U8001): {}",
            failed.message
        )?;
        return Ok(ExitStatus::Internal);
    }
    // A program that serves starts its serve threads here, before `main`, over the listeners
    // bound above; they accept nothing until its first server operation (S4 amendment B.2).
    // Any other run holds its listeners until it ends.
    let live = if serves && !listeners.listeners.is_empty() {
        // The engine's panic hook, from this thread, before any serve thread starts.
        una_http::serve::install_panic_hook();
        // The toolchain's test builds: the placement probe of `serve_owner_spread` (#1660).
        #[cfg(feature = "test-hooks")]
        {
            let unkeyed = crate::variable(options.environment, "UNA_TEST_SERVE_UNKEYED").as_deref()
                == Some("1");
            let log = crate::variable(options.environment, "UNA_TEST_SERVE_OWNERS");
            if (unkeyed || log.is_some())
                && let Err(error) =
                    una_http::serve::place::probe::install(unkeyed, log.as_deref().map(Path::new))
            {
                writeln!(stderr, "error: the placement probe: {error}")?;
                return Ok(ExitStatus::Usage);
            }
        }
        match una_http::serve::live::Live::start(listeners, &served_grants, serve_threads) {
            Ok(live) => {
                // One line per address it serves, before the program runs (`spec/31 §6` rule 14).
                for notice in &notices {
                    writeln!(stderr, "{notice}")?;
                }
                Some(std::sync::Arc::new(live))
            }
            Err(message) => {
                writeln!(stderr, "error: {message}")?;
                return Ok(ExitStatus::Usage);
            }
        }
    } else {
        drop(listeners);
        None
    };
    let engine = script.as_deref().and_then(|events| {
        scripted::Engine::start(options.environment, &served_grants, events, &stop)
    });
    // The serve engine learns of the host stop and of the hard stop from the run's watcher
    // (ADR 0123 rule 18).
    if let Some(live) = &live {
        let live = std::sync::Arc::clone(live);
        stop.on_stops(Box::new(move |hard| {
            if hard {
                live.hard_stop();
            } else {
                live.host_stop();
            }
        }));
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
    // The trace file of `--trace` is made before the run starts: one that cannot be made is
    // a usage error, as for the executable (`spec/16 §3` rule 4).
    let trace_file = match options.trace {
        Some(path) => match TraceFile::start(path) {
            Ok(file) => Some(file),
            Err(error) => {
                writeln!(
                    stderr,
                    "error: cannot write the trace to `{}`: {error}",
                    path.display()
                )?;
                return Ok(ExitStatus::Usage);
            }
        },
        None => None,
    };
    let keep = options.json;
    let sources = &package.sources;
    let (sender, receiver) = channel::<Line>();
    let program = &analysis.program;
    let entry_path = entry.as_str();
    let host = &host;
    let max_depth = options.max_depth;
    let mut limits = limits(&options, Some(&stop));
    limits.serve = match &engine {
        Some(engine) => Some(engine.port()),
        None => live
            .as_deref()
            .map(|live| live as &dyn una_http::serve::program::Port),
    };
    // The toolchain's test builds: fewer places for the sockets of `Http` requests than the
    // descriptor limit gives, so a test reaches the cap of `spec/27 §2` rule 3 (#1730).
    #[cfg(feature = "test-hooks")]
    if let Some(places) = crate::variable(options.environment, "UNA_TEST_HTTP_PLACES") {
        limits.http_places = places.parse().ok();
    }
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
                    trace: trace_file,
                    keep,
                    sources,
                };
                let run = match drive {
                    Some(drive) => {
                        una_interp::run_drive(program, entry_path, &mut world, host, limits, drive)
                            .ok()
                    }
                    None if serving => {
                        una_interp::run_serve(program, entry_path, &mut world, host, limits)
                    }
                    None => una_interp::run_main(program, entry_path, &mut world, host, limits),
                };
                let traced = world.trace.take().map(TraceFile::finish);
                (run, traced)
            })?;
        for line in receiver {
            match line {
                Line::Out(line) if options.json => output.push(line),
                Line::Out(line) => {
                    if write_error.is_none()
                        && let Err(error) = writeln!(stdout, "{line}").and_then(|()| stdout.flush())
                    {
                        write_error = Some(error);
                    }
                }
                // A started task that ended alone (`spec/27 §7`): its line as it happens.
                Line::Err(line) => {
                    let _ = writeln!(stderr, "{line}").and_then(|()| stderr.flush());
                }
            }
        }
        Ok::<_, io::Error>(
            worker
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload)),
        )
    });
    drop(ceiling);
    // The serve engine ends with the run: what the run answered is written, as a host stop
    // drains, for at most the grace period, then the hard stop closes what is left; after the
    // hard stop or an internal error, the hard stop at once (ADR 0123 rule 18).
    if let Some(live) = &live {
        let hard = matches!(
            &evaluated,
            Ok((Some(run), _)) if matches!(
                run.completion,
                Completion::Aborted(Abort::HostStop { .. } | Abort::Fault { .. })
            )
        );
        live.end(
            std::time::Duration::from_secs(options.stop_grace.max(1)),
            hard,
        );
    }
    drop(watching);
    // The scripted engine ends with the run; what it leaves and `--serve-out` are reported
    // after the run's own diagnostics, as the executable reports them (`scripted_end`).
    let ended = engine.as_ref().map(scripted::Engine::end);
    let (evaluated, traced) = match evaluated {
        Ok((run, traced)) => (Ok(run), traced),
        Err(error) => (Err(error), None),
    };
    // A trace file that could not be written is reported last, after the run's own
    // diagnostics, the scripted engine's warnings and `--serve-out`, with exit code 2 or the
    // run's when higher, as the executable reports it (`spec/31 §8` rule 6).
    let trace_failed = match (traced, options.trace) {
        (Some(Err(error)), Some(path)) => Some(format!(
            "error: cannot write the trace to `{}`: {error}",
            path.display()
        )),
        _ => None,
    };
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
    // `spec/26 §2` rule 2: a failure of `init` of a driven module ends the run as one of
    // `main` does, and its message names `init` of that module (#1809).
    let failed = if manifest.drive.is_some() && manifest.serve.is_none() {
        format!("`init` of the driven module `{entry}`")
    } else {
        "`main`".to_owned()
    };
    let diagnostics: Vec<Diagnostic> = completion_diagnostic(
        &run.completion,
        &failed,
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
        let status = scripted_end(ended, options.serve_out, stderr, ExitStatus::Usage)?;
        return trace_end(trace_failed, stderr, status);
    }
    let status = match &run.completion {
        Completion::Ok(_) => ExitStatus::Success,
        Completion::Failed { .. }
        | Completion::Aborted(
            Abort::Depth { .. }
            | Abort::Memory { .. }
            | Abort::HeaderValue { .. }
            | Abort::NotText { .. }
            | Abort::NoRandomBytes
            | Abort::Tables { .. }
            | Abort::HostStop { .. }
            | Abort::Unstarted { .. },
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
    let serve_out = options.serve_out;
    let status = finish(
        &document,
        &diagnostics,
        &package.sources,
        options,
        (stdout, &mut *stderr),
        status,
    )?;
    let status = scripted_end(ended, serve_out, stderr, status)?;
    trace_end(trace_failed, stderr, status)
}

/// The line of a trace file that could not be written, last, as the executable writes it: the
/// exit code is then the higher of `status` and 2.
fn trace_end(
    failed: Option<String>,
    stderr: &mut dyn Write,
    status: ExitStatus,
) -> io::Result<ExitStatus> {
    let Some(line) = failed else {
        return Ok(status);
    };
    writeln!(stderr, "{line}")?;
    Ok(if status.code() > ExitStatus::Usage.code() {
        status
    } else {
        ExitStatus::Usage
    })
}

/// After a scripted run's own diagnostics, as the executable orders them (`spec/31 §8` rule
/// 6): what the engine left not at rest, as warnings, then `--serve-out`, whose file that
/// cannot be written is a usage error; the exit code is the higher of the run's and 2.
fn scripted_end(
    ended: Option<(String, Vec<String>)>,
    serve_out: Option<&Path>,
    stderr: &mut dyn Write,
    status: ExitStatus,
) -> io::Result<ExitStatus> {
    let Some((out, problems)) = ended else {
        return Ok(status);
    };
    for problem in problems {
        writeln!(
            stderr,
            "warning: the scripted engine at the end of the run: {problem}"
        )?;
    }
    if let Some(path) = serve_out
        && let Err(error) = una_http::serve::script_file::write_out(path, &out)
    {
        writeln!(
            stderr,
            "error: {}",
            una_http::serve::script_file::out_error(path, &error)
        )?;
        return Ok(if status.code() > ExitStatus::Usage.code() {
            status
        } else {
            ExitStatus::Usage
        });
    }
    Ok(status)
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

/// Whether one of the functions `names` of `module` that start the run uses the host's
/// `HttpServer`, so the run serves (`spec/31 §1` rule 10).
fn uses_http_server(analysis: &Analysis, module: &str, names: &[&str]) -> bool {
    analysis.headers.iter().any(|header| {
        header.module == module
            && names.contains(&header.name.as_str())
            && header
                .uses
                .iter()
                .any(|need| need.host && need.capability == "HttpServer")
    })
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
    // Core IR erases units of measure, which `same_type` therefore cannot see: the host hands
    // the state on unchanged, so its units MUST be those of the result of `init` at every
    // position (`spec/26 §2` rule 2, U2028 in the checks too). Fails closed when the checker
    // recorded none.
    let units = analysis.drive_units.as_ref().is_some_and(|units| {
        [&units.step, &units.on_stop]
            .into_iter()
            .flatten()
            .all(|state| *state == units.init)
    });
    if !units {
        return Err(format!(
            "the state of the driven module `{module}` does not have the units of measure of the result of `init` at every position of its type in `step` and `on_stop`"
        ));
    }
    Ok(init)
}

/// The usage error of a package with a `serve` section whose module lacks the shapes of
/// ADR 0123 rule 16: `pub handle(request: internal Request) -> internal Reply`, with `Request`
/// and `Reply` of `std.host`, and, when declared, `pub on_stop() -> Unit`. The message names
/// the first difference found. `diverges` and `fails` are the checker's (U3027, U3029).
pub(crate) fn served_module(
    analysis: &Analysis,
    sources: &SourceDb,
    module: &str,
) -> Result<(), String> {
    let modules = Modules {
        sources,
        interner: &analysis.interner,
        trees: &analysis.trees,
        mounts: &analysis.mounts,
    };
    let core = |name: &str| {
        let found = analysis.program.modules.iter().find(|m| m.path == module)?;
        found.decls.iter().find_map(|decl| match decl {
            Decl::Fun(fun) if fun.name == name => Some(fun.as_ref()),
            _ => None,
        })
    };
    let shape = "`pub handle(request: internal Request) -> internal Reply`";
    let missing =
        |why: &str| format!("the module `{module}` of the `serve` section has no {shape}: {why}");
    let Some((_, _, handle)) = modules.function(&format!("{module}.handle")) else {
        return Err(missing("it declares no `handle`"));
    };
    if !handle.public {
        return Err(missing("its `handle` is not public"));
    }
    if !handle.type_params.is_empty() {
        return Err(missing("its `handle` has type parameters"));
    }
    if !matches!(handle.params.as_slice(), [request] if request.convention.is_none()) {
        return Err(missing(
            "its `handle` does not take exactly one parameter, passed by value",
        ));
    }
    // `spec/31 §7` rule 2: no `requires`, which the host could never establish (U7008 in the
    // checks too).
    let requires = |function: &una_syntax::ast::FunctionDecl| {
        function
            .clauses
            .iter()
            .any(|clause| matches!(clause.kind, una_syntax::ast::ClauseKind::Requires(_)))
    };
    if requires(handle) {
        return Err(missing("its `handle` declares `requires`"));
    }
    let Some(fun) = core("handle") else {
        return Err(missing("it declares no `handle`"));
    };
    let internal = crate::build::internal_label();
    if !matches!(fun.params.as_slice(), [(_, request)] if is_host_kind(request, "Request", &internal))
    {
        return Err(missing(
            "the parameter of its `handle` is not `internal Request` of `std.host`",
        ));
    }
    if !is_host_kind(&fun.result, "Reply", &internal) {
        return Err(missing(
            "the result of its `handle` is not `internal Reply` of `std.host`",
        ));
    }
    if let Some((_, _, on_stop)) = modules.function(&format!("{module}.on_stop")) {
        let shaped = on_stop.public
            && !requires(on_stop)
            && on_stop.type_params.is_empty()
            && on_stop.params.is_empty()
            && core("on_stop").is_some_and(|fun| {
                fun.params.is_empty()
                    && is_unit(&fun.result)
                    && matches!(&fun.result, Type::Base { refinements, .. } if refinements.is_empty())
            });
        if !shaped {
            return Err(format!(
                "`on_stop` of the module `{module}` of the `serve` section is not `pub on_stop() -> Unit`"
            ));
        }
    }
    Ok(())
}

/// Whether `ty` is the kind `name` of `std.host`, labeled exactly `label`, with no refinement:
/// a refined kind is a precondition no call of the host establishes (`spec/31 §7` rule 2). The
/// kinds of `std.host` are the prelude's in Core IR, whose module path a package cannot take.
fn is_host_kind(ty: &Type, name: &str, label: &una_core::Label) -> bool {
    matches!(ty, Type::Base { base: Base::Named { module, name: found, args }, refinements, label: at }
        if module == "prelude" && found == name && args.is_empty() && at == label && refinements.is_empty())
}

/// Whether two types are the same `S` of a driven module's state, labels aside (`spec/26 §2`
/// rule 2). The labels are the checker's: each hand-off of the host is a flow into the
/// parameter's declared type, U4001 otherwise, so a state labeled higher in `step` or `on_stop`
/// is the same `S` at every position, the parameters and result of a function type included
/// (issue #1644). A refinement anywhere in `S` is U7008 in `una check` (`spec/26 §2` rule 2,
/// "No preconditions"), since nothing proves the state the host hands on satisfies it; as
/// defense in depth the refinements are compared here too, predicate by predicate and in
/// order, so a package that reaches the host past that check with a refinement on one side
/// only is still refused. A function type's effects and failures are compared exactly.
/// O(size of the types).
fn same_type(a: &Type, b: &Type) -> bool {
    let all = |xs: &[Type], ys: &[Type]| {
        xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| same_type(x, y))
    };
    match (a, b) {
        (
            Type::Fn {
                params: xs,
                result: x,
                uses: xu,
                fails: xf,
                ..
            },
            Type::Fn {
                params: ys,
                result: y,
                uses: yu,
                fails: yf,
                ..
            },
        ) => all(xs, ys) && same_type(x, y) && xu == yu && xf == yf,
        (
            Type::Base {
                base: x,
                refinements: xr,
                ..
            },
            Type::Base {
                base: y,
                refinements: yr,
                ..
            },
        ) => {
            // The predicates, not where they are written: one kind or range written in two
            // signatures is one refinement.
            xr.len() == yr.len()
                && xr
                    .iter()
                    .zip(yr)
                    .all(|(p, q)| p.clause.term == q.clause.term)
                && match (x, y) {
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
                }
        }
        _ => false,
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
    function_of(program, entry, "main")
}

/// The function `name` of module `module` in Core IR.
pub(crate) fn function_of<'a>(program: &'a Program, module: &str, name: &str) -> Option<&'a Fun> {
    let module = program.modules.iter().find(|m| m.path == module)?;
    module.decls.iter().find_map(|decl| match decl {
        Decl::Fun(fun) if fun.name == name => Some(fun.as_ref()),
        _ => None,
    })
}

/// The `serve:` grants among `grants`, with their TLS files' paths, for
/// [`una_http::serve::prepare`].
fn serve_inputs(grants: &[Grant]) -> Vec<una_http::serve::ServeInput<'_>> {
    grants
        .iter()
        .filter_map(|grant| match grant {
            Grant::Serve {
                grant,
                certificate,
                key,
            } => Some(una_http::serve::ServeInput {
                grant,
                certificate: certificate.as_deref(),
                key: key.as_deref(),
            }),
            _ => None,
        })
        .collect()
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

/// U6001 when `failed`, `main` or `init` of a driven module, failed, U6003 when a call went too deep, U6004 for a header value with a
/// control character, U6005 for host text that is not valid UTF-8, U6009 when the host gave no
/// random bytes for keyed hashing, U8001 on a fault.
fn completion_diagnostic(
    completion: &Completion,
    failed: &str,
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
                format!("{failed} failed with `{shown}`"),
                sources.location(main),
            ))
        }
        Completion::Aborted(abort) => Some(abort_diagnostic(abort, sources, root)),
    }
}

/// The one line on stderr of a started task at `path` that ended alone after `abort` (`spec/27
/// §7`).
fn task_aborted_line(path: &str, abort: &Abort, sources: &SourceDb) -> String {
    let (message, span) = match abort {
        Abort::Depth { span, limit } => (
            format!("this call exceeds the maximum call depth of {limit}"),
            Some(*span),
        ),
        _ => ("it stopped".to_owned(), None),
    };
    let location = span.map(|span| sources.location(span));
    una_interp::concurrency::ended_alone(
        path,
        &message,
        location
            .as_ref()
            .map(|at| (at.file.as_str(), at.start.line, at.start.col)),
    )
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
        // Before any code runs, so with no position of its own (`spec/13 §5`).
        Abort::NoRandomBytes => Diagnostic::error(
            Code::U6009,
            "the host gave no random bytes for keyed hashing",
            root.clone(),
        ),
        Abort::Tables { span } => Diagnostic::error(
            Code::U6003,
            "this call makes a hash table after the run's table counter reached its limit",
            sources.location(*span),
        ),
        Abort::HostStop { span, hard } => Diagnostic::error(
            Code::U6007,
            host_stop_message(*hard),
            sources.location(*span),
        ),
        Abort::Unstarted { span, cap } => Diagnostic::error(
            Code::U6003,
            match cap {
                Some(cap) => format!(
                    "this `start` exceeds the {cap} started tasks this system allows at once, a quarter of `vm.max_map_count`"
                ),
                None if span.is_some() => {
                    "this `start` could not run its task: no stack could be mapped for it"
                        .to_owned()
                }
                None => "the root task could not run: no stack could be mapped for it".to_owned(),
            },
            span.map_or_else(|| root.clone(), |span| sources.location(span)),
        ),
        Abort::Fault { message, span } => Diagnostic::error(
            Code::U8001,
            format!("evaluation stopped: {message}"),
            span.map_or_else(|| root.clone(), |span| sources.location(span)),
        ),
    }
}

/// `una test [path]`: checks the package at tier 1, then runs every test, an `example` in
/// editions 0.1 and 0.2 (`spec/08 §2`, `spec/11 §6`). The output names the declaration by the
/// package's edition and prints `una.test/2` from edition 0.3 (ADR 0105 rule 6).
pub(crate) fn test_command(
    path: &Path,
    options: RunOptions<'_>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> io::Result<ExitStatus> {
    let package = match crate::package::load_in(
        path,
        &std::collections::BTreeMap::new(),
        options.solving.itaoca.as_ref(),
    ) {
        Ok(package) => package,
        Err(error) => {
            writeln!(stderr, "error: {error}")?;
            return Ok(ExitStatus::Usage);
        }
    };
    let (manifest, _) = crate::package::project(&package);
    // The declared budget applies before any example runs (ADR 0109 rule 4).
    let options = match with_budget(options, manifest.max_memory, stderr)? {
        Ok(options) => options,
        Err(status) => return Ok(status),
    };
    // Edition `0.1` redacts labeled fields in an example's results and reasons; edition `0.2`
    // shows every field, since an example's data come from the source (ADR 0078).
    let edition = manifest.edition;
    let redact = edition == una_project::Edition::V0_1;
    let schema = if edition >= una_project::Edition::V0_3 {
        TEST_SCHEMA_0_3
    } else {
        TEST_SCHEMA
    };
    let mut analysis = on_large_stack(|| analyze(&package, None));
    if analysis.diagnostics.is_empty() {
        let found = tier_one(&package, &analysis.program, &options.solving);
        analysis.diagnostics = found.diagnostics;
    }
    if !analysis.diagnostics.is_empty() {
        let document = TestDocument {
            schema,
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
    let program = &analysis.program;
    let max_depth = options.max_depth;
    let limits = limits(&options, None);
    let ceiling = Ceiling::raise(&options);
    // The copy of the program is made, and dropped, on a thread of `CHECK_STACK_BYTES`, never
    // on the evaluator's, whose stack `--max-depth` sizes: the derived `Clone` and `Drop` of
    // Core IR recurse once per nested expression, which the checker bounds (#2264).
    let tested = on_large_stack(|| {
        let mut tested = program.clone();
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
        tested
    });
    let shared = &tested;
    let evaluated = std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("una-test".to_owned())
            .stack_size(stack)
            .spawn_scoped(scope, move || {
                una_interp::run_examples(shared, limits, redact)
            })?;
        Ok::<_, io::Error>(
            worker
                .join()
                .unwrap_or_else(|payload| std::panic::resume_unwind(payload)),
        )
    });
    let copy = std::sync::Mutex::new(Some(tested));
    on_large_stack(|| drop(copy.lock().ok().and_then(|mut copy| copy.take())));
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
            schema,
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
            "{} {} {} ... {verdict}",
            edition.test_word(),
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
        edition.test_word()
    } else {
        edition.tests_word()
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

/// `una.test/1`, with keys in the order of ADR 0032 rule 5, or `una.test/2` from edition 0.3:
/// the same document with the key `examples` renamed `tests` (ADR 0105 rule 6).
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

impl Serialize for TestDocument<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let runs = if self.schema == TEST_SCHEMA_0_3 {
            "tests"
        } else {
            "examples"
        };
        let mut document = serializer.serialize_struct("TestDocument", 7)?;
        document.serialize_field("schema", self.schema)?;
        document.serialize_field("tier", &self.tier)?;
        document.serialize_field("redacted", &self.redacted)?;
        document.serialize_field("diagnostics", &self.diagnostics)?;
        document.serialize_field(runs, &self.examples)?;
        document.serialize_field("passed", &self.passed)?;
        document.serialize_field("failed", &self.failed)?;
        document.end()
    }
}

/// One example of `una.test/1`, or one test of `una.test/2`.
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

/// The scripted engine behind `--serve-script` (ADR 0123 rule 21, `spec/31 §8` rule 6): the
/// connections of a checked script in place of listeners, and `--serve-out`. It runs only in
/// the toolchain's own test builds, when `UNA_TEST_RUN_SERVER` is 1 (`spec/31 §8` rule 6); every
/// other run reports the script as not run yet, while `una run` serves live peers (§1 rule 10).
///
/// It belongs to the run of `una run`, beside the live serve engine this file starts, and opens
/// no connection: the script stands for the peers.
mod scripted {
    use std::sync::Arc;

    use una_core::http::serve::ServeGrant;
    use una_http::serve::program::Port;
    use una_http::serve::script_file::Event;
    use una_interp::stopping::RunStop;

    use crate::Environment;

    /// The scripted engine of a run.
    #[cfg(feature = "test-hooks")]
    pub(crate) struct Engine(una_http::serve::script::ScriptPort);

    /// No build but the toolchain's tests has the scripted engine.
    #[cfg(not(feature = "test-hooks"))]
    pub(crate) enum Engine {}

    impl Engine {
        /// The engine of the grants `grants` fed by `events`, whose host stop is `stop`'s, when this
        /// build and `environment` run it; `None` otherwise.
        #[cfg(feature = "test-hooks")]
        pub(crate) fn start(
            environment: Environment<'_>,
            grants: &[ServeGrant],
            events: &[Event],
            stop: &Arc<RunStop>,
        ) -> Option<Engine> {
            if crate::variable(environment, "UNA_TEST_RUN_SERVER").as_deref() != Some("1") {
                return None;
            }
            let stop = Arc::clone(stop);
            Some(Engine(una_http::serve::script::ScriptPort::from_script(
                grants,
                events,
                Box::new(move || stop.request_local()),
            )))
        }

        #[cfg(not(feature = "test-hooks"))]
        pub(crate) fn start(
            _: Environment<'_>,
            _: &[ServeGrant],
            _: &[Event],
            _: &Arc<RunStop>,
        ) -> Option<Engine> {
            None
        }

        /// The engine as the program's operations meet it.
        pub(crate) fn port(&self) -> &dyn Port {
            #[cfg(feature = "test-hooks")]
            return &self.0;
            #[cfg(not(feature = "test-hooks"))]
            match *self {}
        }

        /// Ends the engine once the program has ended: the text of `--serve-out`, and what the
        /// engine left not at rest (counters, peers, timers, connections accepted before any server
        /// operation), which the run reports as warnings.
        pub(crate) fn end(&self) -> (String, Vec<String>) {
            #[cfg(feature = "test-hooks")]
            return {
                let ended = self.0.end();
                (
                    una_http::serve::script::out_text(&ended.wire),
                    ended.problems,
                )
            };
            #[cfg(not(feature = "test-hooks"))]
            match *self {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Int` with the label `label` and the refinements whose predicates are `predicates`.
    fn int(label: una_core::Label, predicates: &[bool]) -> Type {
        Type::Base {
            base: Base::Int,
            refinements: predicates
                .iter()
                .map(|p| una_core::Refinement {
                    name: "R".to_owned(),
                    clause: una_core::Clause {
                        term: una_core::Term::Bool(*p),
                        text: p.to_string(),
                        span: None,
                    },
                })
                .collect(),
            label,
        }
    }

    /// `fn(param) -> result`, public, with no effects or failures.
    fn function(param: Type, result: Type) -> Type {
        Type::Fn {
            params: vec![param],
            result: Box::new(result),
            uses: Vec::new(),
            fails: None,
            label: una_core::Label::public(),
        }
    }

    #[test]
    fn a_driven_state_is_compared_with_labels_aside_and_refinements_included() {
        // `spec/26 §2` rule 2, issue #1644: the run-time shape check, reached directly, since a
        // refinement in the state is U7008 in `una check` before any run.
        let public = una_core::Label::public;
        let secret = || una_core::Label::symbol("s");
        // Labels aside at every position, a function type's parameter and result included.
        assert!(same_type(&int(public(), &[]), &int(secret(), &[])));
        assert!(same_type(
            &function(int(public(), &[]), int(public(), &[])),
            &function(int(secret(), &[]), int(secret(), &[]))
        ));
        // Refinements compared by predicate, wherever they are written, and in order.
        assert!(same_type(&int(public(), &[true]), &int(secret(), &[true])));
        assert!(!same_type(&int(public(), &[]), &int(public(), &[true])));
        assert!(!same_type(
            &int(public(), &[true]),
            &int(public(), &[false])
        ));
        assert!(!same_type(
            &int(public(), &[true, false]),
            &int(public(), &[false, true])
        ));
        assert!(!same_type(
            &function(int(public(), &[true]), int(public(), &[])),
            &function(int(public(), &[]), int(public(), &[]))
        ));
        assert!(!same_type(
            &function(int(public(), &[]), int(public(), &[true])),
            &function(int(public(), &[]), int(public(), &[]))
        ));
        // A function type is never the same as a base type.
        assert!(!same_type(
            &function(int(public(), &[]), int(public(), &[])),
            &int(public(), &[])
        ));
    }
}
