//! The host stop of a run (ADR 0116 rules 14, 16 and 17, `spec/26 §6`), which `una run` and
//! native executables share: the requests that reach the run, the grace period, the hard stop
//! and the end of a run that does not stop.
//!
//! A request is SIGINT or SIGTERM, which `una-sys` counts for the process; the end of
//! `--deadline`, which the watcher sends as one; or the run's own stop, by
//! `--stop-after-iterations`, which is local to the run. Requests are sticky. The first one
//! starts the grace period; the hard stop comes at its end, or at a second request; a third
//! request, or the end of a second grace period after the hard stop, ends the process with
//! SIGTERM's default action.

use std::cell::Cell;
use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use una_http::client::{Interrupt, Waited};
use una_sys::signals::Readiness;

/// The grace period when `--stop-grace` is not given, in seconds (rule 16).
pub const DEFAULT_GRACE_SECS: u64 = 10;

/// The longest `--deadline`, a year of seconds (`spec/13 §5`).
pub const MAX_DEADLINE_SECS: u64 = 31_536_000;

/// The longest `--stop-grace`, a day of seconds (`spec/13 §5`).
pub const MAX_GRACE_SECS: u64 = 86_400;

/// How often the watcher looks at the run's own requests and at its end while it waits for a
/// first request. A signal or the deadline wakes it at once, through the wake pipe; but the
/// run's own requests (`--stop-after-iterations`) do not write the pipe, which would stay
/// readable for every later run of the process, and the end of the run has to be seen too, so
/// the watcher also wakes every 100 ms: an O(1) look, ten times a second, while it waits.
const IDLE_STEP: Duration = Duration::from_millis(100);

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

/// The host stop state of one run.
#[derive(Debug)]
pub struct RunStop {
    /// The process's requests when the run started: those after it are the run's.
    baseline: u32,
    /// The run's own requests, by `--stop-after-iterations`.
    local: AtomicU32,
    /// 0 before the hard stop; 1 when the grace period ended; 2 at a second request.
    hard: AtomicU8,
    /// The grace period, in seconds.
    grace: u64,
    /// Whether the run has ended, which ends its watcher.
    done: AtomicBool,
}

impl RunStop {
    /// The state of a run that starts now, with a grace period of `grace` seconds.
    pub fn new(grace: u64) -> Arc<RunStop> {
        Arc::new(RunStop {
            baseline: una_sys::attention::stop_requests(),
            local: AtomicU32::new(0),
            hard: AtomicU8::new(0),
            grace,
            done: AtomicBool::new(false),
        })
    }

    /// The host stop requests that have reached the run so far.
    pub fn requests(&self) -> u32 {
        una_sys::attention::stop_requests()
            .saturating_sub(self.baseline)
            .saturating_add(self.local.load(Ordering::SeqCst))
    }

    /// Whether a host stop request has reached the run: sticky (rule 14).
    pub fn stopped(&self) -> bool {
        self.requests() > 0
    }

    /// The run's own stop request, by `--stop-after-iterations` (rule 19).
    pub fn request_local(&self) {
        self.local.fetch_add(1, Ordering::SeqCst);
    }

    /// The hard stop, once it has come (rule 16).
    pub fn hard(&self) -> Option<HardStop> {
        match self.hard.load(Ordering::SeqCst) {
            1 => Some(HardStop::Grace(self.grace)),
            2 => Some(HardStop::Second),
            _ => None,
        }
    }

    /// Marks the run as ended: its watcher ends at its next look.
    pub fn finish(&self) {
        self.done.store(true, Ordering::SeqCst);
    }

    fn done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    /// Starts the watcher of the run, a thread that sends the request of `deadline` when it
    /// passes, and brings the hard stop and the end of the process when they are due. It runs
    /// no Una code and ends when the run does ([`RunStop::finish`]).
    pub fn watch(self: &Arc<Self>, deadline: Option<Duration>) -> std::io::Result<()> {
        let stop = Arc::clone(self);
        std::thread::Builder::new()
            .name("una-stop".to_owned())
            .stack_size(64 * 1024)
            .spawn(move || watch(&stop, deadline))
            .map(drop)
    }

