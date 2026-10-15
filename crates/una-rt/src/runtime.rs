//! The runtime in safe Rust: the command line, grants, the call depth, host operations and the
//! trace of a native executable (`spec/16 §3`). [`crate::ffi`] calls it from the C ABI.

use std::cell::{Cell, RefCell};
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
    /// The maximum call depth. The current depth is `Hot::depth`, its one copy (ADR 0102).
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
        let depth = HOT.with(|hot| hot.depth.get());
        let left = self.max_depth.saturating_sub(depth);
        TaskTemplate {
            tracing: self.trace.is_some(),
            depth,
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
    static HOT: Hot = const { Hot::inactive() };
}

#[cfg(test)]
thread_local! {
    /// How many checks of this thread took the slow path (ADR 0102): the tests count
    /// operations, not time.
    static SLOW_PATHS: Cell<usize> = const { Cell::new(0) };
}

/// Counts a check that takes the slow path, in tests.
fn slow_path() {
    #[cfg(test)]
    SLOW_PATHS.with(|count| count.set(count.get() + 1));
}

/// What the checks at every call and loop iteration read, mirrored from the thread's state so
/// that the common case needs no borrow of it (ADR 0102). Only `Copy` cells: the thread-local
/// has no destructor, so its first access registers none, and nothing here allocates, which
/// would move the point where the memory budget fires.
struct Hot {
    /// Whether the thread has a run's state.
    active: Cell<bool>,
    /// The current call depth, its one copy.
    depth: Cell<usize>,
    max_depth: Cell<usize>,
    stack_base: Cell<usize>,
    stack_budget: Cell<usize>,
    /// The heap the run may hold before a check reports it ([`mem_ceiling`]).
    mem_ceiling: Cell<usize>,
    /// `State::shielded`, and the length of `State::cancel`: no flag is unshielded when
    /// `shielded >= flags`.
    shielded: Cell<usize>,
    flags: Cell<usize>,
}

impl Hot {
    /// The record of a thread without a run's state: every check lets the call proceed.
    const fn inactive() -> Hot {
        Hot {
            active: Cell::new(false),
            depth: Cell::new(0),
            max_depth: Cell::new(0),
            stack_base: Cell::new(0),
            stack_budget: Cell::new(0),
            mem_ceiling: Cell::new(usize::MAX),
            shielded: Cell::new(0),
            flags: Cell::new(0),
        }
    }

    /// Whether a flag past the shielded ones is set: at once false on `main`'s thread, which
    /// has no flag; otherwise each one loaded, as `State::cancelled` does, behind a shared
    /// borrow, which allocates nothing.
    fn cancelled(&self) -> bool {
        self.shielded.get() < self.flags.get()
            && STATE.with(|cell| cell.borrow().as_ref().is_some_and(State::cancelled))
    }
}

/// The heap a run may hold before a check reports it: `baseline` plus the budget of
/// `max_memory` MiB, both saturating. `held > mem_ceiling(baseline, max_memory)` exactly when
/// [`over_budget`] reports (ADR 0102 rule 5).
fn mem_ceiling(baseline: usize, max_memory: usize) -> usize {
    baseline.saturating_add(max_memory.saturating_mul(1024 * 1024))
}

/// Installs `state` as the thread's, at the call depth `depth`, and mirrors it in [`Hot`]: the
/// one place, with [`take_state`], that writes either (ADR 0102 rule 3).
fn install(state: State, depth: usize) {
    HOT.with(|hot| {
        hot.depth.set(depth);
        hot.max_depth.set(state.max_depth);
        hot.stack_base.set(state.stack_base);
        hot.stack_budget.set(state.stack_budget);
        hot.mem_ceiling
            .set(mem_ceiling(state.baseline, state.max_memory));
        hot.shielded.set(state.shielded);
        hot.flags.set(state.cancel.len());
        hot.active.set(true);
    });
    STATE.with(|cell| *cell.borrow_mut() = Some(state));
}

