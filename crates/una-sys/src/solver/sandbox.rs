//! The solver child's sandbox on macOS (ADR 0124 rule 7, Question 6): a profile that denies
//! everything by default and allows only what the front end needs, applied by `sandbox_init`.
//! When that fails because an outer sandbox already confines `una` (as the `hunt` gate's does,
//! and some harnesses do), the child asks the system whether it is sandboxed, checks each
//! operation of a fixed list with `sandbox_check`, and takes the outer sandbox as its
//! confinement only if it denies at least outbound and inbound network access and writing
//! files at `/` and outside the temporary directory. `sandbox_init` and `sandbox_check` are
//! deprecated interfaces, used because they are the ones the system offers to a process
//! confining itself; probe P5 showed them to behave as this module expects.
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
/// Asks without reporting a denial as a violation to the system log.
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
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init(text) => write!(f, "the sandbox profile was refused: {text}"),
            Self::Check(op) => write!(f, "cannot ask the outer sandbox about {op}"),
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

/// One operation of the fixed list, with its name in the report and its path filter.
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

/// The list of rule 7, in the order the report keeps.
const PROBES: [Probe; 8] = [
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
    // SAFETY: `PROFILE` is a NUL-terminated text, and `error` a valid place for the pointer
    // the call writes on failure, which is freed below.
    let applied = unsafe { sandbox_init(PROFILE.as_ptr(), 0, &raw mut error) };
    if applied == 0 {
        return Ok(Confinement::Own);
    }
    let text = if error.is_null() {
        String::from("no reason given")
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
