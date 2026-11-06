//! The host stop of a run (ADR 0116 rules 14, 16 and 17, `spec/26 §6`), which `una run` and
//! native executables share: the requests that reach the run, the grace period, the hard stop
//! and the end of a run that does not stop.
//!
//! A request is SIGINT or SIGTERM, which `una-sys` counts for the process; or one of the run's
//! own, local to it: the end of `--deadline`, which the watcher sends, and a local request
//! ([`RunStop::request_local`]). Each own request counts once, however many threads make it.
//! The budget of `--stop-after-iterations` and of a test's `stop after N` is no request: each
//! loop instance counts its own iterations and stops by itself (ADR 0116 rule 19, #1472).
//! Requests are sticky. The first one starts the grace period; the hard stop comes at its end,
//! or at a second request; a third request, or the end of a second grace period after the hard
//! stop, ends the process with SIGTERM's default action, unless the run has ended first.
//!
//! Every interruptible wait waits in slices of at most [`SLICE`] and looks at the run's
//! requests between them: the wake pipe, which signals and the deadline write, makes a stop
//! prompt, and the slices bound how late a stop is seen when the pipe cannot tell, as when a
//! request of another run left it readable.

use std::cell::Cell;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use una_http::client::{Interrupt, Waited};
use una_sys::signals::Readiness;

/// The grace period when `--stop-grace` is not given, in seconds (rule 16).
pub const DEFAULT_GRACE_SECS: u64 = 10;

/// The longest `--deadline`, a year of seconds (`spec/13 §5`).
pub const MAX_DEADLINE_SECS: u64 = 31_536_000;

/// The longest `--stop-grace`, a day of seconds (`spec/13 §5`).
pub const MAX_GRACE_SECS: u64 = 86_400;

/// The longest slice of an interruptible wait: the operations' waits, the driver's wait for a
/// tick, and the watcher's wait for a first request. Between slices, each looks at the run's
/// requests, and the watcher at the run's end. A signal or the deadline wakes a wait at once,
/// through the wake pipe; but the pipe stays readable after a request of another run of the
/// process, and the end of a run writes nothing. So a stop is seen within one slice at worst, at the cost of an O(1) look ten
/// times a second while a wait lasts.
pub const SLICE: Duration = Duration::from_millis(100);

/// How often the watcher looks at the requests during the grace periods.
const GRACE_STEP: Duration = Duration::from_millis(20);

/// Why the hard stop came (rule 16), for the note of U6007.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardStop {
    /// The grace period, of this many seconds, ended.
    Grace(u64),
    /// A second host stop request came.
    Second,
}

/// The message of U6007, "the host stopped the run before it finished", with why the hard
/// stop came (ADR 0116 rule 16): the same in both engines.
pub fn hard_stop_message(hard: HardStop) -> String {
    match hard {
        HardStop::Grace(1) => {
            "the host stopped the run before it finished, after the grace period of 1 second"
                .to_owned()
        }
        HardStop::Grace(seconds) => format!(
            "the host stopped the run before it finished, after the grace period of {seconds} seconds"
        ),
        HardStop::Second => {
            "the host stopped the run before it finished, at a second host stop request".to_owned()
        }
    }
}

/// The run's life, for the watcher's escalation (rule 17): settled by one compare-exchange, so
/// that a run that has ended is never killed, and a kill that has begun is not undone.
const RUNNING: u8 = 0;
const ENDED: u8 = 1;
const KILLING: u8 = 2;

/// The host stop state of one run.
#[derive(Debug)]
pub struct RunStop {
    /// The process's requests when the run's state was made: those after it are the run's.
    /// The state is made before the signal handlers are installed, so a signal that comes while
    /// they are installed is counted, never absorbed.
    baseline: u32,
    /// The run's own requests: a local one, and the deadline's. Each is set once, so racing
    /// threads make one request.
    own: AtomicBool,
    deadline: AtomicBool,
    /// 0 before the hard stop; 1 when the grace period ended; 2 at a second request.
    hard: AtomicU8,
    /// The grace period, in seconds, at least 1.
    grace: u64,
    /// [`RUNNING`], [`ENDED`] or [`KILLING`].
    life: AtomicU8,
    /// What the host does besides at the host stop and at the hard stop ([`RunStop::on_stops`]).
    hook: StopHooks,
}

