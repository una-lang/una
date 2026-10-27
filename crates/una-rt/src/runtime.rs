//! The runtime in safe Rust: the command line, grants, the call depth, host operations and the
//! trace of a native executable (`spec/16 §3`). [`crate::ffi`] calls it from the C ABI.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use serde_json::Value;
use una_diag::{Code, Diagnostic, Location, Position, render_human};
use una_interp::stopping::{HardStop, RunStop};
use una_interp::{Clock, Event, EventOutcome, Grant};

use crate::host::OpSite;
use crate::render::{Failure, Types};
use crate::values;

/// The exit code of a usage error (`spec/13 §5`, ADR 0036).
const USAGE: i32 = 2;

/// The bytes of stack that the run's live threads reserve: the thread that runs `main`'s code
/// and every task's. With [`crate::held`], it bounds the words that can hold a reference, which
/// decides how a count at its limit is reported ([`count_limit_reached`], ADR 0113).
static STACKS: AtomicUsize = AtomicUsize::new(0);

/// A thread's stack, counted in [`STACKS`] while the value lives: made when the thread starts,
/// dropped as it ends.
pub(crate) struct StackCount(usize);

impl StackCount {
    /// Counts a stack of `bytes`.
    pub(crate) fn new(bytes: usize) -> StackCount {
        STACKS.fetch_add(bytes, Ordering::SeqCst);
        StackCount(bytes)
    }
}

impl Drop for StackCount {
    fn drop(&mut self) {
        STACKS.fetch_sub(self.0, Ordering::SeqCst);
    }
}

/// The bytes of stack that the live tasks have used, each counted up to the highest point its
/// stack reached, in steps of [`STACK_STEP`]: charged to the memory budget with the heap (ADR
/// 0061 amendment of 2026-10-18, ADR 0117 rule 2).
static STACK_CHARGE: AtomicUsize = AtomicUsize::new(0);

/// The least step in which a task's stack is charged, so that the charge moves once per 16 KiB
/// of growth rather than at every call; [`stack_step`] raises it to the page size.
const STACK_STEP: usize = 16 * 1024;

thread_local! {
    /// The stack charge of the task the thread runs whose state has ended, held until its
    /// stack is cleared (`sched`, review of S1, N2); swapped with the task's slots.
    static ENDED_CHARGE: Cell<usize> = const { Cell::new(0) };
}

/// Ends `bytes` of stack charge: once the stack that held them is cleared or unmapped.
pub(crate) fn release_stack_charge(bytes: usize) {
    STACK_CHARGE.fetch_sub(bytes, Ordering::Relaxed);
}

/// The step of the stack charge: [`STACK_STEP`], or the page size when it is larger, since a
/// page is resident whole.
fn stack_step() -> usize {
    static STEP: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *STEP.get_or_init(|| STACK_STEP.max(una_sys::coroutine::page_size()))
}

/// What the run holds against its memory budget: the heap, and the stacks its tasks used.
fn charged() -> usize {
    crate::held().saturating_add(STACK_CHARGE.load(Ordering::Relaxed))
}

/// Charges the stack of the thread's task up to `used` bytes, when that is within its budget:
/// the charge rises to the next step, at most the budget. False when `used` is past the budget,
/// which the caller reports as too deep. O(1), at most once per step of growth.
fn charge_stack(hot: &Hot, used: usize) -> bool {
    let budget = hot.stack_budget.get();
    if used > budget {
        return false;
    }
    let target = used
        .checked_next_multiple_of(stack_step())
        .unwrap_or(budget)
        .min(budget);
    let charged = hot.stack_charged.get();
    if target > charged {
        STACK_CHARGE.fetch_add(target - charged, Ordering::Relaxed);
        hot.stack_charged.set(target);
    }
    true
}

/// Whether a stop is ending the process (ADR 0113): the first thread to stop ends it, and any
/// thread that stops afterwards waits for the end instead of writing a second report.
static ENDING: AtomicBool = AtomicBool::new(false);

/// Claims the end of the process for the calling thread's stop. When another stop claimed it
/// first, the thread parks until the process ends, and never returns.
fn end_once() {
    if ENDING.swap(true, Ordering::SeqCst) {
        #[cfg(test)]
        PARKED.fetch_add(1, Ordering::SeqCst);
        loop {
            std::thread::park();
        }
    }
}

/// How many stops parked behind the one that ends the process, in tests.
#[cfg(test)]
static PARKED: AtomicUsize = AtomicUsize::new(0);

/// How many parked stops a test waits for before the stop that ends the process exits.
#[cfg(test)]
pub(crate) static TEST_EXPECT_PARKED: AtomicUsize = AtomicUsize::new(0);

/// In tests that expect parked stops: waits, at most 10 seconds, for them to park, then says on
/// stdout how many did, so that the test sees the later stops parked rather than not reached.
#[cfg(test)]
fn report_parked() {
    let expected = TEST_EXPECT_PARKED.load(Ordering::SeqCst);
    if expected == 0 {
        return;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while PARKED.load(Ordering::SeqCst) < expected && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    println!(
        "una-rt test: parked stops: {}",
        PARKED.load(Ordering::SeqCst)
    );
    let _ = io::stdout().flush();
}

/// The bytes that the steps of copies have announced and not yet allocated, process-wide
/// (ADR 0113 rule 4): a step checks `held() + reserved` before it allocates, so that copies made
/// at once, by blocks in sibling tasks, see each other.
static RESERVED: AtomicUsize = AtomicUsize::new(0);

/// The bytes of one step, counted in [`RESERVED`] until the value is dropped: after the step
/// allocated, or while a panic unwinds. Between the allocation and the drop, the bytes are counted
/// twice, in `held()` too, which only makes a concurrent step stricter.
#[must_use = "a reservation counts until it is dropped, after the allocation it announces"]
pub(crate) struct Reservation(usize);

impl Drop for Reservation {
    fn drop(&mut self) {
        RESERVED.fetch_sub(self.0, Ordering::SeqCst);
        #[cfg(test)]
        LIVE_RESERVATIONS.with(|live| live.set(live.get() - 1));
    }
}

#[cfg(test)]
thread_local! {
    /// The reservations this thread holds, in tests: 0 again after every copy, a refused or a
    /// panicked one included.
    pub(crate) static LIVE_RESERVATIONS: Cell<usize> = const { Cell::new(0) };
    /// Refuses steps of this thread, in tests: `Some((k, n))` lets k steps pass and refuses the
    /// next n, as a heap at its ceiling would, then lets the later ones pass.
    pub(crate) static REFUSE_STEP: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}

/// Announces a step of `bytes` and checks it: the reservation, when `held() + reserved` stays
/// within the run's ceiling, or `None` when it would not, with nothing announced. A step larger
/// than any allocation can be (`isize::MAX` bytes) is refused before it is announced, so the
/// process-wide sum never wraps.
pub(crate) fn try_reserve(bytes: usize) -> Option<Reservation> {
    #[cfg(test)]
    {
        let refused = REFUSE_STEP.with(|cell| match cell.get() {
            Some((0, refuse)) => {
                cell.set((refuse > 1).then(|| (0, refuse - 1)));
                true
            }
            Some((pass, refuse)) => {
                cell.set(Some((pass - 1, refuse)));
                false
            }
            None => false,
        });
        if refused {
            return None;
        }
    }
    if bytes > isize::MAX as usize {
        return None;
    }
    let reserved = RESERVED
        .fetch_add(bytes, Ordering::SeqCst)
        .saturating_add(bytes);
    let reservation = Reservation(bytes);
    #[cfg(test)]
    LIVE_RESERVATIONS.with(|live| live.set(live.get() + 1));
    let held = charged();
    HOT.with(|hot| !hot.active.get() || held.saturating_add(reserved) <= hot.mem_ceiling.get())
        .then_some(reservation)
}

/// The message of a copy's refused step: `message`, or the message of a call over the memory
/// budget.
pub(crate) fn refusal_message(message: Option<String>) -> String {
    message.unwrap_or_else(|| {
        let max_memory = with_state(|state| state.max_memory).unwrap_or_default();
        format!("this call exceeds the memory limit of {max_memory} MiB")
    })
}

/// How many pairs a structural equality may hold at once, at `values::EQUALITY_PAIR_BYTES` each,
/// within the run's memory budget (ADR 0061, amendment of 2026-10-17, #815): fixed by the budget,
/// as the interpreter fixes it, not by what the heap holds.
pub(crate) fn equality_pairs() -> usize {
    with_state(|state| state.max_memory.saturating_mul(1024 * 1024)).map_or(usize::MAX, |budget| {
        budget / crate::values::EQUALITY_PAIR_BYTES
    })
}

/// Stops the run or the task with U6003 at the block `site`, for a copy's refused step (ADR 0113
/// rule 4): with `message`, or the message of a call over the memory budget. The caller holds no
/// borrow of the heap, since a stop of `main` ends the process.
pub(crate) fn refuse(site: usize, message: Option<String>) {
    let message = refusal_message(message);
    let abort = STATE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let location = cell.as_ref().map(|state| site_location(state, site))?;
        stop_or_abort(&mut cell, Code::U6003, location, message)
    });
    if let Some((state, code, location, message)) = abort {
        abort_run(state, code, location, message);
    }
}

thread_local! {
    /// Whether the thread runs a copy inside [`guarded`]: a panic there is reported as U8001 by
    /// the copy's caller, so the panic hook keeps its message instead of printing it.
    static GUARDED: Cell<bool> = const { Cell::new(false) };
    /// The message of the last panic inside [`guarded`] on this thread.
    static PANICKED: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Installs, once, the panic hook of the runtime, chained to the hook in place: inside
/// [`guarded`] it keeps the panic's message for U8001; anywhere else it passes the panic on, so
/// every other panic prints as before (ADR 0113 rule 5).
pub(crate) fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if GUARDED.with(Cell::get) {
                let message = info
                    .payload()
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
                    .or_else(|| info.payload().downcast_ref::<String>().cloned())
                    .unwrap_or_default();
                PANICKED.with(|cell| *cell.borrow_mut() = Some(message));
            } else {
                previous(info);
            }
        }));
    });
}

/// What a [`guarded`] region of a `together` block does, for the message of a panic in it.
pub(crate) const COPY: &str = "copied a value";
/// A task's start, for [`guarded`].
pub(crate) const START: &str = "started a task";
/// An adoption of a batch's events, for [`guarded`].
pub(crate) const ADOPTION: &str = "adopted the events of a batch";

