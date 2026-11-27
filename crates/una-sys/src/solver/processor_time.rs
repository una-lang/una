//! The end of a solver child past its processor-time limit (ADR 0124 rule 6, spec 12 §4,
//! #2268): the child ends by `SIGXCPU`, which the parent reports as `cpu`, whatever handler
//! the front end installs for that signal.
//!
//! Z3 5.1.0's front end installs a handler of `SIGXCPU` (`register_on_timeout_proc` in Z3's
//! `util/timeout.cpp`) that writes `timeout` through `std::cout`, takes a mutex and exits.
//! None of that is async-signal-safe: a signal that lands while the solver is inside `malloc`
//! can make the handler allocate in a heap whose lock and state are half-updated, and glibc
//! then aborts (`sysmalloc: Assertion`, seen once in 69 runs on glibc 2.28, #2268), or the
//! handler waits forever on a lock the interrupted code holds (about 1 in 100 runs of an
//! allocation-heavy query under load, on glibc 2.28 and 2.39, #2268): a deadlocked child uses
//! no processor time, so only the wall-clock guard ends it. The outcome was a `crash` or a
//! `wallclock`, never a proof, but a time-limited query must end the same way each time.
//!
//! So no handler of `SIGXCPU` ever runs in the child. [`hold`] blocks the signal in the
//! calling thread while the child is still single-threaded, before the limits are set, and
//! every thread started later, Z3's included, inherits the mask. [`watch`] then starts one
//! thread that waits for the signal synchronously (`sigwait`, which takes a pending signal
//! without running its handler). When the kernel sends it, that thread, outside any signal
//! context, sets the signal's action back to its default, unblocks it in itself and raises it:
//! the process ends by `SIGXCPU` (no core: the core limit is zero). Every call it makes is a
//! plain system call wrapper that is safe in any thread at any time. If the raise were ever to
//! return, the thread writes `timeout`, the word the front end's own handler writes, and exits
//! with status 0, the exit the verifier also reads as `cpu`; if the wait itself fails, it ends
//! the child with status 4, a child with no answer, which the parent reports as a crash. Neither path can
//! make a proof.
#![allow(unsafe_code)]

use std::io;

/// The signal set holding `SIGXCPU` alone.
fn processor_time_signal() -> io::Result<libc::sigset_t> {
    // SAFETY: an all-zero `sigset_t` is a valid object for `sigemptyset` to initialize.
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: `set` is a valid, writable `sigset_t`.
    if unsafe { libc::sigemptyset(&raw mut set) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `set` was initialized by `sigemptyset`; `SIGXCPU` is a valid signal number.
    if unsafe { libc::sigaddset(&raw mut set, libc::SIGXCPU) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(set)
}

/// Blocks `SIGXCPU` in the calling thread, so that no handler of it runs there or in any
/// thread it starts later. The solver child calls it first, while it has one thread.
///
/// # Errors
///
/// The error of `pthread_sigmask`; the caller fails closed.
pub fn hold() -> io::Result<()> {
    let set = processor_time_signal()?;
    // SAFETY: `set` is an initialized signal set that the call only reads; the old mask is not
    // asked for.
    let status =
        unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut()) };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status));
    }
    Ok(())
}

/// Starts the thread that waits for `SIGXCPU` and ends the process by it. Called after
/// [`hold`], once the process is confined, so the thread runs under the confinement.
///
/// # Errors
///
/// The error of building the signal set or starting the thread; the caller fails closed.
pub fn watch() -> io::Result<()> {
    let set = processor_time_signal()?;
    std::thread::Builder::new()
        .spawn(move || end_by_the_signal(set))
        .map(drop)
}

/// The waiting thread: never returns.
fn end_by_the_signal(set: libc::sigset_t) {
    loop {
        let mut signal: libc::c_int = 0;
        // SAFETY: `set` is an initialized signal set, blocked in this thread (inherited from
        // the thread that called `hold`), and `signal` is a valid place for the call to write.
        let status = unsafe { libc::sigwait(&raw const set, &raw mut signal) };
        match status {
            0 if signal == libc::SIGXCPU => break,
            0 => {}
            libc::EINTR => {}
            error => fail(error),
        }
    }
    // SAFETY: an all-zero `sigaction` is valid; it is set to the default action with an empty
    // mask before the call reads it.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = libc::SIG_DFL;
    // SAFETY: `action.sa_mask` is a valid, writable `sigset_t`.
    unsafe { libc::sigemptyset(&raw mut action.sa_mask) };
    // SAFETY: `action` is fully initialized and only read; the old action is not asked for.
    // The front end's handler is replaced from a plain thread, never from a signal handler.
    unsafe { libc::sigaction(libc::SIGXCPU, &raw const action, std::ptr::null_mut()) };
    // SAFETY: `set` is an initialized signal set that the call only reads.
    unsafe { libc::pthread_sigmask(libc::SIG_UNBLOCK, &raw const set, std::ptr::null_mut()) };
    // SAFETY: `raise` sends the signal to this thread; with the default action and the signal
    // unblocked here, it ends the whole process.
    unsafe { libc::raise(libc::SIGXCPU) };
    // Not reached on a system that delivers the signal; the exit the verifier also reads as
    // `cpu`, never an answer.
    write_all(1, b"timeout\n");
    // SAFETY: `_exit` ends the process at once, without running anything of the front end's.
    unsafe { libc::_exit(0) }
}

/// Ends the child when the wait itself fails, with status 4 and no answer, which the parent
/// reports as a crash. A library crate does not print (conventions), so the status is the
/// whole report; the `limits` gate's processor-time case exercises the wait on each target.
fn fail(_error: libc::c_int) -> ! {
    // SAFETY: `_exit` ends the process at once.
    unsafe { libc::_exit(4) }
}

/// Writes `bytes` to descriptor `fd` with plain `write` calls, retrying an interrupted call and
/// giving up on any other failure: the exit that follows is the outcome either way.
fn write_all(fd: libc::c_int, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        // SAFETY: `bytes` is a valid slice of `bytes.len()` readable bytes.
        let written = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if let Ok(n) = usize::try_from(written) {
            if n == 0 {
                return;
            }
            bytes = bytes.get(n..).unwrap_or_default();
        } else if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return;
        }
    }
}
