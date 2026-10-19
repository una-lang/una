//! The reactor (ADR 0117 rule 3, slice S7, `docs/plan/0117-s7-design.md` §3 and §4).
//!
//! One thread per process waits on a `kqueue` or `epoll` instance ([`una_sys::poll`]) and wakes
//! the tasks parked on sockets: by readiness, by their deadline, by a host stop, or by a
//! cancellation. `Http` in a task runs on the task's own coroutine, and each wait of its socket
//! parks the task here ([`Parked`]), so a request that waits for its server holds no thread.
//! The reactor runs no Una code and touches no task's state: a wake sends into the mailbox the
//! task parks on ([`sched::channel`]), which pushes a resume job to the task's worker.
//!
//! - **The table.** A slab of waits behind one mutex, with a free list; each wait holds its
//!   wake, its timer in a [`Wheel`], and its links in the list of waits that a host stop
//!   interrupts. A wait's token is its slot and the slot's generation, so an event for a slot
//!   reused since is ignored.
//! - **Registrations are one-shot,** added when a wait begins and deleted when it ends.
//! - **Costs.** Register, wake and deregister are O(1): a slab slot, a wheel timer, a list link
//!   and one system call each. A timer that fires is O(1) amortized ([`Wheel`]); a host stop
//!   wakes the k waits it interrupts in O(k).
//! - **Failing closed.** When the reactor cannot start, `Http` stays on the I/O pool of slice
//!   S1 ([`get`] gives `None`). A wait that cannot be registered fails, and its request with it.

use std::cell::Cell;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use una_http::client::{Error, Interrupt, Waited};
use una_interp::stopping::RunStop;
use una_sys::poll::{Events, Interest, Poller};

use crate::runtime::Cancel;
use crate::sched::{self, Pool, Sender};
use crate::wheel::{TimerId, Wheel};

/// The token of the socket that wakes the reactor's own thread.
const WAKE: u64 = u64::MAX;

/// The token of the pipe of host stops (`una_sys::signals`).
const PIPE: u64 = u64::MAX - 1;

/// The end of a list of slots, and the bound on slots: below it, no token is [`WAKE`] or
/// [`PIPE`].
const NIL: u32 = u32::MAX - 1;

/// The stack of the reactor's thread, which only moves wakes between lists.
const REACTOR_STACK: usize = 256 * 1024;

/// The events one wait of the poller reads at most.
const EVENTS: usize = 256;

/// How a parked wait ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Woken {
    /// The descriptor is ready, has reached its end, or failed: the next try tells.
    Ready,
    /// The wait's time passed.
    TimedOut,
    /// The pipe of host stops was woken: the waiter reads its run's stop.
    Stopped,
    /// A cancellation of the task interrupted the wait (ADR 0117 rule 4, first bullet).
    Cancelled,
    /// The wait could not be registered.
    Failed,
}

/// How to wake a parked wait: the sending end of its mailbox, of which the first send counts.
type Wake = Arc<Sender<Woken>>;

/// The milliseconds since the reactor's clock started, rounded down.
fn millis() -> u64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    let origin = *ORIGIN.get_or_init(Instant::now);
    u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The time on [`millis`] at which a wait of `timeout` that starts now ends: rounded up, one
/// millisecond later, so the reactor never ends a wait early.
fn deadline(timeout: Duration) -> u64 {
    let wanted = u64::try_from(timeout.as_nanos().div_ceil(1_000_000)).unwrap_or(u64::MAX);
    millis().saturating_add(wanted).saturating_add(1)
}

/// One wait of the table.
struct Slot {
    generation: u32,
    used: bool,
    /// Taken by the first event, timer or stop that wakes the wait.
    wake: Option<Wake>,
    timer: Option<TimerId>,
    /// Whether the wait is on the list of those a host stop interrupts, and its links there.
    listed: bool,
    prev: u32,
    next: u32,
}

/// What the pipe of host stops is to the reactor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pipe {
    /// Not registered: no wait has needed it, or the handlers are not installed.
    Unwatched,
    /// Registered: its event wakes every wait on the list.
    Watched,
    /// Its one event came. It stays readable for good, so it is not registered again, and
    /// waits a host stop interrupts look at their run's stop in slices instead.
    Fired,
}