/// Runs `f`, a copy or another step of a `together` block, `doing` what it says, catching a
/// panic: `Err` with the panic's message when it panicked, which is a defect of the runtime that
/// the caller reports as U8001 (ADR 0113 rule 2). No panic unwinds out of such a step, so none
/// reaches the generated code's `extern "C"` frames.
pub(crate) fn guarded<R>(doing: &str, f: impl FnOnce() -> R) -> Result<R, String> {
    let outer = GUARDED.with(|cell| cell.replace(true));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    GUARDED.with(|cell| cell.set(outer));
    result.map_err(|_| {
        let message = PANICKED
            .with(|cell| cell.borrow_mut().take())
            .unwrap_or_default();
        format!("internal error: the runtime panicked while it {doing}: {message}")
    })
}

/// The trace file of the run, once `start` made it: a stop that writes no trace removes it.
static TRACE_FILE: OnceLock<PathBuf> = OnceLock::new();

/// The bytes that 2^32 − 1 references take, one word each: a count at its limit with less held
/// in the heap and the stacks cannot be the program's (ADR 0113 rule 5).
const COUNT_LIMIT_BYTES: usize = (values::COUNT_LIMIT as usize).saturating_mul(8);

/// The bytes of [`COUNT_LIMIT_BYTES`], or fewer in tests, which lower it to reach U6003.
#[cfg(test)]
pub(crate) static TEST_COUNT_LIMIT_BYTES: AtomicUsize = AtomicUsize::new(COUNT_LIMIT_BYTES);

fn count_limit_bytes() -> usize {
    #[cfg(test)]
    {
        TEST_COUNT_LIMIT_BYTES.load(Ordering::SeqCst)
    }
    #[cfg(not(test))]
    {
        COUNT_LIMIT_BYTES
    }
}

/// A retain found a count at its limit (ADR 0113 rule 5): ends the process from whatever thread
/// runs it, and never returns. With at least [`COUNT_LIMIT_BYTES`] held in the heap and the
/// stacks, the run is over a limit of the native runtime: U6003, exit code 1. With less, no
/// program holds that many references, so the count is a defect of the runtime or the generated
/// code: U8001, exit code 3. Either way one line goes to stderr, with no position, since a retain
/// has no site, and no trace is written; a failed write of it exits with code 2 (`spec/13 §5`).
pub(crate) fn count_limit_reached() -> ! {
    // The allocator's ceiling stays: the threads that still run until the process ends keep
    // their hard cap, and this stop allocates nothing.
    end_once();
    // The trace file, made when the run started (ADR 0117 rule 10), is removed: this stop
    // writes none.
    if let Some(path) = TRACE_FILE.get() {
        let _ = std::fs::remove_file(path);
    }
    let held = crate::held().saturating_add(STACKS.load(Ordering::SeqCst));
    let (code, message, exit) = if held >= count_limit_bytes() {
        (
            Code::U6003,
            "the native runtime's reference count of a value reached its limit (4294967295)",
            1,
        )
    } else {
        (
            Code::U8001,
            "internal error: a reference count of the native runtime reached its limit (4294967295) with less memory held than that many references take",
            3,
        )
    };
    let written = writeln!(io::stderr(), "[{code}] Error: {message}");
    #[cfg(test)]
    report_parked();
    // Nothing unwinds through the compiled frames (spec/16 §4 rule 3).
    #[allow(clippy::exit)]
    std::process::exit(if written.is_ok() { exit } else { USAGE });
}

/// Whether the program writes the floating-point environment on each of its threads (ADR 0114
/// rule 6.4): set once at start, read by every thread the runtime spawns for it.
pub(crate) static FLOATS: AtomicBool = AtomicBool::new(false);

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
    /// Whether the program is of edition 0.3, which writes and tests the floating-point
    /// environment before `main` (ADR 0114 rule 6.4).
    pub float_environment: bool,
    /// The host-driven step, when the program is driven (ADR 0116 rule 3): its tick period and
    /// step budget, in milliseconds.
    pub drive: Option<DriveMeta>,
    /// How U3012 names the grants of the manifest: `` `[run] grants` `` in `una.toml`, and
    /// `` `run grants` `` in `ita.una`.
    pub grants_section: String,
    /// Whether the program has a `for ever`, so the run takes host stops: the executable then
    /// installs the handlers of SIGINT and SIGTERM (ADR 0116 rule 18).
    pub stops: bool,
    /// Whether the program has a `tasks` statement, so `main` runs as a coroutine that takes
    /// turns with the tasks it starts (ADR 0117 rules 1 and 11).
    pub tasks: bool,
    /// Whether the executable was built with check elision (ADR 0112), which `--virtual-time`
    /// refuses (ADR 0117 rule 12). No backend of this repository elides checks yet.
    pub elision: bool,
    /// `L_lim` of the program, as a message writes it (ADR 0117 IF4), embedded for edition `0.4`
    /// only: without it, the run takes no grant with limits.
    pub limit_label: Option<String>,
    /// Whether `L_lim` flows to `internal`, so that grants with limits are allowed.
    pub limits_allowed: bool,
    /// Whether the program holds a task construct or a `for ever`: the run then starts every
    /// worker and both helper pools at full size before `main`, so that no thread starts at a
    /// moment that earlier code decided (ADR 0116 amendment, P8; `spec/27 §1` rule 4; #1145).
    pub threads: bool,
}

/// The `drive` section of a driven program, as `una build` embeds it (ADR 0116 rule 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriveMeta {
    /// The tick period, in milliseconds.
    pub ticks: u64,
    /// The step budget, in milliseconds, when one is set.
    pub budget: Option<u64>,
}

/// One `uses` clause of `main`, or of a driven function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Need {
    /// The function whose header has it: `main`, or `init`, `step` or `on_stop`.
    pub function: String,
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
                    function: match need.get("function") {
                        Some(function) => function.as_str()?.to_owned(),
                        None => "main".to_owned(),
                    },
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
        let float_environment = value
            .get("float_environment")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let drive = match value.get("drive") {
            Some(drive) => Some(DriveMeta {
                ticks: drive.get("ticks")?.as_u64()?,
                budget: match drive.get("budget") {
                    None | Some(Value::Null) => None,
                    Some(budget) => Some(budget.as_u64()?),
                },
            }),
            None => None,
        };
        let grants_section = match value.get("grants_section") {
            Some(section) => section.as_str()?.to_owned(),
            None => "`[run] grants`".to_owned(),
        };
        let stops = value.get("stops").and_then(Value::as_bool).unwrap_or(false);
        let tasks = value.get("tasks").and_then(Value::as_bool).unwrap_or(false);
        // A description that says nothing of elision was built before it existed, without it.
        let elision = value
            .get("elision")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // A description without `L_lim` takes no limits (`spec/27 §9` rule 1).
        let limit_label = match value.get("limit_label") {
            Some(label) => Some(label.as_str()?.to_owned()),
            None => None,
        };
        let limits_allowed = value
            .get("limits_allowed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let threads = value
            .get("threads")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Some(Meta {
            stops,
            tasks,
            elision,
            limit_label,
            limits_allowed,
            threads,
            grants,
            uses,
            sites,
            types,
            ops,
            failure,
            float_environment,
            drive,
            grants_section,
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
    /// `--stop-after-iterations <n>`: the host's stop once `n` iterations have started (ADR
    /// 0116 rule 19).
    pub stop_after: Option<u64>,
    /// `--virtual-time`: the run's clock is virtual (ADR 0116 rule 19, `spec/26 §8`).
    pub virtual_time: bool,
    /// `--deadline <seconds>`: a host stop request after this many seconds (ADR 0116 rule 14).
    pub deadline: Option<u64>,
    /// `--stop-grace <seconds>`: the grace period before the hard stop, 10 when not given
    /// (rule 16).
    pub stop_grace: Option<u64>,
    /// `--workers`, `--task-depth`, `--io-threads` and `--file-threads` (ADR 0117 rules 2 and
    /// 3).
    pub concurrency: una_interp::Concurrency,
}

/// The command line, without the program name, or the message of a usage error.
pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        max_depth: una_interp::DEFAULT_MAX_DEPTH,
        max_memory: una_interp::DEFAULT_MAX_MEMORY,
        ..Options::default()
    };
    // The flags of tasks, checked once every flag is read: `--task-depth` against `--max-depth`.
    let mut tasks: [Option<usize>; 4] = [None; 4];
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
            "--stop-after-iterations" => {
                let text = value()?;
                let n: u64 = text
                    .parse()
                    .map_err(|_| format!("`{flag} {text}` is not a number"))?;
                if n == 0 || n > i64::MAX.unsigned_abs() {
                    return Err(format!(
                        "`{flag} {n}` is out of range; give from 1 to {}",
                        i64::MAX
                    ));
                }
                options.stop_after = Some(n);
            }
            // A switch: it takes no value.
            "--virtual-time" => {
                if inline.is_some() {
                    return Err(format!("`{flag}` takes no value"));
                }
                options.virtual_time = true;
            }
            "--deadline" | "--stop-grace" => {
                let text = value()?;
                let n: u64 = text
                    .parse()
                    .map_err(|_| format!("`{flag} {text}` is not a number"))?;
                let most = if flag == "--deadline" {
                    una_interp::stopping::MAX_DEADLINE_SECS
                } else {
                    una_interp::stopping::MAX_GRACE_SECS
                };
                if n == 0 || n > most {
                    return Err(format!(
                        "`{flag} {n}` is out of range; give from 1 to {most}"
                    ));
                }
                if flag == "--deadline" {
                    options.deadline = Some(n);
                } else {
                    options.stop_grace = Some(n);
                }
            }
            "--workers" | "--task-depth" | "--io-threads" | "--file-threads" => {
                let n = una_interp::concurrency::number(flag, &value()?)?;
                let slot = match flag {
                    "--workers" => 0,
                    "--task-depth" => 1,
                    "--io-threads" => 2,
                    _ => 3,
                };
                if let Some(cell) = tasks.get_mut(slot) {
                    *cell = Some(n);
                }
            }
            other => {
                return Err(format!(
                    "unexpected argument `{other}`; the options are --grant <spec>, --trace <file>, --max-depth <n>, --max-memory <MiB>, --max-response <MiB>, --http-timeout <seconds>, --stop-after-iterations <n>, --deadline <seconds>, --stop-grace <seconds>, --virtual-time, --workers <n>, --task-depth <n>, --io-threads <n> and --file-threads <n>"
                ));
            }
        }
    }
    let [workers, task_depth, io_threads, file_threads] = tasks;
    options.concurrency = una_interp::Concurrency::from_flags(
        options.max_depth,
        workers,
        task_depth,
        io_threads,
        file_threads,
    )?;
    Ok(options)
}

