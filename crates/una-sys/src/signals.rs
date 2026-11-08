//! The handlers of SIGINT and SIGTERM (ADR 0116 rules 14, 17 and 18).
//!
//! A handler does only async-signal-safe work: it records the host stop request with atomic
//! adds on the attention words ([`crate::attention::request_stop`]), writes one byte to a pipe
//! opened non-blocking and close-on-exec, which wakes any thread waiting on its read end, and,
//! at the third request, restores the default actions and ends the process with the signal, so
//! `kill` still ends a process stuck in an operation that is not interruptible. It allocates
//! nothing, takes no lock and keeps `errno` as it found it.
//!
//! The stop is sticky, so the pipe is never drained: once a request has come, its read end
//! stays readable, and every thread that waits on it afterwards wakes at once.
//!
//! The `unsafe` code here is the calls of the operating system through the
//! `libc` crate, which is Rust declarations of the system interface only, with no C source
//! built (ADR 0116 Appendix A, amendment of 2026-10-19).
#![allow(unsafe_code)]

use std::sync::OnceLock;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

/// The signals that request a host stop.
const SIGNALS: [libc::c_int; 2] = [libc::SIGINT, libc::SIGTERM];

/// SIGTERM, the signal that the end of a second grace period after the hard stop raises
/// (rule 17), for callers that do not depend on `libc`.
pub const SIGTERM: libc::c_int = libc::SIGTERM;

/// The lowest descriptor the wake pipe's ends move to, above those that other code opens and
/// closes, so that no reused descriptor ever receives the handler's byte. Under a soft limit
/// of open descriptors at or below it, the ends move as high as the limit allows instead
/// (#1757): `F_DUPFD_CLOEXEC` refuses a start at or above the limit.
const HIGH_FD: libc::c_int = 512;

/// The fewest open descriptors a run that installs the handlers needs: the three standard
/// streams, the wake pipe's two ends and the two the pipe call opens before they move, with
/// room for the runtime's pollers and the program's own. A soft limit below it is a usage
/// error, never an internal one (#1757).
pub const MIN_DESCRIPTORS: u64 = 16;

/// The ends of the wake pipe, or -1 before [`install`].
static WAKE_READ: AtomicI32 = AtomicI32::new(-1);
static WAKE_WRITE: AtomicI32 = AtomicI32::new(-1);

/// The outcome of the first [`install`], which every later call returns. Never read by a
/// handler, so its lock is harmless.
static INSTALLED: OnceLock<Result<(), InstallError>> = OnceLock::new();

/// Why the handlers could not be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallError {
    /// The message, without a prefix.
    pub message: String,
    /// Whether the cause is the run's configuration, a soft limit of open descriptors too
    /// small for the runtime, which the caller reports as a usage error (exit code 2), not an
    /// internal one (#1757).
    pub usage: bool,
}

impl InstallError {
    fn internal(message: &str) -> Self {
        InstallError {
            message: message.to_owned(),
            usage: false,
        }
    }

    fn limit(soft: u64) -> Self {
        InstallError {
            message: format!(
                "the limit of open files (`ulimit -n`) is {soft}, too small for the runtime's stop handling; raise it to at least {MIN_DESCRIPTORS}"
            ),
            usage: true,
        }
    }
}

/// Installs the handlers of SIGINT and SIGTERM and opens the wake pipe. The runtime calls it
/// only for a run that can reach a `for ever` or runs a driver (rule 18), at start-up, before it
/// starts any thread that could start a process. Every call returns the outcome of the first,
/// and a call that comes while the first runs waits for it. A failure leaves nothing behind:
/// no descriptor open and the previous actions of both signals in place.
pub fn install() -> Result<(), InstallError> {
    INSTALLED.get_or_init(install_once).clone()
}

