//! The operations of `HttpServer` in native code (ADR 0123 rules 1 to 4, 14 and 18, `spec/31
//! §1`), over the serve engine of `crates/una-http` that the interpreter shares, with its
//! semantics (`crates/una-interp/src/machine/serve.rs`): the same checks in the same order, the
//! same table of reply rights, and the same schedule under virtual time, so that a run of the
//! scripted connections of `tests/serve_runs/` gives both engines the same bytes and trace.
//!
//! - **The run's engine** ([`Serving`]) is installed before `main` and is the same for every
//!   task of the run: its [`Port`], its [`Rights`], whether `close` came, the tasks waiting in
//!   `next`, and the abort that ends the run. Without one, a server operation is a defect that
//!   ends the run with U8001, as in the interpreter. A program that serves installs the engine
//!   of its serve threads before `main` ([`install_live`]), and the runs of
//!   `tests/serve_runs/` a scripted one, which only a runtime built with its `test-hooks` has
//!   (`spec/31 §1` rule 10, §8 rule 6).
//! - **Holders.** Each task has a holder number in the run, the root 0 and each started task a
//!   fresh one; a task of `together` or `first of` holds as the task that runs its block, as in
//!   the interpreter, which runs those tasks inline (they perform no server operation, U3033).
//! - **`next`** checks an abort of the run, then `close`, then the host stop, then asks the
//!   engine for a request, one atomic step ([`super::Port::next`]); with none, it lets a ready
//!   task that can make progress run first ([`crate::sched::yield_to_progress`]), and only when
//!   none can does it wait for the engine: on the worker under virtual time, as the interpreter
//!   does, and on a helper of the I/O pool while the task parks under the real clock, ended by
//!   a cancellation, an abort of the run or a host stop. A wait takes no request, so a `next`
//!   that fails leaves every request queued (rule 4.7).
//! - **Reply rights** are words of the heap ([`values::right_word`]); `start` moves the rights
//!   its captures reach ([`move_at_start`]), `reply` checks the holder, and a task's end answers
//!   what it still holds ([`answer_held`]).
//! - **An abort of the run** (`spec/27 §7`) in a run that serves does not end the process at
//!   once: it is recorded, the run's cancellation entry, which every task carries, falls to 0,
//!   and each task stops at its next check point, `next` at the head of its loop, so that the
//!   host answers the rights each task holds with `503` and the task's events are written, as
//!   the interpreter's tasks end. The root then ends the run with the recorded abort. A hard
//!   stop (U6007) and an internal error (U8001) still end the process at once: the host first
//!   answers every right left with `503`, appending no event.
//!
//! Each operation is O(1) beside the bytes of the request or the reply; the move at `start`
//! looks once at each value its marked captures reach; a task's end is O(rights held).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use una_diag::{Code, Location};
use una_http::serve::program::{Look, Port, Waited, peer_text};
use una_http::serve::rights::{Rights, Task, Word};
use una_http::serve::run::{Exchange, Next, Sent};
use una_http::serve::trace::TraceKey;
use una_interp::{Event, EventOutcome};

use crate::runtime::{Cancel, with_state};
use crate::sched::TaskKey;
use crate::values;

/// The status of the auto-answer to a right its task still holds when it returned, failed or
/// ended alone (ADR 0123 rule 4.5).
pub(crate) const UNANSWERED: u16 = 500;

/// The status of the auto-answer to a right its task still holds when it was cancelled, cut by
/// `within` or stopped, with `Retry-After: 1` (ADR 0123 rule 4.5).
pub(crate) const STOPPED: u16 = 503;

/// The variants of `ServeError` (`spec/31 §1`), as words: `index << 1 | 1`.
const INTERRUPTED: i64 = 1;
const TIMED_OUT: i64 = 3;
const CLOSED: i64 = 5;
const NOT_HELD: i64 = 7;
const BAD_REPLY: i64 = 9;

/// The word an operation that stops its task returns: the stop of a task in the generated code
/// (`una_codegen::lower::STOPPED`).
const STOP: i64 = -1;

/// The count of the run's cancellation entry while the run has not aborted, and its entry: it
/// carries the mark of the entries that interrupt a waiting operation (`tasks::RACE_ENTRY`), so
/// that an abort of the run wakes every task parked on one.
const RUN_ENTRY: usize = crate::tasks::RACE_ENTRY | 1;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A task waiting in `next`: its cancellation entries, those its operation does not shield, but
/// the run's, and the deadline on the run's clock that ends its wait.
#[derive(Clone)]
struct Waiter {
    entries: Vec<Cancel>,
    until: Option<u64>,
}

impl Waiter {
    /// Whether the wait has ended at `now` by a cancellation or a deadline, so the task must run
    /// to end its `next`.
    fn ended(&self, now: u64) -> bool {
        cancelled(&self.entries) || self.until.is_some_and(|until| until <= now)
    }
}

/// Whether one of `entries` cancels its task.
fn cancelled(entries: &[Cancel]) -> bool {
    entries
        .iter()
        .any(|(count, index)| count.load(Ordering::SeqCst) < *index)
}

