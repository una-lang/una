//! `tasks` and `start` in native code (`spec/27 §7`, ADR 0117 rules 7 and 10), and what a run's
//! tasks share: the trace written as it happens, and the installations of handlers counted
//! run-wide.
//!
//! - **A statement** ([`tasks_begin`], [`tasks_end`]) runs its block in the current task. Its
//!   [`StartScope`] counts the tasks it started that still run, bounds them, and cancels them
//!   through one cancellation entry when the block fails or stops.
//! - **A started task** ([`start`]) is a coroutine on the run's workers, with its own heap: its
//!   closure is copied out of the starting task's heap and into its own (ADR 0113), its state
//!   comes from the statement's template, and its events are written as they happen. It ends
//!   alone when it runs too deep, with its `aborted` event and one line on stderr; any other
//!   abort ends the run.
//! - **The trace** ([`RunShared`]) is one sink per run: the file of `--trace`, written as a JSON
//!   array whose `[` comes with the first event and whose `]` comes at the end of the run.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use una_diag::{Code, Location};
use una_interp::{Event, EventOutcome};

use crate::runtime::{self, TaskTemplate, with_state};
use crate::sched;
use crate::tasks::Invoke;
use crate::values::{self, Portable};

/// Locks `mutex`, recovering its data when a thread panicked while holding it: each critical
/// section leaves its data consistent at each step.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the tasks of one run share (ADR 0117 rules 2 and 10).
pub(crate) struct RunShared {
    /// How many instances of each handler the streaming tasks installed, run-wide: their ids
    /// `with:<handler>#<n>` count in the order the installations happen.
    pub(crate) installs: Mutex<BTreeMap<String, usize>>,
    /// The trace's sink.
    sink: Mutex<Sink>,
    /// The started tasks of the run that have not ended, and the most this system allows at
    /// once (`una_interp::concurrency::task_cap`).
    pub(crate) live: AtomicUsize,
    pub(crate) cap: Option<usize>,
}

/// The trace file, the events written so far, which number `seq`, and its first failed write.
struct Sink {
    file: Option<io::BufWriter<std::fs::File>>,
    written: usize,
    failed: Option<io::Error>,
}

impl RunShared {
    /// What a run's tasks share, with no trace.
    pub(crate) fn new() -> Arc<RunShared> {
        Arc::new(RunShared {
            installs: Mutex::new(BTreeMap::new()),
            sink: Mutex::new(Sink {
                file: None,
                written: 0,
                failed: None,
            }),
            live: AtomicUsize::new(0),
            cap: una_interp::concurrency::task_cap(),
        })
    }

    /// What a run's tasks share, with the trace file `path`, made now: a file that cannot be
    /// made is the error, before the run starts.
    pub(crate) fn tracing(path: &Path) -> io::Result<Arc<RunShared>> {
        let file = std::fs::File::create(path)?;
        let shared = RunShared::new();
        lock(&shared.sink).file = Some(io::BufWriter::new(file));
        Ok(shared)
    }

    /// Writes `events`, in order, numbering their `seq` (ADR 0117 rule 10). O(events).
    pub(crate) fn write(&self, events: impl IntoIterator<Item = Event>) {
        let mut sink = lock(&self.sink);
        for mut event in events {
            sink.written += 1;
            event.seq = sink.written;
            let separator = if sink.written == 1 { "[" } else { "," };
            let text = una_interp::event_json(&event);
            if sink.failed.is_none()
                && let Some(file) = sink.file.as_mut()
                && let Err(error) = write!(file, "{separator}{text}")
            {
                sink.failed = Some(error);
            }
        }
    }

    /// Ends the trace: closes its array and flushes the file. The error of the first write
    /// that failed, if any. Later events are not written.
    pub(crate) fn finish(&self) -> io::Result<()> {
        let mut sink = lock(&self.sink);
        let opening = if sink.written == 0 { "[" } else { "" };
        let Some(mut file) = sink.file.take() else {
            return Ok(());
        };
        if let Some(error) = sink.failed.take() {
            return Err(error);
        }
        write!(file, "{opening}]")?;
        file.flush()
    }
}

