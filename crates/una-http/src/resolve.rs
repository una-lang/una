//! Reaching a server on bounded pools of threads: name resolution and the TCP connection
//! (`spec/24 §4` rule 2, ADR 0116 amendment P8 and P9, #1148). The reactor of ADR 0117 (slice
//! S7) keeps them: its job on a helper of the I/O pool hands them to these pools too.
//!
//! The host's resolver has no timeout, nothing in the process can cut a lookup, and `connect`
//! cannot be woken without code of `una-sys` of its own. So a request never runs either on its
//! own thread, the I/O helper's: it hands each to a pool and waits for the answer only until its
//! deadline, an absolute instant, or until its task's cancellation abandons it. On either, it
//! stops waiting and fails with `TimedOut` or `Interrupted`; the lookup or the connection attempt
//! in progress runs to its end on the pool's thread, which then closes the connection, if one
//! came, without using it.
//!
//! - **Two pools.** Lookups run on the lookup pool, [`lookup_pool_size`] threads,
//!   `max(2, --io-threads / 4)`: a lookup normally takes milliseconds against an exchange's tens
//!   to hundreds. Connections run on the connect pool, `--io-threads` threads, one per helper, so
//!   connecting is never capped below what the helpers could do themselves (performance parity).
//!   An address literal skips the lookup pool. Both start all their threads at once ([`start`]),
//!   which an engine does before `main` in a run whose grants include `Http`, with its other
//!   pools (P8), so whether a pool thread exists is never state left by earlier code; their
//!   stacks are reserved, 256 KiB each, and committed only as they are used. With the helpers,
//!   the run's `Http` threads are at most `2 × --io-threads + max(2, --io-threads / 4)`.
//! - **Per name and per zone** (#1265, ADR 0117 S7 review R7-10 and R7-13). A request for a
//!   name whose lookup is queued or running joins it while another request still waits for it,
//!   and waits for its answer until its own deadline: requests of one name waiting together share
//!   one thread of the lookup pool, whatever their rate. A lookup whose requests all gave up takes
//!   no new one (review M1): a later request of the name starts its own lookup, so residue never
//!   answers a later request. Each pool admits at most [`zone_share`] of its threads, a quarter
//!   and at least 1, to the steps of one zone ([`zone_of`]: a name's last two labels, an IPv4
//!   literal's `/24`, an IPv4-mapped IPv6 literal's included, another IPv6 literal's `/48`); the
//!   residue of requests that gave up counts in it. A step of a zone at its share waits in the
//!   zone's own line, out of the pool's, so it holds no one behind it, and joins the end of the
//!   pool's line once a step of its zone ends. So the names of one zone that never resolve, a
//!   wildcard zone's, or the requests to the `/24` or `/48` of address literals that drop every
//!   packet hold at most a quarter of a pool, and the other zones keep the rest, first come,
//!   first served. A connection's zone is its request host's, so names of distinct zones that
//!   resolve to one unresponsive network are not bounded together.
//! - **Residue reaches later requests.** A step left running after its request gave up, its
//!   deadline's cut included, keeps its zone's place until it returns: at a zone share of k, k
//!   such steps, which code without tasks can leave one after the other, delay a later request
//!   of their zone, possibly past that request's deadline (k is 1 for pools of fewer than 8
//!   threads; the default `--io-threads` 64 gives a lookup pool of 16 with k = 4 and a connect
//!   pool of 64 with k = 16). That is residue (`spec/26 §7` rule 5), which `L_res` does not
//!   yet count for a deadline's cut: ADR 0117's amendment of 2026-11-28 (Proposed, #1717) makes
//!   it count, item 2, and this share MUST NOT ship before that is accepted and implemented.
//! - **The hand-off.** Each pool job costs, on average, O(1) map operations on the name and the
//!   zone (linear in the name's length), one push and one pop under the pool's mutex, a wake of
//!   one pool thread, and a reply on the request's own channel: about two thread wakes per step,
//!   against the milliseconds the step itself takes.
//! - **Hangs.** When every thread a zone may use holds a step that hangs, the zone's new steps
//!   wait in its line, and each of their requests still gives up exactly at its own deadline. A
//!   job still waiting when its request gives up never runs. Without [`start`], a request fails
//!   with `Unreachable`: every engine starts the pools.
//! - **Space.** Jobs whose requests gave up stay in a line until they are reached or the line
//!   has doubled since it was last pruned, when they are dropped: each line holds at most twice
//!   the jobs it kept at its last prune, or `2 × 16`, so a pool holds O(peak waiting requests +
//!   threads) jobs, names and zones, however long its steps hang. A prune costs amortized O(1)
//!   per job, and the refill of the zones it freed, in arrival order, a sort: amortized
//!   O(log n) per job at worst on that path, n the jobs of the pool's line.
//! - **Residue.** A lookup or a connection whose request gave up keeps its pool thread until it
//!   returns, beside later code: the residue that `L_res` labels (`spec/26 §7` rule 5), at most
//!   the pools' sizes. A connection job looks at its request only between two addresses, so an
//!   attempt in progress runs until it connects or the request's deadline passes. A step that
//!   panics ends as a failure at once: its thread and its zone's place are kept, and its
//!   requests get `Unreachable`. A connection job given its request's place among the sockets
//!   (`spec/27 §2` rule 3, #1824) keeps it until it has closed what it reached, so its residue
//!   never holds a socket outside the cap.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};
use std::time::{Duration, Instant};

use crate::client::Error;
use crate::limit::Abandon;

/// A lookup function: the addresses of a name and a port, as the system's resolver gives them.
pub type Lookup = fn(&str, u16) -> std::io::Result<Vec<SocketAddr>>;

/// A connect function: a TCP connection to an address, within a time-out.
pub type Connect = fn(&SocketAddr, Duration) -> std::io::Result<TcpStream>;

/// The fewest threads of the lookup pool.
pub const MIN_RESOLVERS: usize = 2;

/// The size of the lookup pool for a run of `io_threads` I/O helpers: `max(2, io_threads / 4)`.
/// A lookup normally takes milliseconds against an exchange's tens to hundreds, so a quarter of
/// the helpers serve their lookups without a queue, and lookups that hang hold at most that many.
pub fn lookup_pool_size(io_threads: usize) -> usize {
    (io_threads / 4).max(MIN_RESOLVERS)
}

/// The size of the connect pool for a run of `io_threads` I/O helpers: one thread per helper,
/// so connections are never capped below what the helpers could make themselves.
pub fn connect_pool_size(io_threads: usize) -> usize {
    io_threads.max(1)
}

/// The most threads a run's `Http` requests take for `io_threads` helpers: the helpers, the
/// connect pool and the lookup pool, `2 × io_threads + max(2, io_threads / 4)`.
pub fn thread_bound(io_threads: usize) -> usize {
    io_threads
        .saturating_add(connect_pool_size(io_threads))
        .saturating_add(lookup_pool_size(io_threads))
}

/// The most threads of a pool of `size` threads that the steps of one zone hold at once: a
/// quarter, and at least 1 (#1265). A lookup pool of 16 gives 4, one of 2 gives 1.
pub(crate) fn zone_share(size: usize) -> usize {
    (size.max(1) / 4).max(1)
}

/// The zone of a host (#1265), whose steps share [`zone_share`] of a pool: the host's last two
/// labels, in lower case and without a final dot (`a.b.example.net` and `example.net` are in
/// `example.net`, a name of one label is its own zone), the `/24` of an IPv4 literal, an
/// IPv4-mapped IPv6 literal's included, and the `/48` of another IPv6 literal. Two labels, not
/// the name without its first: a wildcard zone answers for names of any depth, and
/// `x.y.example.net` must not escape `example.net`'s share by its `y`. The cost is fairness
/// only: the names of a public suffix of two labels, such as `co.uk`, share one zone. O(the
/// host's length).
pub(crate) fn zone_of(host: &str) -> String {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<IpAddr>().map(|ip| ip.to_canonical()) {
        Ok(IpAddr::V4(ip)) => {
            let [a, b, c, _] = ip.octets();
            format!("{a}.{b}.{c}.0/24")
        }
        Ok(IpAddr::V6(ip)) => {
            let [a, b, c, ..] = ip.segments();
            format!("{a:x}:{b:x}:{c:x}::/48")
        }
        Err(_) => {
            let name = host.trim_end_matches('.').to_ascii_lowercase();
            match name.rmatch_indices('.').nth(1) {
                Some((at, _)) => name[at + 1..].to_owned(),
                None => name,
            }
        }
    }
}

