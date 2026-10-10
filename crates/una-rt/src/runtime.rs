//! The runtime in safe Rust: the command line, grants, the call depth, host operations and the
//! trace of a native executable (`spec/16 §3`). [`crate::ffi`] calls it from the C ABI.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;
use una_diag::{Code, Diagnostic, Location, Position, render_human};
use una_interp::{Event, EventOutcome, Grant};

use crate::host::OpSite;
use crate::render::{Failure, Types};
use crate::values;

/// The exit code of a usage error (`spec/13 §5`, ADR 0036).
const USAGE: i32 = 2;

/// What `una build` embeds in the executable (`spec/16 §2`), as JSON: the `[run] grants`, every
/// `uses` of `main` with its operations and where it is written, where each call of the
/// program is written, for U6003, and the types and operations that traces and U6001 render.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    /// The grant specs of `[run] grants`.
    pub grants: Vec<String>,
    /// The `uses` clauses of `main`.
    pub uses: Vec<Need>,
    /// The calls of the program, by the index that the generated code passes on each call.
    pub sites: Vec<Location>,
    /// The types the program renders.
    pub types: Types,
    /// The operations of the program, by the index the generated code passes on each one.
    pub ops: Vec<OpSite>,
    /// What U6001 needs when `main` can fail.
    pub failure: Option<Failure>,
}

/// One `uses` clause of `main`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Need {
    /// The reference, without `@`.
    pub name: String,
    /// The capability.
    pub capability: String,
    /// The operations.
    pub ops: Vec<String>,
    /// Where the clause is written.
    pub location: Location,
}

impl Meta {
    /// The description `una build` embedded, or `None` when it is not one.
    pub fn parse(text: &str) -> Option<Meta> {
        let value: Value = serde_json::from_str(text).ok()?;
        let grants = value
            .get("grants")?
            .as_array()?
            .iter()
            .map(|g| g.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?;
        let uses = value
            .get("uses")?
            .as_array()?
            .iter()
            .map(|need| {
                Some(Need {
                    name: need.get("name")?.as_str()?.to_owned(),
                    capability: need.get("capability")?.as_str()?.to_owned(),
                    ops: need
                        .get("ops")?
                        .as_array()?
                        .iter()
                        .map(|op| op.as_str().map(str::to_owned))
                        .collect::<Option<Vec<_>>>()?,
                    location: location(need.get("location")?)?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let sites = match value.get("sites") {
            Some(sites) => sites
                .as_array()?
                .iter()
                .map(location)
                .collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };
        let types = match value.get("types") {
            Some(types) => Types::parse(types)?,
            None => Types::default(),
        };
        let ops = match value.get("ops") {
            Some(ops) => ops
                .as_array()?
                .iter()
                .map(OpSite::parse)
                .collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };
        let failure = match value.get("failure") {
            Some(failure) => Some(Failure {
                location: location(failure.get("location")?)?,
                root: crate::render::index(failure, "root")?,
            }),
            None => None,
        };
        Some(Meta {
            grants,
            uses,
            sites,
            types,
            ops,
            failure,
        })
    }
}

fn location(value: &Value) -> Option<Location> {
    let position = |value: &Value| {
        Some(Position {
            line: u32::try_from(value.get("line")?.as_u64()?).ok()?,
            col: u32::try_from(value.get("col")?.as_u64()?).ok()?,
        })
    };
    Some(Location {
        file: value.get("file")?.as_str()?.to_owned(),
        start: position(value.get("start")?)?,
        end: position(value.get("end")?)?,
    })
}

/// The options of the executable's command line (`spec/16 §3`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// The specs of `--grant` flags, in order.
    pub grants: Vec<String>,
    /// `--trace <file>`.
    pub trace: Option<PathBuf>,
    /// `--max-depth <n>`.
    pub max_depth: usize,
    /// `--max-memory <MiB>` (ADR 0061).
    pub max_memory: usize,
    /// `--max-response <MiB>` and `--http-timeout <seconds>` (`spec/24 §4`).
    pub http: una_http::client::Settings,
}

/// The command line, without the program name, or the message of a usage error.
pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        max_depth: una_interp::DEFAULT_MAX_DEPTH,
        max_memory: una_interp::DEFAULT_MAX_MEMORY,
        ..Options::default()
    };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| rest.next().cloned())
                .ok_or_else(|| format!("`{flag}` needs a value"))
        };
        match flag {
            "--grant" => options.grants.push(value()?),
            "--trace" => options.trace = Some(PathBuf::from(value()?)),
            "--max-depth" => {
                let text = value()?;
                options.max_depth = text
                    .parse()
                    .map_err(|_| format!("`--max-depth {text}` is not a number"))?;
            }
            "--max-memory" => {
                let text = value()?;
                options.max_memory = text
                    .parse()
                    .map_err(|_| format!("`--max-memory {text}` is not a number"))?;
                // No run fits in 0 MiB (#185).
                if options.max_memory == 0 {
                    return Err("`--max-memory 0` holds nothing; give at least 1 MiB".to_owned());
                }
            }
            "--max-response" | "--http-timeout" => {
                let text = value()?;
                let n: u64 = text
                    .parse()
                    .map_err(|_| format!("`{flag} {text}` is not a number"))?;
                if n == 0 {
                    return Err(format!("`{flag} 0` allows nothing; give at least 1"));
                }
                let day = una_http::client::MAX_TIMEOUT_SECS;
                if flag == "--http-timeout" && n > day {
                    return Err(format!(
                        "`{flag} {n}` is longer than a day; give at most {day}"
                    ));
                }
                if flag == "--max-response" {
                    options.http.limits.max_body =
                        usize::try_from(n.saturating_mul(1024 * 1024)).unwrap_or(usize::MAX);
                } else {
                    options.http.timeout = std::time::Duration::from_secs(n);
                }
            }
            other => {
                return Err(format!(
                    "unexpected argument `{other}`; the options are --grant <spec>, --trace <file>, --max-depth <n>, --max-memory <MiB>, --max-response <MiB> and --http-timeout <seconds>"
                ));
            }
        }
    }
    Ok(options)
}

