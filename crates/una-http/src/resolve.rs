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
//! - **One lookup per name** (#1265, ADR 0117 S7 review R7-10). A request for a name whose
//!   lookup is queued or running joins it while another request still waits for it, names
//!   compared without case, and waits for its answer until its own deadline: the requests of one
//!   name that wait at the same time share one thread of the lookup pool, whatever their rate. A
//!   lookup whose requests all gave up takes no new one (review M1 of #1574): it never runs if it
//!   has not started, and else runs to its end unseen, while a later request of the name starts
//!   a lookup of its own on a free thread. So residue never answers a later request, and delays
//!   one only when the pool is full, as before #1265. The bound per zone of R7-13 is left to
//!   issue #1717.
//! - **The hand-off.** Each pool job costs one push and one pop under the pool's mutex, a wake of
//!   one pool thread, and a reply on the request's own channel; a lookup also costs, on average,
//!   O(1) operations on a hash map keyed by its name (linear in the name's length): about two
//!   thread wakes per step, against the milliseconds the step itself takes.
//! - **Hangs.** When every thread of a pool holds a step that hangs, new steps queue, and each of
//!   their requests still gives up exactly at its own deadline. A job still queued when its
//!   request gives up never runs. Without [`start`], a request fails with `Unreachable`: every
//!   engine starts the pools.
//! - **Space.** Jobs whose requests gave up stay in a pool's line until a thread reaches them or
//!   the line has doubled since it was last pruned, when they are dropped: the line holds at most
//!   twice the jobs it kept at its last prune, or `2 × 16`, so a pool holds O(peak waiting
//!   requests + threads) jobs and names, however long its steps hang, at an amortized O(1) per
//!   job.
//! - **Residue.** A lookup or a connection whose request gave up keeps its pool thread until it
//!   returns, beside later code: the residue that `L_res` labels (`spec/26 §7` rule 5), at most
//!   the pools' sizes. A connection job looks at its request only between two addresses, so an
//!   attempt in progress runs until it connects or the request's deadline passes. A step that
//!   panics ends as a failure at once: its thread is kept, and its requests get `Unreachable`.

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