/// The key of a name's shared lookup: the name in lower case, as the resolver compares names.
fn name_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// What a lookup answers its callers: the addresses found, with port 0, or a failure.
type Found = Result<Vec<SocketAddr>, ()>;

/// A job on a pool: what it runs, given whether its request gave up, that flag, its zone, and,
/// for a lookup, the name whose shared lookup it is.
struct Job {
    run: Box<dyn FnOnce(&AtomicBool) + Send>,
    gave_up: Arc<AtomicBool>,
    zone: Arc<str>,
    name: Option<Arc<str>>,
    seq: u64,
}

/// The fewest jobs a line holds before it is pruned of those whose requests gave up.
const PRUNE_FLOOR: usize = 16;

/// A line of jobs, pruned of those whose requests gave up when it has doubled since it was last
/// pruned: amortized O(1) per job, and at most twice the jobs it kept at its last prune, or
/// `2 × PRUNE_FLOOR`.
#[derive(Default)]
struct Line {
    jobs: VecDeque<Job>,
    kept: usize,
}

impl Line {
    /// Whether the line has doubled since it was last pruned.
    fn full(&self) -> bool {
        self.jobs.len() > 2 * self.kept.max(PRUNE_FLOOR)
    }

    /// Takes the jobs whose requests gave up out of the line, in order.
    fn prune(&mut self) -> Vec<Job> {
        let mut live = VecDeque::with_capacity(self.jobs.len());
        let mut dead = Vec::new();
        for job in self.jobs.drain(..) {
            if job.gave_up.load(Ordering::SeqCst) {
                dead.push(job);
            } else {
                live.push_back(job);
            }
        }
        self.jobs = live;
        self.kept = self.jobs.len();
        dead
    }
}

/// A zone of a pool: how many of its jobs the pool admitted, queued in the pool's line or
/// running, at most [`zone_share`], and the line of those waiting for a place.
#[derive(Default)]
struct Zone {
    admitted: usize,
    waiting: Line,
}

/// A name's shared lookup: the callers waiting for its answer, by id, and the flag of its job,
/// set when no caller waits any more.
struct Flight {
    callers: HashMap<u64, mpsc::Sender<Answer<Found>>>,
    gave_up: Arc<AtomicBool>,
}

/// What a pool's mutex keeps: the pool's line of admitted jobs, its zones, the names whose
/// lookups are queued or running, and the next caller's id.
#[derive(Default)]
struct Queues {
    line: Line,
    zones: HashMap<Arc<str>, Zone>,
    flights: HashMap<Arc<str>, Flight>,
    next_caller: u64,
    next_seq: u64,
}

impl Queues {
    /// Adds `job` to the pool's line when its zone has a place, else to its zone's line; returns
    /// how many jobs joined the pool's line, each of which a pool thread must be woken for.
    fn submit(&mut self, share: usize, mut job: Job) -> usize {
        job.seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        let zone = self.zones.entry(Arc::clone(&job.zone)).or_default();
        if zone.admitted < share {
            zone.admitted += 1;
            self.line.jobs.push_back(job);
            if self.line.full() {
                for dead in self.line.prune() {
                    self.forget(dead);
                }
                return self.refill_all(share) + 1;
            }
            1
        } else {
            zone.waiting.jobs.push_back(job);
            if zone.waiting.full() {
                let dead = zone.waiting.prune();
                for job in dead {
                    self.drop_flight(&job);
                }
            }
            0
        }
    }

    /// Drops a job whose request gave up, which held a place of its zone, and gives the place
    /// back. The caller then refills the zone.
    fn forget(&mut self, job: Job) {
        self.drop_flight(&job);
        if let Some(zone) = self.zones.get_mut(&job.zone) {
            zone.admitted = zone.admitted.saturating_sub(1);
        }
    }

    /// Removes the shared lookup whose job `job` is, if it still is the name's.
    fn drop_flight(&mut self, job: &Job) {
        if let Some(name) = &job.name {
            let theirs = self
                .flights
                .get(name)
                .is_some_and(|flight| Arc::ptr_eq(&flight.gave_up, &job.gave_up));
            if theirs {
                self.flights.remove(name);
            }
        }
    }

    /// Gives a place of `zone` back, when one of its admitted jobs ended or was dropped, and
    /// refills it; returns how many jobs joined the pool's line.
    fn release(&mut self, share: usize, zone: &Arc<str>) -> usize {
        if let Some(entry) = self.zones.get_mut(zone) {
            entry.admitted = entry.admitted.saturating_sub(1);
        }
        self.refill(share, zone)
    }

    /// Moves the first jobs still wanted of `zone`'s line to the end of the pool's line while
    /// the zone has a place, dropping those whose requests gave up, and removes a zone left
    /// empty; returns how many joined.
    fn refill(&mut self, share: usize, zone: &Arc<str>) -> usize {
        let mut joined = 0;
        loop {
            let Some(entry) = self.zones.get_mut(zone) else {
                return joined;
            };
            if entry.admitted >= share {
                return joined;
            }
            match entry.waiting.jobs.pop_front() {
                Some(job) if job.gave_up.load(Ordering::SeqCst) => self.drop_flight(&job),
                Some(job) => {
                    entry.admitted += 1;
                    self.line.jobs.push_back(job);
                    joined += 1;
                }
                None => {
                    if entry.admitted == 0 {
                        self.zones.remove(zone);
                    }
                    return joined;
                }
            }
        }
    }

    /// Drops the jobs whose requests gave up from the front of `zone`'s line and returns the
    /// arrival number of its first job still wanted, `u64::MAX` when none is. Amortized O(1):
    /// each job is dropped once.
    fn first_live(&mut self, zone: &Arc<str>) -> u64 {
        loop {
            let Some(entry) = self.zones.get_mut(zone) else {
                return u64::MAX;
            };
            match entry.waiting.jobs.front() {
                Some(job) if job.gave_up.load(Ordering::SeqCst) => {
                    if let Some(dead) = entry.waiting.jobs.pop_front() {
                        self.drop_flight(&dead);
                    }
                }
                Some(job) => return job.seq,
                None => return u64::MAX,
            }
        }
    }

    /// Refills every zone with a free place, after a prune of the pool's line, in the order in
    /// which their first waiting jobs still wanted came, never the maps' (first come, first
    /// served); returns how many jobs joined. The filter costs O(zones), which the line and the
    /// threads bound, and the sort O(z log z) for the z zones with a free place, at most the
    /// jobs the prune dropped: so the prune path costs amortized O(log n) per job at worst, n
    /// the jobs of the pool's line, against O(1) on every other path.
    fn refill_all(&mut self, share: usize) -> usize {
        let free: Vec<Arc<str>> = self
            .zones
            .iter()
            .filter(|(_, zone)| zone.admitted < share)
            .map(|(name, _)| Arc::clone(name))
            .collect();
        let mut keyed: Vec<(u64, Arc<str>)> = free
            .into_iter()
            .map(|zone| (self.first_live(&zone), zone))
            .collect();
        // Arrival numbers are unique; zones with no job still wanted tie at `u64::MAX` and are
        // ordered by name, and only removed.
        keyed.sort_unstable();
        keyed.iter().map(|(_, zone)| self.refill(share, zone)).sum()
    }
}

/// A pool: its queues, the threads it was made with, how many of them started, and the share
/// of one zone.
struct Pool {
    queues: Mutex<Queues>,
    ready: Condvar,
    wanted: usize,
    share: usize,
    threads: AtomicUsize,
}

/// The two pools of the process.
struct Pools {
    lookups: Arc<Pool>,
    connects: Arc<Pool>,
}

/// The process's pools, made once, by [`start`].
static POOLS: OnceLock<Pools> = OnceLock::new();

/// Locks a pool's queues, recovering them when a thread panicked while holding the lock: each
/// critical section leaves them consistent.
fn lock(pool: &Pool) -> MutexGuard<'_, Queues> {
    pool.queues.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Wakes `count` threads of `pool`, for jobs that joined its line.
fn wake(pool: &Pool, count: usize) {
    for _ in 0..count {
        pool.ready.notify_one();
    }
}

/// Makes a pool with `size` threads named `name`, all started now.
fn make(size: usize, name: &str) -> Arc<Pool> {
    let pool = Arc::new(Pool {
        queues: Mutex::new(Queues::default()),
        ready: Condvar::new(),
        wanted: size.max(1),
        share: zone_share(size),
        threads: AtomicUsize::new(0),
    });
    for _ in 0..size.max(1) {
        let serving = Arc::clone(&pool);
        let spawned = std::thread::Builder::new()
            .name(name.to_owned())
            .stack_size(256 * 1024)
            .spawn(move || serve(&serving));
        if spawned.is_ok() {
            pool.threads.fetch_add(1, Ordering::SeqCst);
        }
    }
    pool
}

