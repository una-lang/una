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
    heap_charged().saturating_add(STACK_CHARGE.load(Ordering::Relaxed))
}

/// The most bytes [`heap_charged`] credits a debug build: a quarter of the run's budget, set
/// as `main` starts, 0 before (#1506). A run is so charged at least what it holds less a
/// quarter of its budget, so a check reports it with U6003 once it holds more than 1.25 times
/// its budget, well before the allocator's hard ceiling, twice the budget on the bytes really
/// held (ADR 0061 rule 3), fails an allocation and aborts. Only one allocation of more than
/// three quarters of the budget between two checks can still meet the ceiling first, where a
/// release build's margin is the whole budget: a churning debug heap under a budget of 5 MiB,
/// whose slot table doubles from 5 to 10 MiB, still does. Half the budget was not enough: the
/// same heap met the ceiling first under budgets of 1, 2, 4 and 5 MiB.
static CREDIT_LIMIT: AtomicUsize = AtomicUsize::new(0);

/// The heap the memory budget charges: what the process holds, less the slots that a debug
/// build's quarantine adds to its slot table beyond any capacity a release build's table may
/// hold (#1506), at most [`CREDIT_LIMIT`]. The credited slots are freed slots past the peak of
/// live objects; a live slot is never credited ([`values::credit_added_bytes`]; 0 in a release
/// build). The allocator's hard ceiling stays on the bytes really held. O(1).
///
/// The bytes credited ever are read before the bytes held, and the bytes taken back after:
/// a credit rises after the growth it stands for and falls before the table is freed, so a
/// credit counted here was grown into `held`, and a table freed in `held` has its credit taken
/// back here, for any number of heaps changing meanwhile. A change seen in one read and not the
/// other only charges more.
fn heap_charged() -> usize {
    if !cfg!(debug_assertions) {
        return crate::held();
    }
    let added = values::credit_added_bytes();
    let held = crate::held();
    let removed = values::credit_removed_bytes();
    held.saturating_sub(capped_credit(added, removed))
}