/// What the host does when a host stop comes, `false`, and when the hard stop comes, `true`,
/// besides the run's own check points: the serve engine's stop and hard stop (ADR 0123 rule 18).
/// It runs on the watcher's thread, runs no Una code, and does not wait on a peer.
pub type StopHook = Box<dyn Fn(bool) + Send + Sync>;

/// The run's [`StopHook`], set once, and whether it was called for the host stop and for the
/// hard stop, so that each call happens once.
#[derive(Default)]
struct StopHooks(std::sync::OnceLock<StopHook>, AtomicBool, AtomicBool);

impl std::fmt::Debug for StopHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.get().is_some() {
            "StopHooks(set)"
        } else {
            "StopHooks(none)"
        })
    }
}

impl StopHooks {
    /// Calls the hook for the host stop, `false`, or the hard stop, `true`, at most once each.
    fn run(&self, hard: bool) {
        let Some(hook) = self.0.get() else {
            return;
        };
        let once = if hard { &self.2 } else { &self.1 };
        if !once.swap(true, Ordering::SeqCst) {
            hook(hard);
        }
    }
}

impl RunStop {
    /// The state of a run that starts now, with a grace period of `grace` seconds, at least 1:
    /// a grace period of 0 would bring the hard stop and the end of the process at the first
    /// request, so 0 counts as 1 (the flags already take 1 to 86,400). Callers make it before
    /// they install the signal handlers.
    pub fn new(grace: u64) -> Arc<RunStop> {
        Arc::new(RunStop {
            baseline: una_sys::attention::stop_requests(),
            own: AtomicBool::new(false),
            deadline: AtomicBool::new(false),
            hard: AtomicU8::new(0),
            grace: grace.max(1),
            life: AtomicU8::new(RUNNING),
            hook: StopHooks::default(),
        })
    }

    /// Sets what the host does at the run's host stop and at its hard stop ([`StopHook`]):
    /// once per run; `false` when one was set already, which stays. The watcher calls it with
    /// `false` once a host stop request reached the run, and with `true` once the hard stop
    /// came, each at most once; a stop that came before the hook was set calls it here. A run
    /// without a watcher takes no host stop, and never calls it.
    pub fn on_stops(&self, hook: StopHook) -> bool {
        if self.hook.0.set(hook).is_err() {
            return false;
        }
        if self.stopped() {
            self.hook.run(false);
        }
        if self.hard().is_some() {
            self.hook.run(true);
        }
        true
    }

    /// The grace period between the first host stop request and the hard stop (rule 16).
    pub fn grace(&self) -> Duration {
        Duration::from_secs(self.grace)
    }

    /// The host stop requests that have reached the run so far.
    pub fn requests(&self) -> u32 {
        una_sys::attention::stop_requests()
            .saturating_sub(self.baseline)
            .saturating_add(u32::from(self.own.load(Ordering::SeqCst)))
            .saturating_add(u32::from(self.deadline.load(Ordering::SeqCst)))
    }

    /// Whether a host stop request has reached the run: sticky (rule 14).
    pub fn stopped(&self) -> bool {
        self.requests() > 0
    }

    /// A stop request local to the run, besides the deadline's: one request, however many
    /// threads make it. It wakes the waits on the pipe, as the deadline's request does, so a
    /// request parked on the native reactor in another `for ever` sees it at once. The budget
    /// of `--stop-after-iterations` no longer makes it (rule 19, #1472): that budget stops one
    /// loop instance, and a sticky request it made would carry the stop of a loop under a
    /// secret branch to every later loop.
    pub fn request_local(&self) {
        if self
            .own
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            una_sys::signals::wake();
        }
    }

    /// The request of `--deadline`, local to the run: one request, which also wakes the waits
    /// on the pipe, so they see it at once.
    pub fn request_deadline(&self) {
        if self
            .deadline
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            una_sys::signals::wake();
        }
    }

    /// The hard stop, once it has come (rule 16).
    pub fn hard(&self) -> Option<HardStop> {
        match self.hard.load(Ordering::SeqCst) {
            1 => Some(HardStop::Grace(self.grace)),
            2 => Some(HardStop::Second),
            _ => None,
        }
    }

    /// Marks the run as ended: its watcher ends at its next look, and can no longer kill the
    /// process. When the kill has begun already, the process is ending, and this changes
    /// nothing.
    pub fn finish(&self) {
        let _ = self
            .life
            .compare_exchange(RUNNING, ENDED, Ordering::SeqCst, Ordering::SeqCst);
    }

    fn done(&self) -> bool {
        self.life.load(Ordering::SeqCst) != RUNNING
    }

    /// Starts the watcher of the run, a thread that sends the request of `deadline` when it
    /// passes, and brings the hard stop and the end of the process when they are due. It runs
    /// no Una code. The guard it returns ends the run's watch when dropped, on every path out of
    /// the caller, a panic's included ([`RunStop::finish`]).
    pub fn watch(self: &Arc<Self>, deadline: Option<Duration>) -> std::io::Result<Watching> {
        let stop = Arc::clone(self);
        std::thread::Builder::new()
            .name("una-stop".to_owned())
            .stack_size(64 * 1024)
            .spawn(move || watch(&stop, deadline))?;
        Ok(Watching(Arc::clone(self)))
    }

    /// The interruption of an `Http` operation in the body of a `for ever` (rule 12).
    pub fn interrupt(&self) -> OpInterrupt<'_> {
        OpInterrupt {
            stop: self,
            stale: Cell::new(false),
        }
    }
}

