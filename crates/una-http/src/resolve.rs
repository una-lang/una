//! Reaching a server on a bounded pool of threads: name resolution and the TCP connection
//! (`spec/24 §4` rule 2, ADR 0116 amendment P8 and P9, #1148), until the reactor of ADR 0117 step
//! 6 replaces it.
//!
//! The host's resolver has no timeout, nothing in the process can cut a lookup, and `connect`
//! cannot be woken without code of `una-sys` of its own. So a request never runs either on its
//! own thread, the I/O helper's: it submits them to this pool and waits for the connection only
//! until its deadline, an absolute instant, or until its task's cancellation abandons it. On
//! either, it stops waiting and fails with `TimedOut` or `Interrupted`; the lookup and the
//! connection attempt in progress run to their end on the pool's thread, which then closes the
//! connection, if one came, without using it.
//!
//! - **Bounded.** The pool has [`pool_size`] threads, `max(2, --io-threads / 4)`, all started
//!   at once ([`start`]), which an engine does before `main` in a run whose grants include
//!   `Http`, with its other pools (P8), so whether a pool thread exists is never state left by
//!   earlier code. The process never resolves or connects more than that many times at once,
//!   however many requests wait. When every thread holds a lookup or a connection that hangs,
//!   new ones queue, and each of their requests still gives up exactly at its own deadline.
//!   Without [`start`], a request fails with `Unreachable`: every engine starts the pool.
//! - **Residue.** A lookup or a connection attempt whose request gave up keeps its pool thread
//!   until it returns, beside later code: the residue that `L_res` labels (`spec/26 §7` rule 5),
//!   at most the pool's size at once. A job still queued when its request gives up never runs.
//! - **Costs.** A submission and a take are O(1) under one mutex; a waiting request parks on its
//!   own channel, woken by the answer, by its abandon's hook, or by its deadline, with no polling.

use std::collections::VecDeque;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};
use std::time::{Duration, Instant};

use crate::client::{Error, Interrupt};
use crate::limit::Abandon;

/// A lookup function: the addresses of a name and a port, as the system's resolver gives them.
pub type Lookup = fn(&str, u16) -> std::io::Result<Vec<SocketAddr>>;

/// A connect function: a TCP connection to an address, within a time-out.
pub type Connect = fn(&SocketAddr, Duration) -> std::io::Result<TcpStream>;

/// The fewest pool threads.
pub const MIN_RESOLVERS: usize = 2;

/// The size of the pool for a run of `io_threads` I/O helpers: `max(2, io_threads / 4)`. A
/// lookup and a connection normally take milliseconds against an exchange's tens to hundreds,
/// so a quarter of the helpers serve them without a queue, and the bound keeps the threads that
/// hung lookups and connections can hold to a quarter of those `--io-threads` already allows.
pub fn pool_size(io_threads: usize) -> usize {
    (io_threads / 4).max(MIN_RESOLVERS)
}

/// What a waiting request gets: the connection, or the wake of its abandon.
enum Answer {
    Reached(Result<TcpStream, Error>),
    Abandoned,
}

/// A connection to make: the name and port, the request's deadline, and where to answer.
struct Job {
    name: String,
    port: u16,
    deadline: Instant,
    reply: mpsc::Sender<Answer>,
    /// Set when the request stopped waiting: a job still queued then never runs, and one in
    /// progress closes what it reaches.
    gave_up: Arc<AtomicBool>,
}

/// The pool: its queue, the threads it was made with, and how many of them started.
struct Pool {
    jobs: Mutex<VecDeque<Job>>,
    ready: Condvar,
    wanted: usize,
    threads: AtomicUsize,
}

/// The process's pool, made once, by [`start`].
static POOL: OnceLock<Arc<Pool>> = OnceLock::new();

/// Locks the queue, recovering it when a thread panicked while holding the lock: each critical
/// section leaves it consistent.
fn lock(pool: &Pool) -> MutexGuard<'_, VecDeque<Job>> {
    pool.jobs.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Makes the pool with `size` threads, all started now.
fn make(size: usize) -> Arc<Pool> {
    let pool = Arc::new(Pool {
        jobs: Mutex::new(VecDeque::new()),
        ready: Condvar::new(),
        wanted: size.max(1),
        threads: AtomicUsize::new(0),
    });
    for _ in 0..size.max(1) {
        let serving = Arc::clone(&pool);
        let spawned = std::thread::Builder::new()
            .name("una-http-reach".to_owned())
            .stack_size(256 * 1024)
            .spawn(move || serve(&serving));
        if spawned.is_ok() {
            pool.threads.fetch_add(1, Ordering::SeqCst);
        }
    }
    pool
}

