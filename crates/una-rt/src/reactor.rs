//! The reactor (ADR 0117 rule 3, slice S7, `docs/plan/0117-s7-design.md` §3 and §4).
//!
//! One thread per process waits on a `kqueue` or `epoll` instance ([`una_sys::poll`]) and wakes
//! the tasks parked on it: by a socket's readiness, by their deadline, by their run's host stop,
//! by a cancellation, or by a party that hands them something (a place among the request
//! sockets of [`crate::sockets`], or a connection made on a helper). `Http` in a task runs on
//! the task's own coroutine, and each wait of its socket parks the task here ([`Parked`]), so a
//! request that waits for its server holds no thread. The reactor runs no Una code and touches
//! no task's state: a wake sends into the mailbox the task parks on ([`sched::channel`]), which
//! pushes a resume job to the task's worker.
//!
//! - **The table.** A slab of waits behind one mutex, with a free list; each wait holds its
//!   wake, its timer in a [`Wheel`], and, when its run's host stop interrupts it, its run and
//!   its links in the list of such waits. A wait's token is its slot and the slot's
//!   generation, so an event for a slot reused since is ignored.
//! - **Registrations are one-shot,** added when a wait begins and deleted when it ends.
//! - **Host stops.** The pipe of host stops is registered once; its event wakes the listed
//!   waits whose run has stopped. It stays readable for good after it, so its registration is
//!   then deleted, and the reactor itself looks at the listed waits' runs every 100 ms, keeping
//!   the time of its last look, without waking the others. A run's own stop (its deadline, or
//!   `--stop-after-iterations`) writes the pipe too (`RunStop::request_local`).
//! - **Costs.** Register, wake and deregister are O(1): a slab slot, a wheel timer, a list link
//!   and one system call each. A timer that fires is O(1) amortized ([`Wheel`]). A look at the
//!   listed waits is O(k) for their k, on the reactor's thread only.
//! - **Failing closed.** When the reactor cannot start, `Http` stays on the I/O pool of slice
//!   S1 ([`get`] gives `None`), and the start is tried again at most once a second. A wait that
//!   cannot be registered fails, and its request with it.

use std::cell::{Cell, RefCell};
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
use crate::sockets::{self, Permit};
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

/// How often the reactor looks at the runs of the listed waits once the pipe no longer tells
/// of stops, in milliseconds: the slice of `una_interp::stopping`.
const LOOK: u64 = 100;

/// The least time between two tries to start the reactor.
const RETRY: Duration = Duration::from_secs(1);

/// How a parked wait ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Woken {
    /// The descriptor is ready, has reached its end, or failed: the next try tells. For a wait
    /// without a descriptor, the party it waited for woke it.
    Ready,
    /// The wait's time passed.
    TimedOut,
    /// The wait's run has stopped (ADR 0116 rule 12).
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
    /// The run whose host stop interrupts the wait, when it is on the list of such waits, and
    /// its links there.
    stop: Option<&'static RunStop>,
    prev: u32,
    next: u32,
}

/// What the pipe of host stops is to the reactor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pipe {
    /// Not registered: no wait has needed it, or the handlers are not installed.
    Unwatched,
    /// Registered: its event makes the reactor look at the listed waits' runs.
    Watched,
    /// Its one event came, and its registration was deleted: it stays readable for good, so
    /// the reactor looks at the listed waits' runs every [`LOOK`] milliseconds instead.
    Fired,
}

