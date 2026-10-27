//! Starting the solver child (ADR 0124 rule 5): `una` re-executed, with an empty environment
//! and only its three pipes, through `posix_spawn`. On macOS the attribute
//! `POSIX_SPAWN_CLOEXEC_DEFAULT` makes every inherited descriptor close-on-exec, so the child
//! starts with its three pipes and nothing else; on Linux the child closes the rest itself
//! ([`crate::solver::descriptors`]). The signal mask is cleared, so the child does not inherit
//! a mask that would block `SIGXCPU`. The child changes its own working directory to `/`
//! ([`crate::solver::confine`]). The standard library's `Command` cannot set the spawn
//! attribute, and cannot wait with a deadline, which is why these are here.
#![allow(unsafe_code)]

use std::ffi::CStr;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};

/// How a child ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// By its own exit, with this status.
    Exited(i32),
    /// By this signal.
    Signaled(i32),
}

/// Starts `program` with `args` (the first is its name), the three descriptors as its standard
/// streams, an empty environment and a cleared signal mask, and gives its process id.
///
/// # Errors
///
/// The error `posix_spawn` or an attribute call reports.
pub fn spawn(
    program: &CStr,
    args: &[&CStr],
    stdin: BorrowedFd<'_>,
    stdout: BorrowedFd<'_>,
    stderr: BorrowedFd<'_>,
) -> io::Result<libc::pid_t> {
    let mut argv: Vec<*mut libc::c_char> = args.iter().map(|a| a.as_ptr().cast_mut()).collect();
    argv.push(std::ptr::null_mut());
    let mut envp: [*mut libc::c_char; 1] = [std::ptr::null_mut()];
    // SAFETY: zeroed file actions are the state the init call expects to fill.
    let mut actions: libc::posix_spawn_file_actions_t = unsafe { std::mem::zeroed() };
    // SAFETY: zeroed attributes are the state the init call expects to fill.
    let mut attr: libc::posix_spawnattr_t = unsafe { std::mem::zeroed() };
    // SAFETY: `actions` and `attr` are valid places that the init calls fill; each is destroyed
    // below exactly once after its init succeeded.
    let status = unsafe { libc::posix_spawn_file_actions_init(&raw mut actions) };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status));
    }
    let result = (|| {
        for (fd, target) in [(stdin, 0), (stdout, 1), (stderr, 2)] {
            // SAFETY: `actions` is initialized; the descriptors are open for the call's
            // duration, borrowed by the caller.
            let added = unsafe {
                libc::posix_spawn_file_actions_adddup2(&raw mut actions, fd.as_raw_fd(), target)
            };
            if added != 0 {
                return Err(io::Error::from_raw_os_error(added));
            }
        }
        // SAFETY: `attr` is a valid place the init fills.
        let status = unsafe { libc::posix_spawnattr_init(&raw mut attr) };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        let attr_result = (|| {
            let mut flags: libc::c_short = libc::POSIX_SPAWN_SETSIGMASK as libc::c_short;
            #[cfg(target_os = "macos")]
            {
                flags |= libc::POSIX_SPAWN_CLOEXEC_DEFAULT as libc::c_short;
            }
            // SAFETY: zero is a valid `sigset_t` to empty; `attr` is initialized.
            let mut empty: libc::sigset_t = unsafe { std::mem::zeroed() };
            // SAFETY: `empty` is a valid `sigset_t` owned here.
            unsafe { libc::sigemptyset(&raw mut empty) };
            // SAFETY: `attr` is initialized and `empty` a valid set that the call copies.
            let set = unsafe { libc::posix_spawnattr_setsigmask(&raw mut attr, &raw const empty) };
            if set != 0 {
                return Err(io::Error::from_raw_os_error(set));
            }
            // SAFETY: `attr` is initialized.
            let set = unsafe { libc::posix_spawnattr_setflags(&raw mut attr, flags) };
            if set != 0 {
                return Err(io::Error::from_raw_os_error(set));
            }
            let mut pid: libc::pid_t = 0;
            // SAFETY: every pointer names initialized data that outlives the call: the
            // program's and arguments' NUL-terminated texts, the null-terminated `argv` and
            // `envp`, the actions and attributes; `pid` is a valid place the call writes.
            let spawned = unsafe {
                libc::posix_spawn(
                    &raw mut pid,
                    program.as_ptr(),
                    &raw const actions,
                    &raw const attr,
                    argv.as_mut_ptr(),
                    envp.as_mut_ptr(),
                )
            };
            if spawned != 0 {
                return Err(io::Error::from_raw_os_error(spawned));
            }
            Ok(pid)
        })();
        // SAFETY: `attr` was initialized above and is destroyed once.
        unsafe { libc::posix_spawnattr_destroy(&raw mut attr) };
        attr_result
    })();
    // SAFETY: `actions` was initialized above and is destroyed once.
    unsafe { libc::posix_spawn_file_actions_destroy(&raw mut actions) };
    result
}

/// Whether child `pid` has ended, without waiting.
///
/// # Errors
///
/// The error of `waitpid`.
pub fn try_wait(pid: libc::pid_t) -> io::Result<Option<Ended>> {
    let mut status: libc::c_int = 0;
    // SAFETY: `status` is a valid place the call writes; `WNOHANG` returns at once.
    let waited = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
    if waited == pid {
        Ok(Some(ended(status)))
    } else if waited == 0 {
        Ok(None)
    } else {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EINTR) {
            Ok(None)
        } else {
            Err(error)
        }
    }
}

/// Waits for child `pid` to end.
///
/// # Errors
///
/// The error of `waitpid`, other than an interruption, which is retried.
pub fn wait(pid: libc::pid_t) -> io::Result<Ended> {
    loop {
        let mut status: libc::c_int = 0;
        // SAFETY: `status` is a valid place the call writes.
        let waited = unsafe { libc::waitpid(pid, &raw mut status, 0) };
        if waited == pid {
            return Ok(ended(status));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    }
}

/// Ends child `pid` with `SIGKILL`; the caller then waits for it.
///
/// # Errors
///
/// The error of `kill`, other than a child already gone.
pub fn kill(pid: libc::pid_t) -> io::Result<()> {
    // SAFETY: `kill` takes two integers; `pid` is a child this process spawned and has not
    // yet waited for, so the id names it and no other process.
    if unsafe { libc::kill(pid, libc::SIGKILL) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

/// Reads a wait status.
fn ended(status: libc::c_int) -> Ended {
    if libc::WIFEXITED(status) {
        Ended::Exited(libc::WEXITSTATUS(status))
    } else {
        Ended::Signaled(libc::WTERMSIG(status))
    }
}
