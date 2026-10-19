//! Name resolution for the client (ADR 0117 slice S7, `docs/plan/0117-s7-design.md` §5).
//!
//! The system's resolver (`getaddrinfo`) has no timeout of its own, so each lookup runs on a
//! thread of its own, which may outlive the callers that gave up on it. To keep a name whose
//! lookups stall from starving the others (review R7-10):
//!
//! - **One lookup per name at a time.** Callers that ask for a name while its lookup is in
//!   flight wait for that lookup's answer, each until its own deadline. So a name holds at
//!   most one place among the resolver threads, whatever the rate of its requests: the bound
//!   per name is 1, below a quarter of any bound of the process.
//! - **A bounded number of resolver threads,** `most` ([`limit`]), with a first-come,
//!   first-served line for a place, whose wait the caller's deadline covers. No thread starts
//!   past a deadline.
//! - **A failed name is remembered** for [`FAILED_FOR`]: it answers `Unreachable` at once,
//!   without a new lookup. The time is the monotonic clock of the process: a run under virtual
//!   time has no real network (ADR 0116 rule 19).
//! - **Costs.** O(1) per call on average: hash maps by name, a line whose callers that gave up
//!   are skipped lazily, and a cache of failures pruned when it has doubled. A place given back
//!   wakes the callers that wait for one, at most the threads that resolve (the helpers of the
//!   I/O pool, or the interpreter's thread).

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use crate::client::{Error, left};

/// How long a name whose lookup failed answers `Unreachable` without a new lookup.
pub(crate) const FAILED_FOR: Duration = Duration::from_secs(5);

/// A lookup of a name: its addresses, with port 0.
type Lookup = fn(&str) -> std::io::Result<Vec<SocketAddr>>;

/// The system's lookup.
fn system(name: &str) -> std::io::Result<Vec<SocketAddr>> {
    (name, 0).to_socket_addrs().map(Iterator::collect)
}

/// One lookup in flight, whose answer every caller of its name shares.
#[derive(Default)]
struct Pending {
    answer: Mutex<Option<Result<Vec<SocketAddr>, Error>>>,
    ready: Condvar,
}

/// The resolver threads and their callers.
struct State {
    running: usize,
    most: usize,
    next: u64,
    /// The tickets of the callers waiting for a place, in order.
    line: VecDeque<u64>,
    /// Tickets in `line` whose callers gave up, skipped when they reach its front.
    gave_up: HashSet<u64>,
    pending: HashMap<String, Arc<Pending>>,
    failed: HashMap<String, Instant>,
    /// The size of `failed` after its last pruning.
    pruned: usize,
}

/// The resolver of the process, or of a test.
pub(crate) struct Resolver {
    state: Mutex<State>,
    /// Signaled when a place is given back, a caller leaves the line, or a lookup starts.
    freed: Condvar,
    lookup: Lookup,
}

/// What a caller does once it holds the state.
enum Next {
    /// Wait for the lookup in flight.
    Join(Arc<Pending>),
    /// Start the lookup, on a place just taken.
    Start(Arc<Pending>),
}

/// The resolver of the process.
pub(crate) fn global() -> &'static Resolver {
    static GLOBAL: OnceLock<Resolver> = OnceLock::new();
    GLOBAL.get_or_init(|| Resolver::new(64, system))
}

/// Bounds the resolver threads of the process at `most`, at least 1.
pub(crate) fn limit(most: usize) {
    let resolver = global();
    resolver.lock().most = most.max(1);
    resolver.freed.notify_all();
}

