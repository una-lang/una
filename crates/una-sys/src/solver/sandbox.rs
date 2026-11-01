//! The solver child's sandbox on macOS (ADR 0124 rule 7, Question 6): a profile that denies
//! everything by default and allows only what the front end needs, applied by `sandbox_init`.
//! When that fails because an outer sandbox already confines `una` (as the `hunt` gate's does,
//! and some harnesses do), the child asks the system whether it is sandboxed, checks each
//! operation of a fixed list with `sandbox_check`, and takes the outer sandbox as its
//! confinement only if it denies at least outbound and inbound network access and writing,
//! creating and removing files at `/` and outside the temporary directory. `sandbox_init` and
//! `sandbox_check` are deprecated interfaces, used because they are the ones the system offers
//! to a process confining itself; probe P5 showed them to behave as this module expects.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_int};
use std::fmt;

use super::Confinement;

unsafe extern "C" {
    /// Applies `profile`, a text in the sandbox profile language, to this process.
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
    /// Frees the text `sandbox_init` wrote into `errorbuf`.
    fn sandbox_free_error(errorbuf: *mut c_char);
    /// Whether process `pid` may perform `operation`; with a null operation, whether it is
    /// sandboxed at all. `kind` selects the filter the variadic argument supplies.
    fn sandbox_check(pid: libc::pid_t, operation: *const c_char, kind: c_int, ...) -> c_int;
}

/// The profile of rule 7: deny everything; allow reading the one sysctl the C++ library asks
/// for when Z3 reads the number of processors (probe P7).
pub const PROFILE: &CStr =
    c"(version 1)(deny default)(allow sysctl-read (sysctl-name \"hw.ncpu\"))";

/// `sandbox_check`'s filter kinds: none, and a path.
const FILTER_NONE: c_int = 0;
const FILTER_PATH: c_int = 1;
/// Asks without reporting a denial as a violation to the system log
/// (`SANDBOX_CHECK_NO_REPORT`, a value the library has kept since it exposed the interface; a
/// changed value would make every check answer -1, which fails closed below).
const NO_REPORT: c_int = 0x4000_0000;

/// Why the child is not confined.
#[derive(Debug)]
pub enum SandboxError {
    /// `sandbox_init` failed and no outer sandbox confines the process; the text is the
    /// system's.
    Init(String),
    /// `sandbox_check` itself failed on an operation of the list.
    Check(&'static str),
    /// The outer sandbox allows an operation that rule 7's minimum requires it to deny.
    TooPermissive(&'static str),
    /// Stderr could not be set aside around `sandbox_init`, or put back after it.
    Stderr(String),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init(text) => write!(f, "the sandbox profile was refused: {text}"),
            Self::Check(op) => write!(f, "cannot ask the outer sandbox about {op}"),
            Self::Stderr(e) => write!(f, "cannot set stderr aside around the sandbox: {e}"),
            Self::TooPermissive(op) => {
                write!(
                    f,
                    "the outer sandbox is too permissive for the solver: {op} is allowed"
                )
            }
        }
    }
}

impl std::error::Error for SandboxError {}

/// One operation of the fixed list, with its name in the report, its path filter, and
/// whether rule 7's minimum requires the outer sandbox to deny it.
struct Probe {
    name: &'static str,
    operation: &'static CStr,
    path: Option<Path>,
    required: bool,
}

/// A path filter: a fixed one, or the one the parent passes.
enum Path {
    Root,
    Outside,
    Shell,
}

/// The list of rule 7, in the order the report keeps: the network, then writing, creating
/// and removing files (a limit on the file size stops writing data but not removing or
/// renaming files, so those are required too), then reads, exec and fork, which are recorded.
const PROBES: [Probe; 12] = [
    Probe {
        name: "network-outbound",
        operation: c"network-outbound",
        path: None,
        required: true,
    },
    Probe {
        name: "network-inbound",
        operation: c"network-inbound",
        path: None,
        required: true,
    },
    Probe {
        name: "file-write-data",
        operation: c"file-write-data",
        path: Some(Path::Root),
        required: true,
    },
    Probe {
        name: "file-write-data (outside the temporary directory)",
        operation: c"file-write-data",
        path: Some(Path::Outside),
        required: true,
    },
    Probe {
        name: "file-write-create",
        operation: c"file-write-create",
        path: Some(Path::Root),
        required: true,
    },
    Probe {
        name: "file-write-create (outside the temporary directory)",
        operation: c"file-write-create",
        path: Some(Path::Outside),
        required: true,
    },
    Probe {
        name: "file-write-unlink",
        operation: c"file-write-unlink",
        path: Some(Path::Root),
        required: true,
    },
    Probe {
        name: "file-write-unlink (outside the temporary directory)",
        operation: c"file-write-unlink",
        path: Some(Path::Outside),
        required: true,
    },
    Probe {
        name: "file-read-data",
        operation: c"file-read-data",
        path: Some(Path::Root),
        required: false,
    },
    Probe {
        name: "file-read-data (outside the temporary directory)",
        operation: c"file-read-data",
        path: Some(Path::Outside),
        required: false,
    },
    Probe {
        name: "process-exec*",
        operation: c"process-exec*",
        path: Some(Path::Shell),
        required: false,
    },
    Probe {
        name: "process-fork",
        operation: c"process-fork",
        path: None,
        required: false,
    },
];