/// The state of a run, on the thread that runs `main` and on each thread that runs a task
/// (`spec/19 §8`).
pub(crate) struct State {
    pub(crate) trace: Option<Vec<Event>>,
    trace_path: Option<PathBuf>,
    pub(crate) failed_write: Option<io::Error>,
    depth: usize,
    max_depth: usize,
    sites: Arc<Vec<Location>>,
    failure: Option<Arc<Failure>>,
    /// The grants of the run, and the keys of its `keys:` grants, which every task reads.
    pub(crate) grants: Arc<Vec<Grant>>,
    pub(crate) keys: Arc<BTreeMap<String, Vec<u8>>>,
    /// The limits and deadline of each HTTP request (`spec/24 §4`).
    pub(crate) http: una_http::client::Settings,
    pub(crate) types: Arc<Types>,
    pub(crate) ops: Arc<Vec<OpSite>>,
    /// Whether the last host operation failed.
    pub(crate) host_failed: bool,
    /// The number of instances of each handler that `with` installed so far: in the run, or
    /// in a task, since it started (`tasks`).
    pub(crate) installs: BTreeMap<String, usize>,
    /// Where the stack of the thread starts, and how much of it the code may use: past it, a
    /// call aborts with U6003 before the stack overflows (ADR 0032, ADR 0061 rule 1).
    stack_base: usize,
    stack_budget: usize,
    /// The memory budget in MiB, and what the heap held when `main` started (ADR 0061).
    max_memory: usize,
    baseline: usize,
    /// The path of the task the thread runs, empty for `main`'s (`spec/19 §6` rule 1).
    pub(crate) path: String,
    /// The cancellation flags of the task and of the tasks that run it.
    cancel: Vec<Arc<AtomicBool>>,
    /// How many of `cancel`'s flags an operation in progress shields: those set before it
    /// began, which it completes despite (`spec/19 §6` rule 4). Flags of tasks the operation
    /// starts come after them and still count.
    shielded: usize,
    /// The shields of the operations in progress, outermost first, to restore as each ends.
    operations: Vec<usize>,
    /// In a task, the abort that stopped it, U6003, U6004 or U6005, which its block reports.
    pub(crate) stopped: Option<(Code, Location, String)>,
}

/// What a task's state starts from: the run's settings, and the block's depth, path and
/// cancellation flags.
#[derive(Clone)]
pub(crate) struct TaskTemplate {
    tracing: bool,
    depth: usize,
    max_depth: usize,
    sites: Arc<Vec<Location>>,
    failure: Option<Arc<Failure>>,
    grants: Arc<Vec<Grant>>,
    keys: Arc<BTreeMap<String, Vec<u8>>>,
    http: una_http::client::Settings,
    types: Arc<Types>,
    ops: Arc<Vec<OpSite>>,
    max_memory: usize,
    baseline: usize,
    /// The path of the task that runs the block.
    pub(crate) path: String,
    cancel: Vec<Arc<AtomicBool>>,
    /// The flags of `cancel` that an operation in progress shields (`State::shielded`).
    shielded: usize,
    /// The stack of a task's thread.
    pub(crate) stack: usize,
    /// Where the block is written.
    pub(crate) location: Location,
}