/// The serve engine of a run, as its program meets it, and the run's reply rights.
pub(crate) struct Serving {
    port: Arc<dyn Port + Send + Sync>,
    rights: Rights<Box<Exchange>>,
    /// Set by `close`, once: every later `next` fails with `Closed`.
    closed: AtomicBool,
    /// The holder number of the next started task; the root's is 0.
    holders: AtomicU64,
    /// The tasks waiting in `next`, by their key on the run's workers.
    waiters: Mutex<BTreeMap<TaskKey, Waiter>>,
    /// The abort that ends the run, the first one recorded, and the run's cancellation entry,
    /// which it lowers to 0.
    abort: Mutex<Option<(Code, Location, String)>>,
    entry: Arc<AtomicUsize>,
    /// The key of the run's reduced trace form, and whether the trace writes the server's
    /// values without their labels' redaction (`Port::reveals_trace`, the tests only).
    trace: (TraceKey, bool),
    /// The serve threads of a run over live sockets, which the run's end ends ([`end_engine`]).
    live: Option<Arc<una_http::serve::live::Live>>,
    /// The scripted engine of the tests, which the run's end ends.
    #[cfg(feature = "test-hooks")]
    script: Option<Arc<una_http::serve::script::ScriptPort>>,
}

/// The run's engine, once installed.
static SERVING: Mutex<Option<Arc<Serving>>> = Mutex::new(None);

/// The run's engine, if it serves.
pub(crate) fn serving() -> Option<Arc<Serving>> {
    lock(&SERVING).clone()
}

impl Serving {
    fn new(port: Arc<dyn Port + Send + Sync>) -> Serving {
        Serving {
            trace: (port.trace_key(), port.reveals_trace()),
            port,
            rights: Rights::new(),
            closed: AtomicBool::new(false),
            holders: AtomicU64::new(0),
            waiters: Mutex::new(BTreeMap::new()),
            abort: Mutex::new(None),
            entry: Arc::new(AtomicUsize::new(RUN_ENTRY)),
            live: None,
            #[cfg(feature = "test-hooks")]
            script: None,
        }
    }

    /// Whether a started task aborted the run, which lowered the run's entry: every task stops
    /// at its next check point.
    fn aborted(&self) -> bool {
        self.entry.load(Ordering::SeqCst) < RUN_ENTRY
    }
}

/// Starts the serve threads of a program that serves, before `main` (S4 amendment B.2), over
/// the listeners `prepared` bound for `grants`, `threads` of them, and installs the run's
/// engine over them; they accept nothing until the run's first server operation. The run's
/// host stop and hard stop reach the engine through `stop` (ADR 0123 rule 18). A failure is
/// the message of a usage error.
pub(crate) fn install_live(
    prepared: una_http::serve::Prepared,
    grants: &[una_core::http::serve::ServeGrant],
    threads: usize,
    stop: &una_interp::stopping::RunStop,
) -> Result<(), String> {
    // A test build of the runtime: the placement probe of `serve_owner_spread` (#1660); the
    // executable is the process, as for `env_var`.
    #[cfg(feature = "test-hooks")]
    #[allow(clippy::disallowed_methods)]
    {
        let unkeyed = std::env::var_os("UNA_TEST_SERVE_UNKEYED").is_some_and(|v| v == "1");
        let log = std::env::var_os("UNA_TEST_SERVE_OWNERS");
        if unkeyed || log.is_some() {
            una_http::serve::place::probe::install(
                unkeyed,
                log.as_deref().map(std::path::Path::new),
            )
            .map_err(|error| format!("the placement probe: {error}"))?;
        }
    }
    let live = Arc::new(una_http::serve::live::Live::start(
        prepared, grants, threads,
    )?);
    let hooked = Arc::clone(&live);
    stop.on_stops(Box::new(move |hard| {
        if hard {
            hooked.hard_stop();
        } else {
            hooked.host_stop();
        }
    }));
    let mut serving = Serving::new(Arc::clone(&live) as Arc<dyn Port + Send + Sync>);
    serving.live = Some(live);
    *lock(&SERVING) = Some(Arc::new(serving));
    Ok(())
}

/// Ends the serve threads of a run over live sockets, once, before the process ends: what the
/// run answered is written, as a host stop drains, for at most the grace period `drain`, then
/// the hard stop closes what is left; with `hard`, after the hard stop or an internal error,
/// the hard stop at once (ADR 0123 rule 18). Nothing in a run with no live engine.
pub(crate) fn end_engine(drain: std::time::Duration, hard: bool) {
    if let Some(live) = serving().and_then(|serving| serving.live.clone()) {
        live.end(drain, hard);
    }
}

/// The key of the run's reduced trace form (ADR 0123 rule 1), and whether the trace writes the
/// server's values without their labels' redaction (the toolchain's tests only); `None` in a
/// run that serves nothing.
pub(crate) fn trace() -> Option<(TraceKey, bool)> {
    serving().map(|serving| serving.trace)
}

/// The run's cancellation entry, which the root carries and every task inherits, when the run
/// serves.
pub(crate) fn run_entry() -> Option<Cancel> {
    serving().map(|serving| (Arc::clone(&serving.entry), RUN_ENTRY))
}

/// Whether `entry` is the run's own, which an abort lowers, not a cancellation of the task.
fn is_run_entry(serving: &Serving, (count, _): &Cancel) -> bool {
    Arc::ptr_eq(count, &serving.entry)
}

/// Records the abort of the run with `code` at `location`, when the run serves and the abort is
/// one the run's tasks end by (not U6007 nor U8001): the first one counts, and the root ends
/// the run with it ([`take_abort`]). A started task's abort, `root` false, also lowers the run's
/// entry, so every task stops at its next check point and its `next` ends `aborted`, as the
/// interpreter's tasks see an abort of the run. The root's own abort, `root` true, lowers
/// nothing: the root unwinds through its `tasks` statements, which cancel their tasks, as the
/// interpreter unwinds it. `false` when the caller ends the process at once instead.
pub(crate) fn defer_abort(code: Code, location: &Location, message: &str, root: bool) -> bool {
    let Some(serving) = serving() else {
        return false;
    };
    if matches!(code, Code::U6007 | Code::U8001) {
        return false;
    }
    {
        let mut abort = lock(&serving.abort);
        if abort.is_none() {
            *abort = Some((code, location.clone(), message.to_owned()));
        }
    }
    if !root {
        serving.entry.store(0, Ordering::SeqCst);
        crate::interrupt::lowered(&serving.entry);
    }
    true
}

