//! Starting the solver child (ADR 0124 rule 5): `una` re-executed, with an empty environment
//! and only its three pipes, through `posix_spawn`. On macOS the attribute
//! `POSIX_SPAWN_CLOEXEC_DEFAULT` makes every inherited descriptor close-on-exec, so the child
//! starts with its three pipes and nothing else; on Linux the child closes the rest itself
//! ([`crate::solver::descriptors`]). The signal mask is cleared and every signal's action is
//! reset to its default, so the child inherits neither a mask nor an ignore of `SIGXCPU` that
//! would void its processor-time limit. The child changes its own working directory to `/`
//! ([`crate::solver::confine`]). The standard library's `Command` cannot set the spawn
//! attribute, and cannot wait with a deadline, which is why these are here.
#![allow(unsafe_code)]

use std::ffi::CStr;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::time::Duration;

/// How a child ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// By its own exit, with this status.
    Exited(i32),
    /// By this signal.
    Signaled(i32),
}

/// Starts `program` with `args` (the first is its name), the three descriptors as its standard
/// streams, an empty environment, a cleared signal mask and default signal actions, and gives
/// its process id.
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
    // A source below 3 would be overwritten by an earlier `dup2` of the actions, so each such
    // source is first duplicated above 2 (close-on-exec), as the standard library does.
    let mut raised: Vec<OwnedFd> = Vec::new();
    let mut sources = [stdin.as_raw_fd(), stdout.as_raw_fd(), stderr.as_raw_fd()];
    for (source, borrowed) in sources.iter_mut().zip([stdin, stdout, stderr]) {
        if *source < 3 {
            // The standard library's duplicate is `fcntl` with `F_DUPFD_CLOEXEC` from 3, the
            // call made here before; its floor is not in its contract, so it is checked.
            // Close-on-exec rests on that implementation, pinned by the toolchain's version.
            let duplicate = borrowed.try_clone_to_owned()?;
            if duplicate.as_raw_fd() < 3 {
                return Err(io::Error::other("a duplicate landed on a standard stream"));
            }
            *source = duplicate.as_raw_fd();
            raised.push(duplicate);
        }
    }
    let mut argv: Vec<*mut libc::c_char> = args.iter().map(|a| a.as_ptr().cast_mut()).collect();
    argv.push(std::ptr::null_mut());
    let mut envp: [*mut libc::c_char; 1] = [std::ptr::null_mut()];
    // SAFETY: zeroed file actions are the state the init call expects to fill.
    let mut actions: libc::posix_spawn_file_actions_t = unsafe { std::mem::zeroed() };
    // SAFETY: zeroed attributes are the state the init call expects to fill.
    let mut attr: libc::posix_spawnattr_t = unsafe { std::mem::zeroed() };
    // SAFETY: `actions` is a valid place the init call fills; it is destroyed below exactly
    // once after its init succeeded.
    let status = unsafe { libc::posix_spawn_file_actions_init(&raw mut actions) };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status));
    }
    let result = (|| {
        for (fd, target) in sources.iter().zip(0..) {
            // SAFETY: `actions` is initialized; the descriptors are open for the call's
            // duration, borrowed by the caller or owned by `raised`.
            let added =
                unsafe { libc::posix_spawn_file_actions_adddup2(&raw mut actions, *fd, target) };
            if added != 0 {
                return Err(io::Error::from_raw_os_error(added));
            }
        }
        // SAFETY: `attr` is a valid place the init fills; destroyed below exactly once.
        let status = unsafe { libc::posix_spawnattr_init(&raw mut attr) };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        let attr_result = (|| {
            // SAFETY: zero is a valid `sigset_t` for the set calls to fill.
            let mut empty: libc::sigset_t = unsafe { std::mem::zeroed() };
            // SAFETY: as above.
            let mut all: libc::sigset_t = unsafe { std::mem::zeroed() };
            // SAFETY: `empty` and `all` are valid sets owned here.
            unsafe {
                libc::sigemptyset(&raw mut empty);
                libc::sigfillset(&raw mut all);
            }
            // SAFETY: `attr` is initialized and the sets are valid; the calls copy them.
            let set = unsafe {
                libc::posix_spawnattr_setsigmask(&raw mut attr, &raw const empty)
                    | libc::posix_spawnattr_setsigdefault(&raw mut attr, &raw const all)
            };
            if set != 0 {
                return Err(io::Error::from_raw_os_error(set));
            }
            // SAFETY: `attr` is initialized.
            let set = unsafe { libc::posix_spawnattr_setflags(&raw mut attr, FLAGS) };
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
    drop(raised);
    result
}

/// The spawn attributes: a cleared mask, default actions, and on macOS close-on-exec for
/// every inherited descriptor.
#[cfg(target_os = "macos")]
const FLAGS: libc::c_short = (libc::POSIX_SPAWN_SETSIGMASK
    | libc::POSIX_SPAWN_SETSIGDEF
    | libc::POSIX_SPAWN_CLOEXEC_DEFAULT) as libc::c_short;
#[cfg(target_os = "linux")]
const FLAGS: libc::c_short =
    (libc::POSIX_SPAWN_SETSIGMASK | libc::POSIX_SPAWN_SETSIGDEF) as libc::c_short;

/// Refuses a process id that names no single child: 0 and negative ids are groups.
fn one_child(pid: libc::pid_t) -> io::Result<()> {
    if pid > 0 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not the id of one child",
        ))
    }
}

