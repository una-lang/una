//! Limits on `Http` grants (ADR 0117 rule 8, `spec/27 §9`), enforced in the host layer that both
//! engines share, below every handler and rebinding.
//!
//! A [`Governor`] is made once per run from the run's `Http` grants, and every request of the run
//! goes through it ([`crate::client::send_governed`]). Each hop of a request, its first and each
//! redirect it follows, asks it for a [`Permit`] before its exchange and drops the permit once its
//! response is read, so a hop never holds a slot while the next hop waits (rule 8, No deadlock,
//! item 1).
//!
//! - **Charging.** A hop is charged to every grant that allows its target with its method and
//!   carries a rate or a count in flight: one GCRA update and one count per grant, all at once
//!   under the governor's lock, or none.
//! - **GCRA.** A rate of `n` per period `T` lets a hop through at time `t` when
//!   `t ≥ TAT − (T − T/n)`, and then sets `TAT = max(TAT, t) + T/n`: a burst of `n`, then one
//!   every `T/n`. Times are counted in units of `1/n` nanoseconds, in 128-bit integers, so `T/n`
//!   is exact for every `n`. The clock is the run's ([`RunClock`]): the monotonic clock from the
//!   run's start, or the virtual clock of `--virtual-time` (ADR 0116 rule 19), read in
//!   milliseconds.
//! - **Virtual time.** A wait for a refill is a wait on time. Under virtual time a host operation
//!   runs on its task's thread without parking, in both engines (ADR 0117, amendment on S4, point
//!   4), so while it waits nothing else of the run can run: the wait moves the virtual clock to
//!   the refill at once, or to the request's deadline on the run's clock when that comes first,
//!   as the wait for a deadline does. So limits give the same outcomes and readings of
//!   `Clock.now` in both engines, and take no wall time.
//! - **Waiting.** A hop that cannot be charged at once, or that would pass a waiter, takes the next
//!   ticket and joins the queue of each limited grant it needs; a full queue fails it with
//!   [`Error::RateLimited`]. It is served when it is the first live waiter of each of those
//!   queues and each grant has a token and a slot. So the waiter with the lowest ticket waits only
//!   for capacity, and none starves (item 3). A waiter holds nothing, and a holder never waits, so
//!   no cycle of waits can form (item 2).
//! - **Ending a wait.** The request's deadline ([`Error::TimedOut`]), a host stop
//!   ([`Error::Interrupted`], checked at least every [`STOP_SLICE`]), and an [`Abandon`] that the
//!   engine raises when the waiting task is cancelled ([`Error::Interrupted`], at once) end a
//!   wait; the request is then never sent.
//! - **Costs.** O(G) to find the grants that allow a hop, as [`crate::grant::allowed`] does, then
//!   O(1) per limited grant to charge it, to join a queue, and to release a slot; a waiter that
//!   leaves early is dropped from each queue when it reaches the front, so each ticket is pushed
//!   once and popped once: O(1) amortized.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::client::{Error, Interrupt};
use crate::grant::{Grant, Limits, Method, Url};

/// The longest a waiter sleeps before it looks for a host stop again, on the wall clock: a host
/// stop is the host's time, never the run's.
pub const STOP_SLICE: Duration = Duration::from_millis(20);

/// The clock of a run, as the limits read it (ADR 0116 rule 19): the engines give theirs, the
/// interpreter's `Clock` and the executable's, so that a bucket refills on the run's time.
pub trait RunClock: Send + Sync {
    /// The milliseconds since the run started.
    fn now_millis(&self) -> u64;

    /// The nanoseconds since the run started, at the clock's full resolution: by default its
    /// milliseconds, as a virtual clock moves (ADR 0116 amendment, P13).
    fn now_nanos(&self) -> u128 {
        u128::from(self.now_millis()).saturating_mul(1_000_000)
    }

    /// Whether the clock is virtual: only the run's own waits move it.
    fn is_virtual(&self) -> bool;

    /// Moves a virtual clock to `at` milliseconds when that is later; the real clock ignores it.
    fn advance_to(&self, at: u64);
}

/// The monotonic clock from the moment it was made: the run's clock of a caller that has none,
/// as [`crate::client::send`] has.
#[derive(Clone, Copy, Debug)]
pub struct Monotonic {
    start: Instant,
}

impl Monotonic {
    /// The clock, started now.
    pub fn new() -> Monotonic {
        Monotonic {
            start: Instant::now(),
        }
    }
}

impl Default for Monotonic {
    fn default() -> Monotonic {
        Monotonic::new()
    }
}

impl RunClock for Monotonic {
    fn now_millis(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn now_nanos(&self) -> u128 {
        self.start.elapsed().as_nanos()
    }

    fn is_virtual(&self) -> bool {
        false
    }

    fn advance_to(&self, _at: u64) {}
}

/// A thread's parking spot: a flag and a condition variable, so a wake that comes before the
/// sleep is never lost.
#[derive(Debug, Default)]
struct Parker {
    woken: Mutex<bool>,
    condvar: Condvar,
}

impl Parker {
    /// Sleeps until woken or until `timeout` passes, and consumes the wake.
    fn park(&self, timeout: Duration) {
        let mut woken = self.woken.lock().unwrap_or_else(PoisonError::into_inner);
        if !*woken {
            woken = match self.condvar.wait_timeout(woken, timeout) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
        *woken = false;
    }

    /// Wakes the sleeper, or the next sleep at once.
    fn unpark(&self) {
        let mut woken = self.woken.lock().unwrap_or_else(PoisonError::into_inner);
        *woken = true;
        self.condvar.notify_one();
    }
}

/// What ends the I/O step an exchange waits in when its request is abandoned (#1148): the
/// shutdown of its socket, set by the client once it is connected and run once by
/// [`Abandon::abandon`].
type Hook = Box<dyn FnOnce() + Send>;

/// The hook slot of an [`Abandon`]: at most one, since a request's hops run one at a time.
#[derive(Default)]
struct HookSlot(Mutex<Option<Hook>>);

impl std::fmt::Debug for HookSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HookSlot")
    }
}