impl TaskTemplate {
    /// Whether the task that runs the block, or one that runs it, was cancelled.
    pub(crate) fn cancelled(&self) -> bool {
        unshielded(&self.cancel, self.shielded)
    }
}

/// Whether one of `cancel`'s flags past the first `shielded` is set.
fn unshielded(cancel: &[Arc<AtomicBool>], shielded: usize) -> bool {
    cancel
        .iter()
        .skip(shielded)
        .any(|flag| flag.load(Ordering::SeqCst))
}

impl State {
    /// What the tasks of a block at `site` start from.
    pub(crate) fn for_tasks(&self, site: usize) -> TaskTemplate {
        let left = self.max_depth.saturating_sub(self.depth);
        TaskTemplate {
            tracing: self.trace.is_some(),
            depth: self.depth,
            max_depth: self.max_depth,
            sites: Arc::clone(&self.sites),
            failure: self.failure.clone(),
            grants: Arc::clone(&self.grants),
            keys: Arc::clone(&self.keys),
            http: self.http,
            types: Arc::clone(&self.types),
            ops: Arc::clone(&self.ops),
            max_memory: self.max_memory,
            baseline: self.baseline,
            path: self.path.clone(),
            cancel: self.cancel.clone(),
            shielded: self.shielded,
            stack: una_interp::stack_bytes(left).unwrap_or(STACK_RESERVE * 2),
            location: site_location(self, site),
        }
    }

    /// Whether the task the thread runs was cancelled, past what an operation in progress
    /// shields.
    fn cancelled(&self) -> bool {
        unshielded(&self.cancel, self.shielded)
    }

    /// The id of the `count`th instance of the handler `name` that the thread's task installed
    /// (`spec/19 §7` rule 2): `with:<name>#<count>` in `main`'s; in a task, an id with the
    /// task's path that its block renumbers when it adopts the task's events.
    pub(crate) fn install_id(&self, name: &str, count: usize) -> String {
        if self.path.is_empty() {
            format!("with:{name}#{count}")
        } else {
            format!("with:{name}#@{}#{count}", self.path)
        }
    }

    /// Appends the events of the task `task` of a block, in task order (`spec/19 §7` rule 2):
    /// numbers their `seq` after the events so far, and the handlers the task installed after
    /// those installed so far, `installs` being how many of each it installed.
    pub(crate) fn adopt(
        &mut self,
        task: &str,
        events: Vec<Event>,
        installs: &BTreeMap<String, usize>,
    ) {
        let local = format!("#@{task}#");
        let bases = self.installs.clone();
        if self.trace.is_some() {
            for mut event in events {
                if let Some(rest) = event.instance.strip_prefix("with:")
                    && let Some((name, count)) = rest.split_once(&local)
                    && let Ok(count) = count.parse::<usize>()
                {
                    let base = bases.get(name).copied().unwrap_or(0);
                    event.instance = self.install_id(name, base + count);
                }
                if let Some(trace) = self.trace.as_mut() {
                    event.seq = trace.len() + 1;
                    trace.push(event);
                }
            }
        }
        for (name, &count) in installs {
            *self.installs.entry(name.clone()).or_insert(0) += count;
        }
    }

    /// Appends the event that ends the task `task`, started and then cancelled
    /// (`spec/19 §7` rule 3).
    pub(crate) fn cancelled_event(&mut self, task: String) {
        if let Some(trace) = self.trace.as_mut() {
            trace.push(Event {
                seq: trace.len() + 1,
                task,
                instance: String::new(),
                capability: String::new(),
                op: String::new(),
                args: Vec::new(),
                outcome: EventOutcome::Cancelled,
            });
        }
    }
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// `f` of the state of the run, when the thread runs `main`.
pub(crate) fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| cell.borrow_mut().as_mut().map(f))
}