/// The state of a run, on the thread that runs `main` and on each thread that runs a task
/// (`spec/19 §8`).
pub(crate) struct State {
    /// The task's events, when the run records a trace: those not yet written, in a streaming
    /// task; every one, in a task of `together` or `first of`, until its join adopts them.
    pub(crate) trace: Option<Vec<Event>>,
    /// Whether the task writes its events as they happen: the root task and started tasks
    /// (ADR 0117 rule 10).
    pub(crate) streaming: bool,
    /// The events of operations in progress, opened and not yet ended: no event is written
    /// while one is open, so the slots they hold stay in place.
    pub(crate) open_events: usize,
    /// The `tasks` statements in progress in this task, innermost last, and how many it ran,
    /// for the paths of their tasks (`spec/27 §7`).
    pub(crate) scopes: Vec<Arc<crate::started::StartScope>>,
    pub(crate) tasks_runs: usize,
    /// What the run's tasks share: the trace's sink, the installations counted run-wide, the
    /// started tasks alive.
    pub(crate) shared: Arc<crate::started::RunShared>,
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
    /// The run's `Http` grants and the state of their limits, which every task shares (ADR
    /// 0117 rule 8, `spec/27 §9`).
    pub(crate) http_limits: Arc<una_http::limit::Governor>,
    pub(crate) types: Arc<Types>,
    pub(crate) ops: Arc<Vec<OpSite>>,
    /// Whether the last host operation failed.
    pub(crate) host_failed: bool,
    /// Whether the last host operation was interrupted while it waited, its task cancelled
    /// (ADR 0117 rule 4): its event ends `interrupted`, and the task stops.
    pub(crate) interrupted: bool,
    /// How many `first of` blocks the task has run, for the paths of their branches (`spec/27
    /// §6`).
    pub(crate) races: usize,
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
    /// The cancellation entries of the task and of the tasks that run it (ADR 0113 rule 1).
    cancel: Vec<Cancel>,
    /// How many of `cancel`'s flags an operation in progress shields: those set before it
    /// began, which it completes despite (`spec/19 §6` rule 4). Flags of tasks the operation
    /// starts come after them and still count.
    shielded: usize,
    /// The shields of the operations in progress, outermost first, to restore as each ends.
    operations: Vec<usize>,
    /// In a task, the abort that stopped it, U6003, U6004 or U6005, which its block reports.
    pub(crate) stopped: Option<(Code, Location, String)>,
    /// Whether the thread's task runs the body of a `for ever`, callees included: its `Http`
    /// operations are interruptible there (ADR 0116 rule 14). A task starts outside one.
    pub(crate) in_ever: bool,
    /// The settings of the run's tasks (ADR 0117 rules 2 and 3).
    concurrency: una_interp::Concurrency,
    /// The run's scheduler, made at its first block: none in a run without tasks.
    sched: Option<Arc<crate::sched::Scheduler>>,
}

/// Makes an abort say, on stderr, the debug high-water mark of the heap and the run's ceiling, in
/// the tests that compare them (ADR 0113, I3).
#[cfg(test)]
pub(crate) static TEST_PRINT_PEAK: AtomicBool = AtomicBool::new(false);

/// The decimal digits of `n`.
pub(crate) fn digits(n: usize) -> usize {
    n.checked_ilog10().map_or(1, |log| log as usize + 1)
}

#[cfg(test)]
thread_local! {
    /// How many times adoptions on this thread grew the trace, in tests: O(log n) over n
    /// adoptions (ADR 0113 rule 5).
    pub(crate) static TRACE_GROWTHS: Cell<usize> = const { Cell::new(0) };
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
    http_limits: Arc<una_http::limit::Governor>,
    types: Arc<Types>,
    ops: Arc<Vec<OpSite>>,
    max_memory: usize,
    baseline: usize,
    /// The path of the task that runs the block.
    pub(crate) path: String,
    cancel: Vec<Cancel>,
    /// The entries of `cancel` that an operation in progress shields (`State::shielded`).
    shielded: usize,
    /// The stack of a task.
    pub(crate) stack: usize,
    /// Where the block is written.
    pub(crate) location: Location,
    /// The deadline of the `for ever within` iteration that starts the block (`spec/26 §7`).
    deadline: crate::deadline::Inherited,
    /// The settings of the run's tasks, and its scheduler.
    concurrency: una_interp::Concurrency,
    pub(crate) sched: Arc<crate::sched::Scheduler>,
    /// For the branches of a `first of`, the count of the block among those its task ran: their
    /// paths are `p.f<c>.<i>` (`spec/27 §6`).
    pub(crate) race: Option<usize>,
    /// What the run's tasks share, and whether the task writes its events as they happen: a
    /// started task does (ADR 0117 rule 10).
    pub(crate) shared: Arc<crate::started::RunShared>,
    pub(crate) streaming: bool,
}

impl TaskTemplate {
    /// Whether the task that runs the block, or one that runs it, was cancelled.
    pub(crate) fn cancelled(&self) -> bool {
        unshielded(&self.cancel, self.shielded)
    }

    /// The prefix of the paths of the block's tasks: `p.` in a task `p`, and `f<c>.` more for
    /// the branches of the `c`-th `first of` of its task (`spec/19 §6` rule 1, `spec/27 §6`).
    pub(crate) fn path_prefix_len(&self) -> usize {
        let own = if self.path.is_empty() {
            0
        } else {
            self.path.len() + 1
        };
        own + self.race.map_or(0, |race| 2 + digits(race))
    }

    /// The bytes the template owns: its path, its cancellation entries, its location's file and
    /// its deadlines (ADR 0113 rule 2, `T`).
    pub(crate) fn owned_bytes(&self) -> usize {
        self.path
            .len()
            .saturating_add(self.location.file.len())
            .saturating_add(self.cancel.len().saturating_mul(size_of::<Cancel>()))
            .saturating_add(self.deadline.owned_bytes())
    }
}

/// A cancellation entry of a task, for its own block or a block above it (ADR 0113 rule 1): that
/// block's count, the lowest entry of a task of it that failed or aborted (0 after a U8001), and
/// the entry of the task on this task's path, its index among that block's tasks plus 1. The task
/// is cancelled when the count is below its entry.
pub(crate) type Cancel = (Arc<AtomicUsize>, usize);

/// Whether one of `cancel`'s entries past the first `shielded` cancels the task.
fn unshielded(cancel: &[Cancel], shielded: usize) -> bool {
    cancel
        .iter()
        .skip(shielded)
        .any(|(count, index)| count.load(Ordering::SeqCst) < *index)
}

impl State {
    /// What the tasks of a block at `site` start from.
    /// An executable's scheduler is made before `main`, its threads started (#1145); one is
    /// made here, its threads started on demand, only for code without that start, as tests.
    pub(crate) fn for_tasks(&mut self, site: usize) -> TaskTemplate {
        let depth = HOT.with(|hot| hot.depth.get());
        let left = self.max_depth.saturating_sub(depth);
        let concurrency = self.concurrency;
        let sched = Arc::clone(
            self.sched
                .get_or_insert_with(|| crate::sched::Scheduler::new(concurrency)),
        );
        TaskTemplate {
            concurrency,
            sched,
            tracing: self.trace.is_some(),
            depth,
            max_depth: self.max_depth,
            sites: Arc::clone(&self.sites),
            failure: self.failure.clone(),
            grants: Arc::clone(&self.grants),
            keys: Arc::clone(&self.keys),
            http: self.http,
            http_limits: Arc::clone(&self.http_limits),
            types: Arc::clone(&self.types),
            ops: Arc::clone(&self.ops),
            max_memory: self.max_memory,
            baseline: self.baseline,
            path: self.path.clone(),
            cancel: self.cancel.clone(),
            shielded: self.shielded,
            stack: una_interp::stack_bytes(left).unwrap_or(STACK_RESERVE * 2),
            location: site_location(self, site),
            deadline: crate::deadline::current(),
            race: None,
            shared: Arc::clone(&self.shared),
            streaming: false,
        }
    }

    /// What the branches of a `first of` at `site` start from (`spec/27 §6`): the template of
    /// [`State::for_tasks`], counted as the next race of the task, with the depth limit of
    /// `--task-depth` below the block, within the run's, and a stack sized from it (ADR 0117
    /// rule 2).
    pub(crate) fn for_race(&mut self, site: usize) -> TaskTemplate {
        let mut template = self.for_tasks(site);
        self.races = self.races.saturating_add(1);
        template.race = Some(self.races);
        let left = template
            .max_depth
            .saturating_sub(template.depth)
            .min(template.concurrency.task_depth);
        template.max_depth = template.depth.saturating_add(left);
        template.stack = una_interp::stack_bytes(left).unwrap_or(STACK_RESERVE * 2);
        template
    }

    /// What the tasks of the `tasks` statement at `site` start from (`spec/27 §7`): the
    /// template of [`State::for_tasks`], whose tasks write their events as they happen, with
    /// the depth limit of `--task-depth` below the `start`, within the run's, and a stack sized
    /// from it (ADR 0117 rule 2).
    pub(crate) fn for_start(&mut self, site: usize) -> TaskTemplate {
        let mut template = self.for_tasks(site);
        template.streaming = true;
        let left = template
            .max_depth
            .saturating_sub(template.depth)
            .min(template.concurrency.task_depth);
        template.max_depth = template.depth.saturating_add(left);
        template.stack = una_interp::stack_bytes(left).unwrap_or(STACK_RESERVE * 2);
        template
    }

    /// Opens a `tasks` statement in this task, at most `bound` tasks at once, or the default
    /// bound: `max(64, M)` for the budget `M` in MiB, within the system's cap (ADR 0117 rule 7).
    pub(crate) fn open_tasks(&mut self, bound: Option<usize>) {
        self.tasks_runs = self.tasks_runs.saturating_add(1);
        let path = if self.path.is_empty() {
            format!("s{}", self.tasks_runs)
        } else {
            format!("{}.s{}", self.path, self.tasks_runs)
        };
        let bound = bound.unwrap_or_else(|| {
            una_interp::concurrency::default_bound(self.max_memory, self.shared.cap)
        });
        self.scopes
            .push(Arc::new(crate::started::StartScope::new(path, bound)));
    }

    /// Where the call or block at `site` is written.
    pub(crate) fn location(&self, site: usize) -> Location {
        site_location(self, site)
    }

    /// Whether the task the thread runs was cancelled, past what an operation in progress
    /// shields.
    pub(crate) fn cancelled(&self) -> bool {
        unshielded(&self.cancel, self.shielded)
    }

    /// Writes the events this task holds, when it writes them as they happen and no event of
    /// an operation is open (ADR 0117 rule 10). O(events written).
    pub(crate) fn flush(&mut self) {
        if !self.streaming || self.open_events > 0 {
            return;
        }
        if let Some(trace) = self.trace.as_mut()
            && !trace.is_empty()
        {
            // Drained in place, so the buffer keeps its capacity for the next events.
            self.shared.write(trace.drain(..));
        }
    }