/// The scope of a `tasks` statement in progress (`spec/27 §7`): the prefix of its tasks' paths,
/// its bound, how many tasks it started, how many still run, and the cancellation entry its
/// tasks carry, which it lowers to 0 to cancel them.
pub(crate) struct StartScope {
    /// `s<c>` in the root task, `p.s<c>` in the task `p`.
    pub(crate) path: String,
    bound: usize,
    started: AtomicUsize,
    running: sched::Counter,
    pub(crate) cancel: Arc<AtomicUsize>,
}

/// Opens the `tasks` statement at `site` in the task the thread runs, at most `bound` tasks at
/// once when `bounded` is 1, the integer of the borrowed word `bound`, else the default bound
/// (ADR 0117 rule 7). 0, or 2 when the task stops instead.
pub fn tasks_begin(bound: i64, bounded: i64, site: usize) -> i64 {
    let bound = (bounded != 0).then(|| {
        let value = values::int_value(bound);
        // A bound below 1 is refuted by verification; one beyond `usize` bounds nothing.
        usize::try_from(&value)
            .unwrap_or(if value.sign() == num_bigint::Sign::Minus {
                1
            } else {
                usize::MAX
            })
            .max(1)
    });
    let opened = with_state(|state| {
        if state.stopped.is_some() {
            return false;
        }
        state.open_tasks(bound);
        true
    });
    if opened == Some(true) { 0 } else { 2 }
}

/// Ends the innermost `tasks` statement of the task the thread runs, once its block has ended:
/// when `cancel` is 1, the block failed or stops, and its tasks are cancelled first, waking
/// those parked on an interruptible operation. Parks until every task it started has ended.
pub fn tasks_end(cancel: i64) {
    let Some(Some(scope)) = with_state(|state| state.scopes.pop()) else {
        return;
    };
    if cancel != 0 {
        scope.cancel.store(0, Ordering::SeqCst);
        crate::interrupt::lowered(&scope.cancel);
    }
    scope.running.wait_zero();
}

impl StartScope {
    /// The scope of a statement whose tasks' paths start with `path`, at most `bound` at once.
    pub(crate) fn new(path: String, bound: usize) -> StartScope {
        StartScope {
            path,
            bound: bound.max(1),
            started: AtomicUsize::new(0),
            running: sched::Counter::default(),
            cancel: Arc::new(AtomicUsize::new(ENTRY)),
        }
    }
}

/// The cancellation entry of a statement's tasks, and its count until the statement cancels
/// them, which lowers the count to 0. It carries the mark of the entries whose cancellation
/// interrupts a waiting operation (`tasks::RACE_ENTRY`, ADR 0117 rule 4): started tasks exist
/// in edition `0.4` only, whose interruptions rule 4 gives them.
const ENTRY: usize = crate::tasks::RACE_ENTRY | 1;

/// Whether `message`, of U6003, says a call ran too deep, past the depth limit or short of its
/// stack: the abort that ends a started task alone (ADR 0117 rule 7), whichever task of it,
/// a `together` task inside it included, ran too deep. The memory budget's ends the run.
fn too_deep(message: &str) -> bool {
    message.starts_with("this call exceeds the maximum call depth")
}