/// The watch of a run: dropping it ends the run for its watcher ([`RunStop::finish`]).
#[derive(Debug)]
#[must_use = "dropping the guard ends the watch at once"]
pub struct Watching(Arc<RunStop>);

impl Drop for Watching {
    fn drop(&mut self) {
        self.0.finish();
    }
}

/// One slice of a wait without a descriptor, for `left` at most: true when the pipe was found
/// readable. A failing `poll` sleeps the slice out instead, so nothing spins.
fn sleep_slice(wake: bool, left: Duration) -> bool {
    let slice = left.min(SLICE);
    match una_sys::signals::wait(None, false, wake, slice) {
        Readiness::Woken => true,
        Readiness::Failed => {
            std::thread::sleep(slice);
            false
        }
        Readiness::Ready | Readiness::TimedOut => false,
    }
}

/// What the watcher does: waits for the first request, or sends the deadline's; then the
/// grace period, the hard stop, and the second grace period.
fn watch(stop: &RunStop, deadline: Option<Duration>) {
    let start = Instant::now();
    let mut wake = true;
    loop {
        if stop.done() {
            return;
        }
        if stop.stopped() {
            break;
        }
        let left = deadline.map(|deadline| deadline.saturating_sub(start.elapsed()));
        if left.is_some_and(|left| left.is_zero()) {
            stop.request_deadline();
            break;
        }
        // A pipe that another run of the process woke stays readable: wait without it then.
        if sleep_slice(wake, left.unwrap_or(SLICE)) && !stop.stopped() {
            wake = false;
        }
    }
    // The serve engine closes its listeners at once, and refuses what it did not deliver
    // (ADR 0123 rule 18).
    stop.hook.run(false);
    let grace = Duration::from_secs(stop.grace);
    let first = Instant::now();
    let reason = loop {
        if stop.done() {
            return;
        }
        if stop.requests() >= 2 {
            break 2;
        }
        let elapsed = first.elapsed();
        if elapsed >= grace {
            break 1;
        }
        std::thread::sleep((grace - elapsed).min(GRACE_STEP));
    };
    stop.hard.store(reason, Ordering::SeqCst);
    // The serve engine first: from here it refuses the program's answers and answers every open
    // exchange `503` in one write, closing every connection without waiting on a peer (ADR 0123
    // rule 18 item 6), before any task can reach a check point that sees the hard stop.
    stop.hook.run(true);
    // Every check point of the run takes its slow path, and finds the hard stop.
    una_sys::attention::raise_all();
    let hard = Instant::now();
    loop {
        if stop.done() {
            return;
        }
        let elapsed = hard.elapsed();
        if (stop.requests() >= 3 || elapsed >= grace)
            && stop
                .life
                .compare_exchange(RUNNING, KILLING, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            una_sys::signals::restore_and_raise(una_sys::signals::SIGTERM);
        }
        std::thread::sleep(grace.saturating_sub(elapsed).min(GRACE_STEP));
    }
}

/// The interruption of one `Http` operation (rule 12), over the run's stop state.
#[derive(Debug)]
pub struct OpInterrupt<'a> {
    stop: &'a RunStop,
    /// Whether the wake pipe was found readable without a request of this run, as after
    /// another run of the process: the operation then waits on its socket, and looks at the
    /// run's requests between slices.
    stale: Cell<bool>,
}