/// Locks a hook slot, recovering it when a thread panicked while holding the lock: each
/// critical section leaves it consistent.
fn lock_hook(slot: &HookSlot) -> MutexGuard<'_, Option<Hook>> {
    slot.0.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Default)]
struct AbandonInner {
    abandoned: AtomicBool,
    parker: Parker,
    hook: HookSlot,
}

/// How an engine ends a request whose task was cancelled (ADR 0117 rule 4, first bullet; ADR
/// 0116 P9, #1148). A request still waiting at a limit leaves the queues at once and is never
/// sent. One whose exchange has begun stops at its next I/O step: the client sees the flag as an
/// interruption ([`crate::client::send_covered`]), and its hook wakes the step that waits, by
/// shutting down the connection's socket, with no polling; a resolution or a connection attempt
/// in progress sees the flag when it returns. A clone shares the flag.
#[derive(Clone, Debug, Default)]
pub struct Abandon(Arc<AbandonInner>);

impl Abandon {
    /// A new flag, not raised.
    pub fn new() -> Abandon {
        Abandon::default()
    }

    /// Raises the flag, wakes the wait at a limit, and runs the hook of the I/O step in
    /// progress, once.
    pub fn abandon(&self) {
        self.0.abandoned.store(true, Ordering::SeqCst);
        self.0.parker.unpark();
        let hook = lock_hook(&self.0.hook).take();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// Whether the flag is raised.
    pub fn abandoned(&self) -> bool {
        self.0.abandoned.load(Ordering::SeqCst)
    }

    /// Sets the hook that ends the I/O step in progress when the request is abandoned, in place
    /// of any earlier one, which is dropped unrun. When the flag is already raised, the hook runs
    /// at once: the flag is set before the slot is read, under the slot's lock, so a hook is
    /// never left unrun after an abandon.
    pub(crate) fn on_abandon(&self, hook: Hook) {
        let mut slot = lock_hook(&self.0.hook);
        if self.abandoned() {
            drop(slot);
            hook();
            return;
        }
        *slot = Some(hook);
    }

    /// Drops the hook of the I/O step that ended, unrun.
    pub(crate) fn clear_hook(&self) {
        let hook = lock_hook(&self.0.hook).take();
        drop(hook);
    }
}

/// The state of one limit: a grant's, or a `limiter`'s of `std.limit` ([`Limiter`]).
#[derive(Debug)]
struct Slot {
    /// The rate: at most `n` per period, the period in nanoseconds.
    rate: Option<(u128, u128)>,
    /// At most `n` in flight.
    at_once: Option<u32>,
    /// How many may wait.
    queue_cap: u32,
    /// GCRA's theoretical arrival time, in units of `1/n` nanoseconds since the epoch.
    tat: u128,
    /// The hops in flight.
    in_flight: u32,
    /// The tickets that joined the queue, in order; a ticket whose waiter left stays until it
    /// reaches the front.
    queue: VecDeque<u64>,
    /// The waiters still in the queue.
    waiting: u32,
}

impl Slot {
    /// The slot of a grant's `limits`.
    fn of(limits: &Limits) -> Slot {
        Slot {
            rate: limits
                .rate
                .map(|(count, period)| (u128::from(count), u128::from(period.nanos()))),
            at_once: limits.at_once,
            queue_cap: limits.queue(),
            tat: 0,
            in_flight: 0,
            queue: VecDeque::new(),
            waiting: 0,
        }
    }

    /// `t·n`, `T` and `τ = T·(n − 1)` of GCRA, in units of `1/n` nanoseconds, for a rate.
    fn gcra(&self, now: u128) -> Option<(u128, u128, u128)> {
        let (n, period) = self.rate?;
        Some((
            now.saturating_mul(n),
            period,
            period.saturating_mul(n.saturating_sub(1)),
        ))
    }

    /// Whether a hop could be charged at `now`, nanoseconds since the epoch.
    fn free(&self, now: u128) -> bool {
        let token = self
            .gcra(now)
            .is_none_or(|(t, _, tau)| t.saturating_add(tau) >= self.tat);
        let slot = self.at_once.is_none_or(|n| self.in_flight < n);
        token && slot
    }

    /// How long until the next token, when the rate is what blocks.
    fn token_in(&self, now: u128) -> Option<Duration> {
        let (count, _) = self.rate?;
        let (t, _, tau) = self.gcra(now)?;
        let due = self.tat.checked_sub(tau)?;
        let short = due.checked_sub(t).filter(|short| *short > 0)?;
        let nanos = short.div_ceil(count.max(1));
        Some(Duration::from_nanos(
            u64::try_from(nanos).unwrap_or(u64::MAX),
        ))
    }

    /// Takes a token and a slot; [`Slot::free`] said yes.
    fn charge(&mut self, now: u128) {
        if let Some((t, period, _)) = self.gcra(now) {
            self.tat = self.tat.max(t).saturating_add(period);
        }
        if self.at_once.is_some() {
            self.in_flight = self.in_flight.saturating_add(1);
        }
    }