/// Makes the two pools for `io_threads` helpers.
fn make_pools(io_threads: usize) -> Pools {
    Pools {
        lookups: make(lookup_pool_size(io_threads), "una-http-lookup"),
        connects: make(connect_pool_size(io_threads), "una-http-connect"),
    }
}

/// A pool thread could not start ([`start`]), which the engine reports before `main`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotStarted;

/// Starts the process's pools for a run of `io_threads` I/O helpers, all their threads at once,
/// unless they exist: what an engine calls before `main` in a run whose grants include `Http`.
/// `Err` when a thread could not start, which the engine reports before `main`. The first call
/// sets the sizes for the process.
pub fn start(io_threads: usize) -> Result<(), NotStarted> {
    let pools = POOLS.get_or_init(|| make_pools(io_threads));
    let full = |pool: &Pool| pool.threads.load(Ordering::SeqCst) == pool.wanted;
    if full(&pools.lookups) && full(&pools.connects) {
        Ok(())
    } else {
        Err(NotStarted)
    }
}

/// The threads the process's pools have started, the lookup pool's and the connect pool's:
/// `(0, 0)` before [`start`].
pub fn threads() -> (usize, usize) {
    POOLS.get().map_or((0, 0), |pools| {
        (
            pools.lookups.threads.load(Ordering::SeqCst),
            pools.connects.threads.load(Ordering::SeqCst),
        )
    })
}

/// What a pool thread does: takes the next job of the pool's line whose request still waits,
/// dropping the others unrun, runs it, and gives its zone's place back, a panic of the job
/// included.
fn serve(pool: &Pool) {
    loop {
        let job = {
            let mut queues = lock(pool);
            loop {
                match queues.line.jobs.pop_front() {
                    Some(job) if job.gave_up.load(Ordering::SeqCst) => {
                        queues.drop_flight(&job);
                        let joined = queues.release(pool.share, &job.zone);
                        wake(pool, joined);
                    }
                    Some(job) => break job,
                    None => {
                        queues = pool
                            .ready
                            .wait(queues)
                            .unwrap_or_else(PoisonError::into_inner);
                    }
                }
            }
        };
        let Job {
            run, gave_up, zone, ..
        } = job;
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| run(&gave_up)));
        let joined = lock(pool).release(pool.share, &zone);
        wake(pool, joined);
    }
}

/// What a waiting request gets: the job's answer, the wake of its abandon, or the failure of a
/// step that panicked.
enum Answer<T> {
    Done(T),
    Abandoned,
    Failed,
}

/// Sends [`Answer::Failed`] to a step's request unless the step answered: dropped by the unwind
/// of a step that panics, it fails the request at once with `Unreachable` instead of at its
/// deadline (the abandon's wake keeps the channel open).
struct Failing<T> {
    reply: Option<mpsc::Sender<Answer<T>>>,
}

impl<T> Drop for Failing<T> {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Answer::Failed);
        }
    }
}

/// The time left before `deadline` for a request, or why it stops at once.
fn time_left(deadline: Instant, abandon: &Abandon) -> Result<Duration, Error> {
    if abandon.abandoned() {
        return Err(Error::Interrupted);
    }
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(Error::TimedOut);
    }
    Ok(left)
}

/// Waits for `answers` for `left`, or until the abandon's wake; the error of a wait that ended
/// without the job's answer.
fn wait<T>(
    answers: &mpsc::Receiver<Answer<T>>,
    left: Duration,
    abandon: &Abandon,
) -> Result<T, Error> {
    let answer = answers.recv_timeout(left);
    abandon.clear_hook();
    match answer {
        Ok(Answer::Done(done)) => Ok(done),
        Ok(Answer::Abandoned) => Err(Error::Interrupted),
        Ok(Answer::Failed) => Err(Error::Unreachable),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(Error::TimedOut),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(Error::Unreachable),
    }
}

/// Hands `step` to `pool`, in `zone`, and waits for its answer until `deadline`, `TimedOut`, or
/// until `abandon` wakes the wait, `Interrupted`, with no polling. On either, the job is marked
/// given up: waiting, it never runs; running, it ends alone and its answer is dropped. O(1) on
/// average besides the wait.
///
/// `hold`, when given, is what the request keeps while the step may hold a resource of it, its
/// place among the sockets of the process (`spec/27 §2` rule 3, #1824): the job takes it when
/// it starts and drops it only after the step's answer, and with it any connection the request
/// no longer waits for, is dropped; a job that its request gave up before it started never runs,
/// and the request drops the hold itself. So a place is never given back while a socket of its
/// request is still open on the pool. One more lock, uncontended but for that race, O(1).
fn hand_off<T: Send + 'static>(
    pool: &Pool,
    zone: &str,
    deadline: Instant,
    abandon: &Abandon,
    hold: Option<Hold>,
    step: impl FnOnce(&AtomicBool) -> T + Send + 'static,
) -> Result<T, Error> {
    let left = time_left(deadline, abandon)?;
    // The hold, until the job takes it or its request gives up before the job started.
    let held = hold.map(|hold| Arc::new(Mutex::new(Some(hold))));
    let taken = held.clone();
    let (reply, answers) = mpsc::channel::<Answer<T>>();
    let waker = reply.clone();
    abandon.on_abandon(Box::new(move || {
        let _ = waker.send(Answer::Abandoned);
    }));
    let gave_up = Arc::new(AtomicBool::new(false));
    let job = Job {
        run: Box::new(move |gave_up: &AtomicBool| {
            // A hold its request took back means the request gave up before the job started:
            // the step does not run.
            let hold = match &taken {
                Some(taken) => match take(taken) {
                    Some(hold) => Some(hold),
                    None => return,
                },
                None => None,
            };
            let mut failing = Failing { reply: Some(reply) };
            let answer = step(gave_up);
            if let Some(reply) = failing.reply.take() {
                // An answer its request no longer waits for is dropped here, before the hold.
                let _ = reply.send(Answer::Done(answer));
            }
            drop(failing);
            drop(hold);
        }),
        gave_up: Arc::clone(&gave_up),
        zone: Arc::from(zone),
        name: None,
        seq: 0,
    };
    let joined = lock(pool).submit(pool.share, job);
    wake(pool, joined);
    let answer = wait(&answers, left, abandon);
    if answer.is_err() {
        gave_up.store(true, Ordering::SeqCst);
        // A job that has not started holds nothing of the request: its hold is dropped now. A
        // job that started keeps it until its step's answer is dropped.
        if let Some(held) = &held {
            drop(take(held));
        }
    }
    answer
}

/// What a request keeps while a pool job of it may hold its socket: its place among the
/// sockets of the process (`spec/27 §2` rule 3), given back when dropped.
pub type Hold = Box<dyn Send>;