/// The waits of the reactor. Every operation is O(1), amortized for the slab's growth, but a
/// look at the listed waits, O(k) for their k.
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

    /// Adds a wait with `wake`, due at `at`, on the list of host stops when `stop` names its
    /// run; its slot and generation, or `None` when the table or the wheel is full.
    fn insert(
        &mut self,
        wake: Wake,
        at: u64,
        stop: Option<&'static RunStop>,
    ) -> Option<(u32, u32)> {
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
                    stop: None,
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
        let head = self.head;
        let entry = self.slots.get_mut(slot as usize)?;
        entry.used = true;
        entry.wake = Some(wake);
        entry.timer = Some(timer);
        entry.stop = stop;
        entry.prev = NIL;
        entry.next = if stop.is_some() { head } else { NIL };
        if stop.is_some() {
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
        if entry.stop.take().is_none() {
            return;
        }
        let (prev, next) = (entry.prev, entry.next);
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

    /// The wakes of the listed waits whose run has stopped, which leave the list: O(k) for the
    /// k listed waits.
    fn stopped(&mut self, out: &mut Vec<(Wake, Woken)>) {
        let mut slot = self.head;
        while slot != NIL {
            let Some(entry) = self.slots.get(slot as usize) else {
                return;
            };
            let next = entry.next;
            if entry.stop.is_some_and(RunStop::stopped) {
                self.unlist(slot);
                if let Some(wake) = self
                    .slots
                    .get_mut(slot as usize)
                    .and_then(|e| e.wake.take())
                {
                    out.push((wake, Woken::Stopped));
                }
            }
            slot = next;
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

static REACTOR: OnceLock<Reactor> = OnceLock::new();

/// When the last start of the reactor failed.
static TRIED: Mutex<Option<Instant>> = Mutex::new(None);

/// The reactor, started at the first call; `None` when it cannot start, tried again at most
/// once every [`RETRY`].
pub(crate) fn get() -> Option<&'static Reactor> {
    if let Some(reactor) = REACTOR.get() {
        return Some(reactor);
    }
    let mut tried = TRIED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(reactor) = REACTOR.get() {
        return Some(reactor);
    }
    if tried.is_some_and(|at| at.elapsed() < RETRY) {
        return None;
    }
    *tried = Some(Instant::now());
    let reactor = parts()?;
    // The thread waits for the reactor to be set, which only this call does, under `TRIED`.
    std::thread::Builder::new()
        .name("una-reactor".to_owned())
        .stack_size(REACTOR_STACK)
        .spawn(serve)
        .ok()?;
    let _ = REACTOR.set(reactor);
    *tried = None;
    REACTOR.get()
}

/// The poller and the socket pair of a new reactor, its nudge registered.
fn parts() -> Option<Reactor> {
    let poller = Poller::new().ok()?;
    // `std` opens both ends close-on-exec.
    let (nudge, nudged) = UnixStream::pair().ok()?;
    nudge.set_nonblocking(true).ok()?;
    nudged.set_nonblocking(true).ok()?;
    poller.add(nudged.as_fd(), Interest::Read, WAKE).ok()?;
    Some(Reactor {
        poller,
        nudge,
        nudged,
        table: Mutex::new(Table::new()),
    })
}

/// The reactor's thread: it starts before the reactor is set, so it waits for it.
fn serve() {
    loop {
        if let Some(reactor) = REACTOR.get() {
            return reactor.run();
        }
        std::thread::sleep(Duration::from_millis(1));
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
        // When the reactor last looked at the listed waits' runs (review R7-12).
        let mut looked = millis();
        loop {
            let timeout = {
                let mut table = self.lock();
                let mut until = table.wheel.next_expiration().unwrap_or(u64::MAX);
                if !nudgeable {
                    until = until.min(millis().saturating_add(10));
                }
                if table.pipe != Pipe::Watched && table.head != NIL {
                    until = until.min(looked.saturating_add(LOOK));
                }
                table.until = until;
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
                            // On `epoll` a one-shot registration stays until deleted.
                            let socket = self.nudged.as_fd();
                            let _ = self.poller.delete(socket, Interest::Read);
                            nudgeable = self.poller.add(socket, Interest::Read, WAKE).is_ok();
                        }
                        PIPE => {
                            table.pipe = Pipe::Fired;
                            if let Some(pipe) = una_sys::signals::wake_pipe() {
                                let _ = self.poller.delete(pipe, Interest::Read);
                            }
                            table.stopped(&mut wakes);
                            looked = millis();
                        }
                        _ => {
                            let (slot, generation) = (token as u32, (token >> 32) as u32);
                            if let Some(wake) = table.take(slot, generation) {
                                wakes.push((wake, Woken::Ready));
                            }
                        }
                    }
                }
                // Every [`LOOK`] milliseconds, not at every pass of the loop.
                if table.pipe != Pipe::Watched && millis() >= looked.saturating_add(LOOK) {
                    table.stopped(&mut wakes);
                    looked = millis();
                }
                table.expire(millis(), &mut wakes);
            }
            for (wake, woken) in wakes.drain(..) {
                wake.send(woken);
            }
        }
    }

    /// Parks the calling task until `fd` is ready (when given), `timeout` passes, the run of
    /// `stop` stops (when given), a cancellation of one of `entries` comes, or the party that
    /// `arm` hands its wake to wakes it (`Ready`). `arm` runs once the wait is registered; it
    /// returns true when what the wait is for has come already, and the task does not park.
    /// O(1), besides the registration of `entries` ([`crate::interrupt`]).
    fn wait_on(
        &self,
        fd: Option<(BorrowedFd<'_>, Interest)>,
        timeout: Duration,
        stop: Option<&'static RunStop>,
        entries: &[Cancel],
        arm: impl FnOnce(&crate::interrupt::Wake) -> bool,
    ) -> Woken {
        let (sender, receiver) = sched::channel::<Woken>();
        let wake: Wake = Arc::new(sender);
        let at = deadline(timeout);
        let inserted = {
            let mut table = self.lock();
            if stop.is_some()
                && table.pipe == Pipe::Unwatched
                && let Some(pipe) = una_sys::signals::wake_pipe()
                && self.poller.add(pipe, Interest::Read, PIPE).is_ok()
            {
                table.pipe = Pipe::Watched;
            }
            let id = table.insert(Arc::clone(&wake), at, stop);
            // A listed wait while the pipe does not watch needs the reactor's look within
            // [`LOOK`]: the reactor is woken when it sleeps longer, to shorten its sleep.
            let look = millis().saturating_add(LOOK);
            let looking = stop.is_some() && table.pipe != Pipe::Watched && table.until > look;
            let early = at < table.until || looking;
            if id.is_some() && early {
                table.until = table
                    .until
                    .min(at)
                    .min(if looking { look } else { u64::MAX });
            }
            id.map(|id| (id, early))
        };
        let Some(((slot, generation), early)) = inserted else {
            return Woken::Failed;
        };
        if early {
            self.wake_up();
        }
        if let Some((fd, interest)) = fd
            && self
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
        let ready_wake = Arc::clone(&wake);
        let ready: crate::interrupt::Wake = Arc::new(move || ready_wake.send(Woken::Ready));
        drop((on_cancel, wake));
        // A cancellation before the registration woke nothing: seen here.
        let woken = if crate::interrupt::cancelled(entries) {
            Woken::Cancelled
        } else if arm(&ready) {
            Woken::Ready
        } else {
            drop(ready);
            receiver.recv().unwrap_or(Woken::Failed)
        };
        drop(registration);
        if let Some((fd, interest)) = fd {
            let _ = self.poller.delete(fd, interest);
        }
        self.lock().remove(slot, generation);
        woken
    }

    /// [`Reactor::wait_on`] for a descriptor, with nothing else to arm.
    pub(crate) fn wait(
        &self,
        fd: BorrowedFd<'_>,
        interest: Interest,
        timeout: Duration,
        stop: Option<&'static RunStop>,
        entries: &[Cancel],
    ) -> Woken {
        self.wait_on(Some((fd, interest)), timeout, stop, entries, |_| false)
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

/// What a connection job on a helper hands back: the connection, or why there is none, with
/// the request's place among the sockets, so that a connection its task no longer waits for
/// is closed before its place is given back.
type Opened = std::thread::Result<(Result<TcpStream, Error>, Arc<Permit>)>;

/// The interruption of a request that parks on the reactor (`docs/plan/0117-s7-design.md` §4):
/// its socket is non-blocking, and each wait parks its task.
pub(crate) struct Parked<'a> {
    reactor: &'static Reactor,
    /// The run's stop, when the request is in the body of a `for ever` (ADR 0116 rule 12).
    stop: Option<&'static RunStop>,
    /// The cancellation entries that interrupt the request (ADR 0117 rule 4).
    entries: &'a [Cancel],
    cancelled: Cell<bool>,
    /// The request's place among the sockets of the process (`crate::sockets`), taken before
    /// its first connection and kept until the request ends.
    place: RefCell<Option<Arc<Permit>>>,
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
            place: RefCell::new(None),
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

    /// The failure of a wait that ended without what it waited for.
    fn failure(&self, woken: Woken) -> Error {
        match woken {
            Woken::TimedOut => Error::TimedOut,
            Woken::Cancelled => {
                self.cancelled.set(true);
                Error::Interrupted
            }
            Woken::Stopped => Error::Interrupted,
            Woken::Ready | Woken::Failed => Error::Unreachable,
        }
    }

    /// The request's place among the sockets, waiting for one first come, first served until
    /// `deadline` when none is free; the wait is cut by a cancellation, not by a host stop,
    /// which takes effect when the connection step ends, as for resolution (ADR 0116,
    /// implementation plan, item 6).
    fn hold_place(&self, deadline: Instant) -> Result<Arc<Permit>, Error> {
        if let Some(place) = self.place.borrow().as_ref() {
            return Ok(Arc::clone(place));
        }
        let gate = sockets::gate();
        let place = match gate.try_take() {
            Some(place) => place,
            None => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(Error::TimedOut);
                }
                let waiter = sockets::waiter();
                // `None` until the wait has armed: a waiter that never joined the line does
                // not leave it.
                let mut armed: Option<Option<Permit>> = None;
                let woken = self
                    .reactor
                    .wait_on(None, left, None, self.entries, |ready| {
                        let taken = gate.take_or_wait(&waiter, ready);
                        let now = taken.is_some();
                        armed = Some(taken);
                        now
                    });
                let place = match armed {
                    None => None,
                    Some(Some(place)) => Some(place),
                    Some(None) => gate.settle(&waiter),
                };
                match place {
                    Some(place) if woken != Woken::Cancelled => place,
                    _ => return Err(self.failure(woken)),
                }
            }
        };
        let place = Arc::new(place);
        *self.place.borrow_mut() = Some(Arc::clone(&place));
        Ok(place)
    }
}

