//! Reaching a server on bounded pools of threads: name resolution and the TCP connection
//! (`spec/24 §4` rule 2, ADR 0116 amendment P8 and P9, #1148), until the reactor of ADR 0117 step
//! 6 replaces them.
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
//! - **The hand-off.** Each pool job costs one push and one pop under the pool's mutex, a wake of
//!   one pool thread, and a reply on the request's own channel: O(1), about two thread wakes per
//!   step, against the milliseconds the step itself takes.
//! - **Hangs.** When every thread of a pool holds a step that hangs, new steps queue, and each of
//!   their requests still gives up exactly at its own deadline. A job still queued when its
//!   request gives up never runs. Without [`start`], a request fails with `Unreachable`: every
//!   engine starts the pools.
//! - **Residue.** A lookup or a connection whose request gave up keeps its pool thread until it
//!   returns, beside later code: the residue that `L_res` labels (`spec/26 §7` rule 5), at most
//!   the pools' sizes. A connection job looks at its request only between two addresses, so an
//!   attempt in progress runs until it connects or the request's deadline passes.

use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};
use std::time::{Duration, Instant};

use crate::client::{Error, Interrupt};
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

/// A job on a pool: what it runs, given whether its request gave up, and that flag.
struct Job {
    run: Box<dyn FnOnce(&AtomicBool) + Send>,
    gave_up: Arc<AtomicBool>,
}

/// A pool: its queue, the threads it was made with, and how many of them started.
struct Pool {
    jobs: Mutex<VecDeque<Job>>,
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

/// Locks a queue, recovering it when a thread panicked while holding the lock: each critical
/// section leaves it consistent.
fn lock(pool: &Pool) -> MutexGuard<'_, VecDeque<Job>> {
    pool.jobs.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Makes a pool with `size` threads named `name`, all started now.
fn make(size: usize, name: &str) -> Arc<Pool> {
    let pool = Arc::new(Pool {
        jobs: Mutex::new(VecDeque::new()),
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

/// Starts the process's pools for a run of `io_threads` I/O helpers, all their threads at once,
/// unless they exist: what an engine calls before `main` in a run whose grants include `Http`.
/// `Err` when a thread could not start, which the engine reports before `main`. The first call
/// sets the sizes for the process.
pub fn start(io_threads: usize) -> Result<(), ()> {
    let pools = POOLS.get_or_init(|| make_pools(io_threads));
    let full = |pool: &Pool| pool.threads.load(Ordering::SeqCst) == pool.wanted;
    if full(&pools.lookups) && full(&pools.connects) {
        Ok(())
    } else {
        Err(())
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

/// What a pool thread does: takes the next job whose request still waits, dropping the others
/// unrun, and runs it.
fn serve(pool: &Pool) {
    loop {
        let job = {
            let mut jobs = lock(pool);
            loop {
                match jobs.pop_front() {
                    Some(job) if job.gave_up.load(Ordering::SeqCst) => {}
                    Some(job) => break job,
                    None => {
                        jobs = pool
                            .ready
                            .wait(jobs)
                            .unwrap_or_else(PoisonError::into_inner);
                    }
                }
            }
        };
        (job.run)(&*job.gave_up);
    }
}

/// What a waiting request gets: the job's answer, or the wake of its abandon.
enum Answer<T> {
    Done(T),
    Abandoned,
}

/// Hands `step` to `pool` and waits for its answer until `deadline`, `TimedOut`, or until
/// `abandon` wakes the wait, `Interrupted`, with no polling. On either, the job is marked given
/// up: queued, it never runs; running, it ends alone and its answer is dropped. O(1) besides the
/// wait.
fn hand_off<T: Send + 'static>(
    pool: &Pool,
    deadline: Instant,
    abandon: &Abandon,
    step: impl FnOnce(&AtomicBool) -> T + Send + 'static,
) -> Result<T, Error> {
    if abandon.abandoned() {
        return Err(Error::Interrupted);
    }
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(Error::TimedOut);
    }
    let (reply, answers) = mpsc::channel::<Answer<T>>();
    let waker = reply.clone();
    abandon.on_abandon(Box::new(move || {
        let _ = waker.send(Answer::Abandoned);
    }));
    let gave_up = Arc::new(AtomicBool::new(false));
    lock(pool).push_back(Job {
        run: Box::new(move |gave_up: &AtomicBool| {
            let answer = step(gave_up);
            let _ = reply.send(Answer::Done(answer));
        }),
        gave_up: Arc::clone(&gave_up),
    });
    pool.ready.notify_one();
    let answer = answers.recv_timeout(left);
    abandon.clear_hook();
    match answer {
        Ok(Answer::Done(done)) => Ok(done),
        Ok(Answer::Abandoned) => {
            gave_up.store(true, Ordering::SeqCst);
            Err(Error::Interrupted)
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            gave_up.store(true, Ordering::SeqCst);
            Err(Error::TimedOut)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            gave_up.store(true, Ordering::SeqCst);
            Err(Error::Unreachable)
        }
    }
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
/// name looked up on the lookup pool, unless it is an address literal, then the connection made
/// on the connect pool. The request waits for each until the deadline, `TimedOut`, or until
/// `abandon` wakes it, `Interrupted`, with no polling; a stop is seen once a wait ends.
pub(crate) fn reach(
    host: &str,
    port: u16,
    deadline: Instant,
    interrupt: &dyn Interrupt,
    abandon: &Abandon,
) -> Result<TcpStream, Error> {
    if interrupt.stopped() {
        return Err(Error::Interrupted);
    }
    let Some(pools) = pools() else {
        return Err(Error::Unreachable);
    };
    let addrs = match host.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => {
            let name = host.to_owned();
            let found = hand_off(&pools.lookups, deadline, abandon, move |_| {
                lookup()(&name, port)
            })?;
            if interrupt.stopped() {
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
    if interrupt.stopped() || abandon.abandoned() {
        return Err(Error::Interrupted);
    }
    Ok(stream)
}