/// Runs the executable: parses the command line, checks `main`'s capabilities against the
/// grants, calls `entry`, the compiled `main`, on a thread with the stack that `--max-depth`
/// needs, and writes the trace. Returns the exit code.
pub fn start(args: &[String], meta: &str, entry: impl FnOnce() -> i64 + Send + 'static) -> i32 {
    let mut stderr = io::stderr();
    let Some(meta) = Meta::parse(meta) else {
        let _ = writeln!(
            stderr,
            "error: the executable's embedded description is invalid; rebuild it with `una build`"
        );
        return 3;
    };
    let options = match parse_args(args) {
        Ok(options) => options,
        Err(message) => {
            let _ = writeln!(stderr, "error: {message}");
            return USAGE;
        }
    };
    let grants = match grants(&meta, &options) {
        Ok(grants) => grants,
        Err(message) => {
            let _ = writeln!(stderr, "error: {message}");
            return USAGE;
        }
    };
    let Some(stack) = una_interp::stack_bytes(options.max_depth) else {
        let _ = writeln!(
            stderr,
            "error: `--max-depth {}` needs more stack than the system can address",
            options.max_depth
        );
        return USAGE;
    };
    let ungranted = check_grants(&meta, &grants);
    if !ungranted.is_empty() {
        // A failed write of the diagnostics is a failed write of the output: exit code 2, as
        // `una run` has (`spec/13 §5`, #198).
        let written = write!(
            stderr,
            "{}",
            render_human(&ungranted, &Default::default(), false)
        );
        return if written.is_ok() { 1 } else { USAGE };
    }
    // An unreadable or invalid key file is a usage error before `main`.
    let keys = match keys(&grants) {
        Ok(keys) => keys,
        Err(message) => {
            let _ = writeln!(stderr, "error: {message}");
            return USAGE;
        }
    };
    let state = State {
        trace: options.trace.as_ref().map(|_| Vec::new()),
        trace_path: options.trace.clone(),
        failed_write: None,
        depth: 0,
        max_depth: options.max_depth,
        sites: Arc::new(meta.sites),
        failure: meta.failure.map(Arc::new),
        grants: Arc::new(grants),
        keys: Arc::new(keys),
        http: options.http,
        types: Arc::new(meta.types),
        ops: Arc::new(meta.ops),
        host_failed: false,
        installs: BTreeMap::new(),
        stack_base: 0,
        stack_budget: stack.saturating_sub(STACK_RESERVE),
        max_memory: options.max_memory,
        baseline: 0,
        path: String::new(),
        cancel: Vec::new(),
        shielded: 0,
        operations: Vec::new(),
        stopped: None,
    };
    let budget = options.max_memory.saturating_mul(1024 * 1024);
    let worker = std::thread::Builder::new()
        .name("una-main".to_owned())
        .stack_size(stack)
        .spawn(move || {
            let mut state = state;
            state.stack_base = stack_position(&state);
            state.baseline = crate::held();
            // ADR 0061 rule 3: past twice the budget, allocation fails and the process aborts.
            crate::ceiling(state.baseline.saturating_add(budget.saturating_mul(2)));
            STATE.with(|cell| *cell.borrow_mut() = Some(state));
            let status = entry();
            crate::ceiling(usize::MAX);
            // A debug build of the runtime checks the compiled code's reference counting: it
            // never used a value it had released, and released every value it made, which is
            // unreferenced once `main` returns (`spec/16 §4`).
            //
            // Only the detection of an internal error depends on the build (unlike N-006, where a
            // check of the program did): for a program whose counting is right, the output, exit
            // status and trace are the same in both builds, and a release executable never runs
            // the check. The tests link the debug build, so a counting bug fails them.
            //
            // The tasks of `together` blocks check their own heaps when they end (`tasks`).
            let faults = if cfg!(debug_assertions) {
                (
                    values::faults() + crate::tasks::TASK_FAULTS.load(Ordering::SeqCst),
                    values::live() + crate::tasks::TASK_LEAKS.load(Ordering::SeqCst),
                    values::copies() + crate::tasks::TASK_COPIES.load(Ordering::SeqCst),
                    values::fallbacks() + crate::tasks::TASK_FALLBACKS.load(Ordering::SeqCst),
                )
            } else {
                (0, 0, 0, 0)
            };
            let state = STATE.with(|cell| cell.borrow_mut().take());
            (status, state, faults)
        });
    let (status, state, (faults, leaked, copies, fallbacks)) =
        match worker.map(std::thread::JoinHandle::join) {
            Ok(Ok(result)) => result,
            _ => {
                let _ = writeln!(
                    stderr,
                    "error: cannot start `main` with the stack that `--max-depth {}` needs",
                    options.max_depth
                );
                return USAGE;
            }
        };
    let mut code = if status == 0 { 0 } else { 1 };
    if let Some(state) = state {
        code = code.max(finish(state));
    }
    // A debug build also says how many lists, maps and texts were copied rather than
    // updated in place (#30, #236, #261), so tests can check that an update the code owns is in
    // place. As with the checks above, only a debug build says it, and it changes neither output
    // nor exit status.
    if copies > 0 {
        let _ = writeln!(
            stderr,
            "note: una-rt (debug build): lists, maps and texts copied: {copies}"
        );
    }
    // A record or variant made anew because another value held the one it could reuse costs
    // a constant, so it is counted apart from the copies, which grow with a value's size
    // (#292): a scaling test bounds the copies alone.
    if fallbacks > 0 {
        let _ = writeln!(
            stderr,
            "note: una-rt (debug build): records and variants made anew, not reused: {fallbacks}"
        );
    }
    if faults > 0 {
        let _ = writeln!(
            stderr,
            "error: internal error: uses of released values or misread type descriptors: {faults}"
        );
        code = 3;
    }
    if leaked > 0 {
        let _ = writeln!(
            stderr,
            "error: internal error: values still referenced after `main` returned: {leaked}"
        );
        code = 3;
    }
    code
}