    /// Appends the event that ends this started task, `cancelled` or `aborted`, which names no
    /// operation (`spec/27 §7`).
    pub(crate) fn ended_event(&mut self, outcome: EventOutcome) {
        let task = self.path.clone();
        if let Some(trace) = self.trace.as_mut() {
            trace.push(Event {
                seq: trace.len() + 1,
                task,
                instance: String::new(),
                capability: String::new(),
                op: String::new(),
                args: Vec::new(),
                outcome,
            });
        }
        self.flush();
    }

    /// The id of the `count`th instance of the handler `name` that the thread's task installed
    /// (`spec/19 §7` rule 2): `with:<name>#<count>` in `main`'s; in a task, an id with the
    /// task's path that its block renumbers when it adopts the task's events.
    pub(crate) fn install_id(&self, name: &str, count: usize) -> String {
        // A streaming task counts its installations run-wide (`spec/27 §7`).
        if self.streaming {
            return format!("with:{name}#{count}");
        }
        // Made at its exact size, so that what an adoption allocates for it is its length (ADR
        // 0113 rule 3).
        let local = if self.path.is_empty() {
            0
        } else {
            self.path.len() + 2
        };
        let mut id = String::with_capacity(6 + name.len() + local + digits(count));
        id.push_str("with:");
        id.push_str(name);
        id.push('#');
        if !self.path.is_empty() {
            id.push('@');
            id.push_str(&self.path);
            id.push('#');
        }
        let _ = std::fmt::Write::write_fmt(&mut id, format_args!("{count}"));
        id
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
        if self.streaming {
            self.adopt_streaming(task, events, installs);
            return;
        }
        if self.trace.is_some() {
            // The pattern of the task's ids, made at its exact size (ADR 0113 rule 3).
            let mut local = String::with_capacity(task.len() + 3);
            local.push_str("#@");
            local.push_str(task);
            local.push('#');
            for mut event in events {
                // The handlers the task installed are added after its events, so the counts
                // read here are the block's as they stand: no copy of them is made (ADR 0113
                // rule 3).
                if let Some(rest) = event.instance.strip_prefix("with:")
                    && let Some((name, count)) = rest.split_once(&local)
                    && let Ok(count) = count.parse::<usize>()
                {
                    let base = self.installs.get(name).copied().unwrap_or(0);
                    event.instance = self.install_id(name, base + count);
                }
                if let Some(trace) = self.trace.as_mut() {
                    event.seq = trace.len() + 1;
                    trace.push(event);
                }
            }
        }
        for (name, &count) in installs {
            // A name the block already installed adds no entry, and no copy of the name.
            match self.installs.get_mut(name) {
                Some(installed) => *installed += count,
                None => {
                    self.installs.insert(name.clone(), count);
                }
            }
        }
    }

    /// [`State::adopt`] in a task that writes its events as they happen (ADR 0117 rule 10): the
    /// adopted events are numbered after the installations counted run-wide, under the lock of
    /// those counts. The join writes them after its checked step ([`State::flush`]), whose
    /// allocations are its reservation's only.
    fn adopt_streaming(
        &mut self,
        task: &str,
        events: Vec<Event>,
        installs: &BTreeMap<String, usize>,
    ) {
        let shared = Arc::clone(&self.shared);
        let mut counts = shared
            .installs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(trace) = self.trace.as_mut() {
            // Each string made at its exact size, as the adoption's bound counts them (ADR
            // 0113 rule 3).
            let mut local = String::with_capacity(task.len() + 3);
            local.push_str("#@");
            local.push_str(task);
            local.push('#');
            for mut event in events {
                if let Some(rest) = event.instance.strip_prefix("with:")
                    && let Some((name, count)) = rest.split_once(&local)
                    && let Ok(count) = count.parse::<usize>()
                {
                    let number = counts.get(name).copied().unwrap_or(0) + count;
                    let mut id = String::with_capacity(6 + name.len() + digits(number));
                    id.push_str("with:");
                    id.push_str(name);
                    id.push('#');
                    let _ = std::fmt::Write::write_fmt(&mut id, format_args!("{number}"));
                    event.instance = id;
                }
                trace.push(event);
            }
        }
        for (name, &count) in installs {
            *counts.entry(name.clone()).or_insert(0) += count;
        }
    }

    /// The growth of the trace, when the run records one, that `events` more events need, under
    /// the slot table's policy (ADR 0113 rule 5): its capacity, the doubled target, and the exact
    /// one; `None` when it has room.
    pub(crate) fn trace_growth(&self, events: usize) -> Option<(usize, usize, usize)> {
        let trace = self.trace.as_ref()?;
        let needed = trace.len().saturating_add(events);
        let capacity = trace.capacity();
        (needed > capacity).then(|| (capacity, needed.max(capacity.saturating_mul(2)), needed))
    }

    /// Grows the trace to hold `target` events, the growth an adoption reserved.
    pub(crate) fn grow_trace(&mut self, target: usize) {
        if let Some(trace) = self.trace.as_mut() {
            trace.reserve_exact(target.saturating_sub(trace.len()));
            #[cfg(test)]
            TRACE_GROWTHS.with(|growths| growths.set(growths.get() + 1));
        }
    }

    /// Whether the block has installed a handler `name` so far, so that an adoption adds no new
    /// entry for it.
    pub(crate) fn installed(&self, name: &str) -> bool {
        self.installs.contains_key(name)
    }

    /// Appends the event of a loop that stops at the start of an iteration (ADR 0116 rule 14).
    pub(crate) fn stop_event(&mut self) {
        if let Some(trace) = self.trace.as_mut() {
            trace.push(Event {
                seq: trace.len() + 1,
                task: String::new(),
                instance: String::new(),
                capability: String::new(),
                op: String::new(),
                args: Vec::new(),
                outcome: EventOutcome::Stop,
            });
        }
        self.flush();
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

/// The run's host stop state, set once at start (ADR 0116 rules 14 and 16).
static RUN_STOP: OnceLock<Arc<RunStop>> = OnceLock::new();

/// `--stop-after-iterations`, 0 when it is not given, and the iterations of `for ever` loops
/// and of the host's loop started so far (rule 19).
static STOP_AFTER: AtomicU64 = AtomicU64::new(0);
static ITERATIONS: AtomicU64 = AtomicU64::new(0);

/// The hard stop, once it has come (rule 16).
fn hard_stop() -> Option<HardStop> {
    RUN_STOP.get().and_then(|stop| stop.hard())
}

/// Whether the loop of the host, or a `for ever`, stops at the iteration that starts now
/// (rules 13, 14 and 19): a host stop request has reached the run, which is sticky; or
/// `--stop-after-iterations` iterations have started, which makes the run's own request now.
/// Otherwise the iteration counts as started.
fn iteration_stops() -> bool {
    let Some(stop) = RUN_STOP.get() else {
        return false;
    };
    if stop.stopped() {
        return true;
    }
    if claim_iteration(&ITERATIONS, STOP_AFTER.load(Ordering::SeqCst)) {
        return false;
    }
    stop.request_local();
    true
}

/// Claims the start of one iteration on `count`, in one atomic step: true when it starts.
/// With no limit (0), one `fetch_add`. With a limit, a compare-exchange loop that increments
/// only while the count is below it, so threads that start iterations at once never start more
/// than `limit` of them together.
fn claim_iteration(count: &AtomicU64, limit: u64) -> bool {
    if limit == 0 {
        count.fetch_add(1, Ordering::SeqCst);
        return true;
    }
    count
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |started| {
            (started < limit).then(|| started + 1)
        })
        .is_ok()
}

/// Marks whether the thread's task runs the body of a `for ever` ([`State::in_ever`]).
fn set_in_ever(inside: bool) {
    with_state(|state| state.in_ever = inside);
}

/// The interruption of an `Http` operation of the thread's task, when it runs in the body of
/// a `for ever` (rule 12).
pub(crate) fn op_interrupt() -> Option<&'static RunStop> {
    let inside = with_state(|state| state.in_ever).unwrap_or(false);
    RUN_STOP.get().filter(|_| inside).map(AsRef::as_ref)
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static HOT: Hot = const { Hot::inactive() };
    static DRIVER: RefCell<Option<Driver>> = const { RefCell::new(None) };
}

/// The host's loop of a driven program (ADR 0116 rule 3), on the thread that runs it: its
/// settings, when its first tick was due on the run's clock, the next tick due and the
/// iterations started.
struct Driver {
    ticks: u64,
    origin: Option<u64>,
    next: u64,
}

/// The clock of the run in this process (ADR 0116 rule 19): set once when the executable
/// starts, real, or virtual under `--virtual-time`, and shared by every thread of the run.
///
/// A `OnceLock`, never a lock: after it is set, reading it is a plain load, and a virtual
/// clock's reading one more atomic load. It is read on hot paths, each iteration of a
/// `for ever within`, each `Http` call and each `Clock.now`, so a mutex there would cost
/// every task a contended lock, against the performance parity of native code. An executable
/// runs one run, so one setting suffices; a later virtual run in the same process, as in
/// tests, restarts the virtual clock at 0 in `start`.
static CLOCK: OnceLock<Clock> = OnceLock::new();

/// Makes the run's clock the virtual one when `virtual_time` holds, the real one otherwise.
/// False when the process already has a clock of the other kind: a second run in one
/// process, which only tests make.
fn set_clock(virtual_time: bool) -> bool {
    CLOCK
        .get_or_init(|| Clock::for_run(virtual_time))
        .is_virtual()
        == virtual_time
}

/// The run's clock: the one the executable set, or the real clock before any run started.
pub fn clock() -> &'static Clock {
    CLOCK.get_or_init(Clock::real)
}