/// Whether the run serves and one of its started tasks aborted it, which every task sees.
pub(crate) fn run_aborted() -> bool {
    serving().is_some_and(|serving| serving.aborted())
}

/// Whether the run serves and an abort that ends it was recorded, the root's included.
pub(crate) fn abort_recorded() -> bool {
    serving().is_some_and(|serving| lock(&serving.abort).is_some())
}

/// The abort that ends the run, once its root has ended.
pub(crate) fn take_abort() -> Option<(Code, Location, String)> {
    serving().and_then(|serving| lock(&serving.abort).take())
}

/// The holder number of the task that runs: the root is 0.
fn holder() -> Task {
    with_state(|state| state.serve_task).unwrap_or(0)
}

/// A fresh holder number for a task that `start` starts.
pub(crate) fn fresh_holder() -> Task {
    serving().map_or(0, |serving| {
        serving
            .holders
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1)
    })
}

/// `start` moves to the started task `to` every right that the task that runs holds and that
/// the first `captures` values of `closure` reach (rule 4.2), in their order
/// ([`values::rights_in_captures`]), before the task is queued. A right the task does not hold
/// stays where it is. O(values reached).
pub(crate) fn move_at_start(closure: i64, captures: usize, to: Task) {
    let Some(serving) = serving() else {
        return;
    };
    let from = holder();
    for right in values::rights_in_captures(closure, captures) {
        serving.rights.transfer(from, to, right);
    }
}

/// Answers `503` each right `task` still holds, in the order it received them, and appends no
/// event: a task that never started (ADR 0123 rule 4.5). O(rights held).
pub(crate) fn answer_left(task: Task) {
    if let Some(serving) = serving() {
        for (_, mut exchange) in serving.rights.end(task) {
            exchange.answer(STOPPED);
        }
    }
}

/// The end of the task that runs, with the status of its auto-answers (rule 4.5): each right it
/// still holds, in the order it received them, is answered by the host, once, and appends its
/// `unanswered` event, even when another transition of rule 4.7 ended the exchange first, so
/// the trace does not depend on timing. Quiet code appends no event, as for its operations
/// (S-42, `spec/26 §6` rule 4). O(rights held).
pub(crate) fn answer_held(status: u16) {
    let Some(serving) = serving() else {
        return;
    };
    let ended = serving.rights.end(holder());
    for (_, mut exchange) in ended {
        exchange.answer(status);
        with_state(|state| {
            if state.hushed > 0 {
                return;
            }
            let task = state.path.clone();
            if let Some(trace) = state.trace.as_mut() {
                trace.push(Event {
                    seq: trace.len() + 1,
                    task,
                    instance: String::new(),
                    capability: String::new(),
                    op: String::new(),
                    args: Vec::new(),
                    outcome: EventOutcome::Unanswered(status),
                });
            }
        });
    }
    with_state(crate::runtime::State::flush);
}

/// Answers `503`, with no event, every right still held in the run, at an end of the process
/// that does not wait for its tasks: a hard stop, an internal error (ADR 0123 rule 18 item 6,
/// failing closed).
pub(crate) fn answer_all_left() {
    if let Some(serving) = serving() {
        for (_, _, mut exchange) in serving.rights.end_all() {
            exchange.answer(STOPPED);
        }
    }
}

/// The rights live in the run, for the tests' check that none is left.
#[cfg(feature = "test-hooks")]
fn live() -> usize {
    serving().map_or(0, |serving| serving.rights.live())
}

/// An operation of `HttpServer` in a run with no engine: a defect, as in the interpreter, which
/// ends the run with U8001 at the operation.
fn no_engine(site: i64, op: &str) -> i64 {
    crate::runtime::abort_at(
        usize::try_from(site).unwrap_or(usize::MAX),
        Code::U8001,
        format!("`HttpServer.{op}` in a run that serves nothing"),
    );
    STOP
}

/// Stops the task in its operation, its event left `aborted`: the run aborted.
fn stopped() -> i64 {
    with_state(|state| state.op_stopped = true);
    STOP
}

/// Ends the operation `interrupted`, which stops its task (ADR 0117 rule 4).
fn interrupted() -> i64 {
    with_state(|state| state.interrupted = true);
    STOP
}

/// Whether the run's host stop came (ADR 0116 rule 14): a stopped run never delivers.
fn host_stopped() -> bool {
    crate::runtime::run_stop().is_some_and(|stop| stop.stopped())
}

/// A task in the run's waiters in `next`, removed on every way out.
struct Waiting<'s> {
    serving: &'s Serving,
    key: Option<TaskKey>,
}