/// The waits of the reactor. Every operation is O(1), amortized for the slab's growth.
struct Table {
    slots: Vec<Slot>,
    free: Vec<u32>,
    wheel: Wheel<(u32, u32)>,
    /// The first wait of the list of those a host stop interrupts, or [`NIL`].
    head: u32,
    pipe: Pipe,
    /// When the reactor's wait ends at the latest, on [`millis`]: a timer due before it wakes
    /// the reactor.
    until: u64,
}

impl Table {
    fn new() -> Table {
        Table {
            slots: Vec::new(),
            free: Vec::new(),
            wheel: Wheel::new(millis()),
            head: NIL,
            pipe: Pipe::Unwatched,
            until: u64::MAX,
        }
    }

    /// Adds a wait with `wake`, due at `at`, on the list of host stops when `listed` and the
    /// pipe is watched; its slot and generation, or `None` when the table or the wheel is full.
    fn insert(&mut self, wake: Wake, at: u64, listed: bool) -> Option<(u32, u32)> {
        let slot = match self.free.pop() {
            Some(slot) => slot,
            None => {
                let slot = u32::try_from(self.slots.len())
                    .ok()
                    .filter(|slot| *slot < NIL)?;
                self.slots.push(Slot {
                    generation: 0,
                    used: false,
                    wake: None,
                    timer: None,
                    listed: false,
                    prev: NIL,
                    next: NIL,
                });
                slot
            }
        };
        let generation = self.slots.get(slot as usize)?.generation;
        let Ok(timer) = self.wheel.arm(at, (slot, generation)) else {
            self.free.push(slot);
            return None;
        };
        let listed = listed && self.pipe == Pipe::Watched;
        let head = self.head;
        let entry = self.slots.get_mut(slot as usize)?;
        entry.used = true;
        entry.wake = Some(wake);
        entry.timer = Some(timer);
        entry.listed = listed;
        entry.prev = NIL;
        entry.next = if listed { head } else { NIL };
        if listed {
            if let Some(next) = self.slots.get_mut(head as usize) {
                next.prev = slot;
            }
            self.head = slot;
        }
        Some((slot, generation))
    }

    /// The slot of a wait that is still in the table.
    fn live(&mut self, slot: u32, generation: u32) -> Option<&mut Slot> {
        self.slots
            .get_mut(slot as usize)
            .filter(|entry| entry.used && entry.generation == generation)
    }

    /// The wake of a wait, taken so that nothing else wakes it; `None` when it was woken or
    /// removed already.
    fn take(&mut self, slot: u32, generation: u32) -> Option<Wake> {
        self.live(slot, generation)?.wake.take()
    }

    /// Takes a wait off the list of host stops.
    fn unlist(&mut self, slot: u32) {
        let Some(entry) = self.slots.get_mut(slot as usize) else {
            return;
        };
        if !entry.listed {
            return;
        }
        let (prev, next) = (entry.prev, entry.next);
        entry.listed = false;
        entry.prev = NIL;
        entry.next = NIL;
        match self.slots.get_mut(prev as usize) {
            Some(before) => before.next = next,
            None => self.head = next,
        }
        if let Some(after) = self.slots.get_mut(next as usize) {
            after.prev = prev;
        }
    }

    /// Removes a wait: its timer, its links, its slot. Nothing when it is gone already.
    fn remove(&mut self, slot: u32, generation: u32) {
        let Some(entry) = self.live(slot, generation) else {
            return;
        };
        let timer = entry.timer.take();
        entry.wake = None;
        if let Some(timer) = timer {
            self.wheel.cancel(timer);
        }
        self.unlist(slot);
        if let Some(entry) = self.slots.get_mut(slot as usize) {
            entry.used = false;
            entry.generation = entry.generation.wrapping_add(1);
            self.free.push(slot);
        }
    }

    /// The wakes of every wait on the list of host stops, which empties: O(k) for its k waits.
    fn stops(&mut self, out: &mut Vec<(Wake, Woken)>) {
        while self.head != NIL {
            let slot = self.head;
            self.unlist(slot);
            if let Some(wake) = self
                .slots
                .get_mut(slot as usize)
                .and_then(|e| e.wake.take())
            {
                out.push((wake, Woken::Stopped));
            }
        }
    }