/// The next tick of the host's loop, as the interpreter's driver gives it: an owned `Tick` of
/// `std.drive` after waiting for the next one due, delivering the latest one due with the
/// count of those skipped; or 0 when the loop stops at this iteration start, after a host stop
/// request (rule 3), with its `stop` event traced. A host stop interrupts the wait, and the
/// loop then stops, as at the next iteration start. The step that a tick starts is the body
/// of the host's `for ever`. The first tick is due at the first call, when `init` has
/// returned.
pub fn drive_tick() -> i64 {
    set_in_ever(false);
    let tick = DRIVER.with(|cell| {
        let mut driver = cell.borrow_mut();
        let driver = driver.as_mut()?;
        if iteration_stops() {
            return None;
        }
        // On the run's clock, real or virtual: under virtual time the wait moves the clock to
        // the tick at once (ADR 0116 rule 19); on the real clock a host stop interrupts it.
        let clock = clock();
        let origin = *driver.origin.get_or_insert_with(|| clock.now());
        let period = driver.ticks.max(1);
        let due = origin.saturating_add(driver.next.saturating_mul(period));
        if clock.is_virtual() {
            clock.wait_until(due);
        } else {
            let now = clock.now();
            if now < due {
                let left = std::time::Duration::from_millis(due - now);
                let waited = match RUN_STOP.get() {
                    Some(stop) => una_interp::stopping::wait_tick(stop, left),
                    None => {
                        clock.wait_until(due);
                        true
                    }
                };
                if !waited {
                    return None;
                }
            }
        }
        let elapsed = clock.now().saturating_sub(origin);
        let latest = (elapsed / period).max(driver.next);
        let missed = latest - driver.next;
        let at = latest.saturating_mul(driver.ticks);
        driver.next = latest + 1;
        Some((at, missed))
    });
    match tick {
        Some((at, missed)) => {
            set_in_ever(true);
            let word = |n: u64| {
                values::int_word(&num_bigint::BigInt::from(
                    i64::try_from(n).unwrap_or(i64::MAX),
                ))
            };
            let object = values::data(0);
            values::push(object, word(at));
            values::push(object, word(missed));
            object
        }
        None => {
            with_state(State::stop_event);
            0
        }
    }
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
    /// The highest point of the task's stack charged to the memory budget so far, at most
    /// `stack_budget` (ADR 0061 amendment of 2026-10-18): the stack check of a call compares
    /// with it, so the fast path stays one comparison.
    stack_charged: Cell<usize>,
    /// The heap the run may hold before a check reports it ([`mem_ceiling`]).
    mem_ceiling: Cell<usize>,
    /// `State::shielded`, and the length of `State::cancel`: no flag is unshielded when
    /// `shielded >= flags`.
    shielded: Cell<usize>,
    flags: Cell<usize>,
    /// The thread's attention word, and its value at the last check point (ADR 0116 rule 18,
    /// ADR 0102 amendment of 2026-10-18).
    word: Cell<Option<una_sys::attention::Word>>,
    seen: Cell<u64>,
    /// Whether every check point of the thread counts toward its task's turn: on a worker,
    /// under virtual time (ADR 0117 rule 12). Not swapped: it is the worker's.
    counting: Cell<bool>,
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
            stack_charged: Cell::new(0),
            mem_ceiling: Cell::new(usize::MAX),
            shielded: Cell::new(0),
            flags: Cell::new(0),
            word: Cell::new(None),
            seen: Cell::new(u64::MAX),
            counting: Cell::new(false),
        }
    }

    /// Whether the thread's attention word changed since the last check point: one relaxed
    /// load on the fast path. A change takes the slow path once, which looks for the hard stop.
    ///
    /// Under virtual time, on a worker, every check point takes this path and counts toward the
    /// task's turn; under the real clock the slicer raises the word once a turn has lasted its
    /// slice while another task waits. Either way the turn's end takes the slow path, which
    /// gives the worker to the next task (ADR 0117 rules 2 and 12).
    fn attention(&self) -> bool {
        if self.counting.get() {
            return hard_stop().is_some() | crate::sched::turn_due();
        }
        match self
            .word
            .get()
            .map(|word| word.changed_since(self.seen.get()))
        {
            Some(Some(now)) => {
                self.seen.set(now);
                hard_stop().is_some() | crate::sched::turn_due()
            }
            _ => false,
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

/// A task's thread-locals of this module while it is parked (`sched`, ADR 0117 rule 2): its
/// state, its [`Hot`] record, the guard of a copy in progress, and its test counters.
pub(crate) struct Slots {
    state: Option<State>,
    active: bool,
    /// `Hot`'s depth, max_depth, stack_base, stack_budget, stack_charged, mem_ceiling, shielded
    /// and flags.
    hot: [usize; 8],
    guarded: bool,
    panicked: Option<String>,
    /// [`ENDED_CHARGE`].
    ended: usize,
    #[cfg(test)]
    counters: [usize; 3],
    #[cfg(test)]
    refuse: Option<(usize, usize)>,
}

impl Default for Slots {
    /// The slots of a thread with no run: those of [`Hot::inactive`].
    fn default() -> Slots {
        Slots {
            state: None,
            active: false,
            hot: [0, 0, 0, 0, 0, usize::MAX, 0, 0],
            guarded: false,
            panicked: None,
            ended: 0,
            #[cfg(test)]
            counters: [0; 3],
            #[cfg(test)]
            refuse: None,
        }
    }
}

impl Slots {
    /// The stack charge a finished task still holds, taken out of its slots: what its state's
    /// end set aside, and, after a panic that skipped that end, its live charge. The caller
    /// releases it once the task's stack is cleared.
    pub(crate) fn take_charge(&mut self) -> usize {
        let live = self.hot.get_mut(4).map_or(0, std::mem::take);
        std::mem::take(&mut self.ended).saturating_add(live)
    }
}

impl Drop for Slots {
    /// Slots dropped with a charge still held give it back, so later checks count no phantom
    /// stack; a worker takes it first with [`Slots::take_charge`], to release it in order.
    fn drop(&mut self) {
        release_stack_charge(self.take_charge());
    }
}

/// Exchanges the thread's state and its mirror with `slots`. The state is never borrowed when a
/// task parks or ends: a borrow here is a defect, which ends the process rather than mix two
/// tasks' states.
pub(crate) fn swap_slots(slots: &mut Slots) {
    STATE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => std::mem::swap(&mut *state, &mut slots.state),
        Err(_) => std::process::abort(),
    });
    HOT.with(|hot| {
        slots.active = hot.active.replace(slots.active);
        let cells = [
            &hot.depth,
            &hot.max_depth,
            &hot.stack_base,
            &hot.stack_budget,
            &hot.stack_charged,
            &hot.mem_ceiling,
            &hot.shielded,
            &hot.flags,
        ];
        for (cell, saved) in cells.into_iter().zip(slots.hot.iter_mut()) {
            *saved = cell.replace(*saved);
        }
    });
    slots.guarded = GUARDED.with(|cell| cell.replace(slots.guarded));
    slots.ended = ENDED_CHARGE.with(|cell| cell.replace(slots.ended));
    PANICKED.with(|cell| match cell.try_borrow_mut() {
        Ok(mut panicked) => std::mem::swap(&mut *panicked, &mut slots.panicked),
        Err(_) => std::process::abort(),
    });
    #[cfg(test)]
    {
        let [live, growths, slow] = &mut slots.counters;
        *live = LIVE_RESERVATIONS.with(|cell| cell.replace(*live));
        *growths = TRACE_GROWTHS.with(|cell| cell.replace(*growths));
        *slow = SLOW_PATHS.with(|cell| cell.replace(*slow));
        slots.refuse = REFUSE_STEP.with(|cell| cell.replace(slots.refuse));
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
        hot.stack_charged.set(0);
        hot.mem_ceiling
            .set(mem_ceiling(state.baseline, state.max_memory));
        hot.shielded.set(state.shielded);
        hot.flags.set(state.cancel.len());
        let word = hot.word.get().unwrap_or_else(una_sys::attention::register);
        hot.word.set(Some(word));
        // The word as it stands, read before the hard stop is: a hard stop that comes after
        // the read raises the word, and one that came before it is seen here, with a value no
        // word holds, so the first check point takes the slow path.
        let seen = word.load();
        hot.seen.set(if hard_stop().is_some() {
            u64::MAX
        } else {
            seen
        });
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
        let charged = hot.stack_charged.replace(0);
        // A task's charge outlives its state until its stack is cleared (`sched`); `main`'s, and
        // any outside a worker's coroutine, ends here.
        if una_sys::coroutine::inside() {
            ENDED_CHARGE.with(|ended| ended.set(ended.get().saturating_add(charged)));
        } else {
            release_stack_charge(charged);
        }
        hot.mem_ceiling.set(usize::MAX);
        hot.shielded.set(0);
        hot.flags.set(0);
    });
    cell.take()
}

/// The attention word of the calling thread, registered at its first use: a worker reads it
/// when it starts, so the slicer can raise it (ADR 0117 rule 2). Under virtual time the worker
/// counts every check point of its tasks (rule 12).
pub(crate) fn attention_word() -> una_sys::attention::Word {
    HOT.with(|hot| {
        let word = hot.word.get().unwrap_or_else(una_sys::attention::register);
        hot.word.set(Some(word));
        hot.counting.set(crate::sched::counting());
        word
    })
}

/// Pushes the cancellation entry of a `within` (ADR 0117 rule 5): the task's check points, and
/// those of the tasks it starts, see it once its deadline sets it. O(1).
pub(crate) fn push_cancel(flag: Arc<AtomicUsize>) -> usize {
    with_state(|state| {
        state.cancel.push((flag, 1));
        HOT.with(|hot| hot.flags.set(state.cancel.len()));
        state.cancel.len() - 1
    })
    .unwrap_or(0)
}

/// The cancellation entries that can interrupt a wait of the operation in progress (ADR 0117
/// rule 4): the entries of races (`tasks::RACE_ENTRY`) that the operations around it do not
/// shield, those from the shield it began with. The operation itself completes despite any
/// other cancellation, as before.
pub(crate) fn interruptible_entries() -> Vec<Cancel> {
    with_state(|state| {
        let from = state.operations.last().copied().unwrap_or(0);
        state
            .cancel
            .get(from..)
            .unwrap_or_default()
            .iter()
            .filter(|(_, entry)| entry & crate::tasks::RACE_ENTRY != 0)
            .cloned()
            .collect()
    })
    .unwrap_or_default()
}

/// How many of the task's cancellation entries the operation in progress shields: those before
/// it began (`spec/19 §6` rule 4).
pub(crate) fn shielded() -> usize {
    with_state(|state| state.shielded).unwrap_or(0)
}

/// Removes the cancellation entry `flag` of a `within` that ended. O(entries).
pub(crate) fn pop_cancel(flag: &Arc<AtomicUsize>) {
    with_state(|state| {
        if let Some(at) = state
            .cancel
            .iter()
            .rposition(|(count, _)| Arc::ptr_eq(count, flag))
        {
            state.cancel.remove(at);
        }
        HOT.with(|hot| hot.flags.set(state.cancel.len()));
    });
}

/// `f` of the task's cancellation entries that the operation in progress does not shield
/// (`spec/19 §6` rule 4); `None` without a run's state.
pub(crate) fn unshielded_entries<R>(f: impl FnOnce(&[Cancel]) -> R) -> Option<R> {
    with_state(|state| f(state.cancel.get(state.shielded..).unwrap_or_default()))
}

/// Whether the thread's task stops: it was cancelled, or an abort stopped it.
pub(crate) fn task_stopping() -> bool {
    with_state(|state| state.cancelled() || state.stopped.is_some()).unwrap_or(false)
}

/// `f` of the state of the run, when the thread runs `main`.
pub(crate) fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| cell.borrow_mut().as_mut().map(f))
}