/// The credit [`heap_charged`] subtracts, from the bytes credited ever and taken back.
fn capped_credit(added: usize, removed: usize) -> usize {
    added
        .saturating_sub(removed)
        .min(CREDIT_LIMIT.load(Ordering::Relaxed))
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

/// The bytes that the steps of copies have ever announced, process-wide, modulo 2^64 on every
/// target (ADR 0113 rule 4). With [`RELEASED`] it gives `reserved`, the bytes
/// announced and not yet allocated: a step checks `held() + reserved` before it allocates, so
/// that copies made at once, by blocks in sibling tasks, see each other.
///
/// The count only grows, and only by the compare-exchange of an accepted step in
/// [`try_reserve`]: a value read twice with nothing else read between says that no step of
/// positive size was accepted between the two reads. One counter that rises and falls could
/// not say so: a release and an announcement of the same size would leave it as it was.
static ANNOUNCED: AtomicU64 = AtomicU64::new(0);

/// The bytes that the dropped reservations have given back, process-wide, modulo 2^64 as
/// [`ANNOUNCED`]: each [`Reservation`] adds here, once, exactly what its step added there.
static RELEASED: AtomicU64 = AtomicU64::new(0);

/// `reserved`: the bytes announced and not yet given back. The two counts are read one after
/// the other, so the figure is exact only when no reservation is made or dropped meanwhile; it
/// is for a debug trace and for tests. [`try_reserve`] reads the counts in its own order.
#[cfg(any(test, debug_assertions))]
fn reserved() -> u64 {
    let announced = ANNOUNCED.load(Ordering::SeqCst);
    announced.wrapping_sub(RELEASED.load(Ordering::SeqCst))
}

/// The bytes of one step, counted in `reserved` until the value is dropped: after the step
/// allocated, or while a panic unwinds. Between the allocation and the drop, the bytes are counted
/// twice, in `held()` too, which only makes a concurrent step stricter. Only an accepted step has
/// a reservation, so the drop gives back exactly the bytes that [`try_reserve`] added.
#[must_use = "a reservation counts until it is dropped, after the allocation it announces"]
pub(crate) struct Reservation(u64);

impl Drop for Reservation {
    fn drop(&mut self) {
        RELEASED.fetch_add(self.0, Ordering::SeqCst);
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
    /// The passes of the loop of [`try_reserve`] this thread made, in tests: one per step when
    /// no other thread's step is accepted meanwhile, however many reservations are alive.
    static PASSES: Cell<usize> = const { Cell::new(0) };
    /// What this thread does once inside a pass of [`try_reserve`], in tests: `Some((n, act))`
    /// runs `act` after the pass's n-th read (1 the bytes announced, 2 the bytes given back, 3
    /// the bytes held, in the order the pass makes them), as another thread's step that falls
    /// there would.
    static BETWEEN_READS: Cell<Option<ReadAction>> = const { Cell::new(None) };
}

/// A read of a pass of [`try_reserve`], counted from 1, and what a test does after it.
#[cfg(test)]
type ReadAction = (usize, fn());

/// Runs the action a test set for after the `read`-th read of a pass, once: the action is taken
/// off first, so neither a step it makes nor a repeated pass runs it again.
#[cfg(test)]
fn between_reads(read: usize) {
    if let Some((at, act)) = BETWEEN_READS.with(Cell::get)
        && at == read
    {
        BETWEEN_READS.with(|cell| cell.set(None));
        act();
    }
}

/// Checks a step of `bytes` and, when it fits, announces it: the reservation, when
/// `held() + reserved + bytes` stays within the run's ceiling, or `None` when it would not.
/// A refused step is never announced (#1943): it adds nothing to [`ANNOUNCED`], so no other
/// thread's check ever sees its bytes, and there is nothing to give back. On a thread outside a
/// run no ceiling applies, and the step is announced whenever the sum below does not overflow.
///
/// The order, check then announce, is that of ADR 0113's amendment of 2026-11-19; the text of
/// rule 4 before it says announce, then check. What is compared, the guard, the double count
/// and "it never sees fewer" are those of rule 4.
///
/// No sum wraps. A step larger than any allocation can be (`isize::MAX` bytes) is refused at
/// once, a step that would take `reserved` past `usize::MAX` is refused, and the sum with
/// `held()` saturates, as the `+sat` of ADR 0113. So `reserved`, the difference of two counts
/// that each wrap, is always the true sum of the reservations alive, at most `usize::MAX`.
/// The second of these refusals also applies outside a run, where the earlier code accepted
/// every step: it fails closed, and the caller reports U6003. It takes 2^64 bytes of
/// reservations alive at once on a 64-bit target, 4 GiB on a 32-bit one.
///
/// O(1) expected and lock-free, not wait-free: one pass is three atomic reads, [`charged`] and
/// one compare-exchange, and a pass is repeated only when another thread's step of positive
/// size was accepted meanwhile. So some step always completes, and the compare-exchange never
/// fails without one, but no bound holds for one step alone: a step beside siblings that keep
/// having steps accepted can repeat its pass any number of times.
///
/// # Orderings and interleavings
///
/// A step's bytes move one way: into `ANNOUNCED` (accepted), into `held()` (allocated), into
/// `RELEASED` (dropped). One pass reads against that flow, the source first: `ANNOUNCED` at
/// t1, `RELEASED` at t2, `held()` at t3, and its compare-exchange at t4 succeeds only if
/// `ANNOUNCED` still has the value read at t1. The accesses of the two counts are `SeqCst`, so
/// they are in one order for all threads. The allocator's count is not part of that order (its
/// writes are `Acquire` and `Release`, and [`charged`] reads the stack charge `Relaxed`), and
/// need not be: a step's allocation happens before its own `RELEASED.fetch_add`, on one thread
/// or across the fork handshake of a task's start, and a pass that reads that value of
/// `RELEASED` at t2 is ordered after it, so its read of `held()` at t3 cannot return a value
/// older than that allocation. For a pass that is accepted, and any other step X:
///
/// - X accepted before t1 and not released before t2 is counted in `reserved`. If it has
///   allocated before t3 it is counted in `held()` too: the double count that ADR 0113 rule 4
///   states, which refuses earlier and never later.
/// - X released before t2 had allocated before that, if it allocated at all, so `held()` at t3
///   counts its bytes, unless they were freed since, and then they are not held.
/// - X accepted between t1 and t4 changes `ANNOUNCED`, so the compare-exchange fails and the
///   pass is repeated: no step is accepted on a view that misses an accepted step. This is why
///   the count never falls; a release between t1 and t4 does not fail the pass, and need not,
///   since it only removes bytes that the pass counted.
/// - X accepted after t4 reads an `ANNOUNCED` that holds this step's bytes, and its own pass
///   counts them by the first two cases.
///
/// So of two concurrent steps the later accepted one counts the earlier, in `reserved` or in
/// `held()`: steps cannot together take `held()` past the ceiling (ADR 0113 rule 4), as with the
/// earlier add-then-check, and a pass sees no fewer bytes than are announced or held. A refusal
/// is decided on such a view too: when the sum does not fit, `ANNOUNCED` is read again, and a
/// changed value repeats the pass, since `RELEASED` at t2 may then hold the release of a step
/// accepted after t1 and make the difference meaningless.
///
/// What remains inexact, all within what ADR 0113 states of the budget:
///
/// - Stricter: the double count above. With k steps in flight a pass can count twice the
///   bytes of up to k - 1 of them, and of every reservation their threads hold at once, as the
///   earlier add-then-check could.
/// - Neither: the allocations that are not steps. `held()` counts them when they happen,
///   before or after t3, as it did for the earlier add-then-check; ADR 0113 rule 6 covers them
///   by the checks of `spec/11 §7` and the allocator's hard ceiling, not by the steps.
/// - Looser in one case only: `ANNOUNCED` could show at t4 the value read at t1 after other
///   threads had steps accepted in between whose sizes sum to a positive multiple of 2^64
///   bytes exactly, and the pass would miss those steps. Each of them was accepted within its
///   own ceiling, so this takes 2^64 bytes of steps within one pass of another thread, which
///   no run reaches; the counts are 64 bits wide on every target for this reason. The
///   allocator's hard ceiling (ADR 0061 rule 3) stays under every step either way.
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
    let step = u64::try_from(bytes).ok()?;
    let ceiling = HOT.with(|hot| hot.active.get().then(|| hot.mem_ceiling.get()));
    loop {
        #[cfg(test)]
        PASSES.with(|passes| passes.set(passes.get() + 1));
        let announced = ANNOUNCED.load(Ordering::SeqCst);
        #[cfg(test)]
        between_reads(1);
        let released = RELEASED.load(Ordering::SeqCst);
        #[cfg(test)]
        between_reads(2);
        let held = charged();
        #[cfg(test)]
        between_reads(3);
        let fits = announced
            .wrapping_sub(released)
            .checked_add(step)
            .and_then(|reserved| usize::try_from(reserved).ok())
            .is_some_and(|reserved| {
                ceiling.is_none_or(|ceiling| held.saturating_add(reserved) <= ceiling)
            });
        if !fits {
            if ANNOUNCED.load(Ordering::SeqCst) == announced {
                return None;
            }
            continue;
        }
        if ANNOUNCED
            .compare_exchange(
                announced,
                announced.wrapping_add(step),
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
        {
            #[cfg(test)]
            LIVE_RESERVATIONS.with(|live| live.set(live.get() + 1));
            return Some(Reservation(step));
        }
    }
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
/// every other panic prints as before (ADR 0113 rule 5). The serve engine's hook, which keeps
/// the panics it contains quiet (`spec/31 §6` rule 13), is chained over it here, in the same
/// place, before any thread of the run starts: two installs that ran at once on two threads
/// could lose one of the hooks.
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
    // The serve engine's, chained over it, only in a program that serves ([`enable_serve`]).
    if let Some(hooks) = SERVE.get() {
        (hooks.hook)();
    }
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

/// How a run builds the client's TLS settings before `main`, set only by a program that
/// performs an operation of `Http` ([`enable_http`]): the only reference to the TLS stack from
/// the runtime's start, so an executable that never uses `Http` carries none of it (ADR 0074
/// H7), while one that does builds them up front (ADR 0116 amendment, P8, #1148).
static HTTP_PREPARE: OnceLock<fn()> = OnceLock::new();

/// Lets the runs of this process build the client's TLS settings before `main` when their
/// grants include `Http` (`una_rt_http_enable`).
pub(crate) fn enable_http() {
    let _ = HTTP_PREPARE.set(una_http::client::prepare);
}

/// What the runtime does with the serve engine, set only by a program that performs an
/// operation of `HttpServer` ([`enable_serve`]): the only references from the runtime to the
/// serve threads, their start, stop and join, and the engine's panic hook, so an executable that
/// serves nothing links none of them (ADR 0074 H7; the test
/// `an_executable_that_serves_nothing_carries_no_serve_engine` in `crates/una-cli/tests/http.rs`).
/// It still links what the run's end calls directly in case the run serves: the table of reply
/// rights and the answer of an exchange (`crate::serve::answer_held`, `answer_all_left`).
struct ServeHooks {
    /// Starts the serve threads before `main` ([`crate::serve::install_live`]).
    start: fn(
        una_http::serve::Prepared,
        &[una_core::http::serve::ServeGrant],
        usize,
        &RunStop,
    ) -> Result<(), String>,
    /// Ends them at the run's end ([`crate::serve::end_engine`]).
    end: fn(std::time::Duration, bool),
    /// Installs the serve engine's panic hook over the runtime's (`spec/31 §6` rule 13).
    hook: fn(),
}
static SERVE: OnceLock<ServeHooks> = OnceLock::new();

/// Lets the runs of this process start their serve threads before `main` when their grants
/// include `serve:` (`una_rt_serve_enable`).
pub(crate) fn enable_serve() {
    let _ = SERVE.set(ServeHooks {
        start: crate::serve::install_live,
        end: crate::serve::end_engine,
        hook: una_http::serve::install_panic_hook,
    });
}

/// Ends the serve engine of a run that serves ([`ServeHooks::end`]); nothing in any other.
fn end_serving(drain: std::time::Duration, hard: bool) {
    if let Some(hooks) = SERVE.get() {
        (hooks.end)(drain, hard);
    }
}

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
    /// Whether the code the program can reach makes a hash table, so the run reads the process
    /// key of keyed hashing before `main` (ADR 0119 rule 9).
    pub hashing: bool,
    /// The host-driven step, when the program is driven (ADR 0116 rule 3): its tick period and
    /// step budget, in milliseconds.
    pub drive: Option<DriveMeta>,
    /// How U3012 names the grants of the manifest: `` `[run] grants` `` in `una.toml`, and
    /// `` `run grants` `` in `ita.una`.
    pub grants_section: String,
    /// Whether the program is of edition 0.4, whose runs may take `serve:` grants
    /// (`spec/31 §2` rule 4): the description's `serve_grants`.
    pub serve: bool,
    /// The module of the `serve` section whose driver the program runs (ADR 0123 rule 16):
    /// the description's `serve`, `{"module": ..., "within": null}`, whose serve threads start
    /// before the driver (S4 P8).
    pub serve_section: Option<String>,
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
    /// Whether the program holds a task construct, a `within` or a `for ever`: the run then
    /// starts every worker, both helper pools at full size and the timer thread of `within`
    /// before `main`, so that no thread starts at a moment that earlier code decided (ADR 0116
    /// amendment, P8; `spec/27 §1` rule 4; #1145, #1151).
    pub threads: bool,
    /// The package's declared memory budget in MiB, `[run] max_memory` or the `run` line
    /// `max_memory`, from 1 to 4096, which `--max-memory` may lower and never raise (ADR 0109
    /// rule 4).
    pub max_memory: Option<u64>,
    /// Whether the program is of edition 0.4, whose self-test adds the binary32 probes (ADR
    /// 0120 rule 5).
    pub float32_probes: bool,
    /// The program's frame bound in bytes (`spec/27 §4` rule 1, #1152): the most stack one
    /// level of call depth takes in its compiled code, from which task stacks are sized
    /// ([`task_stack_bytes`]). `None` when the description gives none: the size is then the one
    /// `--max-depth` alone gives.
    pub frame_bound: Option<usize>,
}

/// The largest frame bound a description may give (`spec/27 §4` rule 1): the guard region
/// below each task stack, so that no frame steps over it.
const MAX_FRAME_BOUND: usize = 64 * 1024;

/// The bytes of a task stack beyond its levels of depth (`spec/27 §4` rule 1): the
/// [`STACK_RESERVE`] kept free below the budget for the runtime's work after the last check, a
/// signal handler's frame and the frames at the task's base.
const TASK_STACK_BASE: usize = 16 * 1024 * 1024;

/// The usable bytes of the stack of a task that may still use `depth` levels of call depth, in
/// a program whose frame bound is `frame_bound` (`spec/27 §4` rule 1, ADR 0117 amendment of
/// 2026-11-09, #1152): `16 MiB + depth × frame_bound`, and never more than the size
/// `--max-depth` alone gives, [`una_interp::stack_bytes`], which a program without a bound
/// keeps. Both inputs are public, fixed before `main` and the same for every input, so the
/// size never depends on `secret` data and every pooled stack has one size (#1145). A bound
/// that were too small stops a task with U6003 at a call ([`enter`]), never past its stack.
pub(crate) fn task_stack_bytes(depth: usize, frame_bound: Option<usize>) -> usize {
    let alone = una_interp::stack_bytes(depth).unwrap_or(STACK_RESERVE * 2);
    match frame_bound {
        Some(frame) => depth
            .saturating_mul(frame)
            .saturating_add(TASK_STACK_BASE)
            .min(alone),
        None => alone,
    }
}

/// The largest declared memory budget, in MiB, while edition `0.3` is a preview: the default
/// of `--max-memory`, so a declared budget never widens a run (ADR 0109 rule 4).
pub const MAX_DECLARED_MEMORY: u64 = 4096;

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
                driven: match failure.get("driven") {
                    Some(module) => Some(module.as_str()?.to_owned()),
                    None => None,
                },
            }),
            None => None,
        };
        let float_environment = value
            .get("float_environment")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let hashing = value
            .get("hashing")
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
        let serve = value
            .get("serve_grants")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // The driver applies no `within` (`spec/31 §7` rule 3 item 7 refuses every one today):
        // a description with one is no description, and the executable refuses to start.
        let serve_section = match value.get("serve") {
            None => None,
            Some(section) => {
                let module = section.get("module")?.as_str()?.to_owned();
                if !section.get("within")?.is_null() {
                    return None;
                }
                Some(module)
            }
        };
        let float32_probes = value
            .get("float32_probes")
            .and_then(Value::as_bool)
            .unwrap_or(false);
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
        // A budget outside 1 to 4096 MiB is no description at all, so the executable refuses
        // to start rather than run with a budget the manifest could not declare (ADR 0109
        // rule 4).
        let max_memory = match value.get("max_memory") {
            Some(max_memory) => Some(
                max_memory
                    .as_u64()
                    .filter(|mib| (1..=MAX_DECLARED_MEMORY).contains(mib))?,
            ),
            None => None,
        };
        // A frame bound that no code generator writes, 0, not a multiple of 16 or over the guard
        // region, is no description at all, so the executable refuses to start rather than size
        // its stacks from it (`spec/27 §4` rule 1).
        let frame_bound = match value.get("frame_bound") {
            Some(bound) => Some(
                usize::try_from(bound.as_u64()?)
                    .ok()
                    .filter(|bytes| (16..=MAX_FRAME_BOUND).contains(bytes) && bytes % 16 == 0)?,
            ),
            None => None,
        };
        Some(Meta {
            frame_bound,
            stops,
            tasks,
            elision,
            limit_label,
            limits_allowed,
            threads,
            max_memory,
            grants,
            uses,
            sites,
            types,
            ops,
            failure,
            float_environment,
            hashing,
            drive,
            grants_section,
            serve,
            serve_section,
            float32_probes,
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
    /// `--max-memory <MiB>` (ADR 0061), or the budget [`with_budget`] sets.
    pub max_memory: usize,
    /// Whether the command line gave `--max-memory` (ADR 0109 rule 4).
    pub max_memory_given: bool,
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
    /// `--serve-threads` (ADR 0123 rule 20), from 1 to 256: the serve threads of a run that
    /// serves; 0 until the command line is read.
    pub serve_threads: usize,
    /// `--serve-script <file>`: the connections that replace the listeners of the `serve:`
    /// grants (ADR 0123 rule 21, `spec/31 §8`).
    pub serve_script: Option<PathBuf>,
    /// `--serve-out <file>`: where what the scripted connections received is written.
    pub serve_out: Option<PathBuf>,
}

/// The options with the package's declared budget applied (ADR 0109 rule 4): without
/// `--max-memory` the run gets the declared budget, and the flag may lower it, never raise it.
/// A description with no budget leaves the options as they are. `Err` is the usage error. O(1).
pub fn with_budget(mut options: Options, meta: &Meta) -> Result<Options, String> {
    let Some(declared) = meta.max_memory else {
        return Ok(options);
    };
    let declared_mib = usize::try_from(declared).unwrap_or(usize::MAX);
    if !options.max_memory_given {
        options.max_memory = declared_mib;
    } else if options.max_memory > declared_mib {
        return Err(format!(
            "--max-memory {} is above the package's declared max_memory of {declared} MiB; it may lower the budget, never raise it",
            options.max_memory
        ));
    }
    Ok(options)
}

/// The command line, without the program name, or the message of a usage error.
pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        max_depth: una_interp::DEFAULT_MAX_DEPTH,
        max_memory: una_interp::DEFAULT_MAX_MEMORY,
        ..Options::default()
    };
    // The flags of tasks, checked once every flag is read: `--task-depth` against `--max-depth`.
    let mut tasks: [Option<usize>; 5] = [None; 5];
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
            "--serve-script" => options.serve_script = Some(PathBuf::from(value()?)),
            "--serve-out" => options.serve_out = Some(PathBuf::from(value()?)),
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
                options.max_memory_given = true;
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
            "--workers" | "--task-depth" | "--io-threads" | "--file-threads"
            | "--serve-threads" => {
                let n = una_interp::concurrency::number(flag, &value()?)?;
                let slot = match flag {
                    "--workers" => 0,
                    "--task-depth" => 1,
                    "--io-threads" => 2,
                    "--file-threads" => 3,
                    _ => 4,
                };
                if let Some(cell) = tasks.get_mut(slot) {
                    *cell = Some(n);
                }
            }
            other => {
                return Err(format!(
                    "unexpected argument `{other}`; the options are --grant <spec>, --trace <file>, --max-depth <n>, --max-memory <MiB>, --max-response <MiB>, --http-timeout <seconds>, --stop-after-iterations <n>, --deadline <seconds>, --stop-grace <seconds>, --virtual-time, --workers <n>, --task-depth <n>, --io-threads <n>, --file-threads <n>, --serve-threads <n>, and, in toolchain test builds only, --serve-script <file> and --serve-out <file>"
                ));
            }
        }
    }
    // ADR 0123 rule 21: the flags of a script need one another, checked first, as `una run`
    // checks them before any other value.
    una_http::serve::script_file::check_flags(
        options.serve_script.is_some(),
        options.serve_out.is_some(),
        options.virtual_time,
    )?;
    let [workers, task_depth, io_threads, file_threads, serve_threads] = tasks;
    options.concurrency = una_interp::Concurrency::from_flags(
        options.max_depth,
        workers,
        task_depth,
        io_threads,
        file_threads,
    )?;
    options.serve_threads = una_interp::concurrency::serve_threads(serve_threads)?;
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
    /// for the paths of their tasks (`spec/27 §7`): outside `quiet` code, then inside it, which
    /// counts apart so that quiet code never moves the paths of later public statements (S-39,
    /// `spec/26 §6` rule 4).
    pub(crate) scopes: Vec<Arc<crate::started::StartScope>>,
    pub(crate) tasks_runs: una_interp::concurrency::BlockCounts,
    /// What the run's tasks share: the trace's sink, the installations counted run-wide, the
    /// started tasks alive.
    pub(crate) shared: Arc<crate::started::RunShared>,
    pub(crate) failed_write: Option<io::Error>,
    /// The maximum call depth. The current depth is `Hot::depth`, its one copy (ADR 0102).
    max_depth: usize,
    /// The program's frame bound, which sizes its tasks' stacks ([`task_stack_bytes`]).
    frame_bound: Option<usize>,
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
    /// §6`), outside `quiet` code and inside it, as `tasks_runs`.
    pub(crate) races: una_interp::concurrency::BlockCounts,
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
    /// How many `quiet` expressions the task is in, those of the task that started it at its
    /// `start` included: while one is, a `for ever` that a host stop ends appends no `stop`
    /// event (S-39, `spec/26 §6` rule 4).
    pub(crate) quiet: u32,
    /// How many of those are not quiet by a timing floor alone: while one is, an operation
    /// appends no event (S-42, #1515, `spec/26 §6` rule 4).
    pub(crate) hushed: u32,
    /// The settings of the run's tasks (ADR 0117 rules 2 and 3).
    concurrency: una_interp::Concurrency,
    /// The run's scheduler, made at its first block: none in a run without tasks.
    sched: Option<Arc<crate::sched::Scheduler>>,
    /// The task as the run's reply rights know it, its holder number (ADR 0123 rule 4): 0 for
    /// the root, a fresh one for each started task, and its block's for a task of `together`
    /// or `first of` ([`crate::serve`]).
    pub(crate) serve_task: u64,
    /// Whether the last host operation stopped its task in progress, its event left `aborted`:
    /// a `next` of `HttpServer` once the run aborted ([`crate::serve`]).
    pub(crate) op_stopped: bool,
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
    frame_bound: Option<usize>,
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
    /// For the branches of a `first of`, whether it stands in `quiet` code, and the count of the
    /// block among those its public code ran: their paths are `p.f<c>.<i>`, or `p.qf.<i>` in
    /// quiet code, which counts none (`spec/27 §6`, S-39, S-40).
    pub(crate) race: Option<(bool, usize)>,
    /// What the run's tasks share, and whether the task writes its events as they happen: a
    /// started task does (ADR 0117 rule 10).
    pub(crate) shared: Arc<crate::started::RunShared>,
    pub(crate) streaming: bool,
    /// The `quiet` count of the task that runs the block or starts the task (S-39): a task
    /// runs under the program counter of its `start` (`spec/27 §7` rule 3).
    quiet: u32,
    /// Its `hushed` count (S-42).
    pub(crate) hushed: u32,
    /// The holder number of the task's reply rights: the block's task's, and a fresh one for a
    /// started task, set by `start` ([`crate::serve`]).
    pub(crate) serve_task: u64,
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
        own + self
            .race
            .map_or(0, |(quiet, race)| if quiet { 3 } else { 2 + digits(race) })
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
            frame_bound: self.frame_bound,
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
            stack: task_stack_bytes(left, self.frame_bound),
            location: site_location(self, site),
            deadline: crate::deadline::current(),
            race: None,
            shared: Arc::clone(&self.shared),
            streaming: false,
            quiet: self.quiet,
            hushed: self.hushed,
            serve_task: self.serve_task,
        }
    }

    /// What the branches of a `first of` at `site` start from (`spec/27 §6`): the template of
    /// [`State::for_tasks`], counted as the next race of the task, with the depth limit of
    /// `--task-depth` below the block, within the run's, and a stack sized from it (ADR 0117
    /// rule 2).
    pub(crate) fn for_race(&mut self, site: usize) -> TaskTemplate {
        let mut template = self.for_tasks(site);
        // Quiet code counts no races (S-39, S-40, `spec/26 §6` rule 4).
        let quiet = self.quiet > 0;
        template.race = Some((quiet, self.races.next(quiet)));
        let left = template
            .max_depth
            .saturating_sub(template.depth)
            .min(template.concurrency.task_depth);
        template.max_depth = template.depth.saturating_add(left);
        template.stack = task_stack_bytes(left, template.frame_bound);
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
        template.stack = task_stack_bytes(left, template.frame_bound);
        template
    }

    /// Opens a `tasks` statement in this task, at most `bound` tasks at once, or the default
    /// bound: `max(64, M)` for the budget `M` in MiB, within the system's cap (ADR 0117 rule 7).
    pub(crate) fn open_tasks(&mut self, bound: Option<usize>) {
        // Quiet code does not count its statements, `qs`, so that whether it ran one, which may
        // depend on data above `public`, moves the path of no later statement (S-39, S-40,
        // `spec/26 §6` rule 4).
        let quiet = self.quiet > 0;
        let name = una_interp::concurrency::block_name('s', quiet, self.tasks_runs.next(quiet));
        let path = if self.path.is_empty() {
            name
        } else {
            format!("{}.{name}", self.path)
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
        // A task started inside a `quiet` expression ends without its `cancelled` event (S-39,
        // `spec/26 §6` rule 4); stack exhaustion stays outside the model (`spec/07 §1`).
        if outcome == EventOutcome::Cancelled && self.quiet > 0 {
            self.flush();
            return;
        }
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

    /// Appends the event of a loop that stops at the start of an iteration (ADR 0116 rule 14);
    /// none inside a `quiet` expression, whose program counter the event would reveal (S-39,
    /// `spec/26 §6` rule 4). It then takes no `seq` either.
    pub(crate) fn stop_event(&mut self) {
        if self.quiet > 0 {
            return;
        }
        // The `task` field of its task (`spec/26 §8`), as the interpreter writes it.
        let task = self.path.clone();
        if let Some(trace) = self.trace.as_mut() {
            trace.push(Event {
                seq: trace.len() + 1,
                task,
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
    /// (`spec/19 §7` rule 3); none inside a `quiet` expression of the task that runs its block
    /// (S-39, `spec/26 §6` rule 4).
    pub(crate) fn cancelled_event(&mut self, task: String) {
        if self.quiet > 0 {
            return;
        }
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

/// `--stop-after-iterations`, 0 when it is not given: the budget of each loop instance, a
/// `for ever` entered or the host's loop, which counts its own iterations (rule 19, #1472).
static STOP_AFTER: AtomicU64 = AtomicU64::new(0);

/// The hard stop, once it has come (rule 16).
fn hard_stop() -> Option<HardStop> {
    RUN_STOP.get().and_then(|stop| stop.hard())
}

/// Whether the loop of the host, or a `for ever`, stops at the iteration that starts now
/// (rules 13, 14 and 19, `spec/26 §6`): a host stop request has reached the run, which is
/// sticky; or this loop instance, which has started `started` iterations, has used up the
/// budget of `--stop-after-iterations`. The budget is per loop instance (#1472, S-36): it
/// stops this instance only, makes no host stop request, interrupts no operation and stays
/// with no later loop, so a loop under a secret branch cannot move where a later public loop
/// stops. The caller counts the iteration as started when this is false.
fn iteration_stops(started: u64) -> bool {
    let Some(stop) = RUN_STOP.get() else {
        return false;
    };
    if stop.stopped() {
        return true;
    }
    budget_spent(started, STOP_AFTER.load(Ordering::SeqCst))
}

/// Whether a loop instance that has started `started` iterations has used up a budget of
/// `limit` (0: no budget), so that it stops at the start of iteration `limit + 1`.
fn budget_spent(started: u64, limit: u64) -> bool {
    limit != 0 && started >= limit
}

/// Marks whether the thread's task runs the body of a `for ever` ([`State::in_ever`]).
fn set_in_ever(inside: bool) {
    with_state(|state| state.in_ever = inside);
}

/// The interruption of an `Http` operation of the thread's task, when it runs in the body of
/// a `for ever` (rule 12).
pub(crate) fn run_stop() -> Option<&'static RunStop> {
    RUN_STOP.get().map(AsRef::as_ref)
}

/// The run's host stop state, inside the body of a `for ever` only, where it interrupts an
/// operation that waits (ADR 0116 rule 12).
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
    /// The tick period, in milliseconds.
    ticks: u64,
    /// When the first tick was due, in nanoseconds of the run's clock (P13, #1151): a reading
    /// in whole milliseconds would carry the phase within a millisecond that `init` left into
    /// every tick.
    origin: Option<u128>,
    next: u64,
    /// The iterations the host's loop has started: its own budget (rule 19, #1472).
    started: u64,
}

/// The nanoseconds of a millisecond.
const NANOS_PER_MILLI: u128 = 1_000_000;

/// When tick `next` of a loop whose first tick was due at `origin` is due, with a period of
/// `period` milliseconds: in nanoseconds of the run's clock, exactly `next × period`
/// milliseconds after `origin` (`spec/26 §2` rule 3; P13, #1151).
fn tick_due(origin: u128, next: u64, period: u64) -> u128 {
    origin.saturating_add((u128::from(next) * u128::from(period)).saturating_mul(NANOS_PER_MILLI))
}

/// The tick the driver delivers when the clock reads `now`, in nanoseconds, for a loop whose
/// first tick was due at `origin`, whose next tick is `next` and whose period is `period`
/// milliseconds, at least 1: the latest tick due, never one before `next`, and how many due
/// ticks it skipped. O(1).
fn tick_at(origin: u128, now: u128, next: u64, period: u64) -> (u64, u64) {
    let period = u128::from(period.max(1)) * NANOS_PER_MILLI;
    let elapsed = now.saturating_sub(origin);
    let latest = u64::try_from(elapsed / period)
        .unwrap_or(u64::MAX)
        .max(next);
    (latest, latest - next)
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

/// The timers of the run's scheduler, which a `within` arms on the real clock (`timer`,
/// #1151). An executable whose program has a `within` made the scheduler and started the timer
/// thread before `main` (P8), so this is a borrow and a reference count. Code without that
/// start, as unit tests, makes the scheduler here, and its timer thread starts at the first
/// deadline. `None` on a thread without a run's state.
pub(crate) fn timers() -> Option<Arc<crate::timer::Timers>> {
    with_state(|state| {
        let concurrency = state.concurrency;
        let sched = state
            .sched
            .get_or_insert_with(|| crate::sched::Scheduler::new(concurrency));
        Arc::clone(sched.timers())
    })
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
        if iteration_stops(driver.started) {
            return None;
        }
        driver.started = driver.started.saturating_add(1);
        // On the run's clock, real or virtual, in nanoseconds (P13, #1151): under virtual time
        // the wait moves the clock to the tick at once (ADR 0116 rule 19), in whole
        // milliseconds. On the real clock the wait ends at the tick's exact instant, to the
        // resolution of the system's sleep: `wait_tick` polls only the whole milliseconds
        // left, since `poll` rounds a wait up to whole milliseconds, and sleeps the
        // sub-millisecond rest. A host stop interrupts the polls; during that rest, under 1 ms,
        // it is seen at its end.
        let clock = clock();
        let origin = *driver.origin.get_or_insert_with(|| clock.now_nanos());
        let due = tick_due(origin, driver.next, driver.ticks.max(1));
        if clock.is_virtual() {
            clock.wait_until(u64::try_from(due.div_ceil(NANOS_PER_MILLI)).unwrap_or(u64::MAX));
        } else {
            let now = clock.now_nanos();
            if now < due {
                let left = crate::deadline::duration(due - now);
                let waited = match RUN_STOP.get() {
                    Some(stop) => una_interp::stopping::wait_tick(stop, left),
                    None => {
                        while clock.now_nanos() < due {
                            std::thread::sleep(crate::deadline::duration(
                                due.saturating_sub(clock.now_nanos()),
                            ));
                        }
                        true
                    }
                };
                if !waited {
                    return None;
                }
            }
        }
        let (latest, missed) = tick_at(origin, clock.now_nanos(), driver.next, driver.ticks);
        let at = latest.saturating_mul(driver.ticks);
        driver.next = latest + 1;
        Some((at, missed))
    });
    match tick {
        Some((at, missed)) => {
            set_in_ever(true);
            let word = |n: u64| {
                values::int_word(&una_core::bigint::BigInt::from(
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
    ///
    /// Both are the worker thread's, not a task's: [`swap_slots`] does not swap them, and a
    /// task's install keeps `seen` (#1533). The slicer raises a worker's word at the end of the
    /// turn in progress, whichever task it is, so a raise pending when one task parks or ends
    /// belongs to the next one the worker runs. ADR 0117 rule 2, Pinning, lists a "copy of the
    /// attention word" among a task's slots; the code keeps the copy per worker (amendment
    /// draft `docs/plan/0117-seen-value-note-draft.md`).
    word: Cell<Option<una_sys::attention::Word>>,
    seen: Cell<u64>,
    /// Whether every check point of the thread counts toward its task's turn: on a worker,
    /// under virtual time (ADR 0117 rule 12). Not swapped: it is the worker's.
    counting: Cell<bool>,
    /// Whether the run's clock is virtual: then every check point of the thread, on a worker or
    /// not, reads it for the deadlines of its task's `within`s, which no timer sets
    /// (`deadline::virtual_check`, #1823). Set wherever `counting` is, from the same clock, so
    /// `counting` implies it. Not swapped: the clock is the run's.
    virtual_clock: Cell<bool>,
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
            virtual_clock: Cell::new(false),
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
        // One load on the real clock, as before: `counting` implies `virtual_clock`.
        if self.virtual_clock.get() {
            // Under virtual time no timer sets a `within`'s entry: the check point reads the
            // clock, which a wait of any task may have moved past a deadline, and sets the
            // entries of those it reached, which the cancellation check that follows reads, so
            // `e` is cut here, as in the interpreter (#1823). O(1) while none has passed.
            crate::deadline::virtual_check();
            if self.counting.get() {
                return hard_stop().is_some() | crate::sched::turn_due();
            }
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
        // `seen` is the worker thread's, never swapped with a task's slots (see `Hot::word`).
        // A thread that already has its word keeps the value its last check point saw: a raise
        // since then, as the slicer's at the end of a turn that began before this state was
        // installed, stays pending, so the first check point takes the slow path and the task
        // yields there (#1533). Reading the word anew here lost that raise: on a worker, a
        // turn's end is set when the worker takes the job, before the task installs its state,
        // so a slicer that raised the word in between, under load, was never heard, and the
        // task kept its worker while another waited, `main` with its cancellation included.
        let registered = hot.word.get();
        let word = registered.unwrap_or_else(una_sys::attention::register);
        hot.word.set(Some(word));
        // A new word as it stands, read before the hard stop is: a hard stop that comes after
        // the read raises the word, and one that came before it is seen here, with a value no
        // word holds, so the first check point takes the slow path.
        let seen = if registered.is_some() {
            hot.seen.get()
        } else {
            word.load()
        };
        hot.seen.set(if hard_stop().is_some() {
            u64::MAX
        } else {
            seen
        });
        hot.virtual_clock.set(clock().is_virtual());
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
        hot.virtual_clock.set(clock().is_virtual());
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

/// The bound of the `tasks` statement of the run's `serve` section driver: the sum of `/once`
/// over the run's `serve:` grants, within the default bound of a `tasks` statement, at least 1
/// ([`Grant::serve_driver_bound`], #1724). O(g).
pub(crate) fn serve_driver_bound() -> usize {
    with_state(|state| Grant::serve_driver_bound(&state.grants, state.max_memory, state.shared.cap))
        .unwrap_or(1)
}

/// `f` of the state of the run, when the thread runs `main`.
pub(crate) fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| cell.borrow_mut().as_mut().map(f))
}

/// Whether a `for ever` stops at the iteration that starts now (ADR 0116 rules 13, 14 and 19):
/// a host stop request has come, which is sticky, or the loop instance, which has started
/// `started` iterations by the compiled code's own count, has used up its budget; its `stop`
/// event is then traced. 1 to stop. The compiled code counts the iteration when this is 0.
pub fn ever_start(started: i64) -> i64 {
    // A count is never negative; one that wrapped past 2^63 - 1 is past every budget.
    let started = u64::try_from(started).unwrap_or(u64::MAX);
    if iteration_stops(started) {
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

/// Enters a `quiet` expression: a `for ever` loop, or a call of a function that says
/// `diverges`, under a program counter above `public` (S-39, `spec/26 §6` rule 4); with `hush`,
/// one not marked by a timing floor alone, whose operations append no event (S-42).
pub fn quiet_enter(hush: bool) {
    with_state(|state| {
        state.quiet = state.quiet.saturating_add(1);
        state.hushed = state.hushed.saturating_add(u32::from(hush));
    });
}

/// Leaves a `quiet` expression, on every way out of it: its value, or a failure or a stop of
/// the task that leaves it; `hush` as it entered.
pub fn quiet_leave(hush: bool) {
    with_state(|state| {
        state.quiet = state.quiet.saturating_sub(1);
        state.hushed = state.hushed.saturating_sub(u32::from(hush));
    });
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
    // The declared budget applies right after the command line is read (ADR 0109 rule 4).
    let mut options = match parse_args(args).and_then(|options| with_budget(options, &meta)) {
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
        && let Err(failed) = una_sys::signals::install()
    {
        let _ = writeln!(stderr, "error: {}", failed.message);
        // A soft limit of open files too small for the runtime is a usage error (#1757).
        return if failed.usage { USAGE } else { 3 };
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
    // `spec/31 §7` rule 1: the driver of a `serve` section needs a `serve:` grant, of the
    // description or of a `--grant` flag, before anything starts, as in `una run`.
    if let Some(module) = &meta.serve_section
        && !specs
            .iter()
            .any(|spec| spec.starts_with(una_core::http::serve::PREFIX))
    {
        let _ = writeln!(stderr, "error: {}", una_interp::unserved_section(module));
        return USAGE;
    }
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
    // The scripted engine of the runs of `tests/serve_runs/`, in the toolchain's own test builds
    // only, takes the listeners' place (ADR 0123 rule 21).
    #[cfg(feature = "test-hooks")]
    let scripted = match crate::serve::script::install_from_environment() {
        Ok(scripted) => scripted,
        Err(message) => {
            let _ = writeln!(stderr, "error: {message}");
            return USAGE;
        }
    };
    #[cfg(not(feature = "test-hooks"))]
    let scripted = false;
    // A script of connections stands for the peers of the `serve:` grants (ADR 0123 rule 21):
    // it is read and checked in full before `main`. Its connections run only in the
    // toolchain's own test builds; elsewhere they do not run yet (`spec/31 §8`), as for
    // `una run`.
    let scripted = match &options.serve_script {
        Some(script) => {
            let served: Vec<una_core::http::serve::ServeGrant> = grants
                .iter()
                .filter_map(|grant| match grant {
                    Grant::Serve { grant, .. } => Some((**grant).clone()),
                    _ => None,
                })
                .collect();
            let listeners: Vec<String> = served.iter().map(|g| g.listener()).collect();
            let events = match una_http::serve::script_file::load(script, &listeners) {
                Ok(events) => events,
                Err(message) => {
                    let _ = writeln!(stderr, "error: {message}");
                    return USAGE;
                }
            };
            // The test hook and the flag cannot both name a script.
            if scripted {
                let _ = writeln!(
                    stderr,
                    "error: `--serve-script` and `UNA_TEST_SERVE_SCRIPT` both name a script"
                );
                return USAGE;
            }
            #[cfg(feature = "test-hooks")]
            let runs = crate::serve::script::install_from_script(
                &served,
                &events,
                options.serve_out.clone(),
            );
            #[cfg(not(feature = "test-hooks"))]
            let runs = {
                let _ = (&served, &events);
                false
            };
            if !runs {
                let _ = writeln!(stderr, "error: {}", una_http::serve::script_file::PENDING);
                return USAGE;
            }
            true
        }
        None => scripted,
    };
    // Live peers send in real time, which virtual time cannot place: a run under
    // `--virtual-time` takes no `serve:` grant without `--serve-script` (ADR 0123, S4 amendment
    // C). The scripted engine of the tests stands for such a script.
    if options.virtual_time
        && !scripted
        && grants
            .iter()
            .any(|grant| matches!(grant, Grant::Serve { .. }))
    {
        let _ = writeln!(stderr, "error: {}", una_interp::VIRTUAL_SERVE);
        return USAGE;
    }
    // The trace file is made before `main` starts, and written as the run goes (ADR 0117 rule
    // 10): one that cannot be made is a usage error. It is made after the usage errors of the
    // grants, the key files and the script, as `una run` makes it, so a run those refuse
    // touches no file.
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
    let listeners = match serve(
        if scripted { &[] } else { &grants },
        options.serve_threads.max(1),
        options.concurrency.file_threads,
    ) {
        Ok(prepared) => prepared,
        Err(message) => {
            let _ = writeln!(stderr, "error: {message}");
            return USAGE;
        }
    };
    // A program that serves starts its serve threads here, before `main`, over the listeners
    // bound above; they accept nothing until its first server operation (S4 amendment B.2).
    // Any other run holds its listeners until it ends.
    let serves = meta.serve_section.is_some()
        || (meta.serve && meta.uses.iter().any(|need| need.capability == "HttpServer"));
    let live = SERVE.get().map(|hooks| hooks.start).filter(|_| serves);
    let _listeners = if let Some(install) = live
        && !scripted
        && !listeners.listeners.is_empty()
    {
        let served: Vec<una_core::http::serve::ServeGrant> = grants
            .iter()
            .filter_map(|grant| match grant {
                Grant::Serve { grant, .. } => Some((**grant).clone()),
                _ => None,
            })
            .collect();
        if let Err(message) = install(listeners, &served, options.serve_threads.max(1), &stop) {
            let _ = writeln!(stderr, "error: {message}");
            return USAGE;
        }
        // One line per address it serves, before `main`, as `una run` writes (`spec/31 §6` rule
        // 14): the grants before `meta.grants.len()` are the executable's own.
        for (index, grant) in grants.iter().enumerate() {
            if let Grant::Serve { grant, .. } = grant {
                let _ = writeln!(
                    stderr,
                    "{}",
                    una_http::serve::serving_notice(grant, index < meta.grants.len())
                );
            }
        }
        None
    } else {
        Some(listeners)
    };
    let drive = meta.drive.map(|drive| Driver {
        ticks: drive.ticks,
        origin: None,
        next: 0,
        started: 0,
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
        tasks_runs: una_interp::concurrency::BlockCounts::default(),
        shared,
        failed_write: None,
        max_depth: options.max_depth,
        frame_bound: meta.frame_bound,
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
        races: una_interp::concurrency::BlockCounts::default(),
        installs: BTreeMap::new(),
        stack_base: 0,
        stack_budget: stack.saturating_sub(STACK_RESERVE),
        max_memory: options.max_memory,
        baseline: 0,
        path: String::new(),
        // A run that serves carries its own cancellation entry, which every task inherits and
        // an abort of the run lowers (`crate::serve`).
        cancel: crate::serve::run_entry().into_iter().collect(),
        shielded: 0,
        operations: Vec::new(),
        stopped: None,
        in_ever: false,
        quiet: 0,
        hushed: 0,
        concurrency: options.concurrency,
        sched: None,
        serve_task: 0,
        op_stopped: false,
    };
    // ADR 0116 amendment, P8 (#1148): a run whose grants include `Http` builds the client's TLS
    // settings here, before `main`, with the other state it starts up front. Built at the run's
    // first `https` request, they would be state that earlier code left, saving a later request
    // the time of building them. A run without such a grant never builds them, nor does a
    // program that performs no operation of `Http`, which cannot make a request. The same run
    // starts the client's pools here, `max(2, --io-threads / 4)` lookup threads and
    // `--io-threads` connect threads, all at once, so whether such a thread exists is never
    // history either; a thread that cannot start stops the run before `main` with U6003, as the
    // scheduler's do (`spec/24 §4` rule 2).
    if state
        .grants
        .iter()
        .any(|grant| matches!(grant, Grant::Http(_)))
    {
        if let Some(prepare) = HTTP_PREPARE.get() {
            prepare();
        }
        if una_http::resolve::start(state.concurrency.io_threads).is_err() {
            let written = writeln!(
                stderr,
                "[{}] Error: the lookup and connect threads of this run's requests could not be made",
                Code::U6003
            );
            return if written.is_ok() { 1 } else { USAGE };
        }
    }
    // ADR 0114 rule 6.4: write the default floating-point environment, then test it, before
    // any code of the program runs; each thread the program runs on writes it too.
    let floats = meta.float_environment;
    if floats {
        crate::ffi::write_float_environment();
        // A failed probe is U6006, exit code 1, with no position, written as the runtime's
        // other stops before `main` are.
        // Edition 0.4 adds the binary32 probes (ADR 0120 rule 5).
        let probed = una_core::float::probe_environment().and_then(|()| {
            if meta.float32_probes {
                una_core::float32::probe_environment()
            } else {
                Ok(())
            }
        });
        if let Err(probe) = probed {
            let written = writeln!(
                stderr,
                "[{}] Error: the floating-point environment of this platform is not the default one ({probe})",
                Code::U6006
            );
            return if written.is_ok() { 1 } else { USAGE };
        }
        FLOATS.store(true, Ordering::SeqCst);
    }
    // ADR 0119 rule 9: a program whose code can make a hash table reads the process key from the
    // host's random source before `main`, so before any effect, and a blocking read never stalls
    // a running program. When the host gives no random bytes, the run ends with U6009, exit code
    // 1, with no position, as the runtime's other stops before `main`.
    if meta.hashing && !values::read_process_key() {
        let written = writeln!(
            stderr,
            "[{}] Error: the host gave no random bytes for keyed hashing",
            Code::U6009
        );
        return if written.is_ok() { 1 } else { USAGE };
    }
    // ADR 0116 amendment, P8 (#1145, #1151): a program with a task construct, a `within` or a
    // `for ever` starts its scheduler, every worker, both helper pools at full size and the
    // timer thread of `within`, and fills the stack pool
    // with stacks of the size a block at the top asks for, the largest any task asks for, here,
    // before `main`. Whether a thread exists, or how many stacks are pooled, is then never
    // state that earlier code left, and no construct pays a thread's start or a stack's
    // mapping because of it. A thread that cannot start, or a stack that cannot be mapped,
    // stops the run here, with U6003 and no position, rather than leave a lazy start for
    // later. A program without these constructs starts no thread (`spec/27 §1` rule 4).
    // A `tasks` statement is such a construct; its root runs on this scheduler too, below.
    if threads || meta.tasks {
        let sched = crate::sched::Scheduler::new(state.concurrency);
        // `spec/27 §4` rule 1 (#1152): the pool's stacks have the size a task at the top asks
        // for, from the program's frame bound and `--max-depth`, both public.
        let pooled = task_stack_bytes(state.max_depth, state.frame_bound);
        if sched.start_all(pooled).is_err() {
            let written = writeln!(
                stderr,
                "[{}] Error: the threads and stacks of this run's tasks could not be made",
                Code::U6003
            );
            return if written.is_ok() { 1 } else { USAGE };
        }
        state.sched = Some(sched);
        // The reactor too, for a run whose grants include `Http` or `serve:` under the real
        // clock (ADR 0116 amendment, P8; ADR 0117 slice S7): started here, never at a task's
        // first wait. A task's `next` parks on it, so that a wait for a request holds no helper
        // (#1723). Under virtual time no task parks on it (`reactor::parks`).
        if (!http_grants(&state.grants).is_empty()
            || state
                .grants
                .iter()
                .any(|grant| matches!(grant, Grant::Serve { .. })))
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
        // The budget's baseline is measured as the budget charges (`heap_charged`), and the
        // hard ceiling's on the bytes really held.
        CREDIT_LIMIT.store(budget / 4, Ordering::Relaxed);
        state.baseline = heap_charged();
        let mut held_baseline = crate::held();
        install(state, 0);
        if warm {
            crate::tasks::warm_up(workers);
            let baseline = heap_charged();
            held_baseline = crate::held();
            with_state(|state| {
                state.max_memory = max_memory;
                state.baseline = baseline;
            });
            HOT.with(|hot| hot.mem_ceiling.set(mem_ceiling(baseline, max_memory)));
        }
        // ADR 0061 rule 3: past twice the budget, allocation fails and the process aborts.
        crate::ceiling(held_baseline.saturating_add(budget.saturating_mul(2)));
        DRIVER.with(|cell| *cell.borrow_mut() = drive);
        let status = entry();
        // The root's end answers the reply rights it still holds (ADR 0123 rule 4.5): `503`
        // when the run aborted, which then ends here with its abort, else `500`.
        if crate::serve::serving().is_some() {
            match crate::serve::take_abort() {
                Some((code, location, message)) => {
                    crate::serve::answer_held(crate::serve::STOPPED);
                    if let Some(state) = STATE.with(|cell| take_state(&mut cell.borrow_mut())) {
                        abort_run(state, code, location, message);
                    }
                }
                None => crate::serve::answer_held(crate::serve::UNANSWERED),
            }
        }
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
    // The scripted engine of the tests ends with the run, after every answer (ADR 0123 rule
    // 21).
    #[cfg(feature = "test-hooks")]
    if let Err(message) = crate::serve::script::end() {
        let _ = writeln!(stderr, "error: {message}");
        code = USAGE;
    }
    // The serve threads write what the run answered, then end (ADR 0123 rule 18); after an
    // abort that ends the process at once, `abort_run` made the hard stop already.
    end_serving(
        run_stop().map_or(std::time::Duration::from_secs(1), |stop| stop.grace()),
        false,
    );
    if let Err(error) = state.shared.finish() {
        // The same line as `una run`'s, which names the file.
        match TRACE_FILE.get() {
            Some(path) => {
                let _ = writeln!(
                    stderr,
                    "error: cannot write the trace to `{}`: {error}",
                    path.display()
                );
            }
            None => {
                let _ = writeln!(stderr, "error: cannot write the trace: {error}");
            }
        }
        code = USAGE;
    }
    code
}

/// Ends the run from a started task with `code` at `location`, as an abort of `main` does:
/// U6003 for the run's memory budget, U6007 for the hard stop, U8001 for a defect (`spec/27
/// §7`). Never returns, except in a run that serves, where an abort the run's tasks end by is
/// recorded and the task ends as its abort says, the root then ending the run
/// ([`crate::serve::defer_abort`]).
pub(crate) fn end_run_from_task(code: Code, location: Location, message: String) {
    // In a run that serves, the task ends and every other stops at its next check point, so
    // that the host answers the rights each holds; the root ends the run (`crate::serve`).
    if crate::serve::defer_abort(code, &location, &message, false) {
        return;
    }
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
            // A server exists from edition 0.4 (`spec/31 §2` rule 4).
            if !meta.serve && spec.starts_with(una_core::http::serve::PREFIX) {
                return Err(format!(
                    "`{spec}` needs a program of edition 0.4; this executable is of an earlier edition"
                ));
            }
            Grant::parse(spec, Path::new("")).ok_or_else(|| una_interp::invalid_grant(spec))
        })
        .collect()
}

/// The `serve:` grants of the run, before `main` (`spec/31 §6` rule 14), as `una run` prepares
/// them: the descriptor limit, the TLS files checked on their open descriptors, and one
/// listener per grant, held until the run ends.
fn serve(
    grants: &[Grant],
    serve_threads: usize,
    file_threads: usize,
) -> Result<una_http::serve::Prepared, String> {
    let inputs: Vec<una_http::serve::ServeInput<'_>> = grants
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
        .collect();
    una_http::serve::prepare(&inputs, serve_threads, file_threads)
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
/// At `site`, a text that does not fit the memory budget aborts the run with U6003 (`spec/11
/// §7`, #1223); the caller returns a value the stopped task never uses.
pub fn written(site: i64, ty: usize, word: i64, params: &[i64]) -> i64 {
    let text =
        with_state(|state| state.types.written(ty, word, params)).unwrap_or(Some(String::new()));
    match text {
        Some(text) => values::text_word(text),
        None => {
            write_over_budget(site);
            values::text_word(String::new())
        }
    }
}

/// `written` of the type at `ty`, as the generated code passes it, at `site`.
pub fn write(site: i64, ty: i64, word: i64) -> i64 {
    written(site, usize::try_from(ty).unwrap_or(usize::MAX), word, &[])
}

/// An `Int` as text, at `site` ([`crate::render::int_written`]): one whose text does not fit
/// the memory budget aborts the run with U6003 there (`spec/11 §7`, #1223).
pub fn int_text(site: i64, a: i64) -> i64 {
    match crate::render::int_written(a) {
        Some(text) => values::text_word(text),
        None => {
            write_over_budget(site);
            values::text_word(String::new())
        }
    }
}

/// Aborts the run, or stops the task, with U6003 at `site`: a write does not fit the memory
/// budget (`spec/11 §7`, #1223), as a call over it.
pub(crate) fn write_over_budget(site: i64) {
    refuse(usize::try_from(site).unwrap_or(usize::MAX), None);
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
        // In a run that serves, the root stops as a task does, and ends the run with the abort
        // once its tasks have ended (`crate::serve`).
        Some(state) if crate::serve::defer_abort(code, &location, &message, true) => {
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
        tasks_runs: una_interp::concurrency::BlockCounts::default(),
        shared: Arc::clone(&template.shared),
        failed_write: None,
        max_depth: template.max_depth,
        frame_bound: template.frame_bound,
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
        races: una_interp::concurrency::BlockCounts::default(),
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
        quiet: template.quiet,
        hushed: template.hushed,
        concurrency: template.concurrency,
        sched: Some(Arc::clone(&template.sched)),
        serve_task: template.serve_task,
        op_stopped: false,
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

/// Consumes a pending raise of the thread's attention word without acting on it, as a check
/// point that took the slow path and failed to yield would: the fault #1535 guards against, so
/// a test can show the scheduler still ends the turn.
#[cfg(test)]
pub(crate) fn lose_attention() {
    HOT.with(|hot| {
        if let Some(word) = hot.word.get() {
            hot.seen.set(word.load());
        }
    });
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
        tasks_runs: una_interp::concurrency::BlockCounts::default(),
        shared: crate::started::RunShared::new(),
        failed_write: None,
        max_depth: options.max_depth,
        frame_bound: None,
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
            outcome: None,
            error: None,
            site: None,
        }]),
        host_failed: false,
        interrupted: false,
        races: una_interp::concurrency::BlockCounts::default(),
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
        quiet: 0,
        hushed: 0,
        concurrency: options.concurrency,
        sched: None,
        serve_task: 0,
        op_stopped: false,
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

/// The site of a call that has none, for an abort of an operation whose entry point takes no
/// site (#1042): [`abort_at`] then reports at the fallback location, the start of an unnamed
/// file, since no position of the program is known.
pub(crate) const NO_SITE: usize = usize::MAX;

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
    // Every reply right still held is answered `503`, with no event, before the process ends
    // without waiting for its tasks (ADR 0123 rule 4.5, rule 18 item 6). After the hard stop or
    // an internal error, the engine's hard stop comes first: from it on the engine discards the
    // program's answers, and the serve threads answer each exchange still open `503` in one
    // non-blocking write and close every connection, without waiting on a peer; the wait for
    // them (at most a second) ends before the answers below, which then send nothing. A reply
    // already in progress when the hard stop comes, past its check of the stop, may be lost
    // with its connection.
    if matches!(code, Code::U6007 | Code::U8001) {
        end_serving(std::time::Duration::ZERO, true);
    }
    crate::serve::answer_all_left();
    // One stop ends the process: a stop on another thread may have claimed it (ADR 0113). The
    // end is claimed before the allocator's limit is lifted, so that the stop that lifts it is
    // the one that reports (#2151); a sibling that stops afterwards parks.
    end_once();
    crate::ceiling(usize::MAX);
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
    // A debug build run with `UNA_DEBUG_CHARGE` set prints what the run charged at this stop,
    // term by term, so a stop that depends on load can be traced to its term (#1139): the heap
    // the process holds and its peak, the baseline before `main`, the stacks the live tasks
    // used, the reservations of copies in flight, the thread stacks counted for the count
    // limit, and what this thread allocated.
    // The executable is the process, as for `env_var` in `host.rs`: the variable is read only
    // in a debug build, never in a released runtime.
    #[cfg(debug_assertions)]
    #[allow(clippy::disallowed_methods)]
    let trace = std::env::var_os("UNA_DEBUG_CHARGE").is_some();
    #[cfg(debug_assertions)]
    if trace {
        let _ = writeln!(
            io::stderr(),
            "una-rt charge: held {} credited {} peak {} baseline {} stack_charge {} reserved {} thread_stacks {} thread_allocated {} ceiling {} budget_mib {} on {}",
            crate::held(),
            capped_credit(values::credit_added_bytes(), values::credit_removed_bytes()),
            crate::peak(),
            state.baseline,
            STACK_CHARGE.load(Ordering::Relaxed),
            reserved(),
            STACKS.load(Ordering::SeqCst),
            crate::ffi::thread_allocated(),
            mem_ceiling(state.baseline, state.max_memory),
            state.max_memory,
            std::thread::current().name().unwrap_or("?")
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

/// Reports that `main`, or `init` of a driven module, failed with `failure`, which the caller
/// owns and the report releases: U6001 with the failure's `public` fields (`spec/04 §8`), after
/// the output so far.
pub fn main_failed(failure: i64) {
    // The word of a stopped task (`una_codegen` `STOPPED`) is no failure: `main`'s thread ends
    // the run at an abort, and every cut is caught by its own `within`, so `main` never returns
    // it. Should it, that is an internal error (U8001, exit code 3), never a failure of `main`.
    // In a run that serves, `main` stops when another task aborted the run, which the root's
    // end reports (`crate::serve`).
    if failure == -1 && crate::serve::abort_recorded() {
        return;
    }
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
                "{} failed with `{}`",
                described.driven.as_ref().map_or_else(
                    || "`main`".to_owned(),
                    |module| format!("`init` of the driven module `{module}`")
                ),
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

    #[test]
    fn a_budget_stops_a_loop_instance_at_the_start_of_iteration_n_plus_1() {
        // ADR 0116 rule 19, #1472: a loop instance with a budget of 3 starts iterations 1 to
        // 3 and stops at the start of the fourth; without a budget (0) it never stops; a
        // count past 2^63 - 1, as a wrapped one reads, is past every budget.
        assert!((0..3).all(|started| !budget_spent(started, 3)));
        assert!(budget_spent(3, 3));
        assert!(budget_spent(4, 3));
        assert!(!budget_spent(u64::MAX, 0));
        assert!(budget_spent(u64::MAX, (1 << 63) - 1));
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
        assert!(!options.max_memory_given);
        let options = parse_args(&strings(&["--max-memory", "64"])).unwrap();
        assert_eq!(options.max_memory, 64);
        assert!(options.max_memory_given);
        assert!(parse_args(&strings(&["--max-memory", "many"])).is_err());
        assert!(parse_args(&strings(&["--json"])).is_err());
        assert!(parse_args(&strings(&["--grant"])).is_err());
        assert!(parse_args(&strings(&["--max-depth", "many"])).is_err());
    }

    #[test]
    fn a_declared_budget_is_the_default_and_the_flag_only_lowers_it() {
        // ADR 0109 rule 4.
        let meta = Meta::parse(r#"{"grants":[],"uses":[],"max_memory":256}"#).unwrap();
        assert_eq!(meta.max_memory, Some(256));
        let budget = |args: &[&str]| with_budget(parse_args(&strings(args)).unwrap(), &meta);
        assert_eq!(budget(&[]).unwrap().max_memory, 256);
        assert_eq!(budget(&["--max-memory", "64"]).unwrap().max_memory, 64);
        assert_eq!(budget(&["--max-memory", "256"]).unwrap().max_memory, 256);
        assert_eq!(
            budget(&["--max-memory", "257"]).unwrap_err(),
            "--max-memory 257 is above the package's declared max_memory of 256 MiB; it may lower the budget, never raise it"
        );
        let undeclared = Meta::parse(r#"{"grants":[],"uses":[]}"#).unwrap();
        assert_eq!(undeclared.max_memory, None);
        let options = with_budget(parse_args(&[]).unwrap(), &undeclared).unwrap();
        assert_eq!(options.max_memory, una_interp::DEFAULT_MAX_MEMORY);
        let options = with_budget(
            parse_args(&strings(&["--max-memory", "8192"])).unwrap(),
            &undeclared,
        )
        .unwrap();
        assert_eq!(options.max_memory, 8192);
        // A budget the manifest could not declare makes the description invalid.
        for bad in ["0", "4097", "-1", "\"64\"", "1.5"] {
            let text = format!(r#"{{"grants":[],"uses":[],"max_memory":{bad}}}"#);
            assert!(Meta::parse(&text).is_none(), "{bad}");
        }
        assert!(Meta::parse(r#"{"grants":[],"uses":[],"max_memory":4096}"#).is_some());
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
            "--serve-threads=5",
        ]))
        .unwrap();
        assert_eq!(options.serve_threads, 5);
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
            &["--serve-threads", "0"],
            &["--serve-threads", "257"],
            &["--serve-threads", "x"],
            &["--serve-threads"],
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
    fn a_task_stack_is_sized_from_the_frame_bound_and_never_larger() {
        // `spec/27 §4` rule 1 (#1152): `16 MiB + d × F`, at most the size `--max-depth` alone
        // gives, which a program without a bound keeps.
        let alone = una_interp::stack_bytes(10_000).unwrap();
        assert_eq!(task_stack_bytes(10_000, None), alone);
        assert_eq!(
            task_stack_bytes(10_000, Some(128)),
            16 * 1024 * 1024 + 10_000 * 128
        );
        assert_eq!(task_stack_bytes(10_000, Some(MAX_FRAME_BOUND)), alone);
        for depth in [0, 1, 50, 10_000, 1_000_000, usize::MAX] {
            for bound in [None, Some(16), Some(128), Some(4096), Some(MAX_FRAME_BOUND)] {
                let size = task_stack_bytes(depth, bound);
                assert!(size <= task_stack_bytes(depth, None), "{depth} {bound:?}");
                assert!(size >= TASK_STACK_BASE.min(task_stack_bytes(depth, None)));
                if depth < usize::MAX {
                    assert!(
                        size <= task_stack_bytes(depth + 1, bound),
                        "{depth} {bound:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_description_gives_a_frame_bound_a_code_generator_could_write_or_none() {
        // `spec/27 §4` rule 1: a positive multiple of 16 up to the 64 KiB guard, or absent; any
        // other value is no description, so no stack is sized from it.
        let meta = |bound: &str| Meta::parse(&format!(r#"{{"grants":[],"uses":[]{bound}}}"#));
        assert_eq!(meta("").unwrap().frame_bound, None);
        assert_eq!(
            meta(r#","frame_bound":128"#).unwrap().frame_bound,
            Some(128)
        );
        assert_eq!(
            meta(r#","frame_bound":65536"#).unwrap().frame_bound,
            Some(MAX_FRAME_BOUND)
        );
        for bad in ["0", "8", "120", "65552", "-16", "1.5", "\"128\"", "null"] {
            assert!(meta(&format!(r#","frame_bound":{bad}"#)).is_none(), "{bad}");
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn the_pooled_stack_size_is_the_same_whatever_main_does() {
        // #1152, the invariant of #1145: the pool's stacks are mapped before `main` at one size,
        // from the frame bound and `--max-depth`, both public. Two runs whose `main` differs
        // only in a value standing for `secret` input, which decides how many tasks live at
        // once, see the same size before and after, and so does every task's template.
        let args = strings(&["--workers", "2", "--max-depth", "10000"]);
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        for secret in [0usize, 3, 100] {
            let record = Arc::clone(&seen);
            let status = start(
                &args,
                r#"{"grants":[],"uses":[],"threads":true,"frame_bound":128}"#,
                move || {
                    let observed = with_state(|state| {
                        let sched = state.sched.clone()?;
                        let before = sched.stack_sizes();
                        let template = state.for_tasks(0).stack;
                        sched.churn(secret, template);
                        Some((before, sched.stack_sizes(), template))
                    });
                    record.lock().unwrap().push(observed);
                    0
                },
            );
            assert_eq!(status, 0);
        }
        let size = 16 * 1024 * 1024 + 10_000 * 128;
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        for observed in seen.iter() {
            let (before, after, template) = observed.clone().unwrap().unwrap();
            // Usable sizes are rounded up to whole pages.
            assert_eq!(before.len(), 1, "{before:?}");
            assert!(
                before[0] >= size && before[0] < size + 64 * 1024,
                "{before:?}"
            );
            assert_eq!(after, before);
            assert!(template <= before[0]);
        }
    }

    #[test]
    #[ignore = "run by `the_tls_settings_exist_before_main_exactly_when_a_grant_includes_http`, in a process of its own"]
    fn the_tls_settings_scenario() {
        // A fresh process: nothing has built the TLS settings. A run without an `Http` grant
        // builds none, before or during `main`; one with such a grant has them before `main`
        // runs, before any request.
        start(&[], r#"{"grants":[],"uses":[]}"#, || {
            eprintln!(
                "{STEP}: without http {} pools {:?}",
                una_http::client::tls_ready(),
                una_http::resolve::threads()
            );
            0
        });
        // The program performs an operation of `Http`, as its C `main` then says.
        crate::ffi::una_rt_http_enable();
        start(
            &strings(&["--grant", "http:https://example.com", "--io-threads", "12"]),
            r#"{"grants":[],"uses":[]}"#,
            || {
                eprintln!(
                    "{STEP}: with http {} pools {:?}",
                    una_http::client::tls_ready(),
                    una_http::resolve::threads()
                );
                0
            },
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn the_timer_thread_runs_before_main_and_no_within_starts_it_whatever_ran_before() {
        // #1151, ADR 0116 amendment P8, the deterministic guard of the lazy timer: in a program
        // with the `threads` flag (a `within`, a `for ever` or a task construct), the run's
        // timer thread runs before `main`'s first construct, started exactly once, and a
        // `within` then starts none, whether no `within` ran before it or some did, as code
        // under a `secret` branch may have. Without the flag, no scheduler is made and no timer
        // thread exists. On the old code, whose timer thread started at the first deadline
        // through a `OnceLock`, it was absent before `main`, and the measured `within` started
        // it after a history of 0 earlier ones and not after 1 or 3: a cost inside the
        // construct that earlier code decided.
        type Seen = (Option<(bool, usize)>, Option<usize>);
        let seen: Arc<std::sync::Mutex<Vec<Seen>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let with_flag = r#"{"grants":[],"uses":[],"threads":true}"#;
        for (meta, earlier) in [
            (with_flag, 0),
            (with_flag, 1),
            (with_flag, 3),
            (r#"{"grants":[],"uses":[]}"#, 0),
        ] {
            let record = Arc::clone(&seen);
            let status = start(&[], meta, move || {
                let timers = || {
                    with_state(|state| {
                        state
                            .sched
                            .as_ref()
                            .map(|sched| (sched.timers().running(), sched.timers().starts()))
                    })
                    .flatten()
                };
                let before = timers();
                let mut during = None;
                if before.is_some() {
                    // A minute: no deadline passes in this test.
                    let d = values::int_word(&una_core::bigint::BigInt::from(60_000));
                    for _ in 0..earlier {
                        let id = crate::deadline::within_enter(d);
                        assert_eq!(crate::deadline::within_end(id, 0), 0);
                    }
                    let starts = timers().map_or(0, |(_, starts)| starts);
                    let id = crate::deadline::within_enter(d);
                    assert_eq!(crate::deadline::within_end(id, 0), 0);
                    during = Some(timers().map_or(0, |(_, after)| after) - starts);
                }
                record.lock().unwrap().push((before, during));
                0
            });
            assert_eq!(status, 0);
        }
        assert_eq!(
            *seen.lock().unwrap(),
            [
                (Some((true, 1)), Some(0)),
                (Some((true, 1)), Some(0)),
                (Some((true, 1)), Some(0)),
                (None, None),
            ]
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn the_tls_settings_exist_before_main_exactly_when_a_grant_includes_http() {
        // #1148, ADR 0116 amendment P8: the client's TLS settings and resolver pool are state a run starts up
        // front when its grants include `Http`, so no request after earlier `secret` code finds
        // them built while another builds them. Before the fix they were built at the first
        // `https` request, and `main` found none in either run.
        let (code, stderr) = scenario("the_tls_settings_scenario");
        assert_eq!(code, Some(0), "{stderr}");
        assert_eq!(steps(&stderr), 2, "{stderr}");
        // The client's pools too: none without the grant, and with it `max(2, 12 / 4)` lookup
        // threads and 12 connect threads, all started before `main`.
        assert!(
            stderr.contains("without http false pools (0, 0)"),
            "{stderr}"
        );
        assert!(stderr.contains("with http true pools (3, 12)"), "{stderr}");
    }

    #[test]
    fn a_tick_is_due_at_its_exact_nanosecond_whatever_the_phase_of_its_origin() {
        // P13, #1151: tick `k` of the driver is due exactly `k × ticks` milliseconds after the
        // first one, in nanoseconds, whatever the offset within a millisecond at which `init`
        // returned; one nanosecond before it, the tick before is still the latest due. The old
        // driver read its origin and the clock in whole milliseconds, so a tick was due up to
        // 1 ms early or late, by the phase that `init` left.
        const MS: u128 = 1_000_000;
        for millis in [0u128, 3, 86_400_000] {
            for offset in (0..MS).step_by(9_901).chain([1, MS - 1]) {
                let origin = millis * MS + offset;
                for period in [1u64, 16, 1_000] {
                    for next in [0u64, 1, 2, 7] {
                        let due = tick_due(origin, next, period);
                        assert_eq!(due - origin, u128::from(next) * u128::from(period) * MS);
                        if next > 0 {
                            assert_eq!(tick_at(origin, due - 1, next - 1, period), (next - 1, 0));
                        }
                        assert_eq!(tick_at(origin, due, next, period), (next, 0));
                        // A later reading delivers the latest tick due, with the count skipped.
                        let late = tick_due(origin, next + 3, period);
                        assert_eq!(tick_at(origin, late, next, period), (next + 3, 3));
                    }
                }
            }
        }
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

    /// Runs `f` as a thread of a run whose steps check `ceiling`: what [`try_reserve`] reads of
    /// the thread, set and put back.
    fn with_ceiling<T>(ceiling: usize, f: impl FnOnce() -> T) -> T {
        let before = HOT.with(|hot| (hot.active.replace(true), hot.mem_ceiling.replace(ceiling)));
        let out = f();
        HOT.with(|hot| {
            hot.active.set(before.0);
            hot.mem_ceiling.set(before.1);
        });
        out
    }

    /// The two counts of the reservations, as a test sees them.
    fn counts() -> (u64, u64) {
        (
            ANNOUNCED.load(Ordering::SeqCst),
            RELEASED.load(Ordering::SeqCst),
        )
    }

    #[test]
    #[ignore = "run by `a_refused_step_announces_nothing`, in a process of its own"]
    fn a_refused_step_announces_nothing_scenario() {
        // #1943: a refused step leaves both counts as they were, so no other thread's check can
        // have seen its bytes, and an accepted one moves them by exactly its bytes. `ANNOUNCED`
        // only grows, so an announcement taken back would show in it too. The process has one
        // test thread, and nothing allocates between a read of `charged()` and the steps
        // checked against it.
        let live = || LIVE_RESERVATIONS.with(Cell::get);
        let start = counts();
        let most = isize::MAX as usize;

        // Past any allocation: refused before the loop, in a run or outside one.
        assert!(try_reserve(most + 1).is_none());
        assert!(try_reserve(usize::MAX).is_none());
        assert_eq!((counts(), live()), (start, 0));

        // A run with room for 4096 bytes.
        with_ceiling(charged() + 4096, || {
            for bytes in [4097, 1 << 59, most] {
                assert!(try_reserve(bytes).is_none(), "{bytes}");
                assert_eq!((counts(), live()), (start, 0), "{bytes}");
            }
            // The whole room is accepted, and counted against the next step until its drop.
            let whole = try_reserve(4096);
            assert!(whole.is_some());
            assert_eq!((counts(), live()), ((start.0 + 4096, start.1), 1));
            for bytes in [1, 1 << 59, most] {
                assert!(try_reserve(bytes).is_none(), "{bytes}");
                assert_eq!((counts(), live()), ((start.0 + 4096, start.1), 1));
            }
            let nothing = try_reserve(0);
            assert!(nothing.is_some());
            drop(nothing);
            drop(whole);
            assert_eq!((counts(), live()), ((start.0 + 4096, start.1 + 4096), 0));
            assert_eq!(reserved(), 0);
            // The room is back.
            drop(try_reserve(4096).unwrap());
            assert!(try_reserve(4097).is_none());
        });

        // The #1920 size in a run of the default budget, 4096 MiB: refused, nothing announced.
        let before = counts();
        with_ceiling(mem_ceiling(crate::held(), 4096), || {
            assert!(try_reserve(1 << 59).is_none());
            assert!(try_reserve(1 << 20).is_some());
        });
        assert_eq!(counts(), (before.0 + (1 << 20), before.1 + (1 << 20)));

        // Outside a run no ceiling applies, and the sum of the reservations still never wraps:
        // two steps of `isize::MAX` and one byte fill it, and anything more is refused.
        let before = counts();
        let (a, b) = (try_reserve(most), try_reserve(most));
        assert!(a.is_some() && b.is_some());
        assert_eq!(reserved(), u64::MAX - 1);
        let full = (before.0.wrapping_add(u64::MAX - 1), before.1);
        for bytes in [2, most] {
            assert!(try_reserve(bytes).is_none(), "{bytes}");
            assert_eq!((counts(), live()), (full, 2), "{bytes}");
        }
        // A thread of a run sees the bytes announced outside it, and its sum with what the
        // process holds saturates instead of wrapping to a small figure.
        with_ceiling(usize::MAX - 1, || assert!(try_reserve(0).is_none()));
        with_ceiling(usize::MAX, || assert!(try_reserve(0).is_some()));
        assert_eq!((counts(), live()), (full, 2));
        let c = try_reserve(1);
        assert!(c.is_some());
        assert_eq!(reserved(), u64::MAX);
        assert!(try_reserve(1).is_none());
        drop((a, b, c));
        assert_eq!((reserved(), live()), (0, 0));
        assert_eq!(counts().0, before.0.wrapping_add(u64::MAX));
        assert_eq!(counts().0, counts().1);

        // O(1): a step is one pass of the loop, accepted or refused, whatever the number of
        // reservations alive, since no other thread's step is accepted meanwhile.
        for alive in [1, 1_000, 100_000] {
            let held: Vec<_> = (0..alive).map(|_| try_reserve(1)).collect();
            assert!(held.iter().all(Option::is_some));
            let room = charged() + alive + 8;
            let before = PASSES.with(Cell::get);
            with_ceiling(room, || {
                assert!(try_reserve(8).is_some());
                assert!(try_reserve(9).is_none());
            });
            assert_eq!(PASSES.with(Cell::get) - before, 2, "{alive}");
            assert_eq!(reserved(), alive as u64);
            drop(held);
        }
        assert_eq!((reserved(), live()), (0, 0));
        eprintln!("{STEP}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn a_refused_step_announces_nothing() {
        let (code, stderr) = scenario("a_refused_step_announces_nothing_scenario");
        assert_eq!(code, Some(0), "{stderr}");
        assert_eq!(steps(&stderr), 1, "{stderr}");
    }

    /// One step's bytes in the scenarios of the room for one step.
    const ROOM_STEP: usize = 1 << 20;

    thread_local! {
        /// A step accepted before a pass begins, for an action of `BETWEEN_READS` to finish.
        static PENDING: RefCell<Option<Reservation>> = const { RefCell::new(None) };
        /// The bytes that the actions allocated, and the reservations they keep.
        static ALLOCATED: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
        static KEPT: RefCell<Vec<Reservation>> = const { RefCell::new(Vec::new()) };
    }

    /// The pending step allocates its bytes and drops its reservation.
    fn the_pending_step_allocates_and_releases() {
        let step = PENDING.with(|cell| cell.borrow_mut().take()).unwrap();
        ALLOCATED.with(|cell| cell.borrow_mut().push(Vec::with_capacity(ROOM_STEP)));
        drop(step);
    }

    /// Another step is accepted and keeps its reservation.
    fn another_step_is_accepted() {
        let step = try_reserve(ROOM_STEP).unwrap();
        KEPT.with(|cell| cell.borrow_mut().push(step));
    }

    /// Another step is accepted, allocates, releases and frees its bytes.
    fn another_step_comes_and_goes() {
        let step = try_reserve(ROOM_STEP).unwrap();
        let bytes: Vec<u8> = Vec::with_capacity(ROOM_STEP);
        std::hint::black_box(&bytes);
        drop(step);
        drop(bytes);
    }

    #[test]
    #[ignore = "run by `a_step_between_the_reads_of_a_pass_is_never_missed`, in a process of its own"]
    fn a_step_between_the_reads_of_a_pass_scenario() {
        // ADR 0113 rule 4, "it never sees fewer": a run with room for one step and half of
        // another, and a second step whose pass is interrupted, after each of its three reads
        // in turn, by what another thread's step would do there. One thread, so every
        // interleaving below is exact. The containers of the actions are grown first, so that
        // the room is not spent on them.
        ALLOCATED.with(|cell| cell.borrow_mut().reserve(8));
        KEPT.with(|cell| cell.borrow_mut().reserve(8));
        let passes = || PASSES.with(Cell::get);
        with_ceiling(charged() + ROOM_STEP + ROOM_STEP / 2, || {
            for read in 1..=3 {
                // A step accepted earlier allocates and releases inside the pass: its bytes
                // leave `reserved` for `held()`, and the pass counts them in one or both.
                // Reading the bytes given back after the bytes held would count them in
                // neither, for the action after the first of those two reads.
                let first = try_reserve(ROOM_STEP);
                assert!(first.is_some());
                PENDING.with(|cell| *cell.borrow_mut() = first);
                BETWEEN_READS.with(|cell| {
                    cell.set(Some((read, the_pending_step_allocates_and_releases)));
                });
                assert!(try_reserve(ROOM_STEP).is_none(), "after read {read}");
                assert!(BETWEEN_READS.with(Cell::get).is_none());
                ALLOCATED.with(|cell| cell.borrow_mut().clear());
                assert_eq!(reserved(), 0);

                // Another step is accepted inside the pass: the pass's compare-exchange
                // fails, and its second pass counts that step. An addition in place of the
                // compare-exchange would accept both.
                let before = passes();
                BETWEEN_READS.with(|cell| cell.set(Some((read, another_step_is_accepted))));
                assert!(try_reserve(ROOM_STEP).is_none(), "after read {read}");
                assert_eq!(passes() - before, 3, "two passes, and the other step's one");
                assert_eq!(reserved(), ROOM_STEP as u64);
                KEPT.with(|cell| cell.borrow_mut().clear());
                assert_eq!(reserved(), 0);

                // Another step comes and goes inside the pass, and leaves the room: the
                // pass is repeated, since a step was accepted, and its second pass fits.
                let before = passes();
                BETWEEN_READS.with(|cell| cell.set(Some((read, another_step_comes_and_goes))));
                let step = try_reserve(ROOM_STEP);
                assert!(step.is_some(), "after read {read}");
                assert_eq!(passes() - before, 3, "two passes, and the other step's one");
                drop(step);
                assert_eq!(reserved(), 0);
            }
        });
        assert_eq!(LIVE_RESERVATIONS.with(Cell::get), 0);
        eprintln!("{STEP}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn a_step_between_the_reads_of_a_pass_is_never_missed() {
        let (code, stderr) = scenario("a_step_between_the_reads_of_a_pass_scenario");
        assert_eq!(code, Some(0), "{stderr}");
        assert_eq!(steps(&stderr), 1, "{stderr}");
    }

    #[test]
    #[ignore = "run by `steps_that_fit_only_one_at_a_time_are_never_accepted_together`, in a process of its own"]
    fn steps_that_fit_only_one_at_a_time_scenario() {
        // ADR 0113 rule 4, "Concurrent copies": steps cannot together take `held()` past the
        // ceiling. The run has room for one step of `STEP_BYTES` and half of another, and
        // every thread asks for one step at a time, allocates it, drops its reservation and
        // frees it. `inside` counts the threads between an accepted step and the moment
        // before its bytes are freed: it is never 2, since a second step fits only once the
        // first one's bytes are neither reserved nor held. That fails if the acceptance is not
        // one atomic step with its check (an add in place of the compare-exchange), or if a
        // pass reads `RELEASED` after `held()` (a step that allocates and releases between
        // the two reads is then in neither; `a_step_between_the_reads_of_a_pass_is_never_missed`
        // places that step exactly, where this test meets it only by schedule). The process
        // has only these threads, so nothing else moves `held()` by half a step. Fixed rounds
        // and a barrier: no timing.
        const STEP_BYTES: usize = ROOM_STEP;
        let (threads, rounds) = (8, 40_000);
        let begin = std::sync::Barrier::new(threads);
        let inside = AtomicUsize::new(0);
        let ceiling = charged() + STEP_BYTES + STEP_BYTES / 2;
        let accepted: usize = std::thread::scope(|scope| {
            let all: Vec<_> = (0..threads)
                .map(|_| {
                    scope.spawn(|| {
                        with_ceiling(ceiling, || {
                            begin.wait();
                            let mut accepted = 0usize;
                            for _ in 0..rounds {
                                let Some(step) = try_reserve(STEP_BYTES) else {
                                    continue;
                                };
                                assert_eq!(inside.fetch_add(1, Ordering::SeqCst), 0);
                                // Seen, so that no profile removes the allocation.
                                let bytes: Vec<u8> = Vec::with_capacity(STEP_BYTES);
                                std::hint::black_box(&bytes);
                                drop(step);
                                std::thread::yield_now();
                                assert_eq!(inside.fetch_sub(1, Ordering::SeqCst), 1);
                                drop(bytes);
                                accepted += 1;
                            }
                            accepted
                        })
                    })
                })
                .collect();
            all.into_iter().map(|t| t.join().unwrap()).sum()
        });
        assert!(accepted > 0);
        assert_eq!(reserved(), 0);
        eprintln!("{STEP} {accepted}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn steps_that_fit_only_one_at_a_time_are_never_accepted_together() {
        let (code, stderr) = scenario("steps_that_fit_only_one_at_a_time_scenario");
        assert_eq!(code, Some(0), "{stderr}");
        assert_eq!(steps(&stderr), 1, "{stderr}");
    }

    #[test]
    #[ignore = "run by `refused_steps_never_fail_the_steps_of_sibling_threads`, in a process of its own"]
    fn refused_steps_never_fail_the_steps_of_sibling_threads_scenario() {
        // #1943: threads of one run whose steps are refused, for asking more than the ceiling,
        // never make a sibling's step that fits fail. Announced before it was checked, a refused
        // step of `isize::MAX` bytes was in every check made meanwhile, and two at once wrapped
        // the sum. The ceiling is far above what the process holds, so only a refused step
        // seen by a sibling can fail a step here. No timing is assumed: every thread makes a
        // fixed number of steps, and the refused ones go on until the last fitting one is done.
        //
        // The scenario runs in a process of its own, where no other run changes the
        // allocator's limit. Beside the other tests of this binary, `held()` was read torn
        // while a run that ended lifted the limit, and a fitting step was refused, about four
        // in 200,000: since #2151 that read is never torn
        // (`a_step_that_fits_is_never_refused_while_the_limit_changes`).
        let ceiling = usize::MAX / 4;
        let rounds: usize = 50_000;
        let (refusing, fitting) = (2, 4);
        let begin = std::sync::Barrier::new(refusing + fitting);
        let done = AtomicBool::new(false);
        let (refused, failed) = std::thread::scope(|scope| {
            let hammers: Vec<_> = (0..refusing)
                .map(|_| {
                    scope.spawn(|| {
                        with_ceiling(ceiling, || {
                            begin.wait();
                            let (mut asked, mut accepted) = (0usize, 0usize);
                            while asked < rounds || !done.load(Ordering::SeqCst) {
                                accepted += usize::from(try_reserve(isize::MAX as usize).is_some());
                                asked += 1;
                            }
                            assert_eq!(LIVE_RESERVATIONS.with(Cell::get), 0);
                            (asked, accepted)
                        })
                    })
                })
                .collect();
            let begin = &begin;
            let workers: Vec<_> = (0..fitting)
                .map(|worker| {
                    scope.spawn(move || {
                        with_ceiling(ceiling, || {
                            begin.wait();
                            let mut failed = 0usize;
                            for round in 0..rounds {
                                let step = try_reserve(64 + worker + round % 4096);
                                failed += usize::from(step.is_none());
                                if round % 64 == 0 {
                                    std::thread::yield_now();
                                }
                                drop(step);
                            }
                            assert_eq!(LIVE_RESERVATIONS.with(Cell::get), 0);
                            failed
                        })
                    })
                })
                .collect();
            let failed: usize = workers.into_iter().map(|t| t.join().unwrap()).sum();
            done.store(true, Ordering::SeqCst);
            let refused: Vec<_> = hammers.into_iter().map(|t| t.join().unwrap()).collect();
            (refused, failed)
        });
        assert_eq!(failed, 0, "fitting steps refused beside refused ones");
        for (asked, accepted) in refused {
            assert!(asked >= rounds);
            assert_eq!(accepted, 0, "a step past the ceiling was accepted");
        }
        eprintln!("{STEP}");
    }

    #[test]
    #[ignore = "run by `a_step_that_fits_is_never_refused_while_the_limit_changes`, in a process of its own"]
    fn a_step_that_fits_is_never_refused_while_the_limit_changes_scenario() {
        // #2151 on the allocator of the process: one thread lowers and raises the allocator's
        // limit in turn, as runs that start and end do, while threads of a run make steps that
        // fit and read the bytes held. Each change of the limit used to leave `held()` close to
        // 2^62 between the allocator's two writes, and a step that read it then was refused.
        // The low limit is far above anything this process holds, so no allocation fails. It
        // runs in a process of its own, since it changes the limit every other test has.
        let low = usize::MAX - (1 << 62);
        let (ceiling, rounds, threads) = (usize::MAX / 4, 100_000usize, 4);
        let done = AtomicBool::new(false);
        let begin = std::sync::Barrier::new(threads + 1);
        let (failed, torn) = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..threads)
                .map(|worker| {
                    let (begin, done) = (&begin, &done);
                    scope.spawn(move || {
                        with_ceiling(ceiling, || {
                            begin.wait();
                            let (mut failed, mut torn, mut round) = (0usize, 0usize, 0usize);
                            while round < rounds || !done.load(Ordering::SeqCst) {
                                let step = try_reserve(64 + worker + round % 4096);
                                failed += usize::from(step.is_none());
                                drop(step);
                                torn += usize::from(crate::held() > 1 << 61);
                                round += 1;
                            }
                            (failed, torn)
                        })
                    })
                })
                .collect();
            begin.wait();
            for round in 0..200_000usize {
                crate::ceiling(if round % 2 == 0 { low } else { usize::MAX });
            }
            done.store(true, Ordering::SeqCst);
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .fold((0, 0), |(f, t), (a, b)| (f + a, t + b))
        });
        crate::ceiling(usize::MAX);
        assert_eq!(
            torn, 0,
            "reads of the bytes held torn by a change of the limit"
        );
        assert_eq!(failed, 0, "fitting steps refused while the limit changed");
        eprintln!("{STEP}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn a_step_that_fits_is_never_refused_while_the_limit_changes() {
        let (code, stderr) =
            scenario("a_step_that_fits_is_never_refused_while_the_limit_changes_scenario");
        assert_eq!(code, Some(0), "{stderr}");
        assert_eq!(steps(&stderr), 1, "{stderr}");
    }

    #[test]
    #[ignore = "run by `a_stop_that_finds_the_end_claimed_leaves_the_limit_in_place`, in a process of its own"]
    fn a_stop_that_finds_the_end_claimed_scenario() {
        // #2151: `abort_run` claims the end of the process before it lifts the allocator's
        // limit. A stop that finds the end claimed by a sibling parks with the limit of the
        // run still in place; a watcher waits for it to park, then reports the limit.
        start(&strings(&["--max-memory", "64"]), SITE_META, || {
            assert_ne!(crate::limit(), usize::MAX, "the run has no limit");
            ENDING.store(true, Ordering::SeqCst);
            std::thread::spawn(|| {
                while PARKED.load(Ordering::SeqCst) == 0 {
                    std::thread::yield_now();
                }
                let lifted = crate::limit() == usize::MAX;
                eprintln!("{STEP} lifted {lifted}");
                #[allow(clippy::exit)]
                std::process::exit(0);
            });
            abort_at(0, Code::U6003, "a sibling stopped first".to_owned())
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn a_stop_that_finds_the_end_claimed_leaves_the_limit_in_place() {
        let (code, stderr) = scenario("a_stop_that_finds_the_end_claimed_scenario");
        assert_eq!(code, Some(0), "{stderr}");
        assert!(stderr.contains(&format!("{STEP} lifted false")), "{stderr}");
        assert!(!stderr.contains("U6003"), "{stderr}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn refused_steps_never_fail_the_steps_of_sibling_threads() {
        let (code, stderr) =
            scenario("refused_steps_never_fail_the_steps_of_sibling_threads_scenario");
        assert_eq!(code, Some(0), "{stderr}");
        assert_eq!(steps(&stderr), 1, "{stderr}");
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

    #[test]
    #[ignore = "run by `a_table_the_build_did_not_foresee_faults_at_its_site`, in a process of its own"]
    fn a_table_the_build_did_not_foresee_scenario() {
        // The description says nothing of `hashing`, so the run reads no process key.
        start(&[], SITE_META, || {
            eprintln!("{STEP}");
            crate::values::hash_empty(0, 0);
            eprintln!("{STEP}: past the table");
            0
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn a_table_the_build_did_not_foresee_faults_at_its_site() {
        // ADR 0119 rule 9: no key is read late. A table made in a run whose build found that it
        // makes none is an internal fault, U8001 with exit code 3, at the table's site, with the
        // interpreter's message.
        let (code, stderr) = scenario("a_table_the_build_did_not_foresee_scenario");
        assert_eq!(code, Some(3), "{stderr}");
        assert_eq!(steps(&stderr), 1, "{stderr}");
        assert!(stderr.contains("U8001"), "{stderr}");
        assert!(
            stderr.contains(
                "evaluation stopped: a hash table made in a run whose code the analysis found makes none"
            ),
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