    /// The interruption of an `Http` operation in the body of a `for ever` (rule 12).
    pub fn interrupt(&self) -> OpInterrupt<'_> {
        OpInterrupt {
            stop: self,
            stale: Cell::new(false),
        }
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
            una_sys::signals::request_host_stop();
            break;
        }
        let step = left.map_or(IDLE_STEP, |left| left.min(IDLE_STEP));
        // A pipe that another run of the process woke stays readable: wait without it then.
        if una_sys::signals::wait(None, false, wake, step) == Readiness::Woken && !stop.stopped() {
            wake = false;
        }
    }
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
    // Every check point of the run takes its slow path, and finds the hard stop.
    una_sys::attention::raise_all();
    let hard = Instant::now();
    loop {
        if stop.done() {
            return;
        }
        let elapsed = hard.elapsed();
        if stop.requests() >= 3 || elapsed >= grace {
            una_sys::signals::restore_and_raise(una_sys::signals::SIGTERM);
        }
        std::thread::sleep((grace - elapsed).min(GRACE_STEP));
    }
}

/// The interruption of one `Http` operation (rule 12), over the run's stop state.
#[derive(Debug)]
pub struct OpInterrupt<'a> {
    stop: &'a RunStop,
    /// Whether the wake pipe was found readable without a request of this run, as after
    /// another run of the process: the operation then waits on its socket alone.
    stale: Cell<bool>,
}

impl Interrupt for OpInterrupt<'_> {
    fn stopped(&self) -> bool {
        self.stop.stopped()
    }

    fn wait(&self, socket: RawFd, write: bool, timeout: Duration) -> Waited {
        let end = Instant::now().checked_add(timeout);
        loop {
            let left = end.map_or(timeout, |end| end.saturating_duration_since(Instant::now()));
            match una_sys::signals::wait(Some(socket), write, !self.stale.get(), left) {
                Readiness::Ready => return Waited::Ready,
                Readiness::TimedOut => return Waited::TimedOut,
                Readiness::Woken if self.stop.stopped() => return Waited::Stopped,
                Readiness::Woken => self.stale.set(true),
            }
        }
    }
}

/// Waits `timeout` for the driver's next tick, unless a host stop request reaches the run
/// first (rule 3): true when the wait ended at the tick.
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
        // The run's own requests come only at iteration starts, so a wait with no signal and
        // no deadline sleeps to the tick; a pipe left readable by another run is ignored.
        match una_sys::signals::wait(None, false, wake, left) {
            Readiness::Woken if !stop.stopped() => wake = false,
            Readiness::Woken | Readiness::Ready => {}
            Readiness::TimedOut => return !stop.stopped(),
        }
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn the_watcher_ends_with_its_run() {
        let stop = RunStop::new(1);
        stop.watch(None).unwrap();
        stop.finish();
        // The local request after the end starts no grace period: nothing to observe but that
        // the test process lives on.
        stop.request_local();
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(stop.hard(), None);
    }

    #[test]
    fn a_local_request_brings_the_hard_stop_after_the_grace_period() {
        let stop = RunStop::new(1);
        stop.watch(None).unwrap();
        stop.request_local();
        let start = Instant::now();
        while stop.hard().is_none() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        stop.finish();
        assert_eq!(stop.hard(), Some(HardStop::Grace(1)));
        assert!(start.elapsed() >= Duration::from_millis(900));
    }

    #[test]
    fn a_second_request_brings_the_hard_stop_at_once() {
        let stop = RunStop::new(60);
        stop.watch(None).unwrap();
        stop.request_local();
        std::thread::sleep(Duration::from_millis(150));
        stop.request_local();
        let start = Instant::now();
        while stop.hard().is_none() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        stop.finish();
        assert_eq!(stop.hard(), Some(HardStop::Second));
    }
}
