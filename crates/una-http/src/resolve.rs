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
//! - **At most a quarter of the threads per zone** (the name without its first label), at
//!   least 1, so a zone whose many names never resolve, such as a wildcard one, leaves the other
//!   places to other zones; a caller whose zone has its share waits out of the line.
//! - **A lookup that panics** ends as a failure through a guard in its thread, so its place and
//!   its callers are never lost.
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
    /// The lookups running in each zone ([`zone`]), for the zones that have one.
    zones: HashMap<String, usize>,
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

/// The zone of a name: the name without its first label (`a.example.net` is in
/// `example.net`), or the name itself when it has one label. The lookups that run in one zone
/// are bounded ([`State::per_zone`]), so a zone whose names never resolve, such as a wildcard
/// one that answers nothing for any of its many names, cannot take every place (review R7-13).
fn zone(name: &str) -> &str {
    name.split_once('.').map_or(name, |(_, rest)| rest)
}

impl State {
    /// The most lookups that run at once in one zone: a quarter of the bound, at least 1.
    fn per_zone(&self) -> usize {
        (self.most / 4).max(1)
    }
}

/// Ends a lookup when its thread ends, with a failure when the lookup did not return: a
/// lookup that panics, or a thread that never started, gives its place back and answers its
/// callers (review R7-14).
struct Finishing {
    resolver: &'static Resolver,
    name: String,
    pending: Arc<Pending>,
    done: bool,
}

impl Drop for Finishing {
    fn drop(&mut self) {
        if !self.done {
            let failed = Err(std::io::Error::other("the lookup did not return"));
            self.resolver
                .finish(&self.name, &self.pending, failed, false);
        }
    }
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
                zones: HashMap::new(),
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
                let mut guard = Finishing {
                    resolver: self,
                    name: name.to_owned(),
                    pending: Arc::clone(&pending),
                    done: false,
                };
                // A thread that cannot start drops its closure, and the guard in it ends the
                // lookup with a failure.
                let _ = std::thread::Builder::new()
                    .name("una-http-resolve".to_owned())
                    .spawn(move || {
                        let found = (guard.resolver.lookup)(&guard.name);
                        guard.done = true;
                        guard
                            .resolver
                            .finish(&guard.name, &guard.pending, found, true);
                    });
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
            // A zone that runs its share of lookups: the caller waits out of the line, so it
            // holds no one behind it, and joins its end again once the zone has room.
            let in_zone = state.zones.get(zone(name)).copied().unwrap_or(0);
            if in_zone >= state.per_zone() {
                if ticket.is_some() {
                    Self::leave(&mut state, ticket.take());
                    self.freed.notify_all();
                }
            } else {
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
                    *state.zones.entry(zone(name).to_owned()).or_insert(0) += 1;
                    let pending = Arc::new(Pending::default());
                    state.pending.insert(name.to_owned(), Arc::clone(&pending));
                    break Ok(Next::Start(pending));
                }
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
            let zone = zone(name);
            if let Some(count) = state.zones.get_mut(zone) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    state.zones.remove(zone);
                }
            }
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

    static ZONE_RELEASE: AtomicBool = AtomicBool::new(false);

    fn stalls_a_zone(name: &str) -> std::io::Result<Vec<SocketAddr>> {
        if name.ends_with(".wild.test") {
            let started = Instant::now();
            while !ZONE_RELEASE.load(Ordering::SeqCst)
                && started.elapsed() < Duration::from_secs(20)
            {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Ok(local())
    }

    #[test]
    fn a_stalled_zone_of_many_names_leaves_places_to_the_other_zones() {
        // Review R7-13: four places, so one per zone; sixteen distinct names of a wildcard zone
        // that never answers hold one place between them, and a healthy host still resolves.
        let resolver = leaked(4, stalls_a_zone);
        let soon = Instant::now() + Duration::from_millis(100);
        let callers: Vec<_> = (0..16)
            .map(|i| {
                std::thread::spawn(move || resolver.resolve(&format!("n{i}.wild.test"), 80, soon))
            })
            .collect();
        for caller in callers {
            assert_eq!(caller.join().unwrap(), Err(Error::TimedOut));
        }
        assert_eq!(resolver.lock().running, 1);
        let started = Instant::now();
        let later = Instant::now() + Duration::from_secs(5);
        assert!(resolver.resolve("healthy.example", 80, later).is_ok());
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(resolver.lock().running, 1);
        ZONE_RELEASE.store(true, Ordering::SeqCst);
    }

    fn panics(name: &str) -> std::io::Result<Vec<SocketAddr>> {
        if name == "boom.test" {
            panic!("a lookup that panics");
        }
        Ok(local())
    }

    #[test]
    fn a_lookup_that_panics_gives_its_place_back_and_answers_its_callers() {
        // Review R7-14: with one place, the panicking lookup answers `Unreachable` and gives
        // its place back, so the next name resolves.
        let resolver = leaked(1, panics);
        let deadline = Instant::now() + Duration::from_secs(5);
        let started = Instant::now();
        assert_eq!(
            resolver.resolve("boom.test", 80, deadline),
            Err(Error::Unreachable)
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        {
            let state = resolver.lock();
            assert_eq!(state.running, 0);
            assert!(state.pending.is_empty() && state.zones.is_empty());
        }
        assert!(resolver.resolve("fine.test", 80, deadline).is_ok());
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