/// `start` of the closure `closure` at `site`, which the caller keeps (`spec/27 §7`): parks
/// while the innermost statement runs as many tasks as its bound, then starts the next one on
/// the run's workers, and goes on. 0, or 2 when the task stops instead.
pub fn start(closure: i64, site: usize, invoke: Invoke) -> i64 {
    let scope = with_state(|state| {
        if state.stopped.is_some() {
            return None;
        }
        state.scopes.last().cloned()
    })
    .flatten();
    let Some(scope) = scope else {
        let location = with_state(|state| state.location(site)).unwrap_or_else(nowhere);
        runtime::abort_from_task(
            Code::U8001,
            location,
            "internal error: a `start` outside its `tasks` statement".to_owned(),
        );
        return 2;
    };
    scope.running.acquire(scope.bound);
    let shared = with_state(|state| Arc::clone(&state.shared));
    let Some(shared) = shared else {
        scope.running.release();
        return 2;
    };
    // On Linux, a run holds at most a quarter of `vm.max_map_count` started tasks at once
    // (ADR 0117 rule 2): past it the run aborts here, before a mapping fails.
    if let Some(cap) = shared.cap
        && shared.live.load(Ordering::SeqCst) >= cap
    {
        scope.running.release();
        let location = with_state(|state| state.location(site)).unwrap_or_else(nowhere);
        runtime::abort_from_task(
            Code::U6003,
            location,
            format!(
                "this `start` exceeds the {cap} started tasks this system allows at once, a quarter of `vm.max_map_count`"
            ),
        );
        return 2;
    }
    // The closure is copied out of this task's heap, each step checked against the budget
    // (ADR 0113 rules 2 and 4).
    let form = match runtime::guarded(runtime::COPY, || values::export(closure)) {
        Ok(Ok(form)) => form,
        Ok(Err(values::Refused(message))) => {
            scope.running.release();
            runtime::refuse(site, message);
            return 2;
        }
        Err(message) => {
            scope.running.release();
            let location = with_state(|state| state.location(site)).unwrap_or_else(nowhere);
            runtime::abort_from_task(Code::U8001, location, message);
            return 2;
        }
    };
    let k = scope.started.fetch_add(1, Ordering::SeqCst) + 1;
    let Some(template) = with_state(|state| state.for_start(site)) else {
        scope.running.release();
        return 2;
    };
    let path = format!("{}.{k}", scope.path);
    let entry = (Arc::clone(&scope.cancel), ENTRY);
    shared.live.fetch_add(1, Ordering::SeqCst);
    let sched = Arc::clone(&template.sched);
    let stack = template.stack;
    let ended = Arc::clone(&scope);
    let alive = Arc::clone(&shared);
    let spawned = sched.spawn_started(
        stack,
        move || {
            // A panic of the runtime is caught on the task's own stack, so nothing unwinds
            // across a switch; it ends the run with U8001.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if runtime::FLOATS.load(Ordering::SeqCst) {
                    crate::ffi::write_float_environment();
                }
                let _stack = runtime::StackCount::new(template.stack);
                started(&template, path, entry, form, (site, invoke));
            }));
            if outcome.is_err() {
                runtime::end_run_from_task(
                    Code::U8001,
                    template.location.clone(),
                    "internal error: the runtime panicked in a started task".to_owned(),
                );
            }
        },
        // Once the task's state and heap are gone, its statement counts it ended.
        move || {
            alive.live.fetch_sub(1, Ordering::SeqCst);
            ended.running.release();
        },
    );
    if spawned.is_err() {
        // The task never starts: its statement counts it ended.
        shared.live.fetch_sub(1, Ordering::SeqCst);
        scope.running.release();
        let location = with_state(|state| state.location(site)).unwrap_or_else(nowhere);
        runtime::abort_from_task(
            Code::U6003,
            location,
            "this `start` could not run its task: no stack could be mapped for it".to_owned(),
        );
        return 2;
    }
    0
}

/// A location for an abort with no state to place it.
fn nowhere() -> Location {
    Location {
        file: String::new(),
        start: una_diag::Position { line: 1, col: 1 },
        end: una_diag::Position { line: 1, col: 1 },
    }
}