/// Whether child `pid` has ended, without waiting.
///
/// # Errors
///
/// The error of `waitpid`, or an id that is not one child's.
pub fn try_wait(pid: libc::pid_t) -> io::Result<Option<Ended>> {
    one_child(pid)?;
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

/// Whether child `pid` has ended, without waiting, with the processor time it used (user
/// and system, all its threads) when it has: the parent reads it to tell the kernel's
/// `SIGKILL` at the hard processor-time limit from any other kill (#2321).
///
/// # Errors
///
/// The error of `wait4`, or an id that is not one child's.
pub fn try_wait_with_usage(pid: libc::pid_t) -> io::Result<Option<(Ended, Duration)>> {
    one_child(pid)?;
    let mut status: libc::c_int = 0;
    // SAFETY: an all-zero `rusage` is a valid object for the call to fill.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `status` and `usage` are valid places the call writes; `WNOHANG` returns at once.
    let waited = unsafe { libc::wait4(pid, &raw mut status, libc::WNOHANG, &raw mut usage) };
    if waited == pid {
        Ok(Some((ended(status), processor_time(&usage))))
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

/// The user and system time of a `rusage`, negative fields read as zero.
fn processor_time(usage: &libc::rusage) -> Duration {
    let part = |t: libc::timeval| {
        let seconds = u64::try_from(t.tv_sec).unwrap_or(0);
        let micros = u64::try_from(t.tv_usec).unwrap_or(0);
        Duration::from_secs(seconds).saturating_add(Duration::from_micros(micros))
    };
    part(usage.ru_utime).saturating_add(part(usage.ru_stime))
}

/// Waits for child `pid` to end.
///
/// # Errors
///
/// The error of `waitpid`, other than an interruption, which is retried, or an id that is
/// not one child's.
pub fn wait(pid: libc::pid_t) -> io::Result<Ended> {
    one_child(pid)?;
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

/// Ends child `pid` with `SIGKILL`; the caller then waits for it. The caller MUST NOT call
/// this after the child was waited for, since the id may then name another process.
///
/// # Errors
///
/// The error of `kill`, other than a child already gone, or an id that is not one child's.
pub fn kill(pid: libc::pid_t) -> io::Result<()> {
    one_child(pid)?;
    // SAFETY: `kill` takes two integers; `pid` is positive, so it names one process, the
    // child this process spawned and has not yet waited for.
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