/// Reports a failed write of the output and writes the trace; returns the exit code they
/// need, 0 when both went well.
fn finish(state: State) -> i32 {
    let mut stderr = io::stderr();
    let mut code = 0;
    if let Some(error) = state.failed_write {
        let _ = writeln!(stderr, "error: cannot write the output: {error}");
        code = USAGE;
    }
    if let (Some(path), Some(events)) = (&state.trace_path, state.trace)
        && let Err(error) = std::fs::write(path, una_interp::trace_json(&events))
    {
        let _ = writeln!(
            stderr,
            "error: cannot write the trace to `{}`: {error}",
            path.display()
        );
        code = USAGE;
    }
    code
}

/// The keys of a run's `keys:` grants, by name; the first grant with a name wins, as in
/// `una run`.
type Keys = BTreeMap<String, Vec<u8>>;

/// The grants of the run: the embedded `[run] grants`, then the `--grant` flags, every
/// relative path resolved against the current directory (`spec/16 §3`). A spec that is not a
/// grant is a usage error.
fn grants(meta: &Meta, options: &Options) -> Result<Vec<Grant>, String> {
    meta.grants
        .iter()
        .chain(&options.grants)
        .map(|spec| {
            Grant::parse(spec, Path::new("")).ok_or_else(|| {
                format!(
                    "`{spec}` is not a grant; grants are console, clock, env, keys:<file>, files:<path>:<access> and http:<origin>[:<methods>]"
                )
            })
        })
        .collect()
}

/// The keys of the `keys:` grants, read once `main`'s capabilities are known to be granted, as
/// `una run` reads them after U3012 (#185). A key named in two files keeps the first.
fn keys(grants: &[Grant]) -> Result<Keys, String> {
    let mut keys = Keys::new();
    for grant in grants {
        if let Grant::Keys { file } = grant {
            let text = std::fs::read_to_string(file).map_err(|error| {
                format!("cannot read the key file `{}`: {error}", file.display())
            })?;
            let loaded = una_interp::parse_keys(&text).map_err(|reason| {
                format!("the key file `{}` is invalid: {reason}", file.display())
            })?;
            for (name, key) in loaded {
                keys.entry(name).or_insert(key);
            }
        }
    }
    Ok(keys)
}

/// U3012 for each `uses` of `main` with an operation that no grant provides, as `una run`
/// reports it (`spec/13 §5`): a `files` grant provides the operations of its modes, and every
/// other grant every operation of its capability (#183).
pub fn check_grants(meta: &Meta, grants: &[Grant]) -> Vec<Diagnostic> {
    meta.uses
        .iter()
        .filter_map(|need| {
            let missing: Vec<String> = need
                .ops
                .iter()
                .filter(|op| !grants.iter().any(|g| provides(g, &need.capability, op)))
                .map(|op| format!("`{op}`"))
                .collect();
            (!missing.is_empty()).then(|| {
                Diagnostic::error(
                    Code::U3012,
                    format!(
                        "`main` uses {} of `{}`, which neither `[run] grants` nor a `--grant` flag grants",
                        missing.join(", "),
                        need.capability
                    ),
                    need.location.clone(),
                )
            })
        })
        .collect()
}

/// Whether `grant` provides the operation `op` of `capability`.
fn provides(grant: &Grant, capability: &str, op: &str) -> bool {
    match grant {
        Grant::Files { read, write, .. } => {
            capability == "Files" && ((op == "read" && *read) || (op == "write" && *write))
        }
        // An `http:` grant provides the operations of its methods (`spec/24 §2`).
        Grant::Http(http) => {
            capability == "Http" && http.methods().iter().any(|method| method.op() == op)
        }
        other => other.capability() == capability,
    }
}