    /// The wakes of the waits whose time passed by `now`.
    fn expire(&mut self, now: u64, out: &mut Vec<(Wake, Woken)>) {
        let mut fired = Vec::new();
        self.wheel.advance(now, &mut fired);
        for (_, (slot, generation)) in fired {
            if let Some(entry) = self.live(slot, generation) {
                entry.timer = None;
                if let Some(wake) = entry.wake.take() {
                    out.push((wake, Woken::TimedOut));
                }
            }
        }
    }
}

/// The token of a wait.
fn token(slot: u32, generation: u32) -> u64 {
    (u64::from(generation) << 32) | u64::from(slot)
}

/// The reactor of the process: its poller, the socket pair that wakes its thread, and its
/// waits.
pub(crate) struct Reactor {
    poller: Poller,
    nudge: UnixStream,
    nudged: UnixStream,
    table: Mutex<Table>,
}

static REACTOR: OnceLock<Option<Reactor>> = OnceLock::new();

/// The reactor, started at the first call; `None` when it cannot start, and then for good.
pub(crate) fn get() -> Option<&'static Reactor> {
    REACTOR.get_or_init(start).as_ref()
}

fn start() -> Option<Reactor> {
    let poller = Poller::new().ok()?;
    // `std` opens both ends close-on-exec.
    let (nudge, nudged) = UnixStream::pair().ok()?;
    nudge.set_nonblocking(true).ok()?;
    nudged.set_nonblocking(true).ok()?;
    poller.add(nudged.as_fd(), Interest::Read, WAKE).ok()?;
    std::thread::Builder::new()
        .name("una-reactor".to_owned())
        .stack_size(REACTOR_STACK)
        .spawn(serve)
        .ok()?;
    Some(Reactor {
        poller,
        nudge,
        nudged,
        table: Mutex::new(Table::new()),
    })
}