/// Takes the thread's state out of `cell`, clearing [`Hot`] first, so that no check reads a
/// record whose state is gone.
fn take_state(cell: &mut Option<State>) -> Option<State> {
    HOT.with(|hot| {
        hot.active.set(false);
        hot.depth.set(0);
        hot.max_depth.set(0);
        hot.stack_base.set(0);
        hot.stack_budget.set(0);
        hot.mem_ceiling.set(usize::MAX);
        hot.shielded.set(0);
        hot.flags.set(0);
    });
    cell.take()
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
            install(state, 0);
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
            let state = STATE.with(|cell| take_state(&mut cell.borrow_mut()));
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
    // ADR 0102: the conjunction of the negated abort conditions, on `Hot`; when it fails, the
    // check below decides, with the same position and `held`.
    let proceed = HOT.with(|hot| {
        if !hot.active.get() {
            return true;
        }
        let depth = hot.depth.get();
        let clear = depth < hot.max_depth.get()
            && hot.stack_base.get().abs_diff(position) <= hot.stack_budget.get()
            && held <= hot.mem_ceiling.get()
            && !hot.cancelled();
        if clear {
            // No borrow of the state runs compiled code (ADR 0102 rule 2).
            debug_assert!(STATE.with(|cell| cell.try_borrow_mut().is_ok()));
            hot.depth.set(depth + 1);
        }
        clear
    });
    if proceed {
        return 0;
    }
    slow_path();
    let abort = STATE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let state = cell.as_mut()?;
        if state.cancelled() {
            return Some(None);
        }
        let depth = HOT.with(|hot| hot.depth.get());
        let used = state.stack_base.abs_diff(position);
        let message = if depth >= state.max_depth || used > state.stack_budget {
            format!(
                "this call exceeds the maximum call depth of {}",
                state.max_depth
            )
        } else if let Some(message) = over_budget(state, held) {
            message
        } else {
            HOT.with(|hot| hot.depth.set(depth + 1));
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
    // ADR 0102: the negated abort conditions, on `Hot`.
    let proceed =
        HOT.with(|hot| !hot.active.get() || (held <= hot.mem_ceiling.get() && !hot.cancelled()));
    if proceed {
        return 0;
    }
    slow_path();
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
    // ADR 0102: no unshielded flag set, on `Hot`.
    if HOT.with(|hot| !hot.active.get() || !hot.cancelled()) {
        return 0;
    }
    slow_path();
    with_state(|state| i64::from(state.cancelled())).unwrap_or(0)
}

/// 1 when an operation stopped the task the thread runs, as U6004 and U6005 (S-16) do in
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
        HOT.with(|hot| hot.shielded.set(state.shielded));
    });
}

