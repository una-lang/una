//! The handlers of SIGINT and SIGTERM (ADR 0116 rules 14, 17 and 18).
//!
//! A handler does only async-signal-safe work: it records the host stop request with atomic
//! adds on the attention words ([`crate::attention::request_stop`]), writes one byte to a pipe
//! opened non-blocking and close-on-exec, which wakes any thread waiting on its read end, and,
//! at the third request, restores the default actions and raises the signal again, so `kill`
//! still ends a process stuck in an operation that is not interruptible. It allocates nothing,
//! takes no lock and keeps `errno` as it found it.
//!
//! The only `unsafe` code of the crate is here: the calls of the operating system through the
//! `libc` crate, which is Rust declarations of the system interface only, with no C source
//! built (ADR 0116 Appendix A, amendment of 2026-10-19).
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

/// The signals that request a host stop.
const SIGNALS: [libc::c_int; 2] = [libc::SIGINT, libc::SIGTERM];

/// The ends of the wake pipe, or -1 before [`install`].
static WAKE_READ: AtomicI32 = AtomicI32::new(-1);
static WAKE_WRITE: AtomicI32 = AtomicI32::new(-1);

/// Whether [`install`] has run, so it runs once.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Why the handlers could not be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallError(pub String);

/// Installs the handlers of SIGINT and SIGTERM and opens the wake pipe. The runtime calls it
/// only for a run that can reach a `for ever` or runs a driver (rule 18); other runs keep the
/// default actions. A second call does nothing.
pub fn install() -> Result<(), InstallError> {
    if INSTALLED.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let mut fds: [libc::c_int; 2] = [-1, -1];
    // SAFETY: `fds` is a valid array of two `c_int`s that `pipe` writes the two ends into.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(InstallError("cannot open the wake pipe".to_owned()));
    }
    for fd in fds {
        // SAFETY: `fd` is an open descriptor that `pipe` just returned; `F_GETFL`, `F_SETFL`,
        // `F_GETFD` and `F_SETFD` read and set its flags and touch no memory of the process.
        let set = unsafe {
            let status = libc::fcntl(fd, libc::F_GETFL);
            let descriptor = libc::fcntl(fd, libc::F_GETFD);
            status >= 0
                && descriptor >= 0
                && libc::fcntl(fd, libc::F_SETFL, status | libc::O_NONBLOCK) == 0
                && libc::fcntl(fd, libc::F_SETFD, descriptor | libc::FD_CLOEXEC) == 0
        };
        if !set {
            return Err(InstallError("cannot set the flags of the wake pipe".to_owned()));
        }
    }
    let [read, write] = fds;
    WAKE_READ.store(read, Ordering::Release);
    WAKE_WRITE.store(write, Ordering::Release);
    for signal in SIGNALS {
        // SAFETY: `action` is zero-initialized, a valid `sigaction` (an empty mask, no flags,
        // no handler) before its fields are set; `sigemptyset` initializes its mask in place.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: `action.sa_mask` is a valid `sigset_t` inside `action`, owned here.
        unsafe { libc::sigemptyset(&raw mut action.sa_mask) };
        // SAFETY: `action` is a fully initialized `sigaction` whose handler is
        // `on_signal`, an `extern "C"` function that does only async-signal-safe work; the
        // old action is not asked for (null).
        if unsafe { libc::sigaction(signal, &raw const action, std::ptr::null_mut()) } != 0 {
            return Err(InstallError("cannot install the signal handlers".to_owned()));
        }
    }
    Ok(())
}

/// The read end of the wake pipe, which becomes readable at each host stop request, for a
/// thread that waits with `poll` on it beside its own operation; `None` before [`install`].
pub fn wake_fd() -> Option<libc::c_int> {
    let fd = WAKE_READ.load(Ordering::Acquire);
    (fd >= 0).then_some(fd)
}

/// Reads and discards the bytes waiting in the wake pipe, which never blocks.
pub fn drain_wake() {
    let Some(fd) = wake_fd() else {
        return;
    };
    let mut buffer = [0u8; 64];
    loop {
        // SAFETY: `buffer` is a valid, writable array of `buffer.len()` bytes; the pipe is
        // non-blocking, so `read` returns at once.
        let read = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
        if read <= 0 {
            break;
        }
    }
}

/// Restores the default actions of SIGINT and SIGTERM and raises `signal` again: what the third
/// request does, and the end of a second grace period after the hard stop (rule 17).
pub fn restore_and_raise(signal: libc::c_int) {
    for each in SIGNALS {
        // SAFETY: `SIG_DFL` is the default disposition; `signal` touches no memory of the
        // process, and is async-signal-safe.
        unsafe { libc::signal(each, libc::SIG_DFL) };
    }
    // SAFETY: `raise` sends a signal to the calling thread and is async-signal-safe; with the
    // default action restored, it ends the process as `kill` would.
    unsafe { libc::raise(signal) };
}

/// The handler: [`handle`], which is all safe code, with `errno` kept.
extern "C" fn on_signal(signal: libc::c_int) {
    let saved = errno();
    handle(signal);
    set_errno(saved);
}

/// What a handler does for `signal`, without the `errno` bookkeeping: records the request,
/// wakes the waiters, and escalates at the third request. Async-signal-safe and allocation
/// free, which a test checks by calling it under a counting allocator.
#[doc(hidden)]
pub fn handle(signal: libc::c_int) {
    let requests = crate::attention::request_stop();
    let fd = WAKE_WRITE.load(Ordering::Acquire);
    if fd >= 0 {
        let byte = [1u8];
        // SAFETY: `byte` is a valid one-byte array; the pipe is non-blocking, so a full pipe
        // makes `write` fail at once, which is harmless: a byte already waits there.
        unsafe { libc::write(fd, byte.as_ptr().cast(), 1) };
    }
    if requests >= 3 {
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