/// Starts the process's pool with `size` threads, all at once, unless it exists: what an engine
/// calls before `main` in a run whose grants include `Http`. `Err` when a thread could not start,
/// which the engine reports before `main`. The first call sets the size for the process.
pub fn start(size: usize) -> Result<(), ()> {
    let pool = POOL.get_or_init(|| make(size));
    if pool.threads.load(Ordering::SeqCst) == pool.wanted {
        Ok(())
    } else {
        Err(())
    }
}

/// The pool threads the process has started: 0 before [`start`].
pub fn threads() -> usize {
    POOL.get()
        .map_or(0, |pool| pool.threads.load(Ordering::SeqCst))
}

/// What a pool thread does: takes the next job whose request still waits, dropping the others
/// unrun, reaches its server, and answers; a connection whose request gave up meanwhile is
/// closed at once.
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
        let reached = reach_now(&job);
        if job.gave_up.load(Ordering::SeqCst) {
            // Its request stopped waiting: the connection, if one came, closes here.
            drop(reached);
            continue;
        }
        let _ = job.reply.send(Answer::Reached(reached));
    }
}

/// Resolves the job's name and connects to the first address that accepts, each attempt bounded
/// by the time left before its deadline, on the pool's thread.
fn reach_now(job: &Job) -> Result<TcpStream, Error> {
    let left = |deadline: Instant| {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            Err(Error::TimedOut)
        } else {
            Ok(left)
        }
    };
    let addrs = match job.name.parse::<std::net::IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, job.port)],
        Err(_) => lookup()(&job.name, job.port).map_err(|_| Error::Unreachable)?,
    };
    for addr in addrs {
        if job.gave_up.load(Ordering::SeqCst) {
            return Err(Error::Interrupted);
        }
        match connect()(&addr, left(job.deadline)?) {
            Ok(connected) => return Ok(connected),
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                left(job.deadline)?;
            }
            Err(_) => {}
        }
    }
    Err(Error::Unreachable)
}

/// The system's resolver.
fn system_lookup(name: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
    (name, port).to_socket_addrs().map(Iterator::collect)
}

/// The system's `connect`, within a time-out.
fn system_connect(addr: &SocketAddr, timeout: Duration) -> std::io::Result<TcpStream> {
    TcpStream::connect_timeout(addr, timeout)
}

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

/// The pool, with the `test-hooks` feature: one of [`MIN_RESOLVERS`] is started for the tests
/// of the client, which call it directly, when none was.
#[cfg(feature = "test-hooks")]
fn pool() -> Option<&'static Arc<Pool>> {
    Some(POOL.get_or_init(|| make(MIN_RESOLVERS)))
}

/// The pool, or `None` before [`start`]: a request made before the engine started the pool
/// fails with `Unreachable`, never with a pool started lazily, which would be state left by
/// earlier code (P8).
#[cfg(not(feature = "test-hooks"))]
fn pool() -> Option<&'static Arc<Pool>> {
    POOL.get()
}

/// A TCP connection to `host` and `port` for a request whose deadline is `deadline` (#1148),
/// made on the pool. The request waits for it until the deadline, `TimedOut`, or until `abandon`
/// wakes it, `Interrupted`, with no polling; a stop is seen once the wait ends. An abandon raised
/// before the job is queued queues nothing.
pub(crate) fn reach(
    host: &str,
    port: u16,
    deadline: Instant,
    interrupt: &dyn Interrupt,
    abandon: &Abandon,
) -> Result<TcpStream, Error> {
    if interrupt.stopped() || abandon.abandoned() {
        return Err(Error::Interrupted);
    }
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(Error::TimedOut);
    }
    let Some(pool) = pool() else {
        return Err(Error::Unreachable);
    };
    let (reply, answers) = mpsc::channel();
    let waker = reply.clone();
    abandon.on_abandon(Box::new(move || {
        let _ = waker.send(Answer::Abandoned);
    }));
    let gave_up = Arc::new(AtomicBool::new(false));
    lock(pool).push_back(Job {
        name: host.to_owned(),
        port,
        deadline,
        reply,
        gave_up: Arc::clone(&gave_up),
    });
    pool.ready.notify_one();
    let answer = answers.recv_timeout(left);
    abandon.clear_hook();
    let stream = match answer {
        Ok(Answer::Reached(reached)) => reached,
        Ok(Answer::Abandoned) => {
            gave_up.store(true, Ordering::SeqCst);
            return Err(Error::Interrupted);
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            gave_up.store(true, Ordering::SeqCst);
            return Err(Error::TimedOut);
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            gave_up.store(true, Ordering::SeqCst);
            return Err(Error::Unreachable);
        }
    }?;
    if interrupt.stopped() || abandon.abandoned() {
        return Err(Error::Interrupted);
    }
    Ok(stream)
}