/// The stack kept free below the budget of `main`'s thread, for the runtime's own work after the
/// last check: performing a host operation, rendering a value, writing the trace.
const STACK_RESERVE: usize = 8 * 1024 * 1024;

/// The address of `local`, a variable of the caller's frame: a measure of the stack in use.
fn stack_position<T>(local: &T) -> usize {
    std::ptr::from_ref(local).addr()
}

/// Enters the call at `site`: past the maximum depth, or when the stack of the thread runs
/// short before it, the run aborts with U6003 at the call, as in the interpreter
/// (`spec/11 §7`, ADR 0032), and so it does when the heap the run holds is over its budget
/// (ADR 0061). The output so far stays printed, the trace is written, and the executable exits
/// with code 1.
///
/// Returns 1 when the task must stop instead of making the call: it was cancelled, or, in a
/// task, the abort stops it and goes to its block (`tasks`). Returns 0 otherwise.
pub fn enter(site: usize) -> i64 {
    let marker = site;
    let position = stack_position(&marker);
    let held = crate::held();
    let abort = STATE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let state = cell.as_mut()?;
        if state.cancelled() {
            return Some(None);
        }
        let used = state.stack_base.abs_diff(position);
        let message = if state.depth >= state.max_depth || used > state.stack_budget {
            format!(
                "this call exceeds the maximum call depth of {}",
                state.max_depth
            )
        } else if let Some(message) = over_budget(state, held) {
            message
        } else {
            state.depth += 1;
            return None;
        };
        let location = site_location(state, site);
        Some(stop_or_abort(&mut cell, Code::U6003, location, message))
    });
    match abort {
        None => 0,
        Some(None) => 1,
        Some(Some((state, code, location, message))) => {
            abort_run(state, code, location, message);
            1
        }
    }
}

/// Checks the memory budget at an iteration of the `for` loop at `site`: iterations are not
/// calls, so this is the only check inside a loop that builds a value (ADR 0061 rule 2). Over
/// the budget, the run aborts with U6003 at the loop, as at a call. Returns 1 when the task
/// must stop, as [`enter`] does, and 0 otherwise.
pub fn budget(site: usize) -> i64 {
    let held = crate::held();
    let abort = STATE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let state = cell.as_mut()?;
        if state.cancelled() {
            return Some(None);
        }
        let message = over_budget(state, held)?;
        let location = site_location(state, site);
        Some(stop_or_abort(&mut cell, Code::U6003, location, message))
    });
    match abort {
        None => 0,
        Some(None) => 1,
        Some(Some((state, code, location, message))) => {
            abort_run(state, code, location, message);
            1
        }
    }
}

/// The text of `word`, a value of the type at `ty` in the program's type descriptions, as
/// interpolation writes it, with every field (`spec/11 §5`, ADR 0068). A new reference.
/// `params` are the descriptors of the type parameters that the type holds, by position.
pub fn written(ty: usize, word: i64, params: &[i64]) -> i64 {
    let text = with_state(|state| state.types.written(ty, word, params)).unwrap_or_default();
    values::text_word(text)
}

/// `written` of the type at `ty`, as the generated code passes it.
pub fn write(ty: i64, word: i64) -> i64 {
    written(usize::try_from(ty).unwrap_or(usize::MAX), word, &[])
}

/// Returns 1 when the task must stop before an operation, having been cancelled, and 0
/// otherwise (`spec/19 §6` rule 4).
pub fn poll() -> i64 {
    with_state(|state| i64::from(state.cancelled())).unwrap_or(0)
}

/// 1 when an operation stopped the task the thread runs, as U6004 (N-40) and U6005 (S-16) do in
/// a task: its caller stops it before using the operation's value.
pub fn stopped() -> i64 {
    with_state(|state| i64::from(state.stopped.is_some())).unwrap_or(0)
}

/// Begins an operation, past its [`poll`]: an operation in progress completes, so until
/// [`operation_end`] the cancellations already possible, of the task and of the tasks that run
/// it, are not seen; the limits of depth and memory still are (`spec/19 §6` rule 4, `§7`
/// rule 3, #200).
pub fn operation_begin() {
    with_state(|state| {
        state.operations.push(state.shielded);
        state.shielded = state.cancel.len();
    });
}

/// Ends the operation [`operation_begin`] began: a cancellation that came meanwhile takes
/// effect at the task's next call, operation or loop iteration.
pub fn operation_end() {
    with_state(|state| {
        if let Some(shielded) = state.operations.pop() {
            state.shielded = shielded;
        }
    });
}