impl Interrupt for OpInterrupt<'_> {
    /// Each wait really waits, through `poll`.
    fn waits(&self) -> bool {
        true
    }

    fn stopped(&self) -> bool {
        self.stop.stopped()
    }

    /// Ends at `timeout` exactly, not at the next millisecond: `poll` waits whole milliseconds,
    /// rounded up, so it waits only the whole milliseconds left, and the sub-millisecond rest
    /// is slept to the exact instant, then the socket is polled once without waiting, so a
    /// readiness that came during that rest is still seen ([`precise_rest`], #1151).
    fn wait(&self, socket: BorrowedFd<'_>, write: bool, timeout: Duration) -> Waited {
        let end = Instant::now().checked_add(timeout);
        loop {
            if self.stop.stopped() {
                return Waited::Stopped;
            }
            let left = end.map_or(timeout, |end| end.saturating_duration_since(Instant::now()));
            if left.is_zero() {
                return Waited::TimedOut;
            }
            let Some(slice) = whole_millis(left) else {
                precise_rest(end, left);
                if self.stop.stopped() {
                    return Waited::Stopped;
                }
                return match una_sys::signals::wait(
                    Some(socket.as_raw_fd()),
                    write,
                    !self.stale.get(),
                    Duration::ZERO,
                ) {
                    Readiness::Ready => Waited::Ready,
                    Readiness::Failed => Waited::Failed,
                    Readiness::Woken if self.stop.stopped() => Waited::Stopped,
                    Readiness::Woken | Readiness::TimedOut => Waited::TimedOut,
                };
            };
            match una_sys::signals::wait(Some(socket.as_raw_fd()), write, !self.stale.get(), slice)
            {
                Readiness::Ready => return Waited::Ready,
                Readiness::Failed => return Waited::Failed,
                Readiness::TimedOut => {}
                Readiness::Woken if self.stop.stopped() => return Waited::Stopped,
                Readiness::Woken => self.stale.set(true),
            }
        }
    }
}

/// Waits `timeout` for the driver's next tick, unless a host stop request reaches the run
/// first (rule 3): true when the wait ended at the tick. It waits in slices, so a stop is seen
/// within one slice at worst.
///
/// It ends at `timeout` exactly, to the resolution of the system's sleep, not at the next
/// millisecond (ADR 0116 amendment, P13, #1151): `poll` waits whole milliseconds, rounded up,
/// so the slices cover only the whole milliseconds left, and the sub-millisecond rest is slept
/// to the exact instant ([`precise_rest`]). A stop request that comes during that rest is seen
/// at its end: the wait is uninterruptible for under 1 ms.
pub fn wait_tick(stop: &RunStop, timeout: Duration) -> bool {
    let end = Instant::now().checked_add(timeout);
    let mut wake = true;
    loop {
        if stop.stopped() {
            return false;
        }
        let left = end.map_or(timeout, |end| end.saturating_duration_since(Instant::now()));
        if left.is_zero() {
            return true;
        }
        let Some(slice) = whole_millis(left) else {
            precise_rest(end, left);
            continue;
        };
        // A pipe left readable by another run is ignored from then on.
        if sleep_slice(wake, slice) && !stop.stopped() {
            wake = false;
        }
    }
}

/// The whole milliseconds of `left`, at most [`SLICE`]: what a `poll` can wait without
/// passing the instant `left` ends at, since it rounds a wait up to whole milliseconds. `None`
/// when under one millisecond is left.
fn whole_millis(left: Duration) -> Option<Duration> {
    let millis = u64::try_from(left.as_millis()).unwrap_or(u64::MAX);
    (millis > 0).then(|| Duration::from_millis(millis).min(SLICE))
}