fn install_once() -> Result<(), InstallError> {
    let [read, write] = open_pipe()?;
    WAKE_READ.store(read, Ordering::Release);
    WAKE_WRITE.store(write, Ordering::Release);
    // SAFETY: zero is a valid `sigaction` (no handler, no flags, an empty mask), and every
    // slot of the array is one; `sigaction` fills them as it installs.
    let mut previous: [libc::sigaction; 2] = unsafe { std::mem::zeroed() };
    for (index, (signal, old)) in SIGNALS.iter().zip(previous.iter_mut()).enumerate() {
        let action = handler_action(on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
        // SAFETY: `action` is a fully initialized `sigaction` whose handler is `on_signal`, an
        // `extern "C"` function that does only async-signal-safe work; `old` is a valid
        // `sigaction` that receives the previous action.
        if unsafe { libc::sigaction(*signal, &raw const action, old) } != 0 {
            // Put back the actions installed so far, then close the pipe.
            for (signal, old) in SIGNALS.iter().zip(previous.iter()).take(index) {
                // SAFETY: `old` is the action `sigaction` returned for `signal` above.
                unsafe { libc::sigaction(*signal, old, std::ptr::null_mut()) };
            }
            WAKE_READ.store(-1, Ordering::Release);
            WAKE_WRITE.store(-1, Ordering::Release);
            close_both([read, write]);
            return Err(InstallError::internal("cannot install the signal handlers"));
        }
    }
    Ok(())
}

/// A `sigaction` with `handler`, an empty mask and `SA_RESTART`.
fn handler_action(handler: libc::sighandler_t) -> libc::sigaction {
    // SAFETY: zero is a valid `sigaction` (no handler, no flags, an empty mask).
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = handler;
    action.sa_flags = libc::SA_RESTART;
    // SAFETY: `action.sa_mask` is a valid `sigset_t` inside `action`, owned here.
    unsafe { libc::sigemptyset(&raw mut action.sa_mask) };
    action
}

/// The crate's one binding of `fcntl`, for commands whose argument is an integer.
///
/// # Safety
///
/// `fd` is a descriptor the caller owns, and `cmd` reads no memory through `arg`.
pub(crate) unsafe fn fcntl(fd: libc::c_int, cmd: libc::c_int, arg: libc::c_int) -> libc::c_int {
    // SAFETY: the caller's contract above.
    unsafe { libc::fcntl(fd, cmd, arg) }
}

/// Where the wake pipe's ends move under the soft limit `soft` of open descriptors: from
/// [`HIGH_FD`], or, under a lower limit, from the highest start that leaves room for both
/// ends, `soft - 2`. `F_DUPFD_CLOEXEC` takes the lowest free descriptor from there, so a start
/// below the pipe's own ends still gives a free one. O(1).
fn pipe_start(soft: u64) -> libc::c_int {
    let room = soft.saturating_sub(2);
    libc::c_int::try_from(room).map_or(HIGH_FD, |room| room.min(HIGH_FD))
}

/// The two ends of a new pipe, non-blocking and close-on-exec, at descriptors of
/// [`pipe_start`] or above, else at the lowest free ones; on failure, nothing stays open. A
/// soft limit below [`MIN_DESCRIPTORS`], or one the pipe's descriptors do not fit in, is a
/// usage error (#1757).
fn open_pipe() -> Result<[libc::c_int; 2], InstallError> {
    let soft = crate::io::descriptor_limits()
        .map_err(|_| InstallError::internal("cannot read the limit of open files"))?
        .rlim_cur;
    if soft < MIN_DESCRIPTORS {
        return Err(InstallError::limit(soft));
    }
    let start = pipe_start(soft);
    let mut fds: [libc::c_int; 2] = [-1, -1];
    // SAFETY: `fds` is a valid array of two `c_int`s that the call writes the two ends into.
    #[cfg(target_os = "linux")]
    let opened = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
    // macOS has no `pipe2`: the ends are close-on-exec from their move below, a few
    // instructions later, before any thread of the runtime that could start a process exists.
    // SAFETY: as above.
    #[cfg(target_os = "macos")]
    let opened = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if opened != 0 {
        let full = std::io::Error::last_os_error().raw_os_error() == Some(libc::EMFILE);
        return Err(if full {
            InstallError::limit(soft)
        } else {
            InstallError::internal("cannot open the wake pipe")
        });
    }
    let mut moved: [libc::c_int; 2] = [-1, -1];
    let mut full = false;
    for (fd, high) in fds.iter().zip(moved.iter_mut()) {
        // SAFETY: `fd` is an open descriptor that the pipe call returned; `F_DUPFD_CLOEXEC`
        // duplicates it close-on-exec at the lowest free descriptor from `start`, below the
        // soft limit by `pipe_start`, or, when none is free there, from 0, which is never lower
        // than the lowest free one; `F_GETFL`/`F_SETFL` read and set its status flags; no
        // memory is touched.
        *high = unsafe {
            let mut duplicate = fcntl(*fd, libc::F_DUPFD_CLOEXEC, start);
            if duplicate < 0 {
                duplicate = fcntl(*fd, libc::F_DUPFD_CLOEXEC, 0);
            }
            if duplicate < 0 {
                full |= std::io::Error::last_os_error().raw_os_error() == Some(libc::EMFILE);
            }
            let status = fcntl(*fd, libc::F_GETFL, 0);
            if duplicate >= 0
                && status >= 0
                && fcntl(duplicate, libc::F_SETFL, status | libc::O_NONBLOCK) == 0
            {
                duplicate
            } else {
                if duplicate >= 0 {
                    libc::close(duplicate);
                }
                -1
            }
        };
    }
    close_both(fds);
    if moved.contains(&-1) {
        close_both(moved);
        return Err(if full {
            InstallError::limit(soft)
        } else {
            InstallError::internal("cannot set up the wake pipe")
        });
    }
    Ok(moved)
}

/// Closes the descriptors of `fds` that are open (not -1).
fn close_both(fds: [libc::c_int; 2]) {
    for fd in fds.into_iter().filter(|fd| *fd >= 0) {
        // SAFETY: `fd` is a descriptor this module opened and owns; closing it touches no
        // memory.
        unsafe { libc::close(fd) };
    }
}

/// The read end of the wake pipe, which stays readable from the first host stop request on,
/// for a thread that waits with `poll` on it beside its own operation; `None` before
/// [`install`]. Nothing may close it or read from it.
pub fn wake_fd() -> Option<libc::c_int> {
    let fd = WAKE_READ.load(Ordering::Acquire);
    (fd >= 0).then_some(fd)
}

/// [`wake_fd`] borrowed for the life of the process, for the reactor's poller, once [`install`]
/// has succeeded: only a failed install closes the pipe, and a successful one is never undone.
pub fn wake_pipe() -> Option<std::os::fd::BorrowedFd<'static>> {
    let fd = wake_fd().filter(|_| matches!(INSTALLED.get(), Some(Ok(()))))?;
    // SAFETY: the install succeeded, so `fd` is open, and nothing closes it afterwards.
    Some(unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) })
}