/// An abort with `message` at `location`: in a task, recorded for its block, which the task
/// stops to reach; in `main`'s thread, the state to end the run with.
fn stop_or_abort(
    cell: &mut Option<State>,
    code: Code,
    location: Location,
    message: String,
) -> Option<(State, Code, Location, String)> {
    match cell {
        Some(state) if !state.path.is_empty() => {
            state.stopped = Some((code, location, message));
            None
        }
        _ => cell.take().map(|state| (state, code, location, message)),
    }
}

/// The abort of a task of a block that the thread runs, at `location` with `message`: in a
/// task, it stops it in turn; in `main`'s thread, it ends the run. Returns 0, the value of a
/// stopped block.
pub(crate) fn abort_from_task(code: Code, location: Location, message: String) -> i64 {
    let abort = STATE.with(|cell| stop_or_abort(&mut cell.borrow_mut(), code, location, message));
    if let Some((state, code, location, message)) = abort {
        abort_run(state, code, location, message);
    }
    0
}

/// Aborts the run, or the task the thread runs, with `code` at the call `site`, as a call
/// over the depth does: U6004 for a header value with a control character (N-40), U6005 for host
/// text that is not valid UTF-8 (S-16, ADR 0086). Returns 0, the value the caller drops: a task
/// stops, and its block reports the abort.
pub(crate) fn abort_at(site: usize, code: Code, message: String) -> i64 {
    let abort = STATE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let location = cell.as_ref().map(|state| site_location(state, site))?;
        stop_or_abort(&mut cell, code, location, message)
    });
    if let Some((state, code, location, message)) = abort {
        abort_run(state, code, location, message);
    }
    0
}

/// Starts the state of a task, on its thread, from `template`, with its path and its own
/// cancellation flag.
pub(crate) fn enter_task(template: &TaskTemplate, path: String, flag: Arc<AtomicBool>) {
    let mut cancel = template.cancel.clone();
    cancel.push(flag);
    let mut state = State {
        trace: template.tracing.then(Vec::new),
        trace_path: None,
        failed_write: None,
        depth: template.depth,
        max_depth: template.max_depth,
        sites: Arc::clone(&template.sites),
        failure: template.failure.clone(),
        grants: Arc::clone(&template.grants),
        keys: Arc::clone(&template.keys),
        http: template.http,
        types: Arc::clone(&template.types),
        ops: Arc::clone(&template.ops),
        host_failed: false,
        installs: BTreeMap::new(),
        stack_base: 0,
        stack_budget: template.stack.saturating_sub(STACK_RESERVE),
        max_memory: template.max_memory,
        baseline: template.baseline,
        path,
        cancel,
        shielded: template.shielded,
        operations: Vec::new(),
        stopped: None,
    };
    state.stack_base = stack_position(&state);
    STATE.with(|cell| *cell.borrow_mut() = Some(state));
}

/// Ends the state of the task the thread runs, and returns it.
pub(crate) fn leave_task() -> Option<State> {
    STATE.with(|cell| cell.borrow_mut().take())
}

/// The U6003 message when the heap the run holds, `held` bytes, is over its budget.
fn over_budget(state: &State, held: usize) -> Option<String> {
    let budget = state.max_memory.saturating_mul(1024 * 1024);
    (held.saturating_sub(state.baseline) > budget).then(|| {
        format!(
            "this call exceeds the memory limit of {} MiB",
            state.max_memory
        )
    })
}

fn site_location(state: &State, site: usize) -> Location {
    state.sites.get(site).cloned().unwrap_or(Location {
        file: String::new(),
        start: Position { line: 1, col: 1 },
        end: Position { line: 1, col: 1 },
    })
}

/// Ends the run with `code`, U6003, U6004 or U6005, at `location`, as the interpreter's abort does: the output so far
/// stays printed, the trace is written, and the executable exits with code 1; with U8001, an
/// internal error of the runtime (#424), it exits with code 3 (`spec/13 §5`).
fn abort_run(state: State, code: Code, location: Location, message: String) {
    let least = if code == Code::U8001 { 3 } else { 1 };
    crate::ceiling(usize::MAX);
    let _ = io::stdout().flush();
    let diagnostic = Diagnostic::error(code, message, location);
    let written = write!(
        io::stderr(),
        "{}",
        render_human(&[diagnostic], &Default::default(), false)
    );
    let mut code = finish(state).max(least);
    if written.is_err() {
        code = USAGE;
    }
    // Nothing unwinds through the compiled frames (spec/16 §4 rule 3).
    #[allow(clippy::exit)]
    std::process::exit(code);
}