/// The key of a name's shared lookup: the name in lower case, as the resolver compares names.
fn name_key(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// What a lookup answers its callers: the addresses found, with port 0, or a failure.
type Found = Result<Vec<SocketAddr>, ()>;

/// A job on a pool: what it runs, given whether its request gave up, that flag, and, for a
/// lookup, the name whose shared lookup it is.
struct Job {
    run: Box<dyn FnOnce(&AtomicBool) + Send>,
    gave_up: Arc<AtomicBool>,
    name: Option<Arc<str>>,
}

/// The fewest jobs a line holds before it is pruned of those whose requests gave up.
const PRUNE_FLOOR: usize = 16;

/// A pool's line of jobs, pruned of those whose requests gave up when it has doubled since it
/// was last pruned: amortized O(1) per job, and at most twice the jobs it kept at its last
/// prune, or `2 × PRUNE_FLOOR`.
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

    /// Takes the jobs whose requests gave up out of the line, keeping the order of the others.
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

/// A name's shared lookup: the callers waiting for its answer, by id, and the flag of its job,
/// set when no caller waits any more.
struct Flight {
    callers: HashMap<u64, mpsc::Sender<Answer<Found>>>,
    gave_up: Arc<AtomicBool>,
}

/// What a pool's mutex keeps: the pool's line of jobs, the names whose lookups are queued or
/// running, and the next caller's id.
#[derive(Default)]
struct Queues {
    line: Line,
    flights: HashMap<Arc<str>, Flight>,
    next_caller: u64,
}

impl Queues {
    /// Adds `job` to the end of the pool's line, pruning the line of the jobs whose requests
    /// gave up once it has doubled since it was last pruned. Amortized O(1).
    fn submit(&mut self, job: Job) {
        self.line.jobs.push_back(job);
        if self.line.full() {
            for dead in self.line.prune() {
                self.drop_flight(&dead);
            }
        }
    }

    /// Removes the shared lookup whose job `job` is, if it still is the name's: a later caller
    /// may have started a fresh lookup of the name, which stays.
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
}

/// A pool: its queues, the threads it was made with, and how many of them started.
struct Pool {
    queues: Mutex<Queues>,
    ready: Condvar,
    wanted: usize,
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

/// Makes a pool with `size` threads named `name`, all started now.
fn make(size: usize, name: &str) -> Arc<Pool> {
    let pool = Arc::new(Pool {
        queues: Mutex::new(Queues::default()),
        ready: Condvar::new(),
        wanted: size.max(1),
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
/// dropping the others unrun with their shared lookups, and runs it; a job that panics leaves
/// the thread serving.
fn serve(pool: &Pool) {
    loop {
        let job = {
            let mut queues = lock(pool);
            loop {
                match queues.line.jobs.pop_front() {
                    Some(job) if job.gave_up.load(Ordering::SeqCst) => queues.drop_flight(&job),
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
        let Job { run, gave_up, .. } = job;
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| run(&gave_up)));
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

/// Hands `step` to `pool` and waits for its answer until `deadline`, `TimedOut`, or until
/// `abandon` wakes the wait, `Interrupted`, with no polling. On either, the job is marked given
/// up: queued, it never runs; running, it ends alone and its answer is dropped. A step that
/// panics fails the request at once with `Unreachable`. Amortized O(1) besides the wait.
fn hand_off<T: Send + 'static>(
    pool: &Pool,
    deadline: Instant,
    abandon: &Abandon,
    step: impl FnOnce(&AtomicBool) -> T + Send + 'static,
) -> Result<T, Error> {
    let left = time_left(deadline, abandon)?;
    let (reply, answers) = mpsc::channel::<Answer<T>>();
    let waker = reply.clone();
    abandon.on_abandon(Box::new(move || {
        let _ = waker.send(Answer::Abandoned);
    }));
    let gave_up = Arc::new(AtomicBool::new(false));
    let job = Job {
        run: Box::new(move |gave_up: &AtomicBool| {
            let mut failing = Failing { reply: Some(reply) };
            let answer = step(gave_up);
            if let Some(reply) = failing.reply.take() {
                let _ = reply.send(Answer::Done(answer));
            }
        }),
        gave_up: Arc::clone(&gave_up),
        name: None,
    };
    lock(pool).submit(job);
    pool.ready.notify_one();
    let answer = wait(&answers, left, abandon);
    if answer.is_err() {
        gave_up.store(true, Ordering::SeqCst);
    }
    answer
}

/// Ends a name's shared lookup when its job ends, normally or by a panic: removes it from the
/// pool's names, if it still is the name's, and sends its answer to its callers, a failure when
/// the lookup panicked.
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
/// lookup of its own (review M1 of #1574). The name is looked up with port 0, which the system's
/// resolver ignores, and each caller's port is set on the answer. O(1) on average besides the
/// wait and the name's length.
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
    let (caller, gave_up, submitted) = {
        let mut queues = lock(pool);
        let caller = queues.next_caller;
        queues.next_caller = caller.wrapping_add(1);
        // Only a lookup that a caller still waits for takes a new one (#1265, review M1): the
        // residue of requests that all gave up never changes a later request's outcome by
        // answering it; a later caller starts a lookup of its own.
        if let Some(flight) = queues
            .flights
            .get_mut(&key)
            .filter(|flight| !flight.callers.is_empty())
        {
            flight.callers.insert(caller, reply);
            (caller, Arc::clone(&flight.gave_up), false)
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
                name: Some(Arc::clone(&key)),
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
            queues.submit(job);
            (caller, gave_up, true)
        }
    };
    if submitted {
        pool.ready.notify_one();
    }
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
/// name looked up on the lookup pool, unless it is an address literal, one lookup per name for
/// the requests that wait for it together (#1265), then the connection made on the connect
/// pool. The request waits for each until the deadline, `TimedOut`, or until `abandon` wakes it,
/// `Interrupted`, with no polling; a stop is seen once a wait ends.
pub(crate) fn reach(
    host: &str,
    port: u16,
    deadline: Instant,
    stopped: &dyn Fn() -> bool,
    abandon: &Abandon,
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
    let stream = hand_off(&pools.connects, deadline, abandon, move |gave_up| {
        connect_any(addrs, deadline, gave_up)
    })??;
    if stopped() || abandon.abandoned() {
        return Err(Error::Interrupted);
    }
    Ok(stream)
}

#[cfg(test)]
mod tests {
    //! The per-name bound on the lookup pool and the bounded lines of both pools (#1265, ADR
    //! 0117 S7 review R7-10). Each test makes pools of its own with steps that hang for good or
    //! wait at a gate, and reads how many of them ran and what the pool's queues hold: counts,
    //! not times. A deadline only ends a caller's wait, and the bound on a spin only turns a
    //! hang of the test into a failure.

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

    /// The loopback address with `port`, what a caller of that port gets.
    fn loopback_at(port: u16) -> Found {
        Ok(vec![SocketAddr::from(([127, 0, 0, 1], port))])
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
        // until all were held. Now the requests that wait for a name at once share its lookup.
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
        assert!(lock(&pool).line.jobs.is_empty());
        first.abandon();
        assert_eq!(holder.join().unwrap(), Err(Error::Interrupted));
        // Its callers gone, the lookup is residue, which a later caller does not join: it starts
        // a lookup of its own on a free thread.
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
        // Other names are looked up on the threads left, with each caller's port.
        for port in [443, 8080] {
            let other = look_up(
                &pool,
                name_stalls,
                "b.test",
                port,
                after(30_000),
                &Abandon::new(),
            );
            assert_eq!(other, Ok(loopback_at(port)));
        }
        assert_eq!(NAME_STALLS.load(Ordering::SeqCst), 2);
        assert_eq!(pool.threads.load(Ordering::SeqCst), 8);
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
        assert_eq!(later.join().unwrap(), Ok(loopback_at(80)));
    }

    static RESIDUE_LOOKUPS: AtomicUsize = AtomicUsize::new(0);
    static RESIDUE_GATE: Gate = Gate::new();

    /// The first lookup of `a.z.test` waits at [`RESIDUE_GATE`]; every other answers at once.
    fn first_waits(name: &str, _port: u16) -> std::io::Result<Vec<SocketAddr>> {
        if name == "a.z.test" && RESIDUE_LOOKUPS.fetch_add(1, Ordering::SeqCst) == 0 {
            RESIDUE_GATE.pass();
        }
        Ok(loopback())
    }

    #[test]
    fn a_lookup_left_by_a_cut_request_delays_no_later_request_while_its_pool_has_a_thread() {
        // The decision on #1577 (2026-11-06): the per-name bound without a share per zone. On a
        // pool of 2, the residue of a cut request holds one thread; a later request of the same
        // name, or of a name of its zone, runs on the other thread at once and is answered while
        // the residue still runs: a cut reaches a later request only through a full pool, as
        // before #1265 (`spec/24 §4` rule 2). With a zone share of 1 both waited (#1717).
        let pool = make(2, "test-lookup-residue");
        let cut = Abandon::new();
        let earlier = {
            let (pool, cut) = (Arc::clone(&pool), cut.clone());
            std::thread::spawn(move || {
                look_up(&pool, first_waits, "a.z.test", 80, after(30_000), &cut)
            })
        };
        until(|| RESIDUE_LOOKUPS.load(Ordering::SeqCst) == 1);
        cut.abandon();
        assert_eq!(earlier.join().unwrap(), Err(Error::Interrupted));
        for name in ["a.z.test", "b.z.test", "A.Z.TEST"] {
            let later = look_up(&pool, first_waits, name, 80, after(30_000), &Abandon::new());
            assert_eq!(later, Ok(loopback_at(80)), "{name}");
        }
        // The residue still runs: three lookups of `a.z.test` started, one still at the gate.
        assert_eq!(RESIDUE_LOOKUPS.load(Ordering::SeqCst), 3);
        RESIDUE_GATE.open();
        until(|| lock(&pool).flights.is_empty());
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
        // given up, so once a thread is free it is dropped unrun, and no name is left behind.
        let pool = make(1, "test-lookup-waiting");
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
        until(|| lock(&pool).line.jobs.len() == 5);
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
            queues.flights.is_empty() && queues.line.jobs.is_empty()
        });
        assert_eq!(WAITING_LOOKUPS.load(Ordering::SeqCst), 1);
    }

    static DROP_RUNS: AtomicUsize = AtomicUsize::new(0);
    static DROP_GATE: Gate = Gate::new();

    #[test]
    fn queued_steps_whose_requests_gave_up_never_run_and_the_pool_serves_on() {
        // Connect steps queued while every thread was busy, whose requests gave up, are dropped
        // when a thread reaches them; the pool serves later steps.
        let pool = make(1, "test-connect-drop");
        let holder = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                hand_off(&pool, after(30_000), &Abandon::new(), |_| {
                    DROP_RUNS.fetch_add(1, Ordering::SeqCst);
                    DROP_GATE.pass();
                })
            })
        };
        until(|| DROP_RUNS.load(Ordering::SeqCst) == 1);
        let abandons: Vec<Abandon> = (0..3).map(|_| Abandon::new()).collect();
        let queued: Vec<_> = abandons
            .iter()
            .map(|abandon| {
                let (pool, abandon) = (Arc::clone(&pool), abandon.clone());
                std::thread::spawn(move || {
                    hand_off(&pool, after(30_000), &abandon, |_| {
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
        for i in 0..3 {
            let step = hand_off(&pool, after(5_000), &Abandon::new(), move |_| i);
            assert_eq!(step, Ok(i));
        }
        until(|| lock(&pool).line.jobs.is_empty());
        assert_eq!(DROP_RUNS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_step_that_panics_fails_its_request_at_once() {
        // Review L1 of #1574: before, the abandon's wake kept the channel open, and the request
        // failed only at its deadline, with `TimedOut`.
        let pool = make(1, "test-connect-panic");
        let boom: Result<(), Error> = hand_off(&pool, after(30_000), &Abandon::new(), |_| {
            panic!("a connect step that panics")
        });
        assert_eq!(boom, Err(Error::Unreachable));
        assert_eq!(
            hand_off(&pool, after(30_000), &Abandon::new(), |_| 1),
            Ok(1)
        );
        assert_eq!(pool.threads.load(Ordering::SeqCst), 1);
    }

    /// Panics for `boom.test`.
    fn booms(name: &str, _port: u16) -> std::io::Result<Vec<SocketAddr>> {
        assert_ne!(name, "boom.test", "a lookup that panics");
        Ok(loopback())
    }

    #[test]
    fn a_lookup_that_panics_fails_and_keeps_its_thread() {
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
        assert_eq!(fine, Ok(loopback_at(80)));
        until(|| lock(&pool).flights.is_empty());
        assert_eq!(pool.threads.load(Ordering::SeqCst), 2);
    }

    /// A job that does nothing, given up when `dead`, a lookup of `name` when given.
    fn job(name: Option<&str>, dead: bool) -> Job {
        Job {
            run: Box::new(|_: &AtomicBool| {}),
            gave_up: Arc::new(AtomicBool::new(dead)),
            name: name.map(Arc::from),
        }
    }

    #[test]
    fn lines_of_given_up_jobs_stay_bounded_however_long_steps_hang() {
        // No pool thread takes anything here, as when every thread hangs: the jobs whose
        // requests gave up are pruned once the line doubles, so it holds at most twice the jobs
        // it kept at its last prune, or 2 × PRUNE_FLOOR, for any number of requests, and the
        // names of their lookups go with them. Before #1265 the line kept every job.
        let mut queues = Queues::default();
        queues.submit(job(None, false));
        let bound = 2 * PRUNE_FLOOR + 1;
        for n in 0..200_000 {
            // A name of a stalled zone whose request gave up, and a given-up connect step.
            let name = format!("n{n}.stall.test");
            let flight = Flight {
                callers: HashMap::new(),
                gave_up: Arc::new(AtomicBool::new(true)),
            };
            let mut dead = job(Some(&name), true);
            dead.gave_up = Arc::clone(&flight.gave_up);
            queues.flights.insert(Arc::from(name.as_str()), flight);
            queues.submit(dead);
            queues.submit(job(None, true));
            assert!(
                queues.line.jobs.len() <= bound,
                "{}",
                queues.line.jobs.len()
            );
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

    #[test]
    fn a_prune_keeps_the_jobs_still_wanted_first_come_first_served() {
        // Live jobs keep their order across prunes: the line is first come, first served.
        let mut queues = Queues::default();
        let mut live = Vec::new();
        for i in 0..100 {
            let wanted = job(Some(&format!("live{i}.test")), false);
            live.push(Arc::clone(&wanted.gave_up));
            queues.submit(wanted);
            for _ in 0..3 {
                queues.submit(job(None, true));
            }
        }
        let kept: Vec<_> = queues
            .line
            .jobs
            .iter()
            .filter(|job| !job.gave_up.load(Ordering::SeqCst))
            .map(|job| Arc::clone(&job.gave_up))
            .collect();
        assert_eq!(kept.len(), live.len());
        assert!(kept.iter().zip(&live).all(|(a, b)| Arc::ptr_eq(a, b)));
        assert!(queues.line.jobs.len() <= 2 * live.len() + 1);
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
        // pool of 1, `g.f.test` holds the thread; a queued lookup of `n.f.test` gives up, and a
        // later request of the name starts a fresh lookup that replaces it among the names.
        // When the thread frees, the old job is dropped without removing the fresh one, which
        // runs and answers.
        let pool = make(1, "test-lookup-fresh");
        let holder = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                look_up(&pool, fresh, "g.f.test", 80, after(30_000), &Abandon::new())
            })
        };
        until(|| {
            let queues = lock(&pool);
            queues.flights.contains_key("g.f.test") && queues.line.jobs.is_empty()
        });
        let cut = Abandon::new();
        let first = {
            let (pool, cut) = (Arc::clone(&pool), cut.clone());
            std::thread::spawn(move || look_up(&pool, fresh, "n.f.test", 80, after(30_000), &cut))
        };
        until(|| lock(&pool).line.jobs.len() == 1);
        cut.abandon();
        assert_eq!(first.join().unwrap(), Err(Error::Interrupted));
        let second = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                look_up(&pool, fresh, "n.f.test", 80, after(10_000), &Abandon::new())
            })
        };
        until(|| lock(&pool).line.jobs.len() == 2);
        FRESH_GATE.open();
        assert_eq!(holder.join().unwrap(), Ok(loopback_at(80)));
        assert_eq!(second.join().unwrap(), Ok(loopback_at(80)));
        assert_eq!(FRESH_RUNS.load(Ordering::SeqCst), 1);
        until(|| {
            let queues = lock(&pool);
            queues.flights.is_empty() && queues.line.jobs.is_empty()
        });
    }
}