/// What [`wait`] saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// The descriptor is ready: readable, or writable when asked for.
    Ready,
    /// The wake pipe is readable: a host stop request has come in the process.
    Woken,
    /// Neither, within the time given.
    TimedOut,
    /// `poll` failed with an error other than `EINTR`, such as `ENOMEM`: the caller treats it as
    /// its operation's error, or sleeps the time out, and never waits again at once.
    Failed,
}

/// Waits with `poll` until `fd` is ready (readable, or writable when `write`), until the wake
/// pipe is readable when `wake` is set and the handlers are installed ([`wake_fd`]), or until
/// `timeout` passes, whichever comes first (ADR 0116 rules 3 and 12: an interruptible wait).
/// With no descriptor and no pipe it sleeps for `timeout`. A wait that a signal interrupts
/// (`EINTR`) goes on with the time left; any other error of `poll` is `Failed`. Each call of
/// `poll` waits at most `i32::MAX` milliseconds, rounded up, and the wait goes on until the
/// whole `timeout` has passed. A readable pipe wins over a ready descriptor, so a stop is never
/// missed.
pub fn wait(fd: Option<libc::c_int>, write: bool, wake: bool, timeout: Duration) -> Readiness {
    let pipe = if wake { wake_fd() } else { None };
    let mut fds = [
        libc::pollfd {
            fd: fd.unwrap_or(-1),
            events: if write { libc::POLLOUT } else { libc::POLLIN },
            revents: 0,
        },
        libc::pollfd {
            fd: pipe.unwrap_or(-1),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let end = Instant::now().checked_add(timeout);
    loop {
        let left = end.map_or(timeout, |end| end.saturating_duration_since(Instant::now()));
        let wanted = left.as_nanos().div_ceil(1_000_000);
        let capped = wanted > i32::MAX as u128;
        let millis = libc::c_int::try_from(wanted).unwrap_or(libc::c_int::MAX);
        // SAFETY: `fds` is a valid array of two initialized `pollfd`s that `poll` reads and
        // whose `revents` it writes; a negative `fd` makes `poll` skip that entry.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, millis) };
        if ready > 0 {
            let [own, pipe] = &fds;
            if pipe.revents != 0 {
                return Readiness::Woken;
            }
            if own.revents != 0 {
                return Readiness::Ready;
            }
        } else if ready == 0 {
            if !capped {
                return Readiness::TimedOut;
            }
        } else if errno() != libc::EINTR {
            return Readiness::Failed;
        }
        if left.is_zero() {
            return Readiness::TimedOut;
        }
        for entry in &mut fds {
            entry.revents = 0;
        }
    }
}

/// A host stop request from normal code, not from a signal (rule 14: `--deadline`, a driver
/// shutdown, `stop after`): records it and wakes every thread waiting on the pipe, as a
/// signal does. Returns how many requests have come, this one included.
pub fn request_host_stop() -> u32 {
    let requests = crate::attention::request_stop();
    wake();
    requests
}

/// Makes the wake pipe readable, for good, without counting a request of the process: a
/// request local to one run, such as its `--deadline`, wakes the waits on the pipe at once, and
/// each wait then reads its own run's requests. Nothing before [`install`]. Async-signal-safe.
pub fn wake() {
    let fd = WAKE_WRITE.load(Ordering::Acquire);
    if fd >= 0 {
        let byte = [1u8];
        // SAFETY: `byte` is a valid one-byte array; the pipe is non-blocking, so a full pipe
        // makes `write` fail at once, which is harmless: a byte already waits there.
        unsafe { libc::write(fd, byte.as_ptr().cast(), 1) };
    }
}

/// Ends the process with `signal`'s default action (rule 17): what the third request does,
/// and the end of a second grace period after the hard stop, which passes SIGTERM. It
/// restores the default actions of SIGINT and SIGTERM, unblocks `signal` in the calling
/// thread and raises it; inside a handler the process ends when the handler returns. When the
/// signal cannot end the process, as for the init process of a container, it exits with status
/// `128 + signal`, as a shell reports a death by that signal. Async-signal-safe.
pub fn restore_and_raise(signal: libc::c_int) -> ! {
    let default = handler_action(libc::SIG_DFL);
    for each in SIGNALS {
        // SAFETY: `default` is a fully initialized `sigaction` with `SIG_DFL`; the old action
        // is not asked for (null). `sigaction` is async-signal-safe.
        unsafe { libc::sigaction(each, &raw const default, std::ptr::null_mut()) };
    }
    // SAFETY: zero is a valid, empty `sigset_t`, which `sigemptyset` and `sigaddset` set up in
    // place; `pthread_sigmask` changes only the calling thread's mask. All three are
    // async-signal-safe, and so is `raise`, which sends `signal` to the calling thread.
    unsafe {
        let mut mask: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&raw mut mask);
        libc::sigaddset(&raw mut mask, signal);
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &raw const mask, std::ptr::null_mut());
        libc::raise(signal);
    }
    // SAFETY: `_exit` ends the process at once, without running any code of the process; it
    // is async-signal-safe.
    unsafe { libc::_exit(128 + signal) }
}