/// Confines this process: by [`PROFILE`], or by the outer sandbox when there is one that
/// meets the minimum. `outside_temp` is the path outside the temporary directory to check.
///
/// # Errors
///
/// [`SandboxError`]: the profile refused with no outer sandbox, a check that failed, or an
/// outer sandbox that allows a required operation.
pub fn confine(outside_temp: &CStr) -> Result<Confinement, SandboxError> {
    let mut error: *mut c_char = std::ptr::null_mut();
    // The library writes its own line on the process's stderr when the profile is refused
    // ("sandbox initialization failed: Operation not permitted", under any outer sandbox):
    // the child's stderr must stay empty, since a proof that ends with exit status 1 requires
    // nothing on it (rule 8, #1360), so the call runs with stderr on a pipe of its own and
    // what the library wrote there is dropped; the reason travels in `error`.
    let apply = || {
        // SAFETY: `PROFILE` is a NUL-terminated text, and `error` a valid place for the
        // pointer the call writes on failure, which is freed below.
        unsafe { sandbox_init(PROFILE.as_ptr(), 0, &raw mut error) }
    };
    let (applied, written) =
        with_stderr_aside(apply).map_err(|e| SandboxError::Stderr(e.to_string()))?;
    if applied == 0 {
        return Ok(Confinement::Own);
    }
    let text = if error.is_null() {
        let written = String::from_utf8_lossy(&written).trim().to_owned();
        if written.is_empty() {
            String::from("no reason given")
        } else {
            written
        }
    } else {
        // SAFETY: on failure `error` points at a NUL-terminated text the library allocated,
        // read once here and freed once below.
        let text = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: `error` came from `sandbox_init` and is freed exactly once.
        unsafe { sandbox_free_error(error) };
        text
    };
    // SAFETY: `getpid` cannot fail.
    let pid = unsafe { libc::getpid() };
    // SAFETY: a null operation asks whether the process is sandboxed; no variadic argument is
    // read with `FILTER_NONE`.
    let sandboxed = unsafe { sandbox_check(pid, std::ptr::null(), FILTER_NONE) };
    if sandboxed != 1 {
        return Err(SandboxError::Init(text));
    }
    let mut denied = Vec::new();
    for probe in &PROBES {
        let path: Option<&CStr> = match probe.path {
            None => None,
            Some(Path::Root) => Some(c"/"),
            Some(Path::Outside) => Some(outside_temp),
            Some(Path::Shell) => Some(c"/bin/sh"),
        };
        let answer = match path {
            // SAFETY: the operation is a NUL-terminated text; with `FILTER_NONE` no variadic
            // argument is read.
            None => unsafe {
                sandbox_check(pid, probe.operation.as_ptr(), FILTER_NONE | NO_REPORT)
            },
            // SAFETY: as above, and with `FILTER_PATH` the one variadic argument is the
            // NUL-terminated path, which outlives the call.
            Some(path) => unsafe {
                sandbox_check(
                    pid,
                    probe.operation.as_ptr(),
                    FILTER_PATH | NO_REPORT,
                    path.as_ptr(),
                )
            },
        };
        match answer {
            1 => denied.push(probe.name),
            0 if probe.required => return Err(SandboxError::TooPermissive(probe.name)),
            0 => {}
            _ => return Err(SandboxError::Check(probe.name)),
        }
    }
    Ok(Confinement::Outer { denied })
}

/// The most bytes kept of what `f` writes on stderr.
const STDERR_ASIDE_BYTES: usize = 4096;

/// Runs `f` with descriptor 2 on a pipe of its own, then puts the process's stderr back and
/// gives `f`'s result with what it wrote there, at most [`STDERR_ASIDE_BYTES`]. The pipe's
/// write end does not block, so a writer past its capacity loses the rest instead of hanging
/// the child; no descriptor above 2 is left open, whatever fails.
///
/// # Errors
///
/// The error of the first call that fails. One that fails after `f` ran leaves the result
/// unused: the caller fails closed.
fn with_stderr_aside<T>(f: impl FnOnce() -> T) -> std::io::Result<(T, Vec<u8>)> {
    use std::io::{Error, Read};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    let (mut reader, writer) = std::io::pipe()?;
    for fd in [reader.as_raw_fd(), writer.as_raw_fd()] {
        // SAFETY: `fcntl` on a descriptor this function owns, with integer arguments.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        // SAFETY: as above.
        if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
        {
            return Err(Error::last_os_error());
        }
    }
    // SAFETY: `dup` of the standard error descriptor; the new one is owned below.
    let saved = unsafe { libc::dup(libc::STDERR_FILENO) };
    if saved == -1 {
        return Err(Error::last_os_error());
    }
    // SAFETY: `saved` is a fresh descriptor that nothing else owns.
    let saved = unsafe { OwnedFd::from_raw_fd(saved) };
    // SAFETY: `dup2` between two open descriptors; it touches no memory.
    if unsafe { libc::dup2(writer.as_raw_fd(), libc::STDERR_FILENO) } == -1 {
        return Err(Error::last_os_error());
    }
    drop(writer);
    let result = f();
    // SAFETY: as above; descriptor 2 is the process's stderr again.
    if unsafe { libc::dup2(saved.as_raw_fd(), libc::STDERR_FILENO) } == -1 {
        return Err(Error::last_os_error());
    }
    drop(saved);
    // Descriptor 2 was the pipe's last write end besides the one dropped above, so the reads
    // end at the end of the pipe, or at `EAGAIN` if nothing was written.
    let mut written = Vec::new();
    let mut chunk = [0_u8; 512];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                let room = STDERR_ASIDE_BYTES.saturating_sub(written.len());
                written.extend_from_slice(chunk.get(..read.min(room)).unwrap_or(&[]));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok((result, written))
}