    /// The first live ticket of the queue, after dropping those whose waiters left.
    fn front(&mut self, waiters: &HashMap<u64, Waiter>) -> Option<u64> {
        while let Some(&ticket) = self.queue.front() {
            if waiters.contains_key(&ticket) {
                return Some(ticket);
            }
            self.queue.pop_front();
        }
        None
    }
}

/// A request waiting for its grants.
#[derive(Debug)]
struct Waiter {
    abandon: Abandon,
}

#[derive(Debug)]
struct Inner {
    next: u64,
    /// One entry per grant of the governor: `Some` for a grant with a rate or a count in flight.
    slots: Vec<Option<Slot>>,
    waiters: HashMap<u64, Waiter>,
}

impl Inner {
    /// Wakes the first live waiter of each of `grants`' queues, which may be served now.
    fn wake_fronts(&mut self, grants: &[usize]) {
        let Inner { slots, waiters, .. } = self;
        for &grant in grants {
            if let Some(Some(slot)) = slots.get_mut(grant)
                && let Some(ticket) = slot.front(waiters)
                && let Some(waiter) = waiters.get(&ticket)
            {
                waiter.abandon.0.parker.unpark();
            }
        }
    }

    /// Whether `ticket` is the first live waiter of every one of `grants`, each of which is free.
    fn servable(&mut self, ticket: Option<u64>, grants: &[usize], now: u128) -> bool {
        let Inner { slots, waiters, .. } = self;
        grants.iter().all(|&grant| match slots.get_mut(grant) {
            Some(Some(slot)) => slot.front(waiters) == ticket && slot.free(now),
            _ => true,
        })
    }

    /// Charges every one of `grants`.
    fn charge(&mut self, grants: &[usize], now: u128) {
        for &grant in grants {
            if let Some(Some(slot)) = self.slots.get_mut(grant) {
                slot.charge(now);
            }
        }
    }

    /// Takes `ticket` out of every queue of `grants`: off the front, where a served waiter is,
    /// and lazily elsewhere.
    fn leave(&mut self, ticket: u64, grants: &[usize]) {
        self.waiters.remove(&ticket);
        for &grant in grants {
            if let Some(Some(slot)) = self.slots.get_mut(grant) {
                slot.waiting = slot.waiting.saturating_sub(1);
                if slot.queue.front() == Some(&ticket) {
                    slot.queue.pop_front();
                }
            }
        }
        self.wake_fronts(grants);
    }
}

/// The limits of a run's `Http` grants and their state (ADR 0117 rule 8), shared by every task
/// of the run.
pub struct Governor {
    grants: Vec<Grant>,
    /// Whether some grant carries a rate or a count in flight; otherwise nothing is charged.
    limited: bool,
    clock: Arc<dyn RunClock>,
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for Governor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Governor")
            .field("grants", &self.grants)
            .field("limited", &self.limited)
            .field("virtual", &self.clock.is_virtual())
            .finish_non_exhaustive()
    }
}

impl Governor {
    /// The governor of `grants`, the run's `Http` grants in order, on a monotonic clock of its
    /// own.
    pub fn new(grants: Vec<Grant>) -> Governor {
        Governor::with_clock(grants, Arc::new(Monotonic::new()))
    }

    /// The governor of `grants` on the run's `clock` (ADR 0116 rule 19).
    pub fn with_clock(grants: Vec<Grant>, clock: Arc<dyn RunClock>) -> Governor {
        let slots = grants
            .iter()
            .map(|grant| grant.limits().binding().then(|| Slot::of(grant.limits())))
            .collect::<Vec<_>>();
        Governor {
            limited: slots.iter().any(Option::is_some),
            grants,
            clock,
            inner: Mutex::new(Inner {
                next: 0,
                slots,
                waiters: HashMap::new(),
            }),
        }
    }

    /// The grants it governs, in order.
    pub fn grants(&self) -> &[Grant] {
        &self.grants
    }

    /// Whether some grant carries a rate or a count in flight, so that a hop may wait in a
    /// queue; otherwise nothing is charged and no hop ever waits.
    pub fn limited(&self) -> bool {
        self.limited
    }

    /// Locks the state, recovering it when a thread panicked while holding the lock: every
    /// critical section leaves it consistent.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The milliseconds since the run started, on its clock.
    pub fn now_millis(&self) -> u64 {
        self.clock.now_millis()
    }

    /// The moment `after` from now on its clock, in its first whole millisecond: never one
    /// before it, so a wait that ends when [`Governor::now_millis`] reaches it has waited the
    /// whole of `after` (#1355). Rounding the reading and `after` down each, as before, put it
    /// up to 2 ms early. O(1).
    pub fn millis_after(&self, after: Duration) -> u64 {
        let at = self.clock.now_nanos().saturating_add(after.as_nanos());
        u64::try_from(at.div_ceil(1_000_000)).unwrap_or(u64::MAX)
    }

    /// Nanoseconds since the run started, on its clock.
    fn now(&self) -> u128 {
        u128::from(self.clock.now_millis()).saturating_mul(1_000_000)
    }