/// Takes the hold out of `held`, if it is still there.
fn take(held: &Mutex<Option<Hold>>) -> Option<Hold> {
    held.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// Ends a name's shared lookup when its job ends, normally or by a panic: removes it from the
/// pool's names and sends its answer to its callers, a failure when the lookup panicked.
struct Landing {
    pool: Arc<Pool>,
    name: Arc<str>,
    gave_up: Arc<AtomicBool>,
    found: Option<Found>,
}

impl Drop for Landing {
    fn drop(&mut self) {
        let flight = {
            let mut queues = lock(&self.pool);
            let theirs = queues
                .flights
                .get(&self.name)
                .is_some_and(|flight| Arc::ptr_eq(&flight.gave_up, &self.gave_up));
            if theirs {
                queues.flights.remove(&self.name)
            } else {
                None
            }
        };
        if let (Some(flight), Some(found)) = (flight, self.found.take()) {
            for caller in flight.callers.into_values() {
                let _ = caller.send(Answer::Done(found.clone()));
            }
        }
    }
}

/// The addresses of `name` with `port`, looked up by `lookup` on `pool` (#1148), one lookup per
/// name for the callers that wait for it together (#1265): a caller of a name whose lookup is
/// queued or running joins it while another caller still waits for it, and each caller waits
/// for its answer until its own deadline, `TimedOut`, or until `abandon` wakes it,
/// `Interrupted`. A lookup whose callers all gave up takes no new caller: it never runs if it
/// has not started, and else runs to its end unseen, while a later caller of the name starts a
/// lookup of its own, within its zone's share (review M1 of #1574). The name is looked up with
/// port 0, which the system's resolver ignores, and each caller's port is set on the answer.
/// O(1) on average besides the wait and the name's length.
fn look_up(
    pool: &Arc<Pool>,
    lookup: Lookup,
    name: &str,
    port: u16,
    deadline: Instant,
    abandon: &Abandon,
) -> Result<Found, Error> {
    let left = time_left(deadline, abandon)?;
    let key: Arc<str> = Arc::from(name_key(name));
    let (reply, answers) = mpsc::channel::<Answer<Found>>();
    let waker = reply.clone();
    abandon.on_abandon(Box::new(move || {
        let _ = waker.send(Answer::Abandoned);
    }));
    let (caller, gave_up, joined) = {
        let mut queues = lock(pool);
        let caller = queues.next_caller;
        queues.next_caller = caller.wrapping_add(1);
        // Only a lookup that a caller still waits for takes a new one (#1265, review M1): the
        // residue of requests that all gave up never changes a later request's outcome by
        // answering it; a later caller starts a lookup of its own, within its zone's share.
        if let Some(flight) = queues
            .flights
            .get_mut(&key)
            .filter(|flight| !flight.callers.is_empty())
        {
            flight.callers.insert(caller, reply);
            (caller, Arc::clone(&flight.gave_up), 0)
        } else {
            let gave_up = Arc::new(AtomicBool::new(false));
            let landing_pool = Arc::clone(pool);
            let landing_name = Arc::clone(&key);
            let landing_flag = Arc::clone(&gave_up);
            let job = Job {
                run: Box::new(move |_: &AtomicBool| {
                    // A panic of the lookup leaves the failure for the callers.
                    let mut landing = Landing {
                        pool: landing_pool,
                        name: Arc::clone(&landing_name),
                        gave_up: landing_flag,
                        found: Some(Err(())),
                    };
                    let found = lookup(&landing_name, 0).map_err(drop);
                    landing.found = Some(found);
                }),
                gave_up: Arc::clone(&gave_up),
                zone: Arc::from(zone_of(&key)),
                name: Some(Arc::clone(&key)),
                seq: 0,
            };
            let mut callers = HashMap::new();
            callers.insert(caller, reply);
            queues.flights.insert(
                Arc::clone(&key),
                Flight {
                    callers,
                    gave_up: Arc::clone(&gave_up),
                },
            );
            let joined = queues.submit(pool.share, job);
            (caller, gave_up, joined)
        }
    };
    wake(pool, joined);
    let answer = wait(&answers, left, abandon);
    if answer.is_err() {
        let mut queues = lock(pool);
        if let Some(flight) = queues.flights.get_mut(&key)
            && Arc::ptr_eq(&flight.gave_up, &gave_up)
        {
            flight.callers.remove(&caller);
            if flight.callers.is_empty() {
                flight.gave_up.store(true, Ordering::SeqCst);
            }
        }
    }
    answer.map(|found| {
        found.map(|addrs| {
            addrs
                .into_iter()
                .map(|mut addr| {
                    addr.set_port(port);
                    addr
                })
                .collect()
        })
    })
}

/// Connects to the first of `addrs` that accepts, each attempt bounded by the time left before
/// `deadline`, on a thread of the connect pool; between two addresses it stops when its request
/// gave up. A connection that comes after the request gave up is closed here.
fn connect_any(
    addrs: Vec<SocketAddr>,
    deadline: Instant,
    gave_up: &AtomicBool,
) -> Result<TcpStream, Error> {
    for addr in addrs {
        if gave_up.load(Ordering::SeqCst) {
            return Err(Error::Interrupted);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(Error::TimedOut);
        }
        match connect()(&addr, left) {
            Ok(connected) if gave_up.load(Ordering::SeqCst) => {
                drop(connected);
                return Err(Error::Interrupted);
            }
            Ok(connected) => return Ok(connected),
            Err(_) => {}
        }
    }
    if Instant::now() >= deadline {
        Err(Error::TimedOut)
    } else {
        Err(Error::Unreachable)
    }
}

/// The system's resolver.
fn system_lookup(name: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
    (name, port).to_socket_addrs().map(Iterator::collect)
}

/// The system's `connect`, within a time-out.
fn system_connect(addr: &SocketAddr, timeout: Duration) -> std::io::Result<TcpStream> {
    TcpStream::connect_timeout(addr, timeout)
}

/// The marker the release gate looks for in the shipped `una` and `libuna_rt.a`, by `nm` and in
/// their raw bytes (`scripts/check-release.sh`): it exists only with the `test-hooks` feature,
/// and [`pools`] reads it, so a build with the hooks keeps it (#1148).
#[cfg(feature = "test-hooks")]
#[used]
pub static UNA_HTTP_TEST_HOOKS_MARKER: [u8; 26] = *b"UNA_HTTP_TEST_HOOKS_MARKER";

/// The lookup and the connect function a test put in place of the system's, with the
/// `test-hooks` feature only, which only tests enable.
#[cfg(feature = "test-hooks")]
static TEST_LOOKUP: OnceLock<Lookup> = OnceLock::new();
#[cfg(feature = "test-hooks")]
static TEST_CONNECT: OnceLock<Connect> = OnceLock::new();

/// With the `test-hooks` feature only: makes `lookup` the resolver of the process, for tests
/// whose lookups hang. It takes effect only when nothing set one before.
#[cfg(feature = "test-hooks")]
pub fn use_test_lookup(lookup: Lookup) {
    let _ = TEST_LOOKUP.set(lookup);
}

/// With the `test-hooks` feature only: makes `connect` the connect function of the process, for
/// tests whose connections hang. It takes effect only when nothing set one before.
#[cfg(feature = "test-hooks")]
pub fn use_test_connect(connect: Connect) {
    let _ = TEST_CONNECT.set(connect);
}

/// The resolver: a test's, with the `test-hooks` feature.
#[cfg(feature = "test-hooks")]
fn lookup() -> Lookup {
    TEST_LOOKUP.get().copied().unwrap_or(system_lookup)
}

/// The resolver: the system's.
#[cfg(not(feature = "test-hooks"))]
fn lookup() -> Lookup {
    system_lookup
}

/// The connect function: a test's, with the `test-hooks` feature.
#[cfg(feature = "test-hooks")]
fn connect() -> Connect {
    TEST_CONNECT.get().copied().unwrap_or(system_connect)
}

/// The connect function: the system's.
#[cfg(not(feature = "test-hooks"))]
fn connect() -> Connect {
    system_connect
}

/// The helpers the pools of the tests of the client are made for, when none were started.
#[cfg(feature = "test-hooks")]
const TEST_IO_THREADS: usize = 4;

/// The pools, with the `test-hooks` feature: ones for [`TEST_IO_THREADS`] helpers are started
/// for the tests of the client, which call it directly, when none were.
#[cfg(feature = "test-hooks")]
fn pools() -> Option<&'static Pools> {
    let _ = std::hint::black_box(&UNA_HTTP_TEST_HOOKS_MARKER);
    Some(POOLS.get_or_init(|| make_pools(TEST_IO_THREADS)))
}

/// The pools, or `None` before [`start`]: a request made before the engine started them fails
/// with `Unreachable`, never with pools started lazily, which would be state left by earlier
/// code (P8).
#[cfg(not(feature = "test-hooks"))]
fn pools() -> Option<&'static Pools> {
    POOLS.get()
}

/// A TCP connection to `host` and `port` for a request whose deadline is `deadline` (#1148): the
/// name looked up on the lookup pool, unless it is an address literal, one lookup per name at a
/// time, then the connection made on the connect pool, each step within its zone's share of its
/// pool (#1265). The request waits for each until the deadline, `TimedOut`, or until `abandon`
/// wakes it, `Interrupted`, with no polling; a stop is seen once a wait ends. `hold`, the
/// request's place among the sockets, is kept by the connection job while it runs ([`hand_off`]).
pub(crate) fn reach(
    host: &str,
    port: u16,
    deadline: Instant,
    stopped: &dyn Fn() -> bool,
    abandon: &Abandon,
    hold: Option<Hold>,
) -> Result<TcpStream, Error> {
    if stopped() {
        return Err(Error::Interrupted);
    }
    let Some(pools) = pools() else {
        return Err(Error::Unreachable);
    };
    let addrs = match host.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => {
            let found = look_up(&pools.lookups, lookup(), host, port, deadline, abandon)?;
            if stopped() {
                return Err(Error::Interrupted);
            }
            match found {
                Ok(addrs) if !addrs.is_empty() => addrs,
                _ => return Err(Error::Unreachable),
            }
        }
    };
    let zone = zone_of(host);
    let stream = hand_off(
        &pools.connects,
        &zone,
        deadline,
        abandon,
        hold,
        move |gave_up| connect_any(addrs, deadline, gave_up),
    )??;
    if stopped() || abandon.abandoned() {
        return Err(Error::Interrupted);
    }
    Ok(stream)
}