/// The reactor's thread: it starts inside `get_or_init`, so it waits for the reactor to be set.
fn serve() {
    loop {
        match REACTOR.get() {
            Some(Some(reactor)) => return reactor.run(),
            Some(None) => return,
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    }
}

impl Reactor {
    /// Locks the table, recovering it when a thread panicked while holding the lock: every
    /// critical section leaves it consistent.
    fn lock(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wakes the reactor's thread; a full socket already holds a byte that does.
    fn wake_up(&self) {
        let _ = (&self.nudge).write(&[1]);
    }

    /// The reactor's loop: waits for events until the next timer, then sends the wakes of the
    /// waits that are ready, stopped or due, outside the lock.
    fn run(&self) {
        let mut events = Events::with_capacity(EVENTS);
        let mut wakes: Vec<(Wake, Woken)> = Vec::new();
        // Whether a nudge still wakes the thread: its registration is one-shot, and one that
        // cannot be renewed leaves the thread to look at its timers every 10 ms.
        let mut nudgeable = true;
        loop {
            let timeout = {
                let mut table = self.lock();
                let next = table.wheel.next_expiration();
                table.until = if nudgeable {
                    next.unwrap_or(u64::MAX)
                } else {
                    next.unwrap_or(u64::MAX).min(millis().saturating_add(10))
                };
                let until = table.until;
                (until != u64::MAX).then(|| Duration::from_millis(until.saturating_sub(millis())))
            };
            if self.poller.wait(&mut events, timeout).is_err() {
                // A failing wait, as for `ENOMEM`: the timers still fire, a little later.
                std::thread::sleep(Duration::from_millis(10));
            }
            {
                let mut table = self.lock();
                for token in events.tokens() {
                    match token {
                        WAKE => {
                            let mut drain = [0u8; 64];
                            while matches!((&self.nudged).read(&mut drain), Ok(n) if n > 0) {}
                            nudgeable = self
                                .poller
                                .add(self.nudged.as_fd(), Interest::Read, WAKE)
                                .is_ok();
                        }
                        PIPE => {
                            table.pipe = Pipe::Fired;
                            table.stops(&mut wakes);
                        }
                        _ => {
                            let (slot, generation) = (token as u32, (token >> 32) as u32);
                            if let Some(wake) = table.take(slot, generation) {
                                wakes.push((wake, Woken::Ready));
                            }
                        }
                    }
                }
                table.expire(millis(), &mut wakes);
            }
            for (wake, woken) in wakes.drain(..) {
                wake.send(woken);
            }
        }
    }

    /// Registers the pipe of host stops when it is not yet, and says whether its event will
    /// wake the waits a host stop interrupts.
    fn watches_stops(&self) -> bool {
        let mut table = self.lock();
        if table.pipe == Pipe::Unwatched
            && let Some(pipe) = una_sys::signals::wake_pipe()
            && self.poller.add(pipe, Interest::Read, PIPE).is_ok()
        {
            table.pipe = Pipe::Watched;
        }
        table.pipe == Pipe::Watched
    }

    /// Parks the calling task until `fd` is ready for `interest`, `timeout` passes, a host stop
    /// wakes the pipe (when `stop`, which the caller passes only when it saw the pipe watched:
    /// `Stopped` at once when its event has come since), or a cancellation of one of `entries`
    /// comes. O(1), besides the registration of `entries` ([`crate::interrupt`]).
    pub(crate) fn wait(
        &self,
        fd: BorrowedFd<'_>,
        interest: Interest,
        timeout: Duration,
        stop: bool,
        entries: &[Cancel],
    ) -> Woken {
        let (sender, receiver) = sched::channel::<Woken>();
        let wake: Wake = Arc::new(sender);
        let at = deadline(timeout);
        let inserted = {
            let mut table = self.lock();
            // The pipe's event came since the caller saw it watched: the caller reads its run's
            // stop again, and waits in slices from now on.
            if stop && table.pipe != Pipe::Watched {
                return Woken::Stopped;
            }
            let id = table.insert(Arc::clone(&wake), at, stop);
            let early = at < table.until;
            if id.is_some() && early {
                table.until = at;
            }
            id.map(|id| (id, early))
        };
        let Some(((slot, generation), early)) = inserted else {
            return Woken::Failed;
        };
        if early {
            self.wake_up();
        }
        if self
            .poller
            .add(fd, interest, token(slot, generation))
            .is_err()
        {
            self.lock().remove(slot, generation);
            return Woken::Failed;
        }
        let cancel = Arc::clone(&wake);
        let on_cancel: crate::interrupt::Wake = Arc::new(move || cancel.send(Woken::Cancelled));
        let registration = crate::interrupt::register(entries, &on_cancel);
        drop((on_cancel, wake));
        // A cancellation before the registration woke nothing: seen here.
        let woken = if crate::interrupt::cancelled(entries) {
            Woken::Cancelled
        } else {
            receiver.recv().unwrap_or(Woken::Failed)
        };
        drop(registration);
        let _ = self.poller.delete(fd, interest);
        self.lock().remove(slot, generation);
        woken
    }
}

/// Whether `Http` in the calling code parks on the reactor: a task on a worker under the real
/// clock, with a reactor that started.
pub(crate) fn parks() -> Option<&'static Reactor> {
    if sched::in_task() && !crate::runtime::clock().is_virtual() {
        get()
    } else {
        None
    }
}

/// The interruption of a request that parks on the reactor (`docs/plan/0117-s7-design.md` §4):
/// its socket is non-blocking, and each wait parks its task.
pub(crate) struct Parked<'a> {
    reactor: &'static Reactor,
    /// The run's stop, when the request is in the body of a `for ever` (ADR 0116 rule 12).
    stop: Option<&'static RunStop>,
    /// The cancellation entries that interrupt the request (ADR 0117 rule 4).
    entries: &'a [Cancel],
    cancelled: Cell<bool>,
}

impl<'a> Parked<'a> {
    pub(crate) fn new(
        reactor: &'static Reactor,
        stop: Option<&'static RunStop>,
        entries: &'a [Cancel],
    ) -> Parked<'a> {
        Parked {
            reactor,
            stop,
            entries,
            cancelled: Cell::new(false),
        }
    }

    /// Whether a cancellation of the task interrupted the request: it then ends `interrupted`
    /// and the task stops. A request that completed before it saw the cancellation completed
    /// (`spec/19 §6` rule 4), and the task stops at its next check point.
    pub(crate) fn cancelled(&self) -> bool {
        self.cancelled.get()
    }

    /// Reads the task's cancellation entries, and records one that is cancelled.
    fn cut(&self) -> bool {
        if !self.cancelled.get() && crate::interrupt::cancelled(self.entries) {
            self.cancelled.set(true);
        }
        self.cancelled.get()
    }
}

