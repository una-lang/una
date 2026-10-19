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
//! The `unsafe` code here, as in `dir`, is the calls of the operating system through the
//! `libc` crate, which is Rust declarations of the system interface only, with no C source
//! built (ADR 0116 Appendix A, amendment of 2026-10-19).
#![allow(unsafe_code)]

use std::sync::OnceLock;
use std::sync::atomic::{AtomicI32, Ordering};

/// The signals that request a host stop.
const SIGNALS: [libc::c_int; 2] = [libc::SIGINT, libc::SIGTERM];

/// The lowest descriptor the wake pipe's ends move to, above those that other code opens and
/// closes, so that no reused descriptor ever receives the handler's byte.
const HIGH_FD: libc::c_int = 512;

/// The ends of the wake pipe, or -1 before [`install`].
static WAKE_READ: AtomicI32 = AtomicI32::new(-1);
static WAKE_WRITE: AtomicI32 = AtomicI32::new(-1);

/// The outcome of the first [`install`], which every later call returns. Never read by a
/// handler, so its lock is harmless.
static INSTALLED: OnceLock<Result<(), InstallError>> = OnceLock::new();

/// Why the handlers could not be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallError(pub String);

/// Installs the handlers of SIGINT and SIGTERM and opens the wake pipe. The runtime calls it
/// only for a run that can reach a `for ever` or runs a driver (rule 18), at start-up, before it
/// starts any thread that could start a process. Every call returns the outcome of the first,
/// and a call that comes while the first runs waits for it. A failure leaves nothing behind:
/// no descriptor open and the previous actions of both signals in place.
pub fn install() -> Result<(), InstallError> {
    INSTALLED.get_or_init(install_once).clone()
}

fn install_once() -> Result<(), InstallError> {
    let [read, write] = wake_pipe()?;
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
            return Err(InstallError(
                "cannot install the signal handlers".to_owned(),
            ));
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

/// The two ends of a new pipe, non-blocking and close-on-exec, at descriptors of [`HIGH_FD`] or
/// above; on failure, nothing stays open.
fn wake_pipe() -> Result<[libc::c_int; 2], InstallError> {
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
        return Err(InstallError("cannot open the wake pipe".to_owned()));
    }
    let mut moved: [libc::c_int; 2] = [-1, -1];
    for (fd, high) in fds.iter().zip(moved.iter_mut()) {
        // SAFETY: `fd` is an open descriptor that the pipe call returned; `F_DUPFD_CLOEXEC`
        // duplicates it close-on-exec at the lowest free descriptor from `HIGH_FD`, and
        // `F_GETFL`/`F_SETFL` read and set its status flags; no memory is touched.
        *high = unsafe {
            let duplicate = libc::fcntl(*fd, libc::F_DUPFD_CLOEXEC, HIGH_FD);
            let status = libc::fcntl(*fd, libc::F_GETFL);
            if duplicate >= 0
                && status >= 0
                && libc::fcntl(duplicate, libc::F_SETFL, status | libc::O_NONBLOCK) == 0
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
        return Err(InstallError("cannot set up the wake pipe".to_owned()));
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

/// A host stop request from normal code, not from a signal (rule 14: `--deadline`, a driver
/// shutdown, `stop after`): records it and wakes every thread waiting on the pipe, as a
/// signal does. Returns how many requests have come, this one included.
pub fn request_host_stop() -> u32 {
    let requests = crate::attention::request_stop();
    let fd = WAKE_WRITE.load(Ordering::Acquire);
    if fd >= 0 {
        let byte = [1u8];
        // SAFETY: `byte` is a valid one-byte array; the pipe is non-blocking, so a full pipe
        // makes `write` fail at once, which is harmless: a byte already waits there.
        unsafe { libc::write(fd, byte.as_ptr().cast(), 1) };
    }
    requests
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