impl Interrupt for Parked<'_> {
    fn waits(&self) -> bool {
        true
    }

    fn stopped(&self) -> bool {
        self.cut() || self.stop.is_some_and(RunStop::stopped)
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
            match self
                .reactor
                .wait(socket, interest, left, self.stop, self.entries)
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

    /// Resolution and connection on a helper of the I/O pool while the task parks on the
    /// reactor (decision S7-D2), after the request has taken its place among the sockets: its
    /// deadline covers the wait in the pool's queue, and a cancellation releases the task at
    /// once. The job reads the task's cancellation entries and the deadline before it starts
    /// anything, so a job whose task gave up ends at once; one that connected for a task that
    /// left closes its connection before its place is given back.
    fn open(&self, host: &str, port: u16, deadline: Instant) -> Result<TcpStream, Error> {
        if self.stopped() {
            return Err(Error::Interrupted);
        }
        let place = self.hold_place(deadline)?;
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(Error::TimedOut);
        }
        let result: Arc<Mutex<Option<Opened>>> = Arc::new(Mutex::new(None));
        let (slot, entries, stop, host) = (
            Arc::clone(&result),
            self.entries.to_vec(),
            self.stop,
            host.to_owned(),
        );
        let job = move |done: crate::interrupt::Wake| {
            let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let stopped =
                    || crate::interrupt::cancelled(&entries) || stop.is_some_and(RunStop::stopped);
                (
                    una_http::client::open(&host, port, deadline, &stopped),
                    place,
                )
            }));
            *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(opened);
            done();
        };
        let woken = self
            .reactor
            .wait_on(None, left, None, self.entries, |done| {
                let done = Arc::clone(done);
                let boxed: Box<dyn FnOnce() + Send> = Box::new(move || job(done));
                // A pool that could start no thread runs nothing: the job runs here, as inline.
                match sched::submit(Pool::Io, boxed) {
                    Ok(()) => false,
                    Err(boxed) => {
                        boxed();
                        true
                    }
                }
            });
        if woken != Woken::Ready {
            return Err(self.failure(woken));
        }
        let opened = result.lock().unwrap_or_else(PoisonError::into_inner).take();
        match opened {
            Some(Ok((connected, _place))) => connected,
            Some(Err(payload)) => std::panic::resume_unwind(payload),
            None => Err(Error::Unreachable),
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

    /// A run's stop that lives for the process, as the runtime's does.
    fn run_stop() -> &'static RunStop {
        let leaked: &'static Arc<RunStop> = Box::leak(Box::new(RunStop::new(1)));
        leaked
    }

    #[test]
    fn the_table_wakes_each_wait_once_and_ignores_stale_tokens() {
        let mut table = Table::new();
        let (slot, generation) = table.insert(wake(), u64::MAX - 1, None).unwrap();
        assert!(table.take(slot, generation).is_some());
        assert!(table.take(slot, generation).is_none());
        table.remove(slot, generation);
        // The slot is reused with a new generation: the old token wakes nothing.
        let (again, next) = table.insert(wake(), u64::MAX - 1, None).unwrap();
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
    fn a_host_stop_wakes_the_waits_of_its_run_only_and_a_timer_the_due_ones() {
        let mut table = Table::new();
        let (stopping, going) = (run_stop(), run_stop());
        let now = millis();
        let runs = [
            Some(stopping),
            None,
            Some(stopping),
            Some(going),
            Some(stopping),
        ];
        let ids: Vec<(u32, u32)> = runs
            .iter()
            .map(|run| table.insert(wake(), now + 1_000_000, *run).unwrap())
            .collect();
        // Remove a listed one from the middle of the list.
        let (slot, generation) = ids.get(2).copied().unwrap();
        table.remove(slot, generation);
        let mut out = Vec::new();
        table.stopped(&mut out);
        assert!(out.is_empty());
        stopping.request_local();
        table.stopped(&mut out);
        // Waits 0 and 4: the other run's wait stays listed.
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|(_, woken)| *woken == Woken::Stopped));
        assert_ne!(table.head, NIL);
        out.clear();
        table.stopped(&mut out);
        assert!(out.is_empty());
        table.expire(now + 2_000_000, &mut out);
        // Waits 1 and 3, and none of those woken already.
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|(_, woken)| *woken == Woken::TimedOut));
    }

    #[test]
    fn registering_waking_and_removing_scale_linearly() {
        // O(1) per wait: a hundred times the waits takes about a hundred times the time.
        let stop = run_stop();
        let mut times = Vec::new();
        for count in [1_000usize, 10_000, 100_000] {
            let mut table = Table::new();
            let now = millis();
            let started = Instant::now();
            let ids: Vec<(u32, u32)> = (0..count)
                .map(|i| {
                    let run = (i % 2 == 0).then_some(stop);
                    table.insert(wake(), now + 60_000 + i as u64, run).unwrap()
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
            .spawn(1, 256 * 1024, move || sent.send(body()).unwrap(), || {})
            .unwrap();
        seen
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
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
                None,
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
                None,
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
                None,
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
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn a_wait_of_a_run_that_stops_is_woken_without_its_descriptor() {
        // A run whose stop comes while its request waits, with no pipe event (the handlers are
        // not installed in unit tests): the reactor's look at the listed waits wakes it.
        let reactor = get().unwrap();
        let sched = sched::Scheduler::new(settings(1));
        let (_a, b) = UnixStream::pair().unwrap();
        let stop = run_stop();
        let waited = in_task(&sched, move || {
            reactor.wait(
                b.as_fd(),
                Interest::Read,
                Duration::from_secs(30),
                Some(stop),
                &[],
            )
        });
        std::thread::sleep(Duration::from_millis(50));
        stop.request_local();
        let woken = waited.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(woken, Woken::Stopped);
    }

    /// The environment variable that marks the process a scenario runs in: a scenario does
    /// nothing in a process that did not start it for that purpose, the shared one included.
    const SCENARIO: &str = "UNA_RT_REACTOR_SCENARIO";

    /// Whether this process was started to run the scenario `name`.
    #[allow(clippy::disallowed_methods)]
    fn running_scenario(name: &str) -> bool {
        std::env::var_os(SCENARIO).is_some_and(|value| value == name)
    }

    #[test]
    #[cfg_attr(miri, ignore = "runs a process of its own, which Miri cannot")]
    fn a_run_s_own_stop_wakes_its_parked_waits_with_the_handlers_installed() {
        // Review R7-11: with the handlers installed the pipe is watched, and the reactor does
        // not look at the runs of listed waits; the run's own stop, as at
        // `--stop-after-iterations`, writes the pipe, so the wait wakes at once. The scenario
        // installs the handlers of SIGINT and SIGTERM, so it runs in a process of its own,
        // and the handlers of this process stay as they are.
        let name = "own_stop_with_the_handlers_installed";
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                &format!("reactor::tests::{name}"),
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .env(SCENARIO, name)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{output:?}"
        );
    }

    #[test]
    #[ignore = "a scenario run in a process of its own by the test above"]
    fn own_stop_with_the_handlers_installed() {
        if !running_scenario("own_stop_with_the_handlers_installed") {
            return;
        }
        una_sys::signals::install().unwrap();
        let reactor = get().unwrap();
        let sched = sched::Scheduler::new(settings(1));
        let (_a, b) = UnixStream::pair().unwrap();
        let stop = run_stop();
        let waited = in_task(&sched, move || {
            let started = Instant::now();
            let woken = reactor.wait(
                b.as_fd(),
                Interest::Read,
                Duration::from_secs(30),
                Some(stop),
                &[],
            );
            (woken, started.elapsed())
        });
        std::thread::sleep(Duration::from_millis(50));
        // The pipe is watched: the wait is listed, and only the pipe's event looks at it.
        assert_eq!(reactor.lock().pipe, Pipe::Watched);
        stop.request_local();
        let (woken, took) = waited.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(woken, Woken::Stopped);
        assert!(took < Duration::from_secs(5), "{took:?}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
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
                None,
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

    #[test]
    #[cfg_attr(miri, ignore = "switches coroutines in assembly, which Miri cannot")]
    fn a_wait_without_a_descriptor_ends_when_its_party_wakes_it() {
        // The shape of a wait for a place among the sockets, or for a connection job.
        let reactor = get().unwrap();
        let sched = sched::Scheduler::new(settings(1));
        let waited = in_task(&sched, move || {
            reactor.wait_on(None, Duration::from_secs(30), None, &[], |ready| {
                let ready = Arc::clone(ready);
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(50));
                    ready();
                });
                false
            })
        });
        assert_eq!(
            waited.recv_timeout(Duration::from_secs(10)).unwrap(),
            Woken::Ready
        );
        let at_once = in_task(&sched, move || {
            reactor.wait_on(None, Duration::from_secs(30), None, &[], |_| true)
        });
        assert_eq!(
            at_once.recv_timeout(Duration::from_secs(10)).unwrap(),
            Woken::Ready
        );
    }
}
