//! Name resolution on a bounded pool of threads (`spec/24 §4` rule 2, ADR 0116 amendment P8 and
//! P9, #1148), until the reactor of ADR 0117 step 6 replaces it.
//!
//! The host's resolver has no timeout and nothing in the process can cut a lookup. So a request
//! never runs one on its own thread, the I/O helper's: it submits the lookup to this pool and
//! waits for the answer only until its deadline, an absolute instant, or until its task's
//! cancellation abandons it. On either, it stops waiting and fails with `TimedOut` or
//! `Interrupted`; the lookup runs to its end on its resolver thread, and its answer is dropped.
//!
//! - **Bounded.** The pool has [`pool_size`] threads, `max(2, --io-threads / 4)`, all started
//!   at once ([`start`]), which an engine does before `main` in a run whose grants include
//!   `Http`, with its other pools (P8), so whether a resolver thread exists is never state left
//!   by earlier code. The process never runs more lookups at once than that, however many
//!   requests wait. When every thread holds a lookup that hangs, new lookups queue, and each of
//!   their requests still gives up exactly at its own deadline.
//! - **Residue.** A lookup whose request gave up keeps its resolver thread until it returns,
//!   beside later code: the residue that `L_res` labels (`spec/26 §7` rule 5), at most the pool's
//!   size at once. A lookup still queued when its request gives up never runs.
//! - **Costs.** A submission and a take are O(1) under one mutex; a waiting request parks on its
//!   own channel, woken by the answer, by its abandon's hook, or by its deadline, with no polling.

use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, mpsc};
use std::time::Instant;

use crate::client::{Error, Interrupt};
use crate::limit::Abandon;

/// A lookup function: the addresses of a name and a port, as the system's resolver gives them.
pub type Lookup = fn(&str, u16) -> std::io::Result<Vec<SocketAddr>>;

/// The fewest resolver threads a pool has.
pub const MIN_RESOLVERS: usize = 2;

/// The size of the resolver pool for a run of `io_threads` I/O helpers: `max(2, io_threads /
/// 4)`. A lookup normally takes milliseconds against an exchange's tens to hundreds, so a
/// quarter of the helpers serve their lookups without a queue, and the bound keeps hung lookups
/// to a quarter of the threads that `--io-threads` already allows.
pub fn pool_size(io_threads: usize) -> usize {
    (io_threads / 4).max(MIN_RESOLVERS)
}

/// What a waiting request gets: the lookup's answer, or the wake of its abandon.
enum Answer {
    Found(std::io::Result<Vec<SocketAddr>>),
    Abandoned,
}

/// A lookup submitted to the pool.
struct Job {
    name: String,
    port: u16,
    reply: mpsc::Sender<Answer>,
    /// Set when the request stopped waiting: a job still queued then never runs.
    gave_up: Arc<AtomicBool>,
}

/// The pool: its queue, the threads it was made with, and how many of them started.
struct Pool {
    jobs: Mutex<VecDeque<Job>>,
    ready: Condvar,
    wanted: usize,
    threads: AtomicUsize,
}

/// The process's pool, made once, by [`start`] or by the first lookup.
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
            .name("una-http-resolve".to_owned())
            .stack_size(256 * 1024)
            .spawn(move || serve(&serving));
        if spawned.is_ok() {
            pool.threads.fetch_add(1, Ordering::SeqCst);
        }
    }
    pool
}

/// Starts the process's resolver pool with `size` threads, all at once, unless it exists:
/// what an engine calls before `main` in a run whose grants include `Http`. `Err` when a thread
/// could not start, which the engine reports before `main`. The first call sets the size for
/// the process.
pub fn start(size: usize) -> Result<(), ()> {
    let pool = POOL.get_or_init(|| make(size));
    if pool.threads.load(Ordering::SeqCst) == pool.wanted {
        Ok(())
    } else {
        Err(())
    }
}

/// The resolver threads the process has started: 0 before the pool is made.
pub fn threads() -> usize {
    POOL.get()
        .map_or(0, |pool| pool.threads.load(Ordering::SeqCst))
}

/// What a resolver thread does: takes the next lookup that a request still waits for, runs it
/// to its end, and answers.
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
        let found = lookup()(&job.name, job.port);
        let _ = job.reply.send(Answer::Found(found));
    }
}

/// The system's resolver.
fn system(name: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
    (name, port).to_socket_addrs().map(Iterator::collect)
}

/// The lookup a test put in place of the system's, in debug builds only.
#[cfg(debug_assertions)]
static TEST_LOOKUP: OnceLock<Lookup> = OnceLock::new();

/// Debug builds only: makes `lookup` the resolver of the process, for tests whose lookups hang.
/// It takes effect only when nothing set one before; release builds have no such hook.
#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn use_test_lookup(lookup: Lookup) {
    let _ = TEST_LOOKUP.set(lookup);
}

/// The resolver: the system's, or a test's in debug builds.
fn lookup() -> Lookup {
    #[cfg(debug_assertions)]
    if let Some(test) = TEST_LOOKUP.get() {
        return *test;
    }
    system
}

/// The addresses of `host` and `port` for a request whose deadline is `deadline` (#1148). An
/// address literal needs no lookup. A name is looked up on the pool, and the request waits for
/// its answer until the deadline, `TimedOut`, or until `abandon` wakes it, `Interrupted`, with
/// no polling; a stop is seen once the wait ends.
pub(crate) fn resolve(
    host: &str,
    port: u16,
    deadline: Instant,
    interrupt: &dyn Interrupt,
    abandon: &Abandon,
) -> Result<Vec<SocketAddr>, Error> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    if interrupt.stopped() {
        return Err(Error::Interrupted);
    }
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(Error::TimedOut);
    }
    let pool = POOL.get_or_init(|| make(MIN_RESOLVERS));
    let (reply, answers) = mpsc::channel();
    let waker = reply.clone();
    abandon.on_abandon(Box::new(move || {
        let _ = waker.send(Answer::Abandoned);
    }));
    let gave_up = Arc::new(AtomicBool::new(false));
    lock(pool).push_back(Job {
        name: host.to_owned(),
        port,
        reply,
        gave_up: Arc::clone(&gave_up),
    });
    pool.ready.notify_one();
    let answer = answers.recv_timeout(left);
    gave_up.store(true, Ordering::SeqCst);
    abandon.clear_hook();
    let addrs = match answer {
        Ok(Answer::Found(Ok(addrs))) if !addrs.is_empty() => addrs,
        Ok(Answer::Found(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err(Error::Unreachable);
        }
        Ok(Answer::Abandoned) => return Err(Error::Interrupted),
        Err(mpsc::RecvTimeoutError::Timeout) => return Err(Error::TimedOut),
    };
    if interrupt.stopped() {
        return Err(Error::Interrupted);
    }
    Ok(addrs)
}