    /// A permit for one hop to `url` with `method`, once every limited grant that allows it has
    /// a token and a slot: at once when no request waits at any of them, else in ticket order.
    /// Fails with `RateLimited` when a queue it must join is full, with `TimedOut` at
    /// `deadline`, and with `Interrupted` on a host stop or when `abandon` is raised.
    pub fn acquire(
        &self,
        url: &Url,
        method: Method,
        deadline: Instant,
        interrupt: &dyn Interrupt,
        abandon: &Abandon,
    ) -> Result<Permit<'_>, Error> {
        self.acquire_until(url, method, (deadline, u64::MAX), interrupt, abandon)
    }

    /// [`Governor::acquire`] with two deadlines: `deadline` on the wall clock, and `until`, in
    /// milliseconds on the run's clock, the request's deadline there (its `--http-timeout` from
    /// its start, or a deadline of `within` or `for ever within` that covers it). Under virtual
    /// time a wait for a refill moves the clock to the refill, or to `until` when that comes
    /// first, and the request then fails with `TimedOut`.
    pub fn acquire_until(
        &self,
        url: &Url,
        method: Method,
        (deadline, until): (Instant, u64),
        interrupt: &dyn Interrupt,
        abandon: &Abandon,
    ) -> Result<Permit<'_>, Error> {
        if !self.limited {
            return Ok(Permit {
                governor: self,
                grants: Vec::new(),
            });
        }
        let grants: Vec<usize> = self
            .grants
            .iter()
            .enumerate()
            .filter(|(_, grant)| grant.limits().binding() && grant.allows(url, method))
            .map(|(at, _)| at)
            .collect();
        self.acquire_slots(grants, (deadline, until), interrupt, abandon)
    }

    /// A permit over the slots `grants`, by index: at once when every one is free and no
    /// request waits at any of them, else in ticket order, as [`Governor::acquire_until`] says.
    fn acquire_slots(
        &self,
        grants: Vec<usize>,
        (deadline, until): (Instant, u64),
        interrupt: &dyn Interrupt,
        abandon: &Abandon,
    ) -> Result<Permit<'_>, Error> {
        let mut inner = self.lock();
        let now = self.now();
        if inner.servable(None, &grants, now) {
            inner.charge(&grants, now);
            return Ok(Permit {
                governor: self,
                grants,
            });
        }
        let full = grants.iter().any(|&grant| match inner.slots.get(grant) {
            Some(Some(slot)) => slot.waiting >= slot.queue_cap,
            _ => false,
        });
        if full {
            return Err(Error::RateLimited);
        }
        let ticket = inner.next;
        inner.next = inner.next.wrapping_add(1);
        for &grant in &grants {
            if let Some(Some(slot)) = inner.slots.get_mut(grant) {
                slot.queue.push_back(ticket);
                slot.waiting = slot.waiting.saturating_add(1);
            }
        }
        inner.waiters.insert(
            ticket,
            Waiter {
                abandon: abandon.clone(),
            },
        );
        loop {
            if abandon.abandoned() || interrupt.stopped() {
                inner.leave(ticket, &grants);
                return Err(Error::Interrupted);
            }
            let instant = Instant::now();
            let millis = self.clock.now_millis();
            if instant >= deadline || millis >= until {
                inner.leave(ticket, &grants);
                return Err(Error::TimedOut);
            }
            let now = self.now();
            if inner.servable(Some(ticket), &grants, now) {
                inner.charge(&grants, now);
                inner.leave(ticket, &grants);
                return Ok(Permit {
                    governor: self,
                    grants,
                });
            }
            // First in line where only a rate blocks: sleep until its token.
            let token = grants
                .iter()
                .filter_map(|&grant| match inner.slots.get(grant) {
                    Some(Some(slot)) => slot.token_in(now),
                    _ => None,
                })
                .max();
            // Under virtual time, nothing else of the run runs while a host operation waits:
            // the wait for a refill moves the clock, to the refill or to the deadline.
            if let Some(token) = token
                && self.clock.is_virtual()
            {
                let refill = u64::try_from(token.as_nanos().div_ceil(1_000_000))
                    .unwrap_or(u64::MAX)
                    .max(1);
                self.clock
                    .advance_to(millis.saturating_add(refill).min(until));
                continue;
            }
            let run_left = Duration::from_millis(until.saturating_sub(millis));
            let sleep = (deadline - instant)
                .min(run_left)
                .min(STOP_SLICE)
                .min(token.unwrap_or(STOP_SLICE));
            drop(inner);
            abandon.0.parker.park(sleep);
            inner = self.lock();
        }
    }
}

/// A rate limit computed in code: the one bucket of an installation of `limiter` of `std.limit`
/// (ADR 0117 rule 9, `spec/27 §10`), which every task that uses the binding shares, the started
/// tasks of a `tasks` statement in the installing statement included. It is a governor of one
/// slot with a rate and no count in flight, so `acquire` takes a token by the same GCRA, over the
/// same run clock, with the same queue served by ticket, the same ending of a wait by a deadline,
/// a host stop or an abandon, and the same move of the virtual clock to a refill.
pub struct Limiter {
    governor: Governor,
}

impl std::fmt::Debug for Limiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Limiter")
            .field("virtual", &self.governor.clock.is_virtual())
            .finish_non_exhaustive()
    }
}