impl<'s> Waiting<'s> {
    fn enter(serving: &'s Serving, key: Option<TaskKey>, entries: Vec<Cancel>) -> Waiting<'s> {
        if let Some(key) = key {
            let fresh = lock(&serving.waiters)
                .insert(
                    key,
                    Waiter {
                        entries,
                        until: None,
                    },
                )
                .is_none();
            // INVARIANT: a task runs one operation at a time, so it enters `next` once.
            debug_assert!(fresh, "a task entered `next` twice");
        }
        Waiting { serving, key }
    }

    /// Records the deadline that ends this wait, on the run's clock.
    fn until(&self, until: Option<u64>) {
        if let Some(key) = self.key
            && let Some(waiter) = lock(&self.serving.waiters).get_mut(&key)
        {
            waiter.until = until;
        }
    }

    /// The earliest deadline of the other waiters, which this one's wait may not outlast.
    fn others_until(&self) -> Option<u64> {
        lock(&self.serving.waiters)
            .iter()
            .filter(|(key, _)| Some(**key) != self.key)
            .filter_map(|(_, waiter)| waiter.until)
            .min()
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if let Some(key) = self.key {
            let left = lock(&self.serving.waiters).remove(&key).is_some();
            // INVARIANT: only this guard removes its task, once.
            debug_assert!(left, "a task left `next` twice");
        }
    }
}

/// When a wait for the engine ends at the latest: at the earliest of the waiter's own deadline
/// and the other waiters' earliest, as in the interpreter.
fn wake_at(own: Option<u64>, others: Option<u64>) -> Option<u64> {
    match (own, others) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Whether a wait that timed out at `wake` reached the waiter's own deadline `own`.
fn own_deadline(own: Option<u64>, wake: Option<u64>) -> bool {
    own.is_some() && wake == own
}

/// `next` (ADR 0123 rules 1, 4.1, 4.7 and 18), at the call `site`: the task stops when the run
/// aborted, `Closed` after `close`, `Interrupted` once a host stop came, else the next request
/// delivered, an `Exchange` whose right the calling task holds; it waits while none can be
/// delivered, and its deadline, a `within`'s or a `for ever within`'s, ends the wait with
/// `TimedOut`.
pub fn next(site: i64) -> i64 {
    let Some(serving) = serving() else {
        return no_engine(site, "next");
    };
    // The run's first server operation begins accepting (S4 amendment B.2).
    serving.port.begin();
    let task = holder();
    // The cancellations that end this wait: the task's own, which its operation does not
    // shield, not the run's, which an abort lowers and the loop's head sees first.
    let entries: Vec<Cancel> = crate::runtime::interruptible_entries()
        .into_iter()
        .filter(|entry| !is_run_entry(&serving, entry))
        .collect();
    let clock = crate::runtime::clock();
    let virtual_time = clock.is_virtual();
    let waiting = Waiting::enter(&serving, crate::sched::current_key(), entries.clone());
    loop {
        // An abort of another task ends the whole run (`spec/27 §7`), and this `next` ends with
        // it, taking no request. It comes first: a `close` or a host stop that came with the
        // abort does not turn the run's end into a catchable failure of this task.
        if serving.aborted() {
            return stopped();
        }
        if serving.closed.load(Ordering::SeqCst) {
            return crate::host::failed(CLOSED);
        }
        // A stopped run never delivers: the engine refuses what it has not (rule 18).
        if host_stopped() {
            return crate::host::failed(INTERRUPTED);
        }
        let covering = crate::deadline::left();
        let epoch = serving.port.epoch();
        let retry = match serving.port.next() {
            Next::Exchange(exchange) => return deliver(&serving, task, site, exchange),
            // The engine stopped the run by a fault (`spec/31 §6` rule 13): U8001.
            Next::Failed(failure) => {
                crate::runtime::abort_at(
                    usize::try_from(site).unwrap_or(usize::MAX),
                    Code::U8001,
                    failure.to_string(),
                );
                return STOP;
            }
            Next::Blocked(retry) => retry,
            Next::Empty => None,
        };
        // Past its deadline, the wait ends with no request taken. Under virtual time a wait for
        // the network takes no virtual time (ADR 0116 rule 19), so only a deadline that has
        // passed cuts it.
        if let Some(covering) = covering
            && covering.left.is_zero()
        {
            return timed_out(covering.bound);
        }
        let now = clock.now();
        let until = covering.map(|covering| covering.at_millis());
        waiting.until(until);
        // Another ready task runs first, unless it only waits in `next` itself and its wait has
        // not ended. Each queued task is looked up in the waiters as the scheduler's scan reaches
        // it, O(log waiters), with no copy of the waiters: the scan holds its worker's queue, and
        // the waiters' lock is never held while a queue's is taken, so the order is one way only.
        let aborted = serving.aborted();
        let yielded = crate::sched::yield_to_progress(|key| {
            aborted
                || key.is_none_or(|key| {
                    lock(&serving.waiters)
                        .get(&key)
                        .is_none_or(|waiter| waiter.ended(now))
                })
        });
        if yielded {
            // An abort of the run that came meanwhile comes first, as at the loop's head; a
            // cancellation that came with it does not make the operation `interrupted`.
            if serving.aborted() {
                return stopped();
            }
            if cancelled(&entries) {
                return interrupted();
            }
            continue;
        }
        // The wait lasts no longer than the earliest deadline of any waiter, so one whose
        // deadline passes runs; only its own ends this `next` with `TimedOut`.
        let wake = wake_at(until, waiting.others_until());
        let deadline = match wake {
            Some(at) if !virtual_time => std::time::Instant::now()
                .checked_add(std::time::Duration::from_millis(at.saturating_sub(now))),
            _ => None,
        };
        let waited = if virtual_time || !crate::sched::in_task() {
            // On the worker, as the interpreter waits: every other ready task only waits in
            // `next`, or the run has no tasks.
            let watched = Arc::clone(&serving);
            let entries = entries.clone();
            serving.port.wait(epoch, retry, deadline, &move || {
                host_stopped() || watched.aborted() || cancelled(&entries)
            })
        } else if let Some(waited) = crate::reactor::parks()
            .and_then(|reactor| parked_wait(reactor, &serving, &entries, epoch, retry, deadline))
        {
            waited
        } else {
            // On a helper of the I/O pool while the task parks (ADR 0117 rule 3), only when the
            // reactor is not there or cannot hold the wait: a cancellation or an abort of the
            // run wakes the task, and the helper's wait ends at its next look at its stop.
            let woken = Arc::new(AtomicBool::new(false));
            let raised = Arc::clone(&woken);
            let also: crate::interrupt::Wake =
                Arc::new(move || raised.store(true, Ordering::SeqCst));
            let port = Arc::clone(&serving.port);
            let watched = Arc::clone(&serving);
            let job = move || {
                port.wait(epoch, retry, deadline, &|| {
                    woken.load(Ordering::SeqCst) || host_stopped() || watched.aborted()
                })
            };
            let mut all = entries.clone();
            all.push((Arc::clone(&serving.entry), RUN_ENTRY));
            crate::sched::blocking_interruptible(crate::sched::Pool::Io, &all, Some(also), job)
                // Woken: the loop's head, or the check below, says by what.
                .unwrap_or(Waited::Changed)
        };
        if cancelled(&entries) && !serving.aborted() {
            // A wake this wait claimed, or one its helper may still claim before it sees the
            // cancellation, goes to another waiter: the engine wakes one waiter per request.
            serving.port.pass();
            return interrupted();
        }
        match waited {
            // The loop's head says which: a host stop or an abort of the run.
            Waited::Changed | Waited::Stopped => {}
            // Another waiter's deadline passed: it runs, and this one waits again.
            Waited::TimedOut if !own_deadline(until, wake) => {}
            Waited::TimedOut => {
                if let Some(covering) = crate::deadline::left() {
                    // The call returns once the run's clock has reached the deadline (#1355).
                    covering.reach();
                    return timed_out(covering.bound);
                }
            }
        }
    }
}

/// The wait of a task's `next` parked on the reactor, under the real clock (#1723): the task
/// holds no thread while it waits, so no number of waiting tasks takes a helper of the I/O pool
/// from the operations of the tasks that answer, as `Http`'s lookups and connections, or from
/// `std.limit`'s waits. Each pass looks at the engine once ([`Port::look`]), then parks with its
/// wake left with the engine, registered under the lock the engine's signals are sent under, so
/// no signal sent after the look is lost; the park ends at that wake (a signal for one waiter,
/// or any stop of the engine: `close`, the host stop, the hard stop), at the deadline or a
/// rate's retry time, at a cancellation of the task or an abort of the run, which [`next`] sees
/// at its loop's head, or at the run's host stop, which the reactor also tells. No timer looks at
/// an idle wait, so an idle waiter costs no wake per unit of time. A
/// deadline's cut of a `tasks` statement wakes it not, as it wakes no wait that began on a
/// helper (#1148). `None` when the engine has no such waits or the reactor cannot hold the
/// wait: the caller waits on a helper. O(log p) a pass for the p parked waiters.
fn parked_wait(
    reactor: &'static crate::reactor::Reactor,
    serving: &Serving,
    entries: &[Cancel],
    epoch: u64,
    retry: Option<u64>,
    deadline: Option<std::time::Instant>,
) -> Option<Waited> {
    let stopped = || host_stopped() || serving.aborted() || cancelled(entries);
    let mut all = entries.to_vec();
    all.push((Arc::clone(&serving.entry), RUN_ENTRY));
    loop {
        // The park lasts until the deadline or the retry time, else until a wake only: the
        // reactor then arms no timer for it (#1726 review 2, M3).
        let slice = match serving.port.look(epoch, retry, deadline, &stopped, None)? {
            Look::Done(waited) => return Some(waited),
            Look::Park { slice, .. } => slice,
        };
        let ticket = std::cell::Cell::new(None);
        let done = std::cell::Cell::new(None);
        let woken =
            reactor.wait_on(
                None,
                slice,
                crate::runtime::run_stop(),
                &all,
                |ready| match serving.port.look(
                    epoch,
                    retry,
                    deadline,
                    &stopped,
                    Some(Arc::clone(ready)),
                ) {
                    Some(Look::Done(waited)) => {
                        done.set(Some(waited));
                        true
                    }
                    Some(Look::Park { ticket: held, .. }) => {
                        ticket.set(held);
                        false
                    }
                    None => true,
                },
            );
        if let Some(held) = ticket.get() {
            serving.port.unpark(held);
        }
        if let Some(waited) = done.get() {
            return Some(waited);
        }
        match woken {
            // The loop's head of `next` says by what: a cancellation or an abort of the run.
            crate::reactor::Woken::Cancelled => return Some(Waited::Changed),
            crate::reactor::Woken::Failed => return None,
            // The engine's wake, the slice's end: the next look tells.
            crate::reactor::Woken::Ready
            | crate::reactor::Woken::TimedOut
            | crate::reactor::Woken::Stopped => {}
        }
    }
}

/// `TimedOut` of a `next` that its covering deadline cut: a `for ever within` iteration
/// observes it at the operation.
fn timed_out(bound: crate::deadline::Bound) -> i64 {
    if bound == crate::deadline::Bound::Ever {
        crate::deadline::timed_out();
    }
    crate::host::failed(TIMED_OUT)
}

/// The `Method` of `std.host` for the engine's method (`spec/31 §1`), a variant without fields.
fn method(method: una_http::strict::Method) -> i64 {
    use una_http::strict::Method as M;
    let index = match method {
        M::Get => 0,
        M::Head => 1,
        M::Post => 2,
        M::Put => 3,
        M::Patch => 4,
        M::Delete => 5,
        M::Options => 6,
    };
    (index << 1) | 1
}

/// A record of `fields`, which it takes.
fn record(fields: &[i64]) -> i64 {
    let record = values::data(0);
    for &field in fields {
        values::push(record, field);
        values::release(field);
    }
    record
}

/// A list of `items`, which it takes.
fn list(items: Vec<i64>) -> i64 {
    let list = values::list();
    for item in items {
        values::push(list, item);
        values::release(item);
    }
    list
}

/// The `Request` of `std.host` for what the engine read (`spec/31 §1`): one copy of its bytes
/// into values, its fields in the order of the declaration.
fn request(exchange: &Exchange) -> i64 {
    let read = &exchange.request;
    let text = |t: &str| values::text_word(t.to_owned());
    let segments = list(read.segments.iter().map(|s| text(s)).collect());
    let params = list(
        read.query
            .iter()
            .map(|(name, value)| record(&[text(name), text(value)]))
            .collect(),
    );
    let headers = list(
        read.headers
            .iter()
            .map(|(name, value)| record(&[text(name), text(value)]))
            .collect(),
    );
    record(&[
        method(read.method),
        text(&read.target),
        segments,
        params,
        headers,
        text(&read.body),
        text(&peer_text(exchange.peer())),
        text(exchange.listener()),
    ])
}

/// The exchange `next` took, as an `Exchange` of `std.host`, its right held by `task` (rule
/// 4.1): the request's values are built first, then the right is recorded, so nothing can fail
/// between the record and the return.
fn deliver(serving: &Serving, task: Task, site: i64, exchange: Box<Exchange>) -> i64 {
    let request = request(&exchange);
    match serving.rights.deliver(task, exchange) {
        Ok(right) => record(&[request, values::right_word(right)]),
        Err(mut exchange) => {
            // The table cannot take it: the host answers it, and the run stops, failing closed.
            exchange.answer(STOPPED);
            values::release(request);
            crate::runtime::abort_at(
                usize::try_from(site).unwrap_or(usize::MAX),
                Code::U8001,
                "the run's table of reply rights cannot take another exchange".to_owned(),
            );
            STOP
        }
    }
}

/// The engine's reply for a `Reply` of `std.host`, borrowed: a status out of range stays out of
/// range, which the engine's checks refuse (`BadReply`).
fn reply_of(word: i64) -> una_http::reply::Reply {
    let status = values::int_value(values::field(word, 0));
    let headers = values::list_items(values::field(word, 1))
        .into_iter()
        .map(|header| {
            (
                values::text_value(values::field(header, 0)),
                values::text_value(values::field(header, 1)),
            )
        })
        .collect();
    una_http::reply::Reply {
        status: i64::try_from(&status)
            .ok()
            .and_then(|n| u16::try_from(n).ok())
            .unwrap_or(0),
        headers,
        body: values::text_value(values::field(word, 2)),
    }
}

/// `reply(to, reply)` (rules 4.4, 4.7, 13 and 14), at the call `site`: `NotHeld` for a right the
/// task does not hold, `BadReply` for a reply the host refuses (the right stays held);
/// otherwise the reply is handed over, or discarded when another transition ended the exchange
/// first, and the right is released.
pub fn reply(site: i64, to: i64, reply: i64) -> i64 {
    let Some(serving) = serving() else {
        return no_engine(site, "reply");
    };
    serving.port.begin();
    let Some(right) = values::right_of(to) else {
        return no_engine(site, "reply");
    };
    let right: Word = right;
    let reply = reply_of(reply);
    let task = holder();
    let Ok(mut exchange) = serving.rights.take(task, right) else {
        return crate::host::failed(NOT_HELD);
    };
    match exchange.reply(&reply) {
        Err(_) => {
            serving.rights.restore(task, right, exchange);
            crate::host::failed(BAD_REPLY)
        }
        Ok(Sent::Sent | Sent::Discarded | Sent::NotHeld) => {
            serving.rights.release(task, right);
            0
        }
    }
}

/// `close` (ADR 0123 rule 1): once; it does not begin accepting.
pub fn close(site: i64) -> i64 {
    let Some(serving) = serving() else {
        return no_engine(site, "close");
    };
    if !serving.closed.swap(true, Ordering::SeqCst) {
        serving.port.close();
    }
    0
}

/// The test hook of the runs of `tests/serve_runs/` (ADR 0123 rule 21): a run whose environment
/// names a case directory in `UNA_TEST_SERVE_SCRIPT` serves the scripted engine of that case,
/// its `grant.txt` and `script.jsonl`, in place of listeners. Only the toolchain's own test
/// builds compile it.
#[cfg(feature = "test-hooks")]
pub(crate) mod script {
    use super::*;

    /// Installs the scripted engine when the environment names a case: whether it did, or why
    /// the case cannot be read, a missing or invalid `grant.txt` or `script.jsonl` included,
    /// which the run reports as a usage error before `main`.
    pub(crate) fn install_from_environment() -> Result<bool, String> {
        // A test build of the runtime: the executable is the process, as for `env_var`.
        #[allow(clippy::disallowed_methods)]
        let Some(dir) = std::env::var_os("UNA_TEST_SERVE_SCRIPT") else {
            return Ok(false);
        };
        let dir = std::path::PathBuf::from(dir);
        let read = |name: &str| {
            let path = dir.join(name);
            std::fs::read_to_string(&path).map_err(|error| {
                format!("cannot read the serve case's `{}`: {error}", path.display())
            })
        };
        let grants: Vec<String> = read("grant.txt")?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_owned)
            .collect();
        if grants.is_empty() {
            return Err(format!("the serve case `{}` names no grant", dir.display()));
        }
        let specs: Vec<&str> = grants.iter().map(String::as_str).collect();
        let script = una_http::serve::script::lines(&read("script.jsonl")?)
            .map_err(|error| format!("the serve case's `script.jsonl`: {error}"))?;
        let port = Arc::new(una_http::serve::script::ScriptPort::new(
            &specs,
            script,
            Box::new(|| {
                if let Some(stop) = crate::runtime::run_stop() {
                    stop.request_local();
                }
            }),
        ));
        // `UNA_TEST_SERVE_REVEAL=1`: the trace writes the server's values without their
        // labels' redaction, still in the reduced form, for the tests that compare it.
        #[allow(clippy::disallowed_methods)]
        if std::env::var_os("UNA_TEST_SERVE_REVEAL").is_some_and(|v| v == "1") {
            port.reveal();
        }
        let mut serving = Serving::new(Arc::clone(&port) as Arc<dyn Port + Send + Sync>);
        serving.script = Some(port);
        *lock(&SERVING) = Some(Arc::new(serving));
        Ok(true)
    }

    use std::io::Write as _;

    /// Where `--serve-out` writes, set once by [`install_from_script`].
    static OUT: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();

    /// Installs the scripted engine over the checked events of `--serve-script` (`spec/31 §8`
    /// rule 6) when the environment says `UNA_TEST_RUN_SERVER=1`, as `una run` of the same
    /// test build does: whether it did. `out` is the file of `--serve-out`.
    pub(crate) fn install_from_script(
        grants: &[una_core::http::serve::ServeGrant],
        events: &[una_http::serve::script_file::Event],
        out: Option<std::path::PathBuf>,
    ) -> bool {
        #[allow(clippy::disallowed_methods)]
        if std::env::var_os("UNA_TEST_RUN_SERVER").is_none_or(|v| v != "1") {
            return false;
        }
        let port = Arc::new(una_http::serve::script::ScriptPort::from_script(
            grants,
            events,
            Box::new(|| {
                if let Some(stop) = crate::runtime::run_stop() {
                    stop.request_local();
                }
            }),
        ));
        let _ = OUT.set(out);
        let mut serving = Serving::new(Arc::clone(&port) as Arc<dyn Port + Send + Sync>);
        serving.script = Some(port);
        *lock(&SERVING) = Some(Arc::new(serving));
        true
    }

    /// Ends the scripted engine at the run's end and writes what each connection received, one
    /// JSON line each, then `{"rights_left": n, "problems": [...]}`, to the file that
    /// `UNA_TEST_SERVE_WIRE` names. A run of `--serve-script` reports what the engine left not
    /// at rest as warnings and writes `--serve-out`, as `una run` does: the message of a file
    /// that cannot be written is the `Err`, a usage error.
    pub(crate) fn end() -> Result<(), String> {
        let Some(port) = serving().and_then(|serving| serving.script.clone()) else {
            return Ok(());
        };
        let left = live();
        let ended = port.end();
        let mut written = Ok(());
        if let Some(out) = OUT.get() {
            let mut stderr = std::io::stderr();
            for problem in &ended.problems {
                let _ = writeln!(
                    stderr,
                    "warning: the scripted engine at the end of the run: {problem}"
                );
            }
            if let Some(path) = out {
                let text = una_http::serve::script::out_text(&ended.wire);
                written = una_http::serve::script_file::write_out(path, &text)
                    .map_err(|error| una_http::serve::script_file::out_error(path, &error));
            }
        }
        #[allow(clippy::disallowed_methods)]
        let Some(path) = std::env::var_os("UNA_TEST_SERVE_WIRE") else {
            return written;
        };
        let mut text = String::new();
        for line in &ended.wire {
            text.push_str(&line.to_string());
            text.push('\n');
        }
        text.push_str(
            &serde_json::json!({"rights_left": left, "problems": ended.problems}).to_string(),
        );
        text.push('\n');
        let _ = std::fs::write(path, text);
        written
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use una_http::serve::program::{Look, Port, Waited, Waker};
    use una_http::serve::run::Next;
    use una_http::serve::trace::TraceKey;

    use super::{Serving, own_deadline, parked_wait, wake_at};
    use crate::{reactor, sched};

    /// An engine that counts the looks of its waiters, holds the wake each one leaves, and ends
    /// every wait once `release` is set.
    #[derive(Default)]
    struct Counted {
        looks: AtomicUsize,
        wake: Mutex<Option<Waker>>,
        release: AtomicBool,
    }

    impl Port for Counted {
        fn begin(&self) {}
        fn epoch(&self) -> u64 {
            0
        }
        fn next(&self) -> Next {
            Next::Empty
        }
        fn wait(
            &self,
            _: u64,
            _: Option<u64>,
            _: Option<std::time::Instant>,
            _: &dyn Fn() -> bool,
        ) -> Waited {
            Waited::Changed
        }
        fn look(
            &self,
            _: u64,
            _: Option<u64>,
            _: Option<std::time::Instant>,
            _: &dyn Fn() -> bool,
            wake: Option<Waker>,
        ) -> Option<Look> {
            let n = self.looks.fetch_add(1, Ordering::SeqCst);
            if self.release.load(Ordering::SeqCst) {
                return Some(Look::Done(Waited::Changed));
            }
            let ticket = wake.map(|wake| {
                *self.wake.lock().unwrap() = Some(wake);
                n as u64
            });
            Some(Look::Park {
                ticket,
                slice: None,
            })
        }
        fn close(&self) {}
        fn trace_key(&self) -> TraceKey {
            TraceKey::ZERO
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn an_idle_parked_next_is_looked_at_only_when_woken_and_fails_no_poller_wait() {
        // #1726 reviews 1 (M1) and 2 (M3, L5, L6): a task's `next` with no deadline parks on
        // the reactor with no timer. Counted: over 300 ms with no signal, its wait makes its
        // two looks (the attempt and the one that leaves the wake) and no more, where a 10 ms
        // re-look would make about 60; the wake the engine calls makes exactly one more; and no
        // wait of the reactor's poller fails meanwhile, with a wait of `Duration::MAX` beside
        // it, whose timeout `kevent` refused before.
        let reactor = reactor::get().unwrap();
        let failed = reactor::FAILED_WAITS.load(Ordering::SeqCst);
        let sched = sched::Scheduler::new(una_interp::Concurrency {
            workers: 2,
            task_depth: 10,
            io_threads: 1,
            file_threads: 1,
        });
        let port = Arc::new(Counted::default());
        let serving = Arc::new(Serving::new(
            Arc::clone(&port) as Arc<dyn Port + Send + Sync>
        ));
        let (sent, seen) = std::sync::mpsc::channel();
        let waiting = Arc::clone(&serving);
        sched
            .spawn(
                1,
                256 * 1024,
                move || {
                    sent.send(parked_wait(reactor, &waiting, &[], 0, None, None))
                        .unwrap()
                },
                || {},
            )
            .unwrap();
        let far = Arc::new(AtomicBool::new(false));
        let (far_sent, far_seen) = std::sync::mpsc::channel();
        let far_wake: Arc<Mutex<Option<crate::interrupt::Wake>>> = Arc::default();
        let held = Arc::clone(&far_wake);
        let parked = Arc::clone(&far);
        sched
            .spawn(
                1,
                256 * 1024,
                move || {
                    let woken = reactor.wait_on(None, Some(Duration::MAX), None, &[], |ready| {
                        *held.lock().unwrap() = Some(Arc::clone(ready));
                        parked.store(true, Ordering::SeqCst);
                        false
                    });
                    far_sent.send(woken).unwrap();
                },
                || {},
            )
            .unwrap();
        // A short wait beside them makes the reactor pass again: once its timer fires, the far
        // one is the reactor's next, whose timeout the poller must take.
        let (short_sent, short_seen) = std::sync::mpsc::channel();
        sched
            .spawn(
                1,
                256 * 1024,
                move || {
                    let woken =
                        reactor
                            .wait_on(None, Some(Duration::from_millis(50)), None, &[], |_| false);
                    short_sent.send(woken).unwrap();
                },
                || {},
            )
            .unwrap();
        assert_eq!(
            short_seen.recv_timeout(Duration::from_secs(10)).unwrap(),
            reactor::Woken::TimedOut
        );
        std::thread::sleep(Duration::from_millis(300));
        assert!(far.load(Ordering::SeqCst));
        assert_eq!(port.looks.load(Ordering::SeqCst), 2);
        assert!(seen.try_recv().is_err(), "the wait ended with no wake");
        port.release.store(true, Ordering::SeqCst);
        let wake = port.wake.lock().unwrap().take().unwrap();
        wake();
        assert_eq!(
            seen.recv_timeout(Duration::from_secs(10)).unwrap(),
            Some(Waited::Changed)
        );
        assert_eq!(port.looks.load(Ordering::SeqCst), 3);
        let far_wake = far_wake.lock().unwrap().take().unwrap();
        far_wake();
        assert_eq!(
            far_seen.recv_timeout(Duration::from_secs(10)).unwrap(),
            reactor::Woken::Ready
        );
        assert_eq!(reactor::FAILED_WAITS.load(Ordering::SeqCst), failed);
    }

    #[test]
    fn a_wait_ends_at_the_earliest_deadline_of_any_waiter() {
        assert_eq!(wake_at(None, None), None);
        assert_eq!(wake_at(Some(10), None), Some(10));
        assert_eq!(wake_at(None, Some(7)), Some(7));
        assert_eq!(wake_at(Some(10), Some(7)), Some(7));
        assert_eq!(wake_at(Some(5), Some(7)), Some(5));
    }

    #[test]
    fn only_the_waiters_own_deadline_ends_its_next() {
        assert!(own_deadline(Some(5), wake_at(Some(5), Some(7))));
        assert!(!own_deadline(Some(10), wake_at(Some(10), Some(7))));
        assert!(!own_deadline(None, wake_at(None, Some(7))));
        assert!(!own_deadline(None, None));
    }

    /// Every serve thread of an executable allocates through the process's one counting
    /// allocator (`Cap`, ADR 0061), whose counters all threads update: a parser that allocates
    /// once a chunk makes the serve threads of a body of small chunks contend for them, so 8
    /// threads read such bodies more slowly than 1 (#1780). A chunked body allocates only as
    /// its buffers grow, O(log b) times: here at most its own data again in bytes, where the
    /// reader before #1780 allocated 8 more bytes a chunk, for the line of its size.
    #[cfg(debug_assertions)]
    #[test]
    fn a_chunked_body_allocates_as_it_grows_not_once_a_chunk() {
        use una_http::request::{RequestLimits, RequestReader, Scheme};
        // Growing sizes: the bound is linear in the data, the old cost was 9 times it.
        for chunks in [4 * 1024, 16 * 1024, 64 * 1024] {
            let mut wire = b"1\r\nx\r\n".repeat(chunks);
            wire.extend_from_slice(b"0\r\n\r\n");
            let mut reader = RequestReader::new(RequestLimits::default(), Scheme::Http);
            let head = b"POST / HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n";
            assert!(!reader.feed(head).unwrap().done);
            let before = crate::ffi::thread_allocated();
            let mut done = false;
            // As a serve thread feeds it: a read of at most 16 KiB at a time.
            for piece in wire.chunks(16 * 1024) {
                let progress = reader.feed(piece).unwrap();
                assert_eq!(progress.used, piece.len());
                done = progress.done;
            }
            let allocated = crate::ffi::thread_allocated() - before;
            assert!(done);
            assert_eq!(reader.body_length(), chunks);
            assert!(
                allocated <= 2 * chunks,
                "{allocated} bytes allocated for {chunks} chunks of one byte"
            );
        }
    }
}