/// Ends the operation [`operation_begin`] began: a cancellation that came meanwhile takes
/// effect at the task's next call, operation or loop iteration.
pub fn operation_end() {
    with_state(|state| {
        if let Some(shielded) = state.operations.pop() {
            state.shielded = shielded;
            HOT.with(|hot| hot.shielded.set(shielded));
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
        _ => take_state(cell).map(|state| (state, code, location, message)),
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
/// over the depth does: U6004 for a header value with a control character, U6005 for host
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
    install(state, template.depth);
}

/// Ends the state of the task the thread runs, and returns it.
pub(crate) fn leave_task() -> Option<State> {
    STATE.with(|cell| take_state(&mut cell.borrow_mut()))
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
    HOT.with(|hot| {
        if hot.active.get() {
            hot.depth.set(hot.depth.get().saturating_sub(1));
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

    /// A description with one site, `src/t.una` at 3:3, for the checks' locations.
    const SITE_META: &str = r#"{"grants":[],"uses":[],"sites":[{"file":"src/t.una","start":{"line":3,"col":3},"end":{"line":3,"col":9}}]}"#;

    /// Printed by a scenario at each step it passes, so that the test sees how far it got.
    const STEP: &str = "fast-path-step";

    /// Runs the scenario `name` of this module in a process of its own, since an abort ends the
    /// process: its exit code and its standard error.
    fn scenario(name: &str) -> (Option<i32>, String) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                &format!("runtime::tests::{name}"),
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .output()
            .unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// How many steps a scenario passed.
    fn steps(stderr: &str) -> usize {
        stderr.lines().filter(|line| line.starts_with(STEP)).count()
    }

    #[test]
    fn the_memory_ceiling_is_the_budget_comparison() {
        // ADR 0102 rule 5: `held > mem_ceiling(baseline, max_memory)` exactly when
        // `over_budget` reports, on the edge values and on a grid around them.
        let mib = 1024 * 1024;
        let big = usize::MAX / mib;
        let mut values = vec![0, 1, 2, mib - 1, mib, mib + 1, usize::MAX - 1, usize::MAX];
        values.extend([usize::MAX / 2, usize::MAX - mib, usize::MAX - mib + 1]);
        let memories = [1, 2, 64, big - 1, big, big + 1, usize::MAX];
        for &baseline in &values {
            for &max_memory in &memories {
                let ceiling = mem_ceiling(baseline, max_memory);
                let budget = max_memory.saturating_mul(mib);
                let mut helds = values.clone();
                for delta in [0, 1, 2] {
                    helds.push(ceiling.saturating_sub(delta));
                    helds.push(ceiling.saturating_add(delta));
                    helds.push(baseline.saturating_add(budget).saturating_sub(delta));
                }
                for held in helds {
                    let over = held.saturating_sub(baseline) > budget;
                    assert_eq!(
                        held > ceiling,
                        over,
                        "baseline {baseline}, max_memory {max_memory}, held {held}"
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "run by `the_depth_limit_fires_at_its_exact_call`, in a process of its own"]
    fn the_depth_limit_scenario() {
        // `--max-depth 5`: five calls in, then one call out and in again, then the sixth call.
        start(&strings(&["--max-depth", "5"]), SITE_META, || {
            for _ in 0..5 {
                assert_eq!(enter(0), 0);
                eprintln!("{STEP}");
            }
            leave();
            assert_eq!(enter(0), 0);
            eprintln!("{STEP}");
            enter(0);
            eprintln!("{STEP}: past the limit");
            0
        });
    }

    #[test]
    fn the_depth_limit_fires_at_its_exact_call() {
        // ADR 0102 rule 8: the call at the limit aborts with U6003 at its site, and every call
        // under it runs, the depth going down at `leave`.
        let (code, stderr) = scenario("the_depth_limit_scenario");
        assert_eq!(code, Some(1), "{stderr}");
        assert_eq!(steps(&stderr), 6, "{stderr}");
        assert!(stderr.contains("U6003"), "{stderr}");
        assert!(
            stderr.contains("this call exceeds the maximum call depth of 5"),
            "{stderr}"
        );
        assert!(stderr.contains("src/t.una"), "{stderr}");
    }

    /// Holds exactly up to the memory ceiling of the run, then checks with `check`, which must
    /// let the run proceed; then one byte more, where `check` must abort it.
    fn memory_scenario(check: fn() -> i64) {
        start(&strings(&["--max-memory", "1"]), SITE_META, move || {
            let ceiling = HOT.with(|hot| hot.mem_ceiling.get());
            let held = crate::held();
            let fill: Vec<u8> = Vec::with_capacity(ceiling - held);
            assert_eq!(check(), 0);
            eprintln!("{STEP}");
            let more: Vec<u8> = Vec::with_capacity(1);
            check();
            eprintln!("{STEP}: past the budget");
            drop((fill, more));
            0
        });
    }

    #[test]
    #[ignore = "run by `the_memory_budget_fires_at_its_exact_byte`, in a process of its own"]
    fn the_memory_budget_at_a_call_scenario() {
        memory_scenario(|| enter(0));
    }

    #[test]
    #[ignore = "run by `the_memory_budget_fires_at_its_exact_byte`, in a process of its own"]
    fn the_memory_budget_at_an_iteration_scenario() {
        memory_scenario(|| budget(0));
    }

    #[test]
    fn the_memory_budget_fires_at_its_exact_byte() {
        // ADR 0102 rule 8: holding exactly the budget runs, at a call and at a loop iteration,
        // and one byte more aborts with U6003 at the site.
        for name in [
            "the_memory_budget_at_a_call_scenario",
            "the_memory_budget_at_an_iteration_scenario",
        ] {
            let (code, stderr) = scenario(name);
            assert_eq!(code, Some(1), "{name}: {stderr}");
            assert_eq!(steps(&stderr), 1, "{name}: {stderr}");
            assert!(
                stderr.contains("this call exceeds the memory limit of 1 MiB"),
                "{name}: {stderr}"
            );
            assert!(stderr.contains("src/t.una"), "{name}: {stderr}");
        }
    }

    #[test]
    #[ignore = "run by `the_mechanism_allocates_nothing`, in a process of its own"]
    fn the_mechanism_allocates_nothing_scenario() {
        start(&[], SITE_META, || {
            // Warm: the first operation's shield grows `operations` once, as it does today.
            operation_begin();
            operation_end();
            let before = crate::held();
            operation_begin();
            operation_end();
            let state = STATE.with(|cell| take_state(&mut cell.borrow_mut()));
            let after_take = crate::held();
            if let Some(state) = state {
                install(state, 0);
            }
            let after = crate::held();
            assert_eq!(enter(0), 0);
            leave();
            assert_eq!(budget(0), 0);
            assert_eq!(poll(), 0);
            let checked = crate::held();
            eprintln!("{STEP} {before} {after_take} {after} {checked}");
            0
        });
    }

    #[test]
    fn the_mechanism_allocates_nothing() {
        // ADR 0102 rule 3: installing and taking the state, an operation's shield and the
        // checks hold no byte more, so the memory budget fires where it did.
        let (code, stderr) = scenario("the_mechanism_allocates_nothing_scenario");
        assert_eq!(code, Some(0), "{stderr}");
        let line = stderr
            .lines()
            .find(|line| line.starts_with(STEP))
            .unwrap_or_else(|| panic!("{stderr}"));
        let held: Vec<&str> = line.split_whitespace().skip(1).collect();
        assert_eq!(held.len(), 4, "{line}");
        assert!(held.iter().all(|h| *h == held[0]), "{line}");
    }

    #[test]
    #[ignore = "run by `a_cancellation_from_another_thread_stops_the_next_check`, in a process of its own"]
    fn a_cancellation_from_another_thread_scenario() {
        start(&[], SITE_META, || {
            let template = with_state(|state| state.for_tasks(0)).unwrap();
            let flag = Arc::new(AtomicBool::new(false));
            let (ready, wait_ready) = std::sync::mpsc::channel();
            let (set, wait_set) = std::sync::mpsc::channel();
            let task_flag = Arc::clone(&flag);
            let task = std::thread::spawn(move || {
                enter_task(&template, "1".to_owned(), task_flag);
                // On the fast path: no flag set.
                let clear = (poll(), budget(0), enter(0));
                leave();
                ready.send(()).unwrap();
                wait_set.recv().unwrap();
                // The flag was set by `main`'s thread meanwhile: the very next checks see it.
                let stopped = (poll(), budget(0), enter(0));
                // An operation in progress shields the flag set before it, until it ends.
                operation_begin();
                let shielded = poll();
                operation_end();
                let after = poll();
                leave_task();
                (clear, stopped, shielded, after)
            });
            wait_ready.recv().unwrap();
            flag.store(true, Ordering::SeqCst);
            set.send(()).unwrap();
            let (clear, stopped, shielded, after) = task.join().unwrap();
            eprintln!("{STEP} {clear:?} {stopped:?} {shielded} {after}");
            0
        });
    }

    #[test]
    fn a_cancellation_from_another_thread_stops_the_next_check() {
        // ADR 0102 rule 4: `Hot` never holds whether a flag is set, so a flag that another
        // thread sets after the task's state was installed stops its next call, iteration and
        // operation; an operation in progress shields it until it ends.
        let (code, stderr) = scenario("a_cancellation_from_another_thread_scenario");
        assert_eq!(code, Some(0), "{stderr}");
        assert!(
            stderr.contains(&format!("{STEP} (0, 0, 0) (1, 1, 1) 0 1")),
            "{stderr}"
        );
    }

    #[test]
    #[ignore = "run by `the_checks_take_the_fast_path_at_every_size_and_depth`, in a process of its own"]
    fn the_checks_scaling_scenario() {
        start(&strings(&["--max-depth", "5000"]), SITE_META, || {
            let slow = || SLOW_PATHS.with(Cell::get);
            let checks = |n: u32| {
                let before = slow();
                for _ in 0..n {
                    enter(0);
                    leave();
                    budget(0);
                    poll();
                }
                slow() - before
            };
            let small = checks(1_000);
            let large = checks(100_000);
            for _ in 0..10 {
                enter(0);
            }
            let shallow = checks(1_000);
            for _ in 0..990 {
                enter(0);
            }
            let deep = checks(1_000);
            eprintln!("{STEP} {small} {large} {shallow} {deep}");
            0
        });
    }

    #[test]
    fn the_checks_take_the_fast_path_at_every_size_and_depth() {
        // ADR 0102 rule 6: on `main`'s thread, under every limit, each call, iteration and
        // operation check stays on the fast path, whose work is O(1): the slow path is never
        // taken, whatever the number of calls or the depth. Operations are counted, not
        // time, so the machine's load cannot fail it.
        let (code, stderr) = scenario("the_checks_scaling_scenario");
        assert_eq!(code, Some(0), "{stderr}");
        assert!(stderr.contains(&format!("{STEP} 0 0 0 0")), "{stderr}");
    }
}