impl Limiter {
    /// A bucket of `at_most` tokens per `per` milliseconds, with at most `waiting` calls
    /// waiting, on the run's `clock`. The installation's obligations give `at_most >= 1`,
    /// `per >= 1` and `waiting >= 0`; a count beyond what a slot holds saturates, so a larger
    /// one never wraps into a smaller limit.
    pub fn new(at_most: u64, per: u64, waiting: u64, clock: Arc<dyn RunClock>) -> Limiter {
        let slot = Slot {
            rate: Some((
                u128::from(at_most.max(1)),
                u128::from(per.max(1)).saturating_mul(1_000_000),
            )),
            at_once: None,
            queue_cap: u32::try_from(waiting).unwrap_or(u32::MAX),
            tat: 0,
            in_flight: 0,
            queue: VecDeque::new(),
            waiting: 0,
        };
        Limiter {
            governor: Governor {
                grants: Vec::new(),
                limited: true,
                clock,
                inner: Mutex::new(Inner {
                    next: 0,
                    slots: vec![Some(slot)],
                    waiters: HashMap::new(),
                }),
            },
        }
    }

    /// The milliseconds since the run started, on its clock.
    pub fn now_millis(&self) -> u64 {
        self.governor.now_millis()
    }

    /// Takes a token: at once when one is left and no call waits, else in ticket order once
    /// the bucket refills. Fails with `RateLimited` when `waiting` calls already wait, with
    /// `TimedOut` at `deadline` on the wall clock or `until` in milliseconds of the run's clock,
    /// and with `Interrupted` on a host stop or when `abandon` is raised; under virtual time a
    /// wait for a refill moves the clock to the refill, or to `until` when that comes first.
    pub fn acquire(
        &self,
        (deadline, until): (Instant, u64),
        interrupt: &dyn Interrupt,
        abandon: &Abandon,
    ) -> Result<(), Error> {
        self.governor
            .acquire_slots(vec![0], (deadline, until), interrupt, abandon)
            .map(drop)
    }
}