/// Sleeps the sub-millisecond rest `left` up to the instant `end`, reading the clock again
/// after each sleep, so the wait ends at `end` exactly, to the resolution of the system's
/// sleep, rather than at the next millisecond. Under 1 ms, and uninterruptible.
fn precise_rest(end: Option<Instant>, left: Duration) {
    let mut left = left;
    while !left.is_zero() {
        std::thread::sleep(left);
        left = end.map_or(Duration::ZERO, |end| {
            end.saturating_duration_since(Instant::now())
        });
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsFd;

    use super::*;

    #[test]
    fn the_runs_own_request_is_sticky_and_local() {
        let stop = RunStop::new(1);
        let other = RunStop::new(1);
        assert!(!stop.stopped());
        stop.request_local();
        assert!(stop.stopped());
        assert!(stop.stopped());
        assert!(!other.stopped());
        assert_eq!(stop.hard(), None);
    }

    #[test]
    fn a_tick_wait_without_a_stop_ends_at_the_tick() {
        let stop = RunStop::new(1);
        let start = Instant::now();
        assert!(wait_tick(&stop, Duration::from_millis(20)));
        assert!(start.elapsed() >= Duration::from_millis(20));
        stop.request_local();
        assert!(!wait_tick(&stop, Duration::from_secs(30)));
    }

    /// The median of `values`, which are not empty.
    fn median(values: &mut [Duration]) -> Duration {
        values.sort_unstable();
        values.get(values.len() / 2).copied().unwrap_or_default()
    }

    /// How much later than the machine's own wake-up latency a wait may end, as a median: half
    /// of what the old rounding to whole milliseconds added to a rest of 300 us (#1151, P13).
    const EXCESS: Duration = Duration::from_micros(350);

    /// The rest of each wait past its whole milliseconds, which the wait sleeps to the instant.
    const REST: Duration = Duration::from_micros(300);

    /// The most rounds of 20 paired waits that [`assert_ends_at_its_instant`] takes.
    const ROUNDS: usize = 5;

    /// Asserts that `wait(timeout)` ends at or after `timeout` in every run, and, as a median,
    /// less than [`EXCESS`] after the machine's own latency (#1151, P13; #1573). That latency
    /// is measured beside the wait: each of 20 waits of 1.3 to 4.3 ms, whose rest past the
    /// whole milliseconds is 300 us, is paired with a plain `std::thread::sleep` of that rest,
    /// [`REST`], just before it, which ends as late as the system wakes a sleeper. A sleep of
    /// the whole timeout is no floor: on macOS its timer slack makes it later than the wait,
    /// which polls the whole milliseconds and sleeps only the rest. On a quiet
    /// machine that is a few tens of microseconds, so the bound is tighter than a fixed one;
    /// on a loaded machine both medians grow alike, while the old rounding, late by about
    /// 700 us in every wait on top of that latency, still fails. A round whose medians miss
    /// the bound is measured again, up to [`ROUNDS`] in all: a burst of load that delays the
    /// waits of one round more than the sleeps beside them is retried, a wait late in every
    /// round is not. The bound is the same in every round, and the message lists them all.
    fn assert_ends_at_its_instant(what: &str, mut wait: impl FnMut(Duration)) {
        let mut rounds: Vec<(Duration, Duration)> = Vec::new();
        for _ in 0..ROUNDS {
            let mut late = Vec::new();
            let mut floor = Vec::new();
            for k in 0..20u32 {
                let timeout = Duration::from_micros(1_300 + u64::from(k % 4) * 1_000);
                let start = Instant::now();
                std::thread::sleep(REST);
                floor.push(start.elapsed().saturating_sub(REST));
                let start = Instant::now();
                wait(timeout);
                let took = start.elapsed();
                assert!(took >= timeout, "{what}: {took:?} before {timeout:?}");
                late.push(took - timeout);
            }
            let (late, floor) = (median(&mut late), median(&mut floor));
            rounds.push((late, floor));
            if late < floor + EXCESS {
                return;
            }
        }
        panic!(
            "{what}: the median wait ended {EXCESS:?} or more past the median plain sleep in \
             each of {ROUNDS} rounds, (wait, sleep) past the instant: {rounds:?}"
        );
    }

    #[test]
    fn the_slices_of_a_wait_never_round_its_rest_up() {
        // #1151, P13, deterministically: a wait is cut into the whole milliseconds that `poll`
        // can wait without passing the instant, at most a slice each, and a rest under 1 ms
        // that is slept to the instant, never a millisecond rounded up.
        assert_eq!(whole_millis(Duration::ZERO), None);
        assert_eq!(whole_millis(Duration::from_micros(300)), None);
        assert_eq!(whole_millis(Duration::from_micros(999)), None);
        assert_eq!(
            whole_millis(Duration::from_millis(1)),
            Some(Duration::from_millis(1))
        );
        assert_eq!(
            whole_millis(Duration::from_micros(1_300)),
            Some(Duration::from_millis(1))
        );
        assert_eq!(
            whole_millis(Duration::from_micros(4_300)),
            Some(Duration::from_millis(4))
        );
        assert_eq!(whole_millis(Duration::from_secs(30)), Some(SLICE));
        // The rest is slept to its instant: never before it.
        let end = Instant::now() + Duration::from_micros(300);
        precise_rest(Some(end), Duration::from_micros(300));
        assert!(Instant::now() >= end);
    }

    #[test]
    fn a_tick_wait_ends_at_its_exact_instant_not_at_the_next_millisecond() {
        // #1151, P13: a wait of whole milliseconds plus a fraction ends at or after its exact
        // instant, and close to it. The old wait polled for the time left rounded up to whole
        // milliseconds, so it ended about 700 us late for a rest of 300 us, by the phase of the
        // tick, plus the system's latency; here the rest is slept to the instant, and only
        // that latency is left, which the judge measures beside the waits (#1573).
        let stop = RunStop::new(1);
        assert_ends_at_its_instant("a tick wait", |timeout| {
            assert!(wait_tick(&stop, timeout));
        });
    }

    #[test]
    fn an_operation_wait_times_out_at_its_exact_instant() {
        // #1151, P13 (M3): an `Http` wait bounded by a deadline, on a socket that never becomes
        // readable, times out at or after its exact instant and close to it, not at the next
        // millisecond, judged as the tick wait is (#1573).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let _server = listener.accept().unwrap();
        let stop = RunStop::new(1);
        let interrupt = stop.interrupt();
        assert_ends_at_its_instant("an operation wait", |timeout| {
            let waited = interrupt.wait(client.as_fd(), false, timeout);
            assert!(matches!(waited, Waited::TimedOut), "{waited:?}");
        });
    }

    #[test]
    fn the_watcher_ends_with_its_run() {
        let stop = RunStop::new(1);
        drop(stop.watch(None).unwrap());
        // The local request after the end starts no grace period: nothing to observe but that
        // the test process lives on.
        stop.request_local();
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(stop.hard(), None);
    }

    #[test]
    fn a_local_request_brings_the_hard_stop_after_the_grace_period() {
        let stop = RunStop::new(1);
        let watching = stop.watch(None).unwrap();
        stop.request_local();
        let start = Instant::now();
        while stop.hard().is_none() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(watching);
        assert_eq!(stop.hard(), Some(HardStop::Grace(1)));
        assert!(start.elapsed() >= Duration::from_millis(900));
    }

    #[test]
    fn a_second_request_brings_the_hard_stop_at_once() {
        let stop = RunStop::new(60);
        let watching = stop.watch(None).unwrap();
        stop.request_local();
        std::thread::sleep(Duration::from_millis(150));
        stop.request_deadline();
        let start = Instant::now();
        while stop.hard().is_none() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(watching);
        assert_eq!(stop.hard(), Some(HardStop::Second));
    }

    #[test]
    fn racing_own_requests_make_one_request_and_no_hard_stop() {
        // S6-2: eight threads that each make the run's own request, and the deadline's, add
        // exactly one request of each kind, so no second request brings the hard stop.
        let stop = RunStop::new(60);
        let watching = stop.watch(None).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..100 {
                        stop.request_local();
                    }
                });
            }
        });
        assert_eq!(stop.requests(), 1);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(stop.hard(), None);
        drop(watching);
    }

    #[test]
    fn a_grace_period_of_zero_counts_as_one_second() {
        // S6-8.
        let stop = RunStop::new(0);
        let watching = stop.watch(None).unwrap();
        stop.request_local();
        let start = Instant::now();
        while stop.hard().is_none() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(watching);
        assert_eq!(stop.hard(), Some(HardStop::Grace(1)));
        assert!(start.elapsed() >= Duration::from_millis(900));
    }

    #[test]
    fn an_ended_run_is_never_killed() {
        // S6-5: the escalation and the end of the run settle in one compare-exchange; once the
        // run has ended, the kill cannot begin.
        let stop = RunStop::new(1);
        stop.finish();
        assert!(
            stop.life
                .compare_exchange(RUNNING, KILLING, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        );
        // A watch whose guard is dropped by a panic ends too.
        let other = RunStop::new(1);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _watching = other.watch(None).unwrap();
            std::panic::panic_any("a panic of the run");
        }));
        assert!(caught.is_err());
        assert!(other.done());
    }
}