/// Reports that `main` failed with `failure`, which the caller owns and the report releases:
/// U6001 with the failure's `public` fields (`spec/04 §8`), after the output so far.
pub fn main_failed(failure: i64) {
    let _ = io::stdout().flush();
    let diagnostic = STATE.with(|cell| {
        let cell = cell.borrow();
        let state = cell.as_ref()?;
        let described = state.failure.as_ref()?;
        Some(Diagnostic::error(
            Code::U6001,
            format!(
                "`main` failed with `{}`",
                state.types.shown(described.root, failure)
            ),
            described.location.clone(),
        ))
    });
    values::release(failure);
    if let Some(diagnostic) = diagnostic
        && let Err(error) = write!(
            io::stderr(),
            "{}",
            render_human(&[diagnostic], &Default::default(), false)
        )
    {
        // Reported at the end with exit code 2, as a failed write of the output is (#198).
        with_state(|state| {
            if state.failed_write.is_none() {
                state.failed_write = Some(error);
            }
        });
    }
}

/// Leaves a call.
pub fn leave() {
    STATE.with(|cell| {
        if let Some(state) = cell.borrow_mut().as_mut() {
            state.depth = state.depth.saturating_sub(1);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn the_command_line_takes_grants_a_trace_and_a_depth() {
        let options = parse_args(&strings(&[
            "--grant",
            "console",
            "--grant=env",
            "--trace",
            "t.json",
            "--max-depth",
            "50",
        ]))
        .unwrap();
        assert_eq!(options.grants, ["console", "env"]);
        assert_eq!(options.trace, Some(PathBuf::from("t.json")));
        assert_eq!(options.max_depth, 50);
        assert_eq!(options.max_memory, una_interp::DEFAULT_MAX_MEMORY);
        let options = parse_args(&strings(&["--max-memory", "64"])).unwrap();
        assert_eq!(options.max_memory, 64);
        assert!(parse_args(&strings(&["--max-memory", "many"])).is_err());
        assert!(parse_args(&strings(&["--json"])).is_err());
        assert!(parse_args(&strings(&["--grant"])).is_err());
        assert!(parse_args(&strings(&["--max-depth", "many"])).is_err());
    }

    #[test]
    fn a_debug_runtime_reports_values_left_referenced_or_used_after_release() {
        let meta = r#"{"grants":[],"uses":[]}"#;
        let freed = start(&[], meta, || {
            values::release(values::text_word("freed".to_owned()));
            0
        });
        assert_eq!(freed, 0);
        let leaked = start(&[], meta, || {
            values::text_word("kept".to_owned());
            0
        });
        assert_eq!(leaked, 3);
        let reused = start(&[], meta, || {
            let text = values::text_word("twice".to_owned());
            values::release(text);
            values::release(text);
            0
        });
        assert_eq!(reused, 3);
    }

    #[test]
    fn main_needs_a_grant_for_each_capability_it_uses() {
        let meta = Meta::parse(
            r#"{"grants":[],"uses":[{"name":"console","capability":"Console","ops":["print"],"location":{"file":"src/hello.una","start":{"line":4,"col":3},"end":{"line":4,"col":33}}}]}"#,
        )
        .unwrap();
        assert!(meta.sites.is_empty());
        let missing = check_grants(&meta, &[]);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].code, Code::U3012);
        assert_eq!(
            missing[0].message,
            "`main` uses `print` of `Console`, which neither `[run] grants` nor a `--grant` flag grants"
        );
        assert!(check_grants(&meta, &[Grant::Console]).is_empty());
    }

    #[test]
    fn a_files_grant_provides_only_the_operations_of_its_modes() {
        let meta = Meta::parse(
            r#"{"grants":[],"uses":[{"name":"files","capability":"Files","ops":["read","write"],"location":{"file":"src/copy.una","start":{"line":4,"col":3},"end":{"line":4,"col":36}}}]}"#,
        )
        .unwrap();
        let files = |read, write| Grant::Files {
            prefix: PathBuf::from("data"),
            read,
            write,
        };
        let missing = check_grants(&meta, &[Grant::Console, files(true, false)]);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].code, Code::U3012);
        assert_eq!(
            missing[0].message,
            "`main` uses `write` of `Files`, which neither `[run] grants` nor a `--grant` flag grants"
        );
        assert!(check_grants(&meta, &[files(true, true)]).is_empty());
        assert!(check_grants(&meta, &[files(true, false), files(false, true)]).is_empty());
    }
}