impl Resolver {
    fn new(most: usize, lookup: Lookup) -> Resolver {
        Resolver {
            state: Mutex::new(State {
                running: 0,
                most: most.max(1),
                next: 0,
                line: VecDeque::new(),
                gave_up: HashSet::new(),
                pending: HashMap::new(),
                failed: HashMap::new(),
                pruned: 0,
            }),
            freed: Condvar::new(),
            lookup,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The addresses of `name` with `port`, by `deadline`.
    pub(crate) fn resolve(
        &'static self,
        name: &str,
        port: u16,
        deadline: Instant,
    ) -> Result<Vec<SocketAddr>, Error> {
        let next = self.next(name, deadline)?;
        let pending = match next {
            Next::Join(pending) => pending,
            Next::Start(pending) => {
                let (owned, shared) = (name.to_owned(), Arc::clone(&pending));
                let started = std::thread::Builder::new()
                    .name("una-http-resolve".to_owned())
                    .spawn(move || {
                        let found = (self.lookup)(&owned);
                        self.finish(&owned, &shared, found, true);
                    });
                if started.is_err() {
                    let failed = Err(std::io::Error::other("no resolver thread"));
                    self.finish(name, &pending, failed, false);
                }
                pending
            }
        };
        let mut answer = pending
            .answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(found) = answer.as_ref() {
                return found.clone().map(|addrs| {
                    addrs
                        .into_iter()
                        .map(|mut addr| {
                            addr.set_port(port);
                            addr
                        })
                        .collect()
                });
            }
            let time = left(deadline)?;
            answer = pending
                .ready
                .wait_timeout(answer, time)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Joins the lookup of `name` in flight, or takes a place for a new one, first come,
    /// first served, until `deadline`; `Unreachable` at once for a name that failed lately.
    fn next(&self, name: &str, deadline: Instant) -> Result<Next, Error> {
        let mut state = self.lock();
        let mut ticket: Option<u64> = None;
        let next = loop {
            let failed = state
                .failed
                .get(name)
                .is_some_and(|at| at.elapsed() < FAILED_FOR);
            if failed {
                Self::leave(&mut state, ticket);
                break Err(Error::Unreachable);
            }
            if let Some(pending) = state.pending.get(name) {
                let pending = Arc::clone(pending);
                Self::leave(&mut state, ticket);
                break Ok(Next::Join(pending));
            }
            let mine = match ticket {
                Some(mine) => mine,
                None => {
                    let mine = state.next;
                    state.next = state.next.wrapping_add(1);
                    state.line.push_back(mine);
                    ticket = Some(mine);
                    mine
                }
            };
            if state.running < state.most && Self::front(&mut state) == Some(mine) {
                state.line.pop_front();
                state.running += 1;
                let pending = Arc::new(Pending::default());
                state.pending.insert(name.to_owned(), Arc::clone(&pending));
                break Ok(Next::Start(pending));
            }
            let Ok(time) = left(deadline) else {
                Self::leave(&mut state, ticket);
                break Err(Error::TimedOut);
            };
            state = self
                .freed
                .wait_timeout(state, time)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        };
        drop(state);
        self.freed.notify_all();
        next
    }

    /// The first ticket of the line whose caller still waits, after dropping those that gave
    /// up. O(1) amortized.
    fn front(state: &mut State) -> Option<u64> {
        while let Some(first) = state.line.front().copied() {
            if !state.gave_up.remove(&first) {
                return Some(first);
            }
            state.line.pop_front();
        }
        None
    }

    /// Takes `ticket` out of the line, when the caller had one. O(1).
    fn leave(state: &mut State, ticket: Option<u64>) {
        let Some(ticket) = ticket else {
            return;
        };
        if state.line.front() == Some(&ticket) {
            state.line.pop_front();
        } else {
            state.gave_up.insert(ticket);
        }
    }

    /// Ends the lookup of `name`: gives its place back, remembers a failure when `remember`,
    /// and hands the answer to every caller of the name.
    fn finish(
        &self,
        name: &str,
        pending: &Arc<Pending>,
        found: std::io::Result<Vec<SocketAddr>>,
        remember: bool,
    ) {
        let answer = match found {
            Ok(addrs) if !addrs.is_empty() => Ok(addrs),
            _ => Err(Error::Unreachable),
        };
        {
            let mut state = self.lock();
            state.running = state.running.saturating_sub(1);
            if state
                .pending
                .get(name)
                .is_some_and(|current| Arc::ptr_eq(current, pending))
            {
                state.pending.remove(name);
            }
            if remember && answer.is_err() {
                state.failed.insert(name.to_owned(), Instant::now());
                if state.failed.len() > state.pruned * 2 + 64 {
                    state.failed.retain(|_, at| at.elapsed() < FAILED_FOR);
                    state.pruned = state.failed.len();
                }
            }
        }
        self.freed.notify_all();
        *pending
            .answer
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(answer);
        pending.ready.notify_all();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;

    fn leaked(most: usize, lookup: Lookup) -> &'static Resolver {
        Box::leak(Box::new(Resolver::new(most, lookup)))
    }

    fn local() -> Vec<SocketAddr> {
        vec![SocketAddr::from(([127, 0, 0, 1], 0))]
    }

    static SHARED_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn slow(_name: &str) -> std::io::Result<Vec<SocketAddr>> {
        SHARED_CALLS.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(200));
        Ok(local())
    }

    #[test]
    fn concurrent_callers_of_a_name_share_one_lookup() {
        let resolver = leaked(4, slow);
        let deadline = Instant::now() + Duration::from_secs(10);
        let callers: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(move || resolver.resolve("shared.test", 80, deadline)))
            .collect();
        for caller in callers {
            let addrs = caller.join().unwrap().unwrap();
            assert_eq!(addrs, [SocketAddr::from(([127, 0, 0, 1], 80))]);
        }
        assert_eq!(SHARED_CALLS.load(Ordering::SeqCst), 1);
    }

    static RELEASE: AtomicBool = AtomicBool::new(false);

    fn stalls(name: &str) -> std::io::Result<Vec<SocketAddr>> {
        if name == "stalled.test" {
            let started = Instant::now();
            while !RELEASE.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(20) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Ok(local())
    }

    #[test]
    fn a_stalled_name_holds_one_place_and_never_blocks_the_others() {
        // Review R7-10: two places; many callers of a name whose lookup never answers give up
        // at their deadlines, and the name holds one place, so another name still resolves.
        let resolver = leaked(2, stalls);
        let soon = Instant::now() + Duration::from_millis(100);
        let callers: Vec<_> = (0..16)
            .map(|_| std::thread::spawn(move || resolver.resolve("stalled.test", 80, soon)))
            .collect();
        for caller in callers {
            assert_eq!(caller.join().unwrap(), Err(Error::TimedOut));
        }
        let started = Instant::now();
        let later = Instant::now() + Duration::from_secs(5);
        assert!(resolver.resolve("other.test", 80, later).is_ok());
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(resolver.lock().running, 1);
        RELEASE.store(true, Ordering::SeqCst);
    }

    static FAILED_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn fails(_name: &str) -> std::io::Result<Vec<SocketAddr>> {
        FAILED_CALLS.fetch_add(1, Ordering::SeqCst);
        Err(std::io::Error::other("no such name"))
    }

    #[test]
    fn a_failed_name_is_remembered_for_a_few_seconds() {
        let resolver = leaked(2, fails);
        let deadline = Instant::now() + Duration::from_secs(5);
        assert_eq!(
            resolver.resolve("bad.test", 80, deadline),
            Err(Error::Unreachable)
        );
        assert_eq!(
            resolver.resolve("bad.test", 80, deadline),
            Err(Error::Unreachable)
        );
        assert_eq!(FAILED_CALLS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_line_serves_first_come_first_served_and_skips_those_that_left() {
        let resolver = leaked(1, slow);
        let mut state = resolver.lock();
        state.line.extend([1, 2, 3]);
        Resolver::leave(&mut state, Some(2));
        assert_eq!(Resolver::front(&mut state), Some(1));
        Resolver::leave(&mut state, Some(1));
        assert_eq!(Resolver::front(&mut state), Some(3));
        assert!(state.gave_up.is_empty());
    }
}