/// The tokens and slots of one hop, held while it is in flight; dropping it releases the slots
/// and wakes the first waiter of each queue.
#[derive(Debug)]
pub struct Permit<'g> {
    governor: &'g Governor,
    grants: Vec<usize>,
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        if self.grants.is_empty() {
            return;
        }
        let mut inner = self.governor.lock();
        for &grant in &self.grants {
            if let Some(Some(slot)) = inner.slots.get_mut(grant)
                && slot.at_once.is_some()
            {
                slot.in_flight = slot.in_flight.saturating_sub(1);
            }
        }
        inner.wake_fronts(&self.grants);
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::BorrowedFd;
    use std::sync::atomic::{AtomicU64, AtomicUsize};

    use super::*;
    use crate::client::Waited;

    struct Never;

    impl Interrupt for Never {
        fn stopped(&self) -> bool {
            false
        }
        fn wait(&self, _socket: BorrowedFd<'_>, _write: bool, _timeout: Duration) -> Waited {
            Waited::Ready
        }
        fn waits(&self) -> bool {
            false
        }
    }

    struct Stopped;

    impl Interrupt for Stopped {
        fn stopped(&self) -> bool {
            true
        }
        fn wait(&self, _socket: BorrowedFd<'_>, _write: bool, _timeout: Duration) -> Waited {
            Waited::Stopped
        }
        fn waits(&self) -> bool {
            false
        }
    }

    fn governor(specs: &[&str]) -> Governor {
        Governor::new(
            specs
                .iter()
                .map(|spec| Grant::parse_limited(spec).unwrap().unwrap())
                .collect(),
        )
    }

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    fn soon(millis: u64) -> Instant {
        Instant::now() + Duration::from_millis(millis)
    }

    #[test]
    fn a_bucket_lets_its_burst_through_then_waits() {
        // Three per second: three at once, then the fourth waits about a third of a second.
        let g = governor(&["http:http://h;3/second"]);
        let target = url("http://h/");
        for _ in 0..3 {
            drop(
                g.acquire(&target, Method::Get, soon(50), &Never, &Abandon::new())
                    .unwrap(),
            );
        }
        // Its deadline passes before the token comes.
        let started = Instant::now();
        let late = g.acquire(&target, Method::Get, soon(50), &Never, &Abandon::new());
        assert_eq!(late.err(), Some(Error::TimedOut));
        assert!(started.elapsed() >= Duration::from_millis(50));
        // With time enough, it is served once the token comes.
        let started = Instant::now();
        drop(
            g.acquire(&target, Method::Get, soon(2_000), &Never, &Abandon::new())
                .unwrap(),
        );
        assert!(started.elapsed() < Duration::from_millis(1_000));
    }

    #[test]
    fn a_broad_grant_does_not_bypass_a_narrow_one() {
        // The broad grant has no limits; the narrow one allows one at once, and both allow the
        // target, so a second hop in flight waits for the first.
        let g = governor(&["http:http://h:get,post", "http:http://h;1/once"]);
        let target = url("http://h/x");
        let first = g
            .acquire(&target, Method::Get, soon(1_000), &Never, &Abandon::new())
            .unwrap();
        let second = g.acquire(&target, Method::Get, soon(40), &Never, &Abandon::new());
        assert_eq!(second.err(), Some(Error::TimedOut));
        // A method the narrow grant does not allow is charged only to the broad one.
        drop(
            g.acquire(&target, Method::Post, soon(40), &Never, &Abandon::new())
                .unwrap(),
        );
        drop(first);
        drop(
            g.acquire(&target, Method::Get, soon(40), &Never, &Abandon::new())
                .unwrap(),
        );
    }

    #[test]
    fn a_full_queue_fails_at_once() {
        let g = Arc::new(governor(&["http:http://h;1/once;waiting=1"]));
        let target = url("http://h/");
        let held = g
            .acquire(&target, Method::Get, soon(5_000), &Never, &Abandon::new())
            .unwrap();
        // One waiter fills the queue.
        let waiter = {
            let g = Arc::clone(&g);
            let target = target.clone();
            std::thread::spawn(move || {
                g.acquire(&target, Method::Get, soon(5_000), &Never, &Abandon::new())
                    .map(drop)
            })
        };
        while g.lock().slots[0].as_ref().unwrap().waiting == 0 {
            std::thread::yield_now();
        }
        let refused = g.acquire(&target, Method::Get, soon(5_000), &Never, &Abandon::new());
        assert_eq!(refused.err(), Some(Error::RateLimited));
        drop(held);
        assert_eq!(waiter.join().unwrap(), Ok(()));
    }

    #[test]
    fn waiters_are_served_in_ticket_order() {
        let g = Arc::new(governor(&["http:http://h;1/once"]));
        let target = url("http://h/");
        let held = g
            .acquire(&target, Method::Get, soon(5_000), &Never, &Abandon::new())
            .unwrap();
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut threads = Vec::new();
        for k in 0u32..5 {
            let (shared, order, target) = (Arc::clone(&g), Arc::clone(&order), target.clone());
            threads.push(std::thread::spawn(move || {
                let permit = shared
                    .acquire(&target, Method::Get, soon(5_000), &Never, &Abandon::new())
                    .unwrap();
                order.lock().unwrap().push(k);
                drop(permit);
            }));
            // Each joins the queue before the next starts.
            while g.lock().slots[0].as_ref().unwrap().waiting < k + 1 {
                std::thread::yield_now();
            }
        }
        drop(held);
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(*order.lock().unwrap(), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn crossing_hops_never_deadlock() {
        // Two grants with one slot each; requests alternate between them in opposite orders,
        // each hop releasing its slot before the next asks, as redirect chains do.
        let g = Arc::new(governor(&["http:http://a;1/once", "http:http://b;1/once"]));
        let done = Arc::new(AtomicUsize::new(0));
        let mut threads = Vec::new();
        for (first, second) in [("http://a/", "http://b/"), ("http://b/", "http://a/")] {
            let (g, done) = (Arc::clone(&g), Arc::clone(&done));
            threads.push(std::thread::spawn(move || {
                for _ in 0..200 {
                    for hop in [first, second] {
                        let permit = g
                            .acquire(&url(hop), Method::Get, soon(5_000), &Never, &Abandon::new())
                            .unwrap();
                        std::thread::yield_now();
                        drop(permit);
                    }
                }
                done.fetch_add(1, Ordering::SeqCst);
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(done.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn an_abandoned_wait_leaves_the_queue_at_once() {
        let g = Arc::new(governor(&["http:http://h;1/once;waiting=1"]));
        let target = url("http://h/");
        let held = g
            .acquire(&target, Method::Get, soon(5_000), &Never, &Abandon::new())
            .unwrap();
        let abandon = Abandon::new();
        let waiter = {
            let (g, target, abandon) = (Arc::clone(&g), target.clone(), abandon.clone());
            std::thread::spawn(move || {
                g.acquire(&target, Method::Get, soon(5_000), &Never, &abandon)
                    .map(drop)
            })
        };
        while g.lock().slots[0].as_ref().unwrap().waiting == 0 {
            std::thread::yield_now();
        }
        let started = Instant::now();
        abandon.abandon();
        assert_eq!(waiter.join().unwrap(), Err(Error::Interrupted));
        assert!(started.elapsed() < Duration::from_secs(1));
        // Its place is free again: another may wait.
        assert_eq!(g.lock().slots[0].as_ref().unwrap().waiting, 0);
        drop(held);
    }

    #[test]
    fn a_host_stop_ends_a_wait() {
        let g = governor(&["http:http://h;1/hour"]);
        let target = url("http://h/");
        drop(
            g.acquire(&target, Method::Get, soon(5_000), &Never, &Abandon::new())
                .unwrap(),
        );
        let stopped = g.acquire(&target, Method::Get, soon(5_000), &Stopped, &Abandon::new());
        assert_eq!(stopped.err(), Some(Error::Interrupted));
    }

    #[test]
    fn unlimited_grants_charge_nothing() {
        let g = governor(&["http:http://h", "http:http://h;waiting=1"]);
        let target = url("http://h/");
        let permits: Vec<Permit<'_>> = (0..100)
            .map(|_| {
                g.acquire(&target, Method::Get, soon(10), &Never, &Abandon::new())
                    .unwrap()
            })
            .collect();
        assert_eq!(permits.len(), 100);
    }

    /// A virtual clock, as `--virtual-time` gives: only `advance_to` moves it.
    #[derive(Default)]
    struct Virtual(AtomicU64);

    impl RunClock for Virtual {
        fn now_millis(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
        fn is_virtual(&self) -> bool {
            true
        }
        fn advance_to(&self, at: u64) {
            self.0.fetch_max(at, Ordering::SeqCst);
        }
    }

    /// A real clock that stays at `.0` nanoseconds since its run started, read at full
    /// resolution.
    struct Still(u128);

    impl RunClock for Still {
        fn now_millis(&self) -> u64 {
            u64::try_from(self.0 / 1_000_000).unwrap()
        }
        fn now_nanos(&self) -> u128 {
            self.0
        }
        fn is_virtual(&self) -> bool {
            false
        }
        fn advance_to(&self, _at: u64) {}
    }

    #[test]
    fn a_deadline_on_the_run_s_clock_is_never_a_millisecond_before_its_moment() {
        // #1355: a request's deadline on the run's clock, which ends a wait at a limit, was the
        // reading in whole milliseconds plus the time-out in whole milliseconds, each rounded
        // down, so at 5.7 ms a wait of 100 ms ended at 105 ms, 0.7 ms before its moment, and a
        // `within` of that moment saw its deadline not yet passed. It is now the first whole
        // millisecond at or after the moment, whatever the phase within the millisecond.
        let at_5_7 = Governor::with_clock(Vec::new(), Arc::new(Still(5_700_000)));
        assert_eq!(at_5_7.millis_after(Duration::from_millis(100)), 106);
        for now in [0u128, 1, 5_700_000, 999_999, 1_000_000, 86_400_000_123_456] {
            let governor = Governor::with_clock(Vec::new(), Arc::new(Still(now)));
            for after in [0u64, 1, 999_999, 1_000_000, 100_000_000, 100_300_001] {
                let moment = now + u128::from(after);
                let until = u128::from(governor.millis_after(Duration::from_nanos(after)));
                assert!(until * 1_000_000 >= moment, "{now} + {after}: {until} ms");
                assert!(
                    until * 1_000_000 < moment + 1_000_000,
                    "{now} + {after}: {until} ms"
                );
            }
        }
        // Under virtual time, whole milliseconds, as before.
        let clock = Arc::new(Virtual::default());
        clock.advance_to(7);
        let governor = Governor::with_clock(Vec::new(), clock);
        assert_eq!(governor.millis_after(Duration::from_millis(30)), 37);
    }

    #[test]
    fn under_virtual_time_a_refill_moves_the_clock_and_takes_no_wall_time() {
        // Two per second, five requests: at 0, 0, 500, 1000 and 1500 virtual milliseconds.
        let clock = Arc::new(Virtual::default());
        let g = Governor::with_clock(
            vec![
                Grant::parse_limited("http:http://h;2/second")
                    .unwrap()
                    .unwrap(),
            ],
            Arc::clone(&clock) as Arc<dyn RunClock>,
        );
        let target = url("http://h/");
        let started = Instant::now();
        let mut readings = Vec::new();
        for _ in 0..5 {
            drop(
                g.acquire_until(
                    &target,
                    Method::Get,
                    (soon(60_000), u64::MAX),
                    &Never,
                    &Abandon::new(),
                )
                .unwrap(),
            );
            readings.push(clock.now_millis());
        }
        assert_eq!(readings, [0, 0, 500, 1000, 1500]);
        assert!(started.elapsed() < Duration::from_secs(1));
        // A refill past the request's deadline on the run's clock: the clock moves to the
        // deadline, and the request fails there.
        let hourly = Governor::with_clock(
            vec![
                Grant::parse_limited("http:http://h;1/hour")
                    .unwrap()
                    .unwrap(),
            ],
            Arc::clone(&clock) as Arc<dyn RunClock>,
        );
        let at = clock.now_millis();
        drop(
            hourly
                .acquire_until(
                    &target,
                    Method::Get,
                    (soon(60_000), u64::MAX),
                    &Never,
                    &Abandon::new(),
                )
                .unwrap(),
        );
        let late = hourly.acquire_until(
            &target,
            Method::Get,
            (soon(60_000), at + 700),
            &Never,
            &Abandon::new(),
        );
        assert_eq!(late.err(), Some(Error::TimedOut));
        assert_eq!(clock.now_millis(), at + 700);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    /// The calls of a batch of [`history_growth`].
    const BATCH: usize = 100;

    /// The timed batches of each state in [`history_growth`].
    const WINDOW: usize = 50;

    /// How `op` costs late in the life of a state, against early: one state made by `make`
    /// takes `calls` calls of `op`, the last [`WINDOW`] batches of [`BATCH`] timed; a second,
    /// fresh, takes ten batches of warm-up and then [`WINDOW`] timed batches, the batches of the
    /// two states alternating. Returns the shortest early and the shortest late batch, in
    /// seconds. An `op` whose cost grows with the calls before it, so that `n` calls cost more
    /// than linear, takes longer late than early; one of constant cost does not.
    ///
    /// The measure does not depend on the host's load (#2345): a loaded host only lengthens a
    /// batch, and the shortest of each set is one that ran without interruption; since the two
    /// sets alternate, a slower core or clock speed for a while slows both alike. The measure
    /// it replaces, the ratio of two whole runs, failed under a load of about 200, when the
    /// longer run spent most of its time descheduled.
    fn history_growth<S>(calls: usize, make: impl Fn() -> S, op: impl Fn(&mut S)) -> (f64, f64) {
        // The old state is timed at least four times as far into its life as the fresh one.
        assert!(calls >= 4 * (10 + WINDOW) * BATCH, "{calls}");
        let batch = |state: &mut S| {
            let started = Instant::now();
            for _ in 0..BATCH {
                op(state);
            }
            started.elapsed().as_secs_f64()
        };
        let mut old = make();
        for _ in 0..calls - WINDOW * BATCH {
            op(&mut old);
        }
        let mut fresh = make();
        for _ in 0..10 {
            batch(&mut fresh);
        }
        let (mut early, mut late) = (f64::INFINITY, f64::INFINITY);
        for _ in 0..WINDOW {
            early = early.min(batch(&mut fresh));
            late = late.min(batch(&mut old));
        }
        (early, late)
    }

    /// Whether the late batches of [`history_growth`] take more than four times the early
    /// ones. For a cost that grows linearly with the calls before it, late is about 19 times
    /// early at 25,000 calls, and more at 100,000; for a constant cost it stays near
    /// one. The 20 microseconds absorb the clock's granularity on a batch.
    ///
    /// What it pins is no polynomial growth with history, not a strictly constant cost (#2377):
    /// at 100,000 calls the late batches run about 95 times as far into the state's life as
    /// the early ones, so a cost per call of order history^0.31 or more exceeds four times
    /// (95^0.31 is about 4.1) and is flagged, while a logarithmic one, such as an insert into a
    /// `BTreeMap` that keeps every call, gives about 1.4 and is not.
    fn grows(early: f64, late: f64) -> bool {
        late > early * 4.0 + 20e-6
    }

    #[test]
    fn charging_scales_linearly() {
        // Each charge is O(1) per grant: a hop late in a governor's life costs what an early
        // one does, so `n` hops cost O(n).
        let target = url("http://h/");
        let (early, late) = history_growth(
            100_000,
            || {
                governor(&[
                    "http:http://h;2147483647/second;2147483647/once",
                    "http:http://h:get,post;2147483647/minute",
                ])
            },
            |g| {
                drop(
                    g.acquire(&target, Method::Get, soon(10_000), &Never, &Abandon::new())
                        .unwrap(),
                );
            },
        );
        assert!(!grows(early, late), "{early} then {late}");
    }

    #[test]
    fn the_measure_of_charging_finds_a_quadratic_cost() {
        // The same measure on an operation that reads everything done before it, as a charge
        // that scanned its history would: `n` of them cost O(n²), and it says so.
        let (early, late) = history_growth(25_000, Vec::new, |history: &mut Vec<u64>| {
            for entry in history.iter() {
                std::hint::black_box(entry);
            }
            history.push(std::hint::black_box(1));
        });
        assert!(grows(early, late), "{early} then {late}");
    }

    #[test]
    fn a_limiter_lets_its_burst_through_then_waits_for_its_refill() {
        // Two per 100 ms on the monotonic clock: two at once, the third once a token comes,
        // within 50 ms.
        let limiter = Limiter::new(2, 100, 8, Arc::new(Monotonic::new()));
        let until = (soon(1_000), u64::MAX);
        limiter.acquire(until, &Never, &Abandon::new()).unwrap();
        limiter.acquire(until, &Never, &Abandon::new()).unwrap();
        let started = Instant::now();
        limiter
            .acquire((soon(1_000), u64::MAX), &Never, &Abandon::new())
            .unwrap();
        assert!(started.elapsed() < Duration::from_millis(500));
        // A call whose deadline comes before its refill fails with `TimedOut` at its deadline,
        // unsent. On a run clock that stays still the refill never comes, so the outcome does
        // not depend on how late the thread runs: on the monotonic clock above, a stall of
        // 50 ms before the call let a token in and the call through (#1932).
        let still = Limiter::new(2, 100, 8, Arc::new(Still(0)));
        still.acquire(until, &Never, &Abandon::new()).unwrap();
        still.acquire(until, &Never, &Abandon::new()).unwrap();
        let started = Instant::now();
        let late = still.acquire((soon(5), u64::MAX), &Never, &Abandon::new());
        assert_eq!(late.err(), Some(Error::TimedOut));
        assert!(started.elapsed() >= Duration::from_millis(5));
    }

    #[test]
    fn a_limiter_with_a_full_queue_fails_at_once() {
        // One per minute, nothing may wait: the second call fails with `RateLimited` at once.
        let limiter = Limiter::new(1, 60_000, 0, Arc::new(Monotonic::new()));
        limiter
            .acquire((soon(100), u64::MAX), &Never, &Abandon::new())
            .unwrap();
        let started = Instant::now();
        let full = limiter.acquire((soon(1_000), u64::MAX), &Never, &Abandon::new());
        assert_eq!(full.err(), Some(Error::RateLimited));
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn a_limiter_under_virtual_time_moves_the_clock_to_each_refill() {
        // Two per second, five calls: at 0, 0, 500, 1000 and 1500 virtual milliseconds, in no
        // wall time; a call whose deadline on the run's clock comes before its refill moves
        // the clock to that deadline and fails.
        let clock = Arc::new(Virtual::default());
        let limiter = Limiter::new(2, 1_000, 8, Arc::clone(&clock) as Arc<dyn RunClock>);
        let started = Instant::now();
        let mut readings = Vec::new();
        for _ in 0..5 {
            limiter
                .acquire((soon(10_000), u64::MAX), &Never, &Abandon::new())
                .unwrap();
            readings.push(clock.now_millis());
        }
        assert_eq!(readings, [0, 0, 500, 1_000, 1_500]);
        assert!(started.elapsed() < Duration::from_millis(500));
        let cut = limiter.acquire((soon(10_000), 1_700), &Never, &Abandon::new());
        assert_eq!(cut.err(), Some(Error::TimedOut));
        assert_eq!(clock.now_millis(), 1_700);
    }

    #[test]
    fn a_limiter_s_wait_ends_on_a_host_stop_or_an_abandon() {
        let limiter = Limiter::new(1, 60_000, 8, Arc::new(Monotonic::new()));
        limiter
            .acquire((soon(100), u64::MAX), &Never, &Abandon::new())
            .unwrap();
        let stopped = limiter.acquire((soon(1_000), u64::MAX), &Stopped, &Abandon::new());
        assert_eq!(stopped.err(), Some(Error::Interrupted));
        let abandon = Abandon::new();
        abandon.abandon();
        let abandoned = limiter.acquire((soon(1_000), u64::MAX), &Never, &abandon);
        assert_eq!(abandoned.err(), Some(Error::Interrupted));
    }
}