/// Whether a `for ever` stops at the iteration that starts now (ADR 0116 rules 13 and 14): a
/// host stop request has come, which is sticky; its `stop` event is then traced. 1 to stop.
pub fn ever_start() -> i64 {
    if iteration_stops() {
        with_state(|state| {
            state.in_ever = false;
            state.stop_event();
        });
        1
    } else {
        set_in_ever(true);
        0
    }
}

/// Leaves the body of a `for ever`: the loop ended, by `stop`, a host stop or a failure, and
/// its `on stop` and the code after it are outside the body (rule 14).
pub fn ever_end() {
    set_in_ever(false);
}

/// Runs the executable: parses the command line, checks `main`'s capabilities against the
/// grants, calls `entry`, the compiled `main`, on a thread with the stack that `--max-depth`
/// needs, and writes the trace. Returns the exit code.
pub fn start(args: &[String], meta: &str, entry: impl FnOnce() -> i64 + Send + 'static) -> i32 {
    install_panic_hook();
    let mut stderr = io::stderr();
    let Some(meta) = Meta::parse(meta) else {
        let _ = writeln!(
            stderr,
            "error: the executable's embedded description is invalid; rebuild it with `una build`"
        );
        return 3;
    };
    let mut options = match parse_args(args) {
        Ok(options) => options,
        Err(message) => {
            let _ = writeln!(stderr, "error: {message}");
            return USAGE;
        }
    };
    // ADR 0117 rule 12: under virtual time no check point may be compiled away, so an
    // executable built with check elision (ADR 0112) is refused, before `main` starts.
    if meta.elision && options.virtual_time {
        let _ = writeln!(
            stderr,
            "error: this executable was built with check elision, which `--virtual-time` refuses; rebuild it with `--no-elision`"
        );
        return USAGE;
    }
    // Under virtual time the run follows the interpreter's schedule, on one worker (ADR 0117
    // rule 12).
    if options.virtual_time {
        options.concurrency.workers = 1;
    }
    // The trace file is made before `main` starts, and written as the run goes (ADR 0117 rule
    // 10): one that cannot be made is a usage error.
    let shared = match &options.trace {
        Some(path) => match crate::started::RunShared::tracing(path) {
            Ok(shared) => {
                let _ = TRACE_FILE.set(path.clone());
                shared
            }
            Err(error) => {
                let _ = writeln!(
                    stderr,
                    "error: cannot write the trace to `{}`: {error}",
                    path.display()
                );
                return USAGE;
            }
        },
        None => crate::started::RunShared::new(),
    };
    // The run's host stop state is made first, so a signal that comes while the handlers are
    // installed counts for the run.
    let stop = RunStop::new(
        options
            .stop_grace
            .unwrap_or(una_interp::stopping::DEFAULT_GRACE_SECS),
    );
    // ADR 0116 rule 18: only a run that can reach a `for ever` or runs a driver changes what
    // SIGINT and SIGTERM do; any other keeps their default actions.
    if meta.stops
        && let Err(una_sys::signals::InstallError(message)) = una_sys::signals::install()
    {
        let _ = writeln!(stderr, "error: {message}");
        return 3;
    }
    // The run's watcher when a request can come: a signal, or the deadline (rules 14 and 16).
    // Its guard ends the watch on every way out, a panic's included.
    let watching = if meta.stops || options.deadline.is_some() {
        match stop.watch(options.deadline.map(std::time::Duration::from_secs)) {
            Ok(watching) => Some(watching),
            Err(_) => {
                let _ = writeln!(stderr, "error: cannot start the watcher of host stops");
                return 3;
            }
        }
    } else {
        None
    };
    let _ = RUN_STOP.set(Arc::clone(&stop));
    STOP_AFTER.store(options.stop_after.unwrap_or(0), Ordering::SeqCst);
    let grants = match grants(&meta, &options) {
        Ok(grants) => grants,
        Err(message) => {
            let _ = writeln!(stderr, "error: {message}");
            return USAGE;
        }
    };
    // A grant with limits needs `L_lim` at most `internal` (ADR 0117 IF4): the embedded grants
    // passed `una check`, and are checked again with the flags' (`spec/27 §9` rule 5).
    let limit = meta
        .limit_label
        .as_deref()
        .map(|label| (label, meta.limits_allowed));
    let specs: Vec<String> = meta.grants.iter().chain(&options.grants).cloned().collect();
    if let Err(message) = una_interp::check_limited_flags(&specs, limit) {
        let _ = writeln!(stderr, "error: {message}");
        return USAGE;
    }
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
    let drive = meta.drive.map(|drive| Driver {
        ticks: drive.ticks,
        origin: None,
        next: 0,
    });
    if !set_clock(options.virtual_time) {
        let _ = writeln!(
            stderr,
            "error: this process already ran a run on a clock of the other kind"
        );
        return 3;
    }
    // Each run's virtual clock starts at 0 (`spec/26 §8`), also in a process that ran one
    // before, as tests do: one atomic store, before any of the run's code reads it.
    if let Clock::Virtual(virtual_clock) = clock() {
        virtual_clock.restart();
    }
    let threads = meta.threads;
    let mut state = State {
        trace: options.trace.as_ref().map(|_| Vec::new()),
        streaming: true,
        open_events: 0,
        scopes: Vec::new(),
        tasks_runs: 0,
        shared,
        failed_write: None,
        max_depth: options.max_depth,
        sites: Arc::new(meta.sites),
        failure: meta.failure.map(Arc::new),
        // The run's limits refill on the run's clock, set just before (ADR 0116 rule 19).
        http_limits: Arc::new(una_http::limit::Governor::with_clock(
            http_grants(&grants),
            Arc::new(clock().clone()),
        )),
        grants: Arc::new(grants),
        keys: Arc::new(keys),
        http: options.http,
        types: Arc::new(meta.types),
        ops: Arc::new(meta.ops),
        host_failed: false,
        interrupted: false,
        races: 0,
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
        in_ever: false,
        concurrency: options.concurrency,
        sched: None,
    };
    // ADR 0114 rule 6.4: write the default floating-point environment, then test it, before
    // any code of the program runs; each thread the program runs on writes it too.
    let floats = meta.float_environment;
    if floats {
        crate::ffi::write_float_environment();
        // A failed probe is U6006, exit code 1, with no position, written as the runtime's
        // other stops before `main` are.
        if let Err(probe) = una_core::float::probe_environment() {
            let written = writeln!(
                stderr,
                "[{}] Error: the floating-point environment of this platform is not the default one ({probe})",
                Code::U6006
            );
            return if written.is_ok() { 1 } else { USAGE };
        }
        FLOATS.store(true, Ordering::SeqCst);
    }
    // ADR 0116 amendment, P8 (#1145): a program with a task construct or a `for ever` starts
    // its scheduler, every worker and both helper pools at full size, and fills the stack pool
    // with stacks of the size a block at the top asks for, the largest any task asks for, here,
    // before `main`. Whether a thread exists, or how many stacks are pooled, is then never
    // state that earlier code left, and no construct pays a thread's start or a stack's
    // mapping because of it. A thread that cannot start, or a stack that cannot be mapped,
    // stops the run here, with U6003 and no position, rather than leave a lazy start for
    // later. A program without these constructs starts no thread (`spec/27 §1` rule 4).
    // A `tasks` statement is such a construct; its root runs on this scheduler too, below.
    if threads || meta.tasks {
        let sched = crate::sched::Scheduler::new(state.concurrency);
        if sched.start_all(stack).is_err() {
            let written = writeln!(
                stderr,
                "[{}] Error: the threads and stacks of this run's tasks could not be made",
                Code::U6003
            );
            return if written.is_ok() { 1 } else { USAGE };
        }
        state.sched = Some(sched);
        // The reactor too, for a run whose grants include `Http` under the real clock (ADR
        // 0116 amendment, P8; ADR 0117 slice S7): started here, never at a task's first wait.
        // Under virtual time no task parks on it (`reactor::parks`).
        if !http_grants(&state.grants).is_empty()
            && !clock().is_virtual()
            && !crate::reactor::start_up_front()
        {
            let written = writeln!(
                stderr,
                "[{}] Error: the reactor of this run's requests could not be made",
                Code::U6003
            );
            return if written.is_ok() { 1 } else { USAGE };
        }
    }
    let budget = options.max_memory.saturating_mul(1024 * 1024);
    // An executable whose program has a `tasks` statement runs `main` as a coroutine on
    // worker 0 of the scheduler started above, so that the root takes turns with the tasks it
    // starts (ADR 0117 rules 1 and 11); any other keeps its thread, as before. One scheduler
    // serves the run: its workers, helpers, stack pool and slicer all started before `main`.
    let sched = if meta.tasks {
        state.sched.clone()
    } else {
        None
    };
    // #1254: a run with threads also runs one real task on every worker before `main`
    // (`tasks::warm_up`), under no budget and before the baseline is read, so its transient
    // charge never stops a run and what it leaves on the workers counts for no program. It
    // runs on the thread or coroutine that then runs `main`.
    let warm = threads;
    let workers = state.concurrency.workers;
    let main = move || {
        if floats {
            crate::ffi::write_float_environment();
        }
        let _stack = StackCount::new(stack);
        let mut state = state;
        state.stack_base = stack_position(&state);
        let max_memory = state.max_memory;
        if warm {
            state.max_memory = usize::MAX;
        }
        state.baseline = crate::held();
        install(state, 0);
        if warm {
            crate::tasks::warm_up(workers);
            let baseline = crate::held();
            with_state(|state| {
                state.max_memory = max_memory;
                state.baseline = baseline;
            });
            HOT.with(|hot| hot.mem_ceiling.set(mem_ceiling(baseline, max_memory)));
        }
        let baseline = with_state(|state| state.baseline).unwrap_or(0);
        // ADR 0061 rule 3: past twice the budget, allocation fails and the process aborts.
        crate::ceiling(baseline.saturating_add(budget.saturating_mul(2)));
        DRIVER.with(|cell| *cell.borrow_mut() = drive);
        let status = entry();
        crate::ceiling(usize::MAX);
        // A debug build of the runtime checks the compiled code's reference counting: it
        // never used a value it had released, and released every value it made, which is
        // unreferenced once `main` returns (`spec/16 §4`).
        //
        // Only the detection of an internal error depends on the build (unlike a refined function kind, where a
        // check of the program did): for a program whose counting is right, the output, exit
        // status and trace are the same in both builds, and a release executable never runs
        // the check. The tests link the debug build, so a counting bug fails them.
        //
        // The tasks of `together` blocks check their own heaps when they end (`tasks`).
        let faults = if cfg!(debug_assertions) {
            (
                values::faults()
                    + values::unbalanced_literals()
                    + crate::tasks::TASK_FAULTS.load(Ordering::SeqCst),
                values::live() + crate::tasks::TASK_LEAKS.load(Ordering::SeqCst),
                values::copies() + crate::tasks::TASK_COPIES.load(Ordering::SeqCst),
                values::fallbacks() + crate::tasks::TASK_FALLBACKS.load(Ordering::SeqCst),
                crate::tasks::BLOCK_CHECKS.load(Ordering::SeqCst),
            )
        } else {
            (0, 0, 0, 0, 0)
        };
        let state = STATE.with(|cell| take_state(&mut cell.borrow_mut()));
        (status, state, faults)
    };
    let ran = match &sched {
        Some(sched) => {
            let (sender, receiver) = crate::sched::channel();
            let spawned = sched.spawn_root(
                stack,
                move || {
                    // A panic of the runtime is caught on the root's own stack, so nothing
                    // unwinds across a switch.
                    if let Ok(result) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(main))
                    {
                        sender.send(result);
                    }
                },
                || {},
            );
            match spawned {
                Ok(()) => receiver.recv(),
                Err(_) => None,
            }
        }
        None => std::thread::Builder::new()
            .name("una-main".to_owned())
            .stack_size(stack)
            .spawn(main)
            .ok()
            .and_then(|worker| worker.join().ok()),
    };
    let Some((status, state, (faults, leaked, copies, fallbacks, block_checks))) = ran else {
        let _ = writeln!(
            stderr,
            "error: cannot start `main` with the stack that `--max-depth {}` needs",
            options.max_depth
        );
        return USAGE;
    };
    drop(watching);
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
            "error: internal error: uses of released values, misread type descriptors or literals whose count did not return to its start: {faults}"
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
    if block_checks > 0 {
        let _ = writeln!(
            stderr,
            "error: internal error: checks of the copies and bookkeeping of `together` blocks failed: {block_checks}"
        );
        code = 3;
    }
    code
}