/// The handler: [`handle`], which is all safe code, with `errno` kept.
extern "C" fn on_signal(signal: libc::c_int) {
    let saved = errno();
    handle(signal);
    set_errno(saved);
}

/// What a handler does for `signal`, without the `errno` bookkeeping: records the request and
/// wakes the waiters ([`request_host_stop`]), and ends the process at the third request.
/// Async-signal-safe and allocation free, which a test checks by calling it under a counting
/// allocator.
#[doc(hidden)]
pub fn handle(signal: libc::c_int) {
    if request_host_stop() >= 3 {
        restore_and_raise(signal);
    }
}

#[cfg(target_os = "macos")]
fn errno_location() -> *mut libc::c_int {
    // SAFETY: `__error` returns the address of the calling thread's `errno`; it is
    // async-signal-safe.
    unsafe { libc::__error() }
}

#[cfg(target_os = "linux")]
fn errno_location() -> *mut libc::c_int {
    // SAFETY: `__errno_location` returns the address of the calling thread's `errno`; it is
    // async-signal-safe.
    unsafe { libc::__errno_location() }
}

fn errno() -> libc::c_int {
    // SAFETY: `errno_location` returns a valid, aligned pointer to this thread's `errno`.
    unsafe { *errno_location() }
}

fn set_errno(value: libc::c_int) {
    // SAFETY: `errno_location` returns a valid, aligned pointer to this thread's `errno`,
    // which only this thread writes.
    unsafe { *errno_location() = value };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipe_starts_at_the_high_descriptor_or_as_high_as_the_limit_allows() {
        // #1757: under a soft limit at or below `HIGH_FD`, a start of `HIGH_FD` is refused
        // with `EINVAL`; the start leaves room for both ends below the limit.
        assert_eq!(pipe_start(1_048_576), HIGH_FD);
        assert_eq!(pipe_start(u64::MAX), HIGH_FD);
        assert_eq!(pipe_start(515), HIGH_FD);
        assert_eq!(pipe_start(514), HIGH_FD);
        assert_eq!(pipe_start(513), 511);
        assert_eq!(pipe_start(512), 510);
        assert_eq!(pipe_start(256), 254);
        assert_eq!(pipe_start(64), 62);
        assert_eq!(pipe_start(MIN_DESCRIPTORS), 14);
    }

    #[test]
    fn a_limit_below_the_minimum_is_a_usage_error_that_names_it() {
        let error = InstallError::limit(8);
        assert!(error.usage);
        assert!(error.message.contains("is 8"), "{}", error.message);
        assert!(error.message.contains("at least 16"), "{}", error.message);
    }
}