/// One started task, on its coroutine: makes its state from `template`, with its path and the
/// cancellation entry of its statement, copies its closure into its heap, runs it, and ends:
/// alone after an abort of depth or stack, with its `cancelled` event when its statement or a
/// deadline cancelled it, and ending the run after any other abort.
fn started(
    template: &TaskTemplate,
    path: String,
    entry: runtime::Cancel,
    form: Portable,
    (site, invoke): (usize, Invoke),
) {
    runtime::enter_task(template, path, entry);
    let imported = runtime::guarded(runtime::COPY, || values::import(&form));
    drop(form);
    let closure = match imported {
        Ok(Ok(closure)) => Some(closure),
        // The budget refused a step of the copy: the run's memory, which ends the run.
        Ok(Err(values::Refused(message))) => {
            runtime::refuse(site, message);
            None
        }
        Err(message) => {
            runtime::end_run_from_task(Code::U8001, template.location.clone(), message);
            None
        }
    };
    if let Some(closure) = closure {
        let (value, status) = invoke(closure, None, site);
        if status != 2 {
            values::release(value);
            values::release(closure);
        }
        if status == 1 {
            // A started task cannot fail (U3029): reaching it is a defect, which ends the run.
            runtime::end_run_from_task(
                Code::U8001,
                template.location.clone(),
                "internal error: a started task failed, which its checks exclude".to_owned(),
            );
        }
    }
    let ended = with_state(|state| {
        let depth = matches!(&state.stopped, Some((Code::U6003, _, message)) if too_deep(message));
        let ended = match &state.stopped {
            None if state.cancelled() => Some(EventOutcome::Cancelled),
            None => None,
            Some(_) if depth => Some(EventOutcome::TaskAborted),
            Some(_) => None,
        };
        (ended, state.stopped.clone(), depth, state.path.clone())
    });
    if let Some((ended, stopped, depth, path)) = ended {
        match (stopped, depth) {
            // Too deep: the task ends alone, and the others run on (ADR 0117 rule 7).
            (Some((_, location, message)), true) => {
                let line = una_interp::concurrency::ended_alone(
                    &path,
                    &message,
                    Some((
                        location.file.as_str(),
                        location.start.line,
                        location.start.col,
                    )),
                );
                let _ = writeln!(io::stderr(), "{line}");
            }
            // Any other abort ends the run, with the trace so far.
            (Some((code, location, message)), false) => {
                runtime::end_run_from_task(code, location, message);
            }
            (None, _) => {}
        }
        if let Some(outcome) = ended {
            with_state(|state| state.ended_event(outcome));
        }
    }
    with_state(runtime::State::flush);
    crate::tasks::exit_task();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(task: &str) -> Event {
        Event {
            seq: 0,
            task: task.to_owned(),
            instance: String::new(),
            capability: String::new(),
            op: String::new(),
            args: Vec::new(),
            outcome: EventOutcome::Cancelled,
        }
    }

    #[test]
    fn the_trace_is_written_as_it_goes_and_ends_as_one_array() {
        // ADR 0117 rule 10: events are numbered in the order they are written, and the file is
        // one JSON array once the run ends, also with no event.
        let dir = std::env::temp_dir().join(format!("una-rt-sink-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("trace.json");
        let shared = RunShared::tracing(&path).unwrap();
        shared.write(vec![event("s1.1"), event("")]);
        shared.write(vec![event("s1.2")]);
        shared.finish().unwrap();
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let seqs: Vec<(u64, &str)> = written
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["seq"].as_u64().unwrap(), e["task"].as_str().unwrap_or("")))
            .collect();
        assert_eq!(seqs, [(1, "s1.1"), (2, ""), (3, "s1.2")]);
        let empty = dir.join("empty.json");
        RunShared::tracing(&empty).unwrap().finish().unwrap();
        assert_eq!(std::fs::read_to_string(&empty).unwrap(), "[]");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_trace_file_that_cannot_be_made_is_known_before_the_run() {
        let missing = std::env::temp_dir()
            .join(format!("una-rt-no-such-dir-{}", std::process::id()))
            .join("trace.json");
        assert!(RunShared::tracing(&missing).is_err());
    }

    #[test]
    fn a_statement_bounds_its_tasks_and_waits_for_them() {
        // ADR 0117 rules 3 and 7: `acquire` parks at the bound until a task ends, and
        // `wait_zero` until every one has; outside a coroutine the waiting thread parks.
        let scope = Arc::new(StartScope::new("s1".to_owned(), 2));
        scope.running.acquire(scope.bound);
        scope.running.acquire(scope.bound);
        let ending = Arc::clone(&scope);
        let ended = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            ending.running.release();
            std::thread::sleep(std::time::Duration::from_millis(20));
            ending.running.release();
            ending.running.release();
        });
        // The third waits for a release.
        scope.running.acquire(scope.bound);
        scope.running.wait_zero();
        ended.join().unwrap();
        assert_eq!(scope.path, "s1");
    }
}