/// Reports a failed write of the output and writes the trace; returns the exit code they
/// need, 0 when both went well.
fn finish(mut state: State) -> i32 {
    let mut stderr = io::stderr();
    let mut code = 0;
    if let Some(error) = state.failed_write.take() {
        let _ = writeln!(stderr, "error: cannot write the output: {error}");
        code = USAGE;
    }
    // The events the task still holds are written, and the trace's array is closed (ADR 0117
    // rule 10).
    state.streaming = true;
    state.open_events = 0;
    state.flush();
    if let Err(error) = state.shared.finish() {
        let _ = writeln!(stderr, "error: cannot write the trace: {error}");
        code = USAGE;
    }
    code
}

/// Ends the run from a started task with `code` at `location`, as an abort of `main` does:
/// U6003 for the run's memory budget, U6007 for the hard stop, U8001 for a defect (`spec/27
/// §7`). Never returns.
pub(crate) fn end_run_from_task(code: Code, location: Location, message: String) {
    let state = STATE.with(|cell| take_state(&mut cell.borrow_mut()));
    match state {
        Some(state) => abort_run(state, code, location, message),
        // The task's state is gone, after a panic of the runtime: the run ends all the same,
        // with one line on stderr and no trace, as a stop with no state does.
        None => {
            end_once();
            let _ = io::stdout().flush();
            let _ = writeln!(io::stderr(), "[{code}] Error: {message}");
            let exit = if code == Code::U8001 { 3 } else { 1 };
            // Nothing unwinds through the compiled frames (spec/16 §4 rule 3).
            #[allow(clippy::exit)]
            std::process::exit(exit);
        }
    }
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

/// The `Http` grants among `grants`, in order: what the run's governor of limits holds (ADR 0117
/// rule 8).
fn http_grants(grants: &[Grant]) -> Vec<una_http::grant::Grant> {
    grants
        .iter()
        .filter_map(|grant| match grant {
            Grant::Http(http) => Some(http.clone()),
            _ => None,
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
                        "`{}` uses {} of `{}`, which neither {} nor a `--grant` flag grants",
                        need.function,
                        missing.join(", "),
                        need.capability,
                        meta.grants_section
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
    let held = charged();
    // ADR 0102: the conjunction of the negated abort conditions, on `Hot`; when it fails, the
    // check below decides, with the same position and `held`.
    let proceed = HOT.with(|hot| {
        if !hot.active.get() {
            return true;
        }
        // The attention word first: under virtual time each check point counts toward its
        // task's turn, whatever the other conditions say (ADR 0117 rule 12).
        let attention = hot.attention();
        let depth = hot.depth.get();
        let used = hot.stack_base.get().abs_diff(position);
        let clear = !attention
            && depth < hot.max_depth.get()
            && (used <= hot.stack_charged.get() || charge_stack(hot, used))
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
    // The end of the task's turn gives its worker to the next task (ADR 0117 rule 2); the heap
    // is measured again once the task runs again.
    let held = if crate::sched::yield_due() {
        charged()
    } else {
        held
    };
    let abort = STATE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let state = cell.as_mut()?;
        if state.cancelled() {
            return Some(None);
        }
        // ADR 0116 rule 16: the hard stop cancels the run, or the task, at this call.
        if let Some(hard) = hard_stop() {
            let location = site_location(state, site);
            let message = una_interp::stopping::hard_stop_message(hard);
            return Some(stop_or_abort(&mut cell, Code::U6007, location, message));
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
    let held = charged();
    // ADR 0102: the negated abort conditions, on `Hot`.
    let proceed = HOT.with(|hot| {
        if !hot.active.get() {
            return true;
        }
        // The attention word first, as in [`enter`].
        let attention = hot.attention();
        !attention && held <= hot.mem_ceiling.get() && !hot.cancelled()
    });
    if proceed {
        return 0;
    }
    slow_path();
    let held = if crate::sched::yield_due() {
        charged()
    } else {
        held
    };
    let abort = STATE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let state = cell.as_mut()?;
        if state.cancelled() {
            return Some(None);
        }
        // ADR 0116 rule 16: the hard stop cancels the run, or the task, at this iteration.
        if let Some(hard) = hard_stop() {
            let location = site_location(state, site);
            let message = una_interp::stopping::hard_stop_message(hard);
            return Some(stop_or_abort(&mut cell, Code::U6007, location, message));
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

/// Checks the memory budget at an element that a `together for` over a range makes, at the
/// loop's `site`, as the interpreter does while it makes the list (#773, ADR 0061 rule 2): over
/// the budget, the run aborts with U6003 at the loop. Returns false when the run or the task
/// stops, and true otherwise. Cancellation is not checked here: the tasks check it.
pub(crate) fn within_budget(site: usize) -> bool {
    let held = charged();
    if HOT.with(|hot| !hot.active.get() || held <= hot.mem_ceiling.get()) {
        return true;
    }
    slow_path();
    let abort = STATE.with(|cell| {
        let mut cell = cell.borrow_mut();
        let state = cell.as_mut()?;
        let message = over_budget(state, held)?;
        let location = site_location(state, site);
        Some(stop_or_abort(&mut cell, Code::U6003, location, message))
    });
    match abort {
        None => true,
        Some(None) => false,
        Some(Some((state, code, location, message))) => {
            abort_run(state, code, location, message);
            false
        }
    }
}

/// The text of `word`, a value of the type at `ty` in the program's type descriptions, as
/// interpolation writes it, with every field (`spec/11 §5`, ADR 0068). A new reference.
/// `params` are the descriptors of the type parameters that the type holds, by position.
pub fn written(site: i64, ty: usize, word: i64, params: &[i64]) -> i64 {
    let text =
        with_state(|state| state.types.written(ty, word, params)).unwrap_or(Some(String::new()));
    match text {
        Some(text) => values::text_word(text),
        None => {
            int_text_abort(site);
            values::text_word(String::new())
        }
    }
}

/// `written` of the type at `ty`, as the generated code passes it, at `site`.
pub fn write(site: i64, ty: i64, word: i64) -> i64 {
    written(site, usize::try_from(ty).unwrap_or(usize::MAX), word, &[])
}

/// An `Int` as text, at `site` ([`values::int_text`]): one of more than `MAX_INT_DIGITS`
/// digits is never written, the run aborting with U6003 there (`spec/11 §7`, #923).
pub fn int_text(site: i64, a: i64) -> i64 {
    match values::int_text(a) {
        Some(text) => text,
        None => {
            int_text_abort(site);
            values::text_word(String::new())
        }
    }
}

/// Aborts the run, or stops the task, with U6003 at `site`: an integer of more than
/// `MAX_INT_DIGITS` digits was to be written as text (`spec/11 §7`, #923). The caller returns
/// a value the stopped task never uses.
pub(crate) fn int_text_abort(site: i64) {
    abort_at(
        usize::try_from(site).unwrap_or(usize::MAX),
        Code::U6003,
        una_core::INT_TEXT_LIMIT_MESSAGE.to_owned(),
    );
}

/// Returns 1 when the task must stop before an operation, having been cancelled, and 0
/// otherwise (`spec/19 §6` rule 4).
pub fn poll() -> i64 {
    // ADR 0102: no unshielded flag set, on `Hot`. An operation is a check point of its task's
    // turn too (ADR 0117 rule 12), counted on the attention path.
    if HOT.with(|hot| !hot.active.get() || (!hot.attention() && !hot.cancelled())) {
        return 0;
    }
    slow_path();
    crate::sched::yield_due();
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
/// cancellation entry, for its block.
pub(crate) fn enter_task(template: &TaskTemplate, path: String, entry: Cancel) {
    // Made at its exact size, so that a start allocates what it reserved (ADR 0113 rule 2).
    let mut cancel = Vec::with_capacity(template.cancel.len() + 1);
    cancel.extend_from_slice(&template.cancel);
    cancel.push(entry);
    let mut state = State {
        trace: template.tracing.then(Vec::new),
        streaming: template.streaming,
        open_events: 0,
        scopes: Vec::new(),
        tasks_runs: 0,
        shared: Arc::clone(&template.shared),
        failed_write: None,
        max_depth: template.max_depth,
        sites: Arc::clone(&template.sites),
        failure: template.failure.clone(),
        grants: Arc::clone(&template.grants),
        keys: Arc::clone(&template.keys),
        http: template.http,
        http_limits: Arc::clone(&template.http_limits),
        types: Arc::clone(&template.types),
        ops: Arc::clone(&template.ops),
        host_failed: false,
        interrupted: false,
        races: 0,
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
        in_ever: false,
        concurrency: template.concurrency,
        sched: Some(Arc::clone(&template.sched)),
    };
    state.stack_base = stack_position(&state);
    install(state, template.depth);
    crate::deadline::adopt(template.deadline.clone());
}

/// What a task reports to its block when its code has ended (ADR 0113 rule 3): its events, the
/// handlers it installed, its first failed write and the abort that stopped it, taken from its
/// state, which stays installed for the export of its value.
pub(crate) type Report = (
    Vec<Event>,
    BTreeMap<String, usize>,
    Option<io::Error>,
    Option<(Code, Location, String)>,
);

/// Takes the report of the task the thread runs out of its state.
pub(crate) fn take_report() -> Option<Report> {
    with_state(|state| {
        (
            state.trace.as_mut().map(std::mem::take).unwrap_or_default(),
            std::mem::take(&mut state.installs),
            state.failed_write.take(),
            state.stopped.clone(),
        )
    })
}

/// Installs on this thread the state of a run with the default options, with the task path
/// `path`, a trace when `tracing`, and one site whose file is `file`, so that a test can run a
/// block in process; [`leave_task`] ends it. With a path, a stop is recorded in the state, as a
/// task's is, instead of ending the process.
#[cfg(test)]
pub(crate) fn install_test_state_as(path: String, tracing: bool, file: String) {
    // A small call depth, so that a test's tasks, which blocks nest, take small stacks; four
    // workers, so that the tests whose tasks wait for each other run them on distinct workers.
    let args = ["--max-depth", "64", "--workers", "4"].map(str::to_owned);
    let Ok(options) = parse_args(&args) else {
        return;
    };
    let at = Position { line: 1, col: 1 };
    let mut state = State {
        trace: tracing.then(Vec::new),
        streaming: false,
        open_events: 0,
        scopes: Vec::new(),
        tasks_runs: 0,
        shared: crate::started::RunShared::new(),
        failed_write: None,
        max_depth: options.max_depth,
        sites: Arc::new(vec![Location {
            file,
            start: at,
            end: at,
        }]),
        failure: None,
        grants: Arc::new(Vec::new()),
        keys: Arc::new(BTreeMap::new()),
        http: options.http,
        http_limits: Arc::new(una_http::limit::Governor::new(Vec::new())),
        types: Arc::new(Types::default()),
        // One operation, which the tasks of a traced test record events of.
        ops: Arc::new(vec![OpSite {
            capability: "Log".to_owned(),
            op: "write".to_owned(),
            params: Vec::new(),
            args: Vec::new(),
            result: crate::host::Shown {
                ty: 0,
                redacted: None,
            },
            error: None,
        }]),
        host_failed: false,
        interrupted: false,
        races: 0,
        installs: BTreeMap::new(),
        stack_base: 0,
        stack_budget: usize::MAX,
        max_memory: options.max_memory,
        baseline: 0,
        path,
        cancel: Vec::new(),
        shielded: 0,
        operations: Vec::new(),
        stopped: None,
        in_ever: false,
        concurrency: options.concurrency,
        sched: None,
    };
    state.stack_base = stack_position(&state);
    install(state, 0);
}

/// Gives the test task the thread runs an ancestor's cancellation entry whose count is `count`,
/// with entry 1: the task, and the blocks it runs, are cancelled once `count` is 0, as after a
/// failure of the ancestor's first task. `Hot` mirrors the entries, as `install` keeps it.
#[cfg(test)]
pub(crate) fn cancel_test_task(count: Arc<AtomicUsize>) {
    with_state(|state| {
        state.cancel.push((count, 1));
        HOT.with(|hot| hot.flags.set(state.cancel.len()));
    });
}

/// The instance ids of the events of the test run's trace, in order.
#[cfg(test)]
pub(crate) fn test_trace() -> Vec<String> {
    with_state(|state| {
        state
            .trace
            .iter()
            .flatten()
            .map(|event| event.instance.clone())
            .collect()
    })
    .unwrap_or_default()
}

/// The bytes the test task the thread runs may still allocate before a check refuses a step: its
/// ceiling less what the process holds.
#[cfg(test)]
pub(crate) fn test_room() -> usize {
    HOT.with(|hot| hot.mem_ceiling.get())
        .saturating_sub(charged())
}

/// The abort that stopped the test task the thread runs, if any: its code and message.
#[cfg(test)]
pub(crate) fn test_stopped() -> Option<(Code, String)> {
    with_state(|state| {
        state
            .stopped
            .as_ref()
            .map(|(code, _, message)| (*code, message.clone()))
    })
    .flatten()
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
    // One stop ends the process: a stop on another thread may have claimed it (ADR 0113).
    end_once();
    let _ = io::stdout().flush();
    let diagnostic = Diagnostic::error(code, message, location);
    let written = write!(
        io::stderr(),
        "{}",
        render_human(&[diagnostic], &Default::default(), false)
    );
    #[cfg(all(test, debug_assertions))]
    if TEST_PRINT_PEAK.load(Ordering::SeqCst) {
        let _ = writeln!(
            io::stderr(),
            "una-rt test: peak {} ceiling {}",
            crate::peak(),
            mem_ceiling(state.baseline, state.max_memory)
        );
    }
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
    // The word of a stopped task (`una_codegen` `STOPPED`) is no failure: `main`'s thread ends
    // the run at an abort, and every cut is caught by its own `within`, so `main` never returns
    // it. Should it, that is an internal error (U8001, exit code 3), never a failure of `main`.
    debug_assert_ne!(failure, -1, "`main` returned the stop of a task");
    if failure == -1 {
        let state = STATE.with(|cell| take_state(&mut cell.borrow_mut()));
        if let Some(state) = state {
            let location = state
                .failure
                .as_ref()
                .map_or_else(|| site_location(&state, usize::MAX), |f| f.location.clone());
            abort_run(
                state,
                Code::U8001,
                location,
                "internal error: `main` returned the stop of a task, which no failure is"
                    .to_owned(),
            );
        }
        return;
    }
    let _ = io::stdout().flush();
    let diagnostic = STATE.with(|cell| {
        let cell = cell.borrow();
        let state = cell.as_ref()?;
        let described = state.failure.as_ref()?;
        Some(Diagnostic::error(
            Code::U6001,
            format!(
                "`main` failed with `{}`",
                state
                    .types
                    .shown(described.root, failure)
                    .unwrap_or_else(|| "<an integer of more than 4300 digits>".to_owned())
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

    #[test]
    fn threads_racing_on_a_stop_count_start_exactly_that_many_iterations() {
        // ADR 0116 rule 19: the claim is one atomic step, so eight threads that try 1,000
        // claims each start exactly 1,000 iterations under a limit of 1,000.
        let count = AtomicU64::new(0);
        let started = AtomicU64::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..1_000 {
                        if claim_iteration(&count, 1_000) {
                            started.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                });
            }
        });
        assert_eq!(started.load(Ordering::SeqCst), 1_000);
        assert_eq!(count.load(Ordering::SeqCst), 1_000);
        // Without a limit, every claim starts an iteration.
        let free = AtomicU64::new(0);
        assert!((0..10).all(|_| claim_iteration(&free, 0)));
        assert_eq!(free.load(Ordering::SeqCst), 10);
    }

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    #[cfg_attr(miri, ignore = "starts the scheduler's threads, which Miri cannot")]
    fn the_reactor_runs_before_main_in_a_run_granted_http() {
        // ADR 0116 amendment P8, ADR 0117 slice S7: a program with a task construct whose
        // grants include `Http` has the reactor's thread before `main` runs, so no request
        // starts it at a moment that earlier code decided. The reactor is one per process,
        // so the test asks once, from a run that has every reason to start it.
        let seen = Arc::new(std::sync::Mutex::new(None));
        let record = Arc::clone(&seen);
        let status = start(
            &strings(&["--grant", "http:https://example.com"]),
            r#"{"grants":[],"uses":[],"threads":true}"#,
            move || {
                *record.lock().unwrap() = Some(crate::reactor::started());
                0
            },
        );
        assert_eq!(status, 0);
        assert_eq!(*seen.lock().unwrap(), Some(true));
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
    fn the_command_line_takes_virtual_time_as_a_switch() {
        assert!(!parse_args(&[]).unwrap().virtual_time);
        let options = parse_args(&strings(&[
            "--virtual-time",
            "--stop-after-iterations",
            "3",
        ]))
        .unwrap();
        assert!(options.virtual_time);
        assert_eq!(options.stop_after, Some(3));
        assert!(parse_args(&strings(&["--virtual-time=yes"])).is_err());
    }

    #[test]
    fn the_flags_of_tasks_take_their_ranges_in_any_order() {
        let options = parse_args(&strings(&[
            "--task-depth",
            "40",
            "--max-depth",
            "50",
            "--workers=1",
            "--io-threads",
            "8",
            "--file-threads",
            "2",
        ]))
        .unwrap();
        let tasks = options.concurrency;
        assert_eq!(
            (
                tasks.workers,
                tasks.task_depth,
                tasks.io_threads,
                tasks.file_threads
            ),
            (1, 40, 8, 2)
        );
        for wrong in [
            &["--workers", "0"][..],
            &["--workers", "1025"],
            &["--workers", "many"],
            &["--max-depth", "50", "--task-depth", "51"],
            &["--io-threads", "4097"],
            &["--file-threads", "257"],
            &["--file-threads"],
        ] {
            assert!(parse_args(&strings(wrong)).is_err(), "{wrong:?}");
        }
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
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn a_program_with_tasks_starts_every_thread_before_main() {
        // #1145, ADR 0116 amendment P8: when the description says the program holds a task
        // construct or a `for ever`, the scheduler exists before `main` runs, with every worker
        // and both helper pools started at full size and the stack pool full; otherwise no thread
        // starts.
        let args: Vec<String> = "--workers 3 --io-threads 5 --file-threads 2"
            .split(' ')
            .map(str::to_owned)
            .collect();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        for meta in [
            r#"{"grants":[],"uses":[],"threads":true}"#,
            r#"{"grants":[],"uses":[]}"#,
        ] {
            let record = Arc::clone(&seen);
            let status = start(&args, meta, move || {
                let observed = with_state(|state| {
                    state
                        .sched
                        .as_ref()
                        .map(|sched| (sched.started(), sched.stacks()))
                });
                record.lock().unwrap().push(observed);
                0
            });
            assert_eq!(status, 0);
        }
        assert_eq!(
            *seen.lock().unwrap(),
            [Some(Some(((3, 2, 5), (64, 0)))), Some(None)]
        );
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
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
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
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
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
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
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
            // The task is the second of a block of two, entry 2; the first's failure, which
            // lowers the count to its own entry, 1, cancels it.
            let count = Arc::new(AtomicUsize::new(3));
            let (ready, wait_ready) = std::sync::mpsc::channel();
            let (set, wait_set) = std::sync::mpsc::channel();
            let entry = (Arc::clone(&count), 2);
            let task = std::thread::spawn(move || {
                enter_task(&template, "2".to_owned(), entry);
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
            count.fetch_min(1, Ordering::SeqCst);
            set.send(()).unwrap();
            let (clear, stopped, shielded, after) = task.join().unwrap();
            eprintln!("{STEP} {clear:?} {stopped:?} {shielded} {after}");
            0
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
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
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
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