#[cfg(test)]
mod tests {
    //! Per-name and per-zone bounds on the pools (#1265, ADR 0117 S7 review R7-10 and R7-13).
    //! Each test makes pools of its own with steps that hang for good, and reads how many of
    //! them ran and what the pool's queues hold: counts, not times. A deadline only ends a
    //! caller's wait, and the bound on a spin only turns a hang of the test into a failure.

    use super::*;

    /// Spins until `ready` holds, failing the test after 30 seconds instead of hanging.
    fn until(ready: impl Fn() -> bool) {
        let started = Instant::now();
        while !ready() {
            assert!(started.elapsed() < Duration::from_secs(30), "never ready");
            std::thread::yield_now();
        }
    }

    /// An instant `ms` milliseconds from now.
    fn after(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    /// Hangs the calling pool thread for good.
    fn hang() -> ! {
        loop {
            std::thread::park();
        }
    }

    /// The loopback address with port 0, a lookup's answer.
    fn loopback() -> Vec<SocketAddr> {
        vec![SocketAddr::from(([127, 0, 0, 1], 0))]
    }

    static NAME_STALLS: AtomicUsize = AtomicUsize::new(0);

    /// Every lookup of `stall.a.test` hangs.
    fn name_stalls(name: &str, _port: u16) -> std::io::Result<Vec<SocketAddr>> {
        if name == "stall.a.test" {
            NAME_STALLS.fetch_add(1, Ordering::SeqCst);
            hang();
        }
        Ok(loopback())
    }

    #[test]
    fn a_name_whose_lookups_stall_holds_one_thread_for_its_waiting_requests() {
        // R7-10: before #1265, each request of a stalled name took a thread of the lookup pool
        // until all were held. Now the requests that wait for a name at once share its lookup,
        // and its residue stays within its zone's share.
        let pool = make(8, "test-lookup-name");
        let first = Abandon::new();
        let holder = {
            let (pool, first) = (Arc::clone(&pool), first.clone());
            std::thread::spawn(move || {
                look_up(
                    &pool,
                    name_stalls,
                    "stall.a.test",
                    80,
                    after(30_000),
                    &first,
                )
            })
        };
        until(|| NAME_STALLS.load(Ordering::SeqCst) == 1);
        // Many callers, in both cases of the name, while the first still waits: each joins the
        // stalled lookup and gives up at its own deadline.
        let callers: Vec<_> = (0..32)
            .map(|i| {
                let pool = Arc::clone(&pool);
                let name = if i % 2 == 0 {
                    "stall.a.test"
                } else {
                    "STALL.A.test"
                };
                std::thread::spawn(move || {
                    look_up(&pool, name_stalls, name, 80, after(50), &Abandon::new())
                })
            })
            .collect();
        for caller in callers {
            assert_eq!(caller.join().unwrap(), Err(Error::TimedOut));
        }
        assert_eq!(NAME_STALLS.load(Ordering::SeqCst), 1);
        first.abandon();
        assert_eq!(holder.join().unwrap(), Err(Error::Interrupted));
        // Its callers gone, the lookup is residue, which a later caller does not join: it starts
        // a lookup of its own, the zone's second place.
        let second = Abandon::new();
        let late = {
            let (pool, second) = (Arc::clone(&pool), second.clone());
            std::thread::spawn(move || {
                look_up(
                    &pool,
                    name_stalls,
                    "stall.a.test",
                    80,
                    after(30_000),
                    &second,
                )
            })
        };
        until(|| NAME_STALLS.load(Ordering::SeqCst) == 2);
        second.abandon();
        assert_eq!(late.join().unwrap(), Err(Error::Interrupted));
        // The zone is at its share of 2: a third lookup waits in its line and never runs.
        let third = look_up(
            &pool,
            name_stalls,
            "stall.a.test",
            80,
            after(50),
            &Abandon::new(),
        );
        assert_eq!(third, Err(Error::TimedOut));
        assert_eq!(NAME_STALLS.load(Ordering::SeqCst), 2);
        assert_eq!(
            lock(&pool).zones.get("a.test").map(|zone| zone.admitted),
            Some(2)
        );
        // Other zones are looked up, with each caller's port.
        let other = look_up(
            &pool,
            name_stalls,
            "b.test",
            443,
            after(30_000),
            &Abandon::new(),
        );
        assert_eq!(other, Ok(Ok(vec![SocketAddr::from(([127, 0, 0, 1], 443))])));
    }

    /// A gate that steps wait at until the test opens it.
    struct Gate {
        open: Mutex<bool>,
        opened: Condvar,
    }

    impl Gate {
        const fn new() -> Gate {
            Gate {
                open: Mutex::new(false),
                opened: Condvar::new(),
            }
        }

        fn pass(&self) {
            let mut open = self.open.lock().unwrap();
            while !*open {
                open = self.opened.wait(open).unwrap();
            }
        }

        fn open(&self) {
            *self.open.lock().unwrap() = true;
            self.opened.notify_all();
        }
    }

    static JOIN_LOOKUPS: AtomicUsize = AtomicUsize::new(0);
    static JOIN_GATE: Gate = Gate::new();

    /// Every lookup of `slow.test` waits at [`JOIN_GATE`].
    fn slow_name(name: &str, _port: u16) -> std::io::Result<Vec<SocketAddr>> {
        if name == "slow.test" {
            JOIN_LOOKUPS.fetch_add(1, Ordering::SeqCst);
            JOIN_GATE.pass();
        }
        Ok(loopback())
    }

    #[test]
    fn a_lookup_whose_requests_all_gave_up_never_answers_a_later_request() {
        // Review M1 of #1574: before the fix, a later request joined the residue of a request
        // that a deadline or a cancellation cut, and could succeed where it alone would time
        // out. Now it starts a lookup of its own, so the earlier request changes nothing.
        let pool = make(16, "test-lookup-join");
        let cut = Abandon::new();
        let earlier = {
            let (pool, cut) = (Arc::clone(&pool), cut.clone());
            std::thread::spawn(move || {
                look_up(&pool, slow_name, "slow.test", 80, after(30_000), &cut)
            })
        };
        until(|| JOIN_LOOKUPS.load(Ordering::SeqCst) == 1);
        cut.abandon();
        assert_eq!(earlier.join().unwrap(), Err(Error::Interrupted));
        let later = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                look_up(
                    &pool,
                    slow_name,
                    "slow.test",
                    80,
                    after(30_000),
                    &Abandon::new(),
                )
            })
        };
        until(|| JOIN_LOOKUPS.load(Ordering::SeqCst) == 2);
        JOIN_GATE.open();
        assert_eq!(
            later.join().unwrap(),
            Ok(Ok(vec![SocketAddr::from(([127, 0, 0, 1], 80))]))
        );
    }

    static RESIDUE_LOOKUPS: AtomicUsize = AtomicUsize::new(0);
    static RESIDUE_GATE: Gate = Gate::new();

    /// Every lookup of `a.z.test` waits at [`RESIDUE_GATE`].
    fn slow_zone(name: &str, _port: u16) -> std::io::Result<Vec<SocketAddr>> {
        if name == "a.z.test" {
            RESIDUE_LOOKUPS.fetch_add(1, Ordering::SeqCst);
            RESIDUE_GATE.pass();
        }
        Ok(loopback())
    }

    #[test]
    fn a_lookup_left_by_a_cut_request_holds_its_zone_place_until_it_returns() {
        // Review M1 of #1574, the zone's part, kept on purpose since the bound needs it: on a
        // pool of 2, whose zone share is 1, the residue of a cut request holds its zone's place,
        // so a later request of the zone waits in its line while the pool's other thread serves
        // other zones; it runs once the residue returns. This is residue that `L_res` does not
        // count for a deadline's cut (`spec/26 §7` rule 5, an open channel).
        let pool = make(2, "test-lookup-residue");
        let cut = Abandon::new();
        let earlier = {
            let (pool, cut) = (Arc::clone(&pool), cut.clone());
            std::thread::spawn(move || {
                look_up(&pool, slow_zone, "a.z.test", 80, after(30_000), &cut)
            })
        };
        until(|| RESIDUE_LOOKUPS.load(Ordering::SeqCst) == 1);
        cut.abandon();
        assert_eq!(earlier.join().unwrap(), Err(Error::Interrupted));
        let later = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                look_up(
                    &pool,
                    slow_zone,
                    "b.z.test",
                    80,
                    after(30_000),
                    &Abandon::new(),
                )
            })
        };
        until(|| {
            lock(&pool)
                .zones
                .get("z.test")
                .is_some_and(|zone| zone.admitted == 1 && zone.waiting.jobs.len() == 1)
        });
        let other = look_up(
            &pool,
            slow_zone,
            "c.y.test",
            80,
            after(30_000),
            &Abandon::new(),
        );
        assert!(matches!(other, Ok(Ok(_))));
        assert!(
            lock(&pool)
                .zones
                .get("z.test")
                .is_some_and(|zone| zone.waiting.jobs.len() == 1)
        );
        RESIDUE_GATE.open();
        assert!(matches!(later.join().unwrap(), Ok(Ok(_))));
        until(|| lock(&pool).zones.is_empty());
    }

    static WAITING_LOOKUPS: AtomicUsize = AtomicUsize::new(0);
    static WAITING_GATE: Gate = Gate::new();

    /// Counts every lookup; that of `g.w.test` waits at [`WAITING_GATE`].
    fn counted(name: &str, _port: u16) -> std::io::Result<Vec<SocketAddr>> {
        WAITING_LOOKUPS.fetch_add(1, Ordering::SeqCst);
        if name == "g.w.test" {
            WAITING_GATE.pass();
        }
        Ok(loopback())
    }

    #[test]
    fn waiting_lookups_whose_requests_gave_up_never_run_and_leave_no_entry() {
        // Review M2 of #1574 (mutation M4): when the last caller of a lookup leaves, its job is
        // given up, so once its zone has room again it is dropped unrun, and no name or zone is
        // left behind.
        let pool = make(2, "test-lookup-waiting");
        let held = Abandon::new();
        let holder = {
            let (pool, held) = (Arc::clone(&pool), held.clone());
            std::thread::spawn(move || {
                look_up(&pool, counted, "g.w.test", 80, after(30_000), &held)
            })
        };
        until(|| WAITING_LOOKUPS.load(Ordering::SeqCst) == 1);
        let abandons: Vec<Abandon> = (0..5).map(|_| Abandon::new()).collect();
        let waiting: Vec<_> = abandons
            .iter()
            .enumerate()
            .map(|(i, abandon)| {
                let (pool, abandon) = (Arc::clone(&pool), abandon.clone());
                std::thread::spawn(move || {
                    look_up(
                        &pool,
                        counted,
                        &format!("n{i}.w.test"),
                        80,
                        after(30_000),
                        &abandon,
                    )
                })
            })
            .collect();
        until(|| {
            lock(&pool)
                .zones
                .get("w.test")
                .is_some_and(|zone| zone.waiting.jobs.len() == 5)
        });
        for abandon in &abandons {
            abandon.abandon();
        }
        for caller in waiting {
            assert_eq!(caller.join().unwrap(), Err(Error::Interrupted));
        }
        held.abandon();
        assert_eq!(holder.join().unwrap(), Err(Error::Interrupted));
        WAITING_GATE.open();
        until(|| {
            let queues = lock(&pool);
            queues.flights.is_empty() && queues.zones.is_empty() && queues.line.jobs.is_empty()
        });
        assert_eq!(WAITING_LOOKUPS.load(Ordering::SeqCst), 1);
    }

    static DROP_RUNS: AtomicUsize = AtomicUsize::new(0);
    static DROP_GATE: Gate = Gate::new();

    #[test]
    fn a_pool_thread_that_drops_a_given_up_job_gives_its_zone_place_back() {
        // Review M2 of #1574 (mutation M5): admitted jobs that gave up while every thread was
        // busy are dropped when a thread reaches them, and each gives its zone's place back, so
        // ordinary timeouts never lock a zone out of its pool.
        let pool = make(1, "test-connect-drop");
        let holder = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                hand_off(
                    &pool,
                    "x.test",
                    after(30_000),
                    &Abandon::new(),
                    None,
                    |_| {
                        DROP_RUNS.fetch_add(1, Ordering::SeqCst);
                        DROP_GATE.pass();
                    },
                )
            })
        };
        until(|| DROP_RUNS.load(Ordering::SeqCst) == 1);
        let zones: [&'static str; 3] = ["y1.test", "y2.test", "y3.test"];
        let abandons: Vec<Abandon> = zones.iter().map(|_| Abandon::new()).collect();
        let queued: Vec<_> = zones
            .iter()
            .copied()
            .zip(&abandons)
            .map(|(zone, abandon)| {
                let (pool, abandon) = (Arc::clone(&pool), abandon.clone());
                std::thread::spawn(move || {
                    hand_off(&pool, zone, after(30_000), &abandon, None, |_| {
                        DROP_RUNS.fetch_add(1, Ordering::SeqCst);
                    })
                })
            })
            .collect();
        until(|| lock(&pool).line.jobs.len() == 3);
        for abandon in &abandons {
            abandon.abandon();
        }
        for caller in queued {
            assert_eq!(caller.join().unwrap(), Err(Error::Interrupted));
        }
        DROP_GATE.open();
        assert_eq!(holder.join().unwrap(), Ok(()));
        for (i, zone) in zones.iter().enumerate() {
            let step = hand_off(&pool, zone, after(5_000), &Abandon::new(), None, move |_| i);
            assert_eq!(step, Ok(i));
        }
        until(|| lock(&pool).zones.is_empty());
        assert_eq!(DROP_RUNS.load(Ordering::SeqCst), 1);
    }

    static NEXT_RUNS: AtomicUsize = AtomicUsize::new(0);
    static NEXT_GATE: Gate = Gate::new();

    #[test]
    fn a_step_that_ends_gives_its_place_to_the_next_of_its_zone() {
        // Standard review L3 of #1574: a zone at its share of 1, whose running step ends
        // normally, lets its waiting step join the pool's line, while another zone runs beside.
        let pool = make(2, "test-connect-next");
        let first = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                hand_off(
                    &pool,
                    "z.test",
                    after(30_000),
                    &Abandon::new(),
                    None,
                    |_| {
                        NEXT_RUNS.fetch_add(1, Ordering::SeqCst);
                        NEXT_GATE.pass();
                        1
                    },
                )
            })
        };
        until(|| NEXT_RUNS.load(Ordering::SeqCst) == 1);
        let second = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                hand_off(&pool, "z.test", after(30_000), &Abandon::new(), None, |_| 2)
            })
        };
        until(|| {
            lock(&pool)
                .zones
                .get("z.test")
                .is_some_and(|zone| zone.admitted == 1 && zone.waiting.jobs.len() == 1)
        });
        assert_eq!(
            hand_off(&pool, "y.test", after(30_000), &Abandon::new(), None, |_| 3),
            Ok(3)
        );
        NEXT_GATE.open();
        assert_eq!(first.join().unwrap(), Ok(1));
        assert_eq!(second.join().unwrap(), Ok(2));
        until(|| lock(&pool).zones.is_empty());
    }

    #[test]
    fn a_step_that_panics_fails_its_request_at_once() {
        // Review L1 of #1574: before, the abandon's wake kept the channel open, and the request
        // failed only at its deadline, with `TimedOut`.
        let pool = make(1, "test-connect-panic");
        let boom: Result<(), Error> = hand_off(
            &pool,
            "boom.test",
            after(30_000),
            &Abandon::new(),
            None,
            |_| panic!("a connect step that panics"),
        );
        assert_eq!(boom, Err(Error::Unreachable));
        assert_eq!(
            hand_off(
                &pool,
                "boom.test",
                after(30_000),
                &Abandon::new(),
                None,
                |_| 1
            ),
            Ok(1)
        );
        until(|| lock(&pool).zones.is_empty());
        assert_eq!(pool.threads.load(Ordering::SeqCst), 1);
    }

    static ZONE_STALLS: AtomicUsize = AtomicUsize::new(0);

    /// Every lookup of a name in `stall.test`, a wildcard zone that never answers, hangs.
    fn zone_stalls(name: &str, _port: u16) -> std::io::Result<Vec<SocketAddr>> {
        if name.ends_with(".stall.test") {
            ZONE_STALLS.fetch_add(1, Ordering::SeqCst);
            hang();
        }
        Ok(loopback())
    }

    #[test]
    fn a_zone_whose_names_stall_holds_a_quarter_of_the_lookup_pool() {
        // R7-13: sixteen distinct names of one stalled zone, at depths one and two below it,
        // on a pool of 8, whose zone share is 2. Before #1265 they took all 8 threads.
        let pool = make(8, "test-lookup-zone");
        assert_eq!(pool.share, 2);
        let abandons: Vec<Abandon> = (0..16).map(|_| Abandon::new()).collect();
        let stalled: Vec<_> = abandons
            .iter()
            .enumerate()
            .map(|(i, abandon)| {
                let (pool, abandon) = (Arc::clone(&pool), abandon.clone());
                let name = if i % 2 == 0 {
                    format!("n{i}.stall.test")
                } else {
                    format!("x.n{i}.stall.test")
                };
                std::thread::spawn(move || {
                    look_up(&pool, zone_stalls, &name, 80, after(30_000), &abandon)
                })
            })
            .collect();
        until(|| lock(&pool).flights.len() == 16 && ZONE_STALLS.load(Ordering::SeqCst) == 2);
        {
            let queues = lock(&pool);
            let zone = queues.zones.get("stall.test").unwrap();
            assert_eq!(zone.admitted, 2);
            assert_eq!(zone.waiting.jobs.len(), 14);
        }
        // The other six threads serve every other zone, many lookups at once.
        let others: Vec<_> = (0..24)
            .map(|i| {
                let pool = Arc::clone(&pool);
                let name = format!("www.z{i}.test");
                std::thread::spawn(move || {
                    look_up(
                        &pool,
                        zone_stalls,
                        &name,
                        80,
                        after(30_000),
                        &Abandon::new(),
                    )
                })
            })
            .collect();
        for other in others {
            assert!(matches!(other.join().unwrap(), Ok(Ok(_))));
        }
        for abandon in &abandons {
            abandon.abandon();
        }
        for caller in stalled {
            assert_eq!(caller.join().unwrap(), Err(Error::Interrupted));
        }
        // The waiting lookups whose callers gave up never ran: still two of the zone's.
        let late = look_up(
            &pool,
            zone_stalls,
            "late.stall.test",
            80,
            after(50),
            &Abandon::new(),
        );
        assert_eq!(late, Err(Error::TimedOut));
        assert_eq!(ZONE_STALLS.load(Ordering::SeqCst), 2);
    }

    static CONNECT_STALLS: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn a_zone_whose_connections_stall_holds_a_quarter_of_the_connect_pool() {
        // A /24 that drops every packet, on a connect pool of 8, whose zone share is 2.
        let pool = make(8, "test-connect-zone");
        let abandons: Vec<Abandon> = (0..10).map(|_| Abandon::new()).collect();
        let stalled: Vec<_> = abandons
            .iter()
            .enumerate()
            .map(|(i, abandon)| {
                let (pool, abandon) = (Arc::clone(&pool), abandon.clone());
                let zone = zone_of(&format!("192.0.2.{}", i + 1));
                std::thread::spawn(move || {
                    hand_off(&pool, &zone, after(30_000), &abandon, None, |_| {
                        CONNECT_STALLS.fetch_add(1, Ordering::SeqCst);
                        hang()
                    })
                })
            })
            .collect();
        until(|| CONNECT_STALLS.load(Ordering::SeqCst) == 2);
        until(|| {
            lock(&pool)
                .zones
                .get("192.0.2.0/24")
                .is_some_and(|zone| zone.admitted == 2 && zone.waiting.jobs.len() == 8)
        });
        let others: Vec<_> = (0..24)
            .map(|i| {
                let pool = Arc::clone(&pool);
                let zone = zone_of(&format!("198.51.{i}.1"));
                std::thread::spawn(move || {
                    hand_off(
                        &pool,
                        &zone,
                        after(30_000),
                        &Abandon::new(),
                        None,
                        move |_| i,
                    )
                })
            })
            .collect();
        for (i, other) in others.into_iter().enumerate() {
            assert_eq!(other.join().unwrap(), Ok(i));
        }
        for abandon in &abandons {
            abandon.abandon();
        }
        for caller in stalled {
            assert!(matches!(caller.join().unwrap(), Err(Error::Interrupted)));
        }
        assert_eq!(CONNECT_STALLS.load(Ordering::SeqCst), 2);
    }

    /// A hold that counts itself while it lives, as a place among the sockets does.
    struct Counted(Arc<AtomicUsize>);

    impl Counted {
        fn hold(count: &Arc<AtomicUsize>) -> Option<Hold> {
            count.fetch_add(1, Ordering::SeqCst);
            Some(Box::new(Counted(Arc::clone(count))))
        }
    }

    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// What a step drops after its hold would be, if the order were wrong: it records whether
    /// the hold still lived when the step's answer was dropped.
    struct Answered(Arc<AtomicUsize>, Arc<AtomicBool>);

    impl Drop for Answered {
        fn drop(&mut self) {
            self.1
                .store(self.0.load(Ordering::SeqCst) == 1, Ordering::SeqCst);
        }
    }

    static HOLD_GATE: Gate = Gate::new();
    static HOLD_STARTED: AtomicBool = AtomicBool::new(false);
    static HOLD_QUEUED_RUNS: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn a_running_step_keeps_its_requests_hold_until_its_answer_is_dropped() {
        // #1824, `spec/27 §2` rule 3: a request abandoned while its connection step runs gets
        // `Interrupted` at once, but its place stays held until the step has returned and its
        // answer, the socket it made, is dropped. A request that gives up while its step is
        // still queued drops the hold itself, at once, and the step never runs.
        let pool = make(1, "test-connect-hold");
        let count = Arc::new(AtomicUsize::new(0));
        let answered_held = Arc::new(AtomicBool::new(false));
        let abandon = Abandon::new();
        let running = {
            let (pool, abandon, count) = (Arc::clone(&pool), abandon.clone(), Arc::clone(&count));
            let flag = Arc::clone(&answered_held);
            std::thread::spawn(move || {
                let hold = Counted::hold(&count);
                hand_off(
                    &pool,
                    "hold.test",
                    after(30_000),
                    &abandon,
                    hold,
                    move |_| {
                        HOLD_STARTED.store(true, Ordering::SeqCst);
                        HOLD_GATE.pass();
                        Answered(count, flag)
                    },
                )
                .map(drop)
            })
        };
        until(|| HOLD_STARTED.load(Ordering::SeqCst));
        // A second request queued behind the running step, which gives up by its deadline.
        let queued_count = Arc::new(AtomicUsize::new(0));
        let queued = hand_off(
            &pool,
            "hold.test",
            after(50),
            &Abandon::new(),
            Counted::hold(&queued_count),
            |_| {
                HOLD_QUEUED_RUNS.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert_eq!(queued, Err(Error::TimedOut));
        assert_eq!(
            queued_count.load(Ordering::SeqCst),
            0,
            "a queued step kept its hold"
        );
        abandon.abandon();
        assert_eq!(running.join().unwrap(), Err(Error::Interrupted));
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "the hold was dropped while the step still ran"
        );
        HOLD_GATE.open();
        until(|| count.load(Ordering::SeqCst) == 0);
        assert!(
            answered_held.load(Ordering::SeqCst),
            "the hold was dropped before the step's answer"
        );
        // The queued step never runs, and the pool serves on.
        assert_eq!(
            hand_off(
                &pool,
                "hold.test",
                after(5_000),
                &Abandon::new(),
                None,
                |_| 1
            ),
            Ok(1)
        );
        assert_eq!(HOLD_QUEUED_RUNS.load(Ordering::SeqCst), 0);
    }

    /// Panics for `boom.test`.
    fn booms(name: &str, _port: u16) -> std::io::Result<Vec<SocketAddr>> {
        assert_ne!(name, "boom.test", "a lookup that panics");
        Ok(loopback())
    }

    #[test]
    fn a_lookup_that_panics_fails_and_keeps_its_thread_and_its_place() {
        let pool = make(2, "test-lookup-panic");
        for _ in 0..4 {
            let boom = look_up(
                &pool,
                booms,
                "boom.test",
                80,
                after(30_000),
                &Abandon::new(),
            );
            assert_eq!(boom, Ok(Err(())));
        }
        let fine = look_up(
            &pool,
            booms,
            "fine.test",
            80,
            after(30_000),
            &Abandon::new(),
        );
        assert!(matches!(fine, Ok(Ok(_))));
        until(|| {
            let queues = lock(&pool);
            queues.zones.is_empty() && queues.flights.is_empty()
        });
        assert_eq!(pool.threads.load(Ordering::SeqCst), 2);
    }

    /// A job of `zone` that does nothing, given up when `dead`, a lookup of `name` when given.
    fn job(zone: &str, name: Option<&str>, dead: bool) -> Job {
        Job {
            run: Box::new(|_: &AtomicBool| {}),
            gave_up: Arc::new(AtomicBool::new(dead)),
            zone: Arc::from(zone),
            name: name.map(Arc::from),
            seq: 0,
        }
    }

    #[test]
    fn lines_of_given_up_jobs_stay_bounded_however_long_steps_hang() {
        // No pool thread takes anything here, as when every thread hangs: the jobs whose
        // requests gave up are pruned once a line doubles, so each line holds at most twice
        // the jobs it kept at its last prune, or 2 × PRUNE_FLOOR, for any number of requests.
        let share = 1;
        let mut queues = Queues::default();
        assert_eq!(queues.submit(share, job("stall.test", None, false)), 1);
        let bound = 2 * PRUNE_FLOOR + 1;
        for n in 0..100_000 {
            // A stalled zone's line: requests that gave up.
            let name = format!("n{n}.stall.test");
            let flight = Flight {
                callers: HashMap::new(),
                gave_up: Arc::new(AtomicBool::new(true)),
            };
            let mut dead = job("stall.test", Some(&name), true);
            dead.gave_up = Arc::clone(&flight.gave_up);
            queues.flights.insert(Arc::from(name.as_str()), flight);
            assert_eq!(queues.submit(share, dead), 0);
            // The pool's line: given-up requests of zones of their own.
            queues.submit(share, job(&format!("z{n}.test"), None, true));
            let waiting = queues.zones.get("stall.test").unwrap().waiting.jobs.len();
            assert!(waiting <= bound, "{waiting}");
            assert!(
                queues.line.jobs.len() <= bound,
                "{}",
                queues.line.jobs.len()
            );
            assert!(queues.zones.len() <= bound + 1, "{}", queues.zones.len());
            assert!(queues.flights.len() <= bound, "{}", queues.flights.len());
        }
        // The live job is never pruned.
        assert!(
            queues
                .line
                .jobs
                .iter()
                .any(|job| !job.gave_up.load(Ordering::SeqCst))
        );
    }

    /// Submits jobs whose requests gave up, each of a zone of its own, until the pool's line is
    /// pruned, as when every thread hangs and requests keep coming.
    fn force_prune(queues: &mut Queues, share: usize) {
        for i in 0.. {
            let before = queues.line.jobs.len();
            queues.submit(share, job(&format!("f{i}.filler"), None, true));
            if queues.line.jobs.len() < before {
                return;
            }
            assert!(i < 1_000, "never pruned");
        }
    }

    /// The zones of the jobs of the pool's line whose zones end with `suffix`, in order.
    fn line_zones(queues: &Queues, suffix: &str) -> Vec<String> {
        queues
            .line
            .jobs
            .iter()
            .filter(|job| job.zone.ends_with(suffix))
            .map(|job| job.zone.to_string())
            .collect()
    }

    #[test]
    fn a_prune_refills_the_zones_it_freed_first_come_first_served() {
        // Review L-B of #1574 (mutation M9, the sort removed): twelve zones at their share of 1,
        // whose waiting jobs came in the reverse order of the zones. Their admitted jobs give up
        // and a prune drops them: the waiting jobs join the pool's line in the order they came,
        // never in the order of the zone map.
        let share = 1;
        let mut queues = Queues::default();
        let zones: Vec<String> = (0..12).map(|i| format!("w{i}.test")).collect();
        let admitted: Vec<Arc<AtomicBool>> = zones
            .iter()
            .map(|zone| {
                let first = job(zone, None, false);
                let flag = Arc::clone(&first.gave_up);
                assert_eq!(queues.submit(share, first), 1);
                flag
            })
            .collect();
        for zone in zones.iter().rev() {
            assert_eq!(queues.submit(share, job(zone, None, false)), 0);
        }
        for flag in &admitted {
            flag.store(true, Ordering::SeqCst);
        }
        force_prune(&mut queues, share);
        let expected: Vec<String> = zones.iter().rev().cloned().collect();
        assert_eq!(line_zones(&queues, ".test"), expected);
    }

    #[test]
    fn a_refill_orders_zones_by_their_first_job_still_wanted() {
        // Standard review N1 of #1574: the order is that of each zone's first waiting job whose
        // request still waits. Zone `a` has the earliest waiting job, given up, then a later live
        // one; zone `b` a live one between them: `b` is refilled first.
        let share = 1;
        let mut queues = Queues::default();
        let held: Vec<Arc<AtomicBool>> = ["a.zone", "b.zone"]
            .iter()
            .map(|zone| {
                let first = job(zone, None, false);
                let flag = Arc::clone(&first.gave_up);
                assert_eq!(queues.submit(share, first), 1);
                flag
            })
            .collect();
        assert_eq!(queues.submit(share, job("a.zone", None, true)), 0);
        assert_eq!(queues.submit(share, job("b.zone", None, false)), 0);
        assert_eq!(queues.submit(share, job("a.zone", None, false)), 0);
        for flag in &held {
            flag.store(true, Ordering::SeqCst);
        }
        force_prune(&mut queues, share);
        assert_eq!(line_zones(&queues, ".zone"), ["b.zone", "a.zone"]);
        assert!(
            queues
                .line
                .jobs
                .iter()
                .all(|job| !job.gave_up.load(Ordering::SeqCst))
        );
    }

    static FRESH_RUNS: AtomicUsize = AtomicUsize::new(0);
    static FRESH_GATE: Gate = Gate::new();

    /// The lookup of `g.f.test` waits at [`FRESH_GATE`]; those of `n.f.test` are counted.
    fn fresh(name: &str, _port: u16) -> std::io::Result<Vec<SocketAddr>> {
        if name == "g.f.test" {
            FRESH_GATE.pass();
        }
        if name == "n.f.test" {
            FRESH_RUNS.fetch_add(1, Ordering::SeqCst);
        }
        Ok(loopback())
    }

    #[test]
    fn a_given_up_lookup_never_removes_the_fresh_lookup_of_its_name() {
        // Review L-B of #1574 (mutation M11, `drop_flight` without its identity check): on a
        // pool of 2, share 1, `g.f.test` holds its zone; a waiting lookup of `n.f.test` gives up,
        // and a later request of the name starts a fresh lookup that replaces it among the
        // names. When the zone frees, the old job is dropped without removing the fresh one,
        // which runs and answers.
        let pool = make(2, "test-lookup-fresh");
        let holder = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                look_up(&pool, fresh, "g.f.test", 80, after(30_000), &Abandon::new())
            })
        };
        until(|| {
            let queues = lock(&pool);
            queues
                .zones
                .get("f.test")
                .is_some_and(|zone| zone.admitted == 1)
                && queues.line.jobs.is_empty()
        });
        let cut = Abandon::new();
        let first = {
            let (pool, cut) = (Arc::clone(&pool), cut.clone());
            std::thread::spawn(move || look_up(&pool, fresh, "n.f.test", 80, after(30_000), &cut))
        };
        until(|| {
            lock(&pool)
                .zones
                .get("f.test")
                .is_some_and(|zone| zone.waiting.jobs.len() == 1)
        });
        cut.abandon();
        assert_eq!(first.join().unwrap(), Err(Error::Interrupted));
        let second = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                look_up(&pool, fresh, "n.f.test", 80, after(30_000), &Abandon::new())
            })
        };
        until(|| {
            lock(&pool)
                .zones
                .get("f.test")
                .is_some_and(|zone| zone.waiting.jobs.len() == 2)
        });
        FRESH_GATE.open();
        assert!(matches!(holder.join().unwrap(), Ok(Ok(_))));
        assert!(matches!(second.join().unwrap(), Ok(Ok(_))));
        assert_eq!(FRESH_RUNS.load(Ordering::SeqCst), 1);
        until(|| {
            let queues = lock(&pool);
            queues.flights.is_empty() && queues.zones.is_empty() && queues.line.jobs.is_empty()
        });
    }

    #[test]
    fn zones_are_the_last_two_labels_or_an_address_prefix() {
        assert_eq!(zone_of("a.b.Example.NET."), "example.net");
        assert_eq!(zone_of("example.net"), "example.net");
        assert_eq!(zone_of("x.y.z.example.net"), "example.net");
        assert_eq!(zone_of("localhost"), "localhost");
        assert_eq!(zone_of("192.0.2.77"), "192.0.2.0/24");
        assert_eq!(zone_of("2001:db8:1:2::5"), "2001:db8:1::/48");
        assert_eq!(zone_of("[2001:db8:1:2::5]"), "2001:db8:1::/48");
        assert_eq!(zone_of("::ffff:192.0.2.1"), "192.0.2.0/24");
        assert_eq!(zone_of("[::ffff:192.0.2.1]"), "192.0.2.0/24");
        assert_eq!(zone_share(2), 1);
        assert_eq!(zone_share(7), 1);
        assert_eq!(zone_share(16), 4);
        assert_eq!(zone_share(64), 16);
    }
}