impl Interrupt for Parked<'_> {
    fn waits(&self) -> bool {
        true
    }

    fn stopped(&self) -> bool {
        self.cut() || self.stop.is_some_and(|stop| stop.stopped())
    }

    fn wait(&self, socket: BorrowedFd<'_>, write: bool, timeout: Duration) -> Waited {
        let interest = if write {
            Interest::Write
        } else {
            Interest::Read
        };
        let end = Instant::now().checked_add(timeout);
        loop {
            if self.stopped() {
                return Waited::Stopped;
            }
            let left = end.map_or(timeout, |end| end.saturating_duration_since(Instant::now()));
            if left.is_zero() {
                return Waited::TimedOut;
            }
            // A pipe that no longer tells of stops: the run's stop is read between slices, as
            // `OpInterrupt` does after a wake of another run.
            let watched = self.stop.is_some() && self.reactor.watches_stops();
            let slice = if self.stop.is_some() && !watched {
                left.min(una_interp::stopping::SLICE)
            } else {
                left
            };
            match self
                .reactor
                .wait(socket, interest, slice, watched, self.entries)
            {
                Woken::Ready => return Waited::Ready,
                Woken::Failed => return Waited::Failed,
                Woken::Cancelled => {
                    self.cancelled.set(true);
                    return Waited::Stopped;
                }
                // The loop reads the stop and the time left again.
                Woken::TimedOut | Woken::Stopped => {}
            }
        }
    }

    /// Resolution and connection on a helper of the I/O pool while the task parks (decision
    /// S7-D2): a cancellation releases the task at once, and the helper ends within the
    /// request's deadline, its stream then closed.
    fn open(&self, host: &str, port: u16, deadline: Instant) -> Result<TcpStream, Error> {
        let (host, stop) = (host.to_owned(), self.stop);
        let job = move || {
            let stopped = || stop.is_some_and(|stop| stop.stopped());
            una_http::client::open(&host, port, deadline, &stopped)
        };
        match sched::blocking_interruptible(Pool::Io, self.entries, job) {
            Some(opened) => opened,
            None => {
                self.cancelled.set(true);
                Err(Error::Interrupted)
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn wake() -> Wake {
        Arc::new(sched::channel::<Woken>().0)
    }

    #[test]
    fn the_table_wakes_each_wait_once_and_ignores_stale_tokens() {
        let mut table = Table::new();
        let (slot, generation) = table.insert(wake(), u64::MAX - 1, false).unwrap();
        assert!(table.take(slot, generation).is_some());
        assert!(table.take(slot, generation).is_none());
        table.remove(slot, generation);
        // The slot is reused with a new generation: the old token wakes nothing.
        let (again, next) = table.insert(wake(), u64::MAX - 1, false).unwrap();
        assert_eq!(again, slot);
        assert_ne!(next, generation);
        assert!(table.take(slot, generation).is_none());
        assert!(table.take(again, next).is_some());
        table.remove(again, next);
        // A removed wait's timer does not fire.
        let mut out = Vec::new();
        table.expire(u64::MAX, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn a_host_stop_wakes_the_listed_waits_and_a_timer_the_due_ones() {
        let mut table = Table::new();
        table.pipe = Pipe::Watched;
        let now = millis();
        let ids: Vec<(u32, u32)> = (0..4)
            .map(|i| table.insert(wake(), now + 1_000_000, i % 2 == 0).unwrap())
            .collect();
        // Remove a listed one from the middle of the list.
        let (slot, generation) = ids.get(2).copied().unwrap();
        table.remove(slot, generation);
        let mut out = Vec::new();
        table.stops(&mut out);
        assert_eq!(out.len(), 1);
        assert!(out.iter().all(|(_, woken)| *woken == Woken::Stopped));
        out.clear();
        table.expire(now + 2_000_000, &mut out);
        // The unlisted two, and none of the woken one.
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|(_, woken)| *woken == Woken::TimedOut));
    }

    #[test]
    fn registering_waking_and_removing_scale_linearly() {
        // O(1) per wait: a hundred times the waits takes about a hundred times the time.
        let mut times = Vec::new();
        for count in [1_000usize, 10_000, 100_000] {
            let mut table = Table::new();
            let now = millis();
            let started = Instant::now();
            let ids: Vec<(u32, u32)> = (0..count)
                .map(|i| {
                    table
                        .insert(wake(), now + 60_000 + i as u64, i % 2 == 0)
                        .unwrap()
                })
                .collect();
            for (slot, generation) in &ids {
                drop(table.take(*slot, *generation));
            }
            for (slot, generation) in ids {
                table.remove(slot, generation);
            }
            times.push(started.elapsed().as_secs_f64());
        }
        let small = times.first().copied().unwrap();
        let large = times.last().copied().unwrap();
        assert!(large < small * 400.0 + 0.1, "{times:?}");
    }

    fn settings(workers: usize) -> una_interp::Concurrency {
        una_interp::Concurrency {
            workers,
            task_depth: 10,
            io_threads: 1,
            file_threads: 1,
        }
    }

    /// Runs `body` as a task of `sched` and returns what it sends.
    fn in_task<T: Send + 'static>(
        sched: &sched::Scheduler,
        body: impl FnOnce() -> T + Send + 'static,
    ) -> std::sync::mpsc::Receiver<T> {
        let (sent, seen) = std::sync::mpsc::channel();
        sched
            .spawn(256 * 1024, move || sent.send(body()).unwrap(), || {})
            .unwrap();
        seen
    }

    #[test]
    fn a_parked_task_is_woken_by_readiness_by_its_time_and_by_a_cancellation() {
        let reactor = get().unwrap();
        let sched = sched::Scheduler::new(settings(1));
        let (mut a, b) = UnixStream::pair().unwrap();
        b.set_nonblocking(true).unwrap();
        // Readiness: a byte written while the task waits.
        let waited = in_task(&sched, move || {
            let started = Instant::now();
            let woken = reactor.wait(
                b.as_fd(),
                Interest::Read,
                Duration::from_secs(30),
                false,
                &[],
            );
            (woken, started.elapsed(), b)
        });
        std::thread::sleep(Duration::from_millis(50));
        a.write_all(b"x").unwrap();
        let (woken, took, b) = waited.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(woken, Woken::Ready);
        assert!(took < Duration::from_secs(10), "{took:?}");
        let mut byte = [0u8];
        (&b).read_exact(&mut byte).unwrap();
        // Time: nothing to read, so the wait ends at its timeout, never before.
        let waited = in_task(&sched, move || {
            let started = Instant::now();
            let woken = reactor.wait(
                b.as_fd(),
                Interest::Read,
                Duration::from_millis(80),
                false,
                &[],
            );
            (woken, started.elapsed(), b)
        });
        let (woken, took, b) = waited.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(woken, Woken::TimedOut);
        assert!(took >= Duration::from_millis(80), "{took:?}");
        // A cancellation: the count of a race falls below the task's entry.
        let count = Arc::new(AtomicUsize::new(3));
        let entry = Arc::clone(&count);
        let waited = in_task(&sched, move || {
            let entries = vec![(entry, 2)];
            let woken = reactor.wait(
                b.as_fd(),
                Interest::Read,
                Duration::from_secs(30),
                false,
                &entries,
            );
            (woken, b)
        });
        std::thread::sleep(Duration::from_millis(50));
        count.fetch_min(0, Ordering::SeqCst);
        crate::interrupt::lowered(&count);
        let (woken, _b) = waited.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(woken, Woken::Cancelled);
        drop(a);
    }

    #[test]
    fn a_parked_task_leaves_its_worker_to_the_others() {
        // ADR 0117 rule 3: with one worker, a task parked on a socket that never becomes ready
        // does not delay a task beside it, which ends first.
        let reactor = get().unwrap();
        let sched = sched::Scheduler::new(settings(1));
        let (_a, b) = UnixStream::pair().unwrap();
        let order = Arc::new(Mutex::new(Vec::new()));
        let parked_order = Arc::clone(&order);
        let parked = in_task(&sched, move || {
            let woken = reactor.wait(
                b.as_fd(),
                Interest::Read,
                Duration::from_millis(500),
                false,
                &[],
            );
            parked_order.lock().unwrap().push("parked");
            woken
        });
        let beside_order = Arc::clone(&order);
        let beside = in_task(&sched, move || beside_order.lock().unwrap().push("beside"));
        beside.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(
            parked.recv_timeout(Duration::from_secs(10)).unwrap(),
            Woken::TimedOut
        );
        assert_eq!(*order.lock().unwrap(), ["beside", "parked"]);
    }
}
