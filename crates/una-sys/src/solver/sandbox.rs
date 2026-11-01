//! The solver child's sandbox on macOS (ADR 0124 rule 7, Question 6, and its amendment of
//! 2026-11-01): a profile that denies everything by default and allows only what the front end
//! needs, applied by `sandbox_init`. When that fails because an outer sandbox already confines
//! `una` (as the `hunt` gate's does, and some harnesses do), the child asks the system whether it
//! is sandboxed and asks the outer sandbox about each probe of a fixed list: the network
//! operations with no filter, toward the loopback addresses over TCP and UDP and toward local
//! sockets, and eight write operations at seven fixed places, none derived from the working
//! directory or the environment. It takes the outer sandbox as its confinement only if every
//! one of those is denied, and then lowers its open-file limit to zero, so that no socket or
//! file can be opened whatever a filter of the outer profile allows. `sandbox_init` and
//! `sandbox_check` are deprecated interfaces, used because they are the ones the system offers
//! to a process confining itself; `sandbox_check_network` is looked up at run time, and its
//! absence fails closed.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString, OsString, c_char, c_int, c_void};
use std::fmt;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

use super::Confinement;

unsafe extern "C" {
    /// Applies `profile`, a text in the sandbox profile language, to this process.
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
    /// Frees the text `sandbox_init` wrote into `errorbuf`.
    fn sandbox_free_error(errorbuf: *mut c_char);
    /// Whether process `pid` may perform `operation`; with a null operation, whether it is
    /// sandboxed at all. `kind` selects the filter the variadic argument supplies.
    fn sandbox_check(pid: libc::pid_t, operation: *const c_char, kind: c_int, ...) -> c_int;
    /// This task's port, which `task_info` takes.
    static mach_task_self_: libc::mach_port_t;
}

/// `sandbox_check_network(token, attributes, operation, socket, address)`: whether the
/// process of the audit token may perform the network `operation` on `socket` toward
/// `address`, an IPv4 or IPv6 address; 1 denied, 0 allowed, -1 an error. A null `attributes`
/// asks without reporting a denial.
type CheckNetwork = unsafe extern "C" fn(
    *const [u32; 8],
    *const c_void,
    *const c_char,
    c_int,
    *const libc::sockaddr,
) -> c_int;

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
/// `task_info`'s flavor for the audit token, of eight words.
const TASK_AUDIT_TOKEN: libc::task_flavor_t = 15;

/// The network operations, each probed with no filter, toward the loopback addresses and
/// toward local sockets.
const NETWORK: [&CStr; 3] = [c"network-outbound", c"network-inbound", c"network-bind"];
/// The ports asked about for each loopback address: a profile's address filter names one port
/// or every port, so these catch every port alike, and the common services' ports; the
/// open-file limit of zero covers a port not in the list.
const PORTS: [u16; 16] = [
    0, 1, 22, 53, 80, 443, 631, 3000, 5000, 5432, 6379, 8000, 8080, 8443, 9000, 65535,
];
/// The write operations, each probed at every place of [`places`].
const WRITES: [&CStr; 8] = [
    c"file-write-data",
    c"file-write-create",
    c"file-write-unlink",
    c"file-write-xattr",
    c"file-write-mode",
    c"file-write-flags",
    c"file-write-times",
    c"file-write-owner",
];
/// The fixed places of the write probes, each by its resolved path; the home directory of the
/// user's account and the user's temporary directory follow.
const FIXED_PLACES: [&str; 5] = ["/", "/Users", "/private/var", "/private/tmp", "/Volumes"];
/// Where the system's daemons keep their sockets, a place of the local-socket probes only.
const SOCKETS: &CStr = c"/private/var/run";

/// Why the child is not confined.
#[derive(Debug)]
pub enum SandboxError {
    /// `sandbox_init` failed and no outer sandbox confines the process; the text is the
    /// system's.
    Init(String),
    /// The outer sandbox could not be asked about a probe, which names the operation: the
    /// library does not know it, or the call failed.
    Check(String),
    /// A place of the write probes could not be found or resolved.
    Place(&'static str, String),
    /// The outer sandbox allows a probe that rule 7's minimum requires it to deny.
    TooPermissive(String),
    /// Stderr could not be set aside around `sandbox_init`.
    Stderr(String),
    /// Stderr could not be put back after `sandbox_init`; `stderr` is the process's own,
    /// kept so that this reason can still be written there.
    Restore {
        /// The system's error.
        error: String,
        /// A duplicate of the process's stderr.
        stderr: OwnedFd,
    },
    /// The open-file limit could not be lowered under the outer sandbox.
    Descriptors(String),
}

impl SandboxError {
    /// The process's own stderr when descriptor 2 could not be put back, for the reason.
    pub fn original_stderr(&self) -> Option<BorrowedFd<'_>> {
        match self {
            Self::Restore { stderr, .. } => Some(stderr.as_fd()),
            _ => None,
        }
    }
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init(text) => write!(f, "the sandbox profile was refused: {text}"),
            Self::Check(probe) => write!(f, "cannot ask the outer sandbox about {probe}"),
            Self::Place(place, e) => {
                write!(f, "cannot find {place} to check the outer sandbox: {e}")
            }
            Self::Stderr(e) => write!(f, "cannot set stderr aside around the sandbox: {e}"),
            Self::Restore { error, .. } => {
                write!(f, "cannot put stderr back after the sandbox: {error}")
            }
            Self::Descriptors(e) => write!(f, "cannot lower the open-file limit: {e}"),
            Self::TooPermissive(probe) => {
                write!(
                    f,
                    "the outer sandbox is too permissive for the solver: {probe} is allowed"
                )
            }
        }
    }
}

impl std::error::Error for SandboxError {}

/// One probe's answer: its name in the report, whether rule 7's minimum requires the outer
/// sandbox to deny it, and the answer, 1 denied, 0 allowed, anything else an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// The probe's name: the operation, then its filter or place when it has one.
    pub name: String,
    /// Whether the minimum requires a denial.
    pub required: bool,
    /// What the outer sandbox answered.
    pub answer: c_int,
}

/// Confines this process: by [`PROFILE`], or by the outer sandbox when there is one that
/// meets the minimum.
///
/// # Errors
///
/// [`SandboxError`]: the profile refused with no outer sandbox, a probe that could not be
/// asked, or an outer sandbox that allows a required probe.
pub fn confine() -> Result<Confinement, SandboxError> {
    // The library writes its own line on the process's stderr when the profile is refused
    // ("sandbox initialization failed: Operation not permitted", under any outer sandbox):
    // the child's stderr must stay empty, since a proof that ends with exit status 1 requires
    // nothing on it (rule 8, #1360), so the call runs with stderr on a pipe of its own and
    // what the library wrote there is dropped; the reason travels in the error buffer, read
    // and freed inside the call, so that no later failure leaks it.
    let apply = || {
        let mut error: *mut c_char = std::ptr::null_mut();
        // SAFETY: `PROFILE` is a NUL-terminated text, and `error` a valid place for the
        // pointer the call writes on failure.
        let applied = unsafe { sandbox_init(PROFILE.as_ptr(), 0, &raw mut error) };
        if error.is_null() {
            return (applied, None);
        }
        // SAFETY: a non-null `error` points at a NUL-terminated text the library allocated,
        // read once here and freed once just after.
        let text = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: `error` came from `sandbox_init` and is freed exactly once.
        unsafe { sandbox_free_error(error) };
        (applied, Some(text))
    };
    let ((applied, text), written) = with_stderr_aside(apply, restore)?;
    if applied == 0 {
        return Ok(Confinement::Own);
    }
    let text = text.unwrap_or_else(|| {
        let written = String::from_utf8_lossy(&written).trim().to_owned();
        if written.is_empty() {
            String::from("no reason given")
        } else {
            written
        }
    });
    // SAFETY: `getpid` cannot fail.
    let pid = unsafe { libc::getpid() };
    // SAFETY: a null operation asks whether the process is sandboxed; no variadic argument is
    // read with `FILTER_NONE`.
    let sandboxed = unsafe { sandbox_check(pid, std::ptr::null(), FILTER_NONE) };
    if sandboxed != 1 {
        return Err(SandboxError::Init(text));
    }
    let denied = judge(&survey()?)?;
    // No descriptor can be opened from here on, a socket least of all: a filter of the outer
    // profile that the probes did not ask about (a port not in [`PORTS`], a socket elsewhere)
    // is then never reached. The front end opens nothing (rule 7).
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `none` is a fully initialized `rlimit` that the call only reads.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const none) } != 0 {
        return Err(SandboxError::Descriptors(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    Ok(Confinement::Outer { denied })
}

/// The denied probes of `answers`, in order, if every required one is denied.
///
/// # Errors
///
/// The first probe that is neither denied nor allowed ([`SandboxError::Check`]), or the first
/// required probe that is allowed ([`SandboxError::TooPermissive`]).
pub fn judge(answers: &[Answer]) -> Result<Vec<String>, SandboxError> {
    let mut denied = Vec::new();
    for answer in answers {
        match answer.answer {
            1 => denied.push(answer.name.clone()),
            0 if answer.required => return Err(SandboxError::TooPermissive(answer.name.clone())),
            0 => {}
            _ => return Err(SandboxError::Check(answer.name.clone())),
        }
    }
    Ok(denied)
}

/// Asks the sandbox of this process about every probe, in the report's order: the network
/// (required), the writes (required), then reads, exec and fork, which are recorded.
///
/// # Errors
///
/// A place that cannot be resolved, or what the network probes need (the audit token, the
/// lookup of `sandbox_check_network`, the probe sockets) that cannot be had.
pub fn survey() -> Result<Vec<Answer>, SandboxError> {
    // SAFETY: `getpid` cannot fail.
    let pid = unsafe { libc::getpid() };
    let places = places()?;
    let network = Network::new(pid)?;
    let mut answers = Vec::new();
    let mut add = |name: String, required: bool, answer: c_int| {
        answers.push(Answer {
            name,
            required,
            answer,
        });
    };
    for operation in NETWORK {
        let op = operation.to_string_lossy();
        add(op.to_string(), true, ask(pid, operation, None));
        for (protocol, family, socket) in network.sockets() {
            let host = if family == libc::AF_INET {
                "127.0.0.1"
            } else {
                "::1"
            };
            let answer = first_not_denied(
                PORTS
                    .iter()
                    .map(|port| network.ask(operation, socket, family, *port)),
            );
            add(format!("{op} {protocol} {host}"), true, answer);
        }
        let paths = places.iter().map(|(_, path)| path.as_c_str());
        let answer = first_not_denied(
            paths
                .chain([SOCKETS])
                .map(|path| ask(pid, operation, Some(path))),
        );
        add(format!("{op} unix-socket"), true, answer);
    }
    for operation in WRITES {
        let op = operation.to_string_lossy();
        for (label, path) in &places {
            add(
                format!("{op} {label}"),
                true,
                ask(pid, operation, Some(path)),
            );
        }
    }
    for (label, path) in places
        .iter()
        .filter(|(label, _)| *label == "/" || *label == HOME)
    {
        add(
            format!("file-read-data {label}"),
            false,
            ask(pid, c"file-read-data", Some(path)),
        );
    }
    // `sandbox_check` refuses the plain name `process-exec` with `EINVAL`.
    add(
        "process-exec*".to_owned(),
        false,
        ask(pid, c"process-exec*", Some(c"/bin/sh")),
    );
    add(
        "process-fork".to_owned(),
        false,
        ask(pid, c"process-fork", None),
    );
    Ok(answers)
}

/// The first answer of `answers` that is not a denial, or 1 when all are.
fn first_not_denied(mut answers: impl Iterator<Item = c_int>) -> c_int {
    answers.find(|answer| *answer != 1).unwrap_or(1)
}

/// `sandbox_check` of `operation` for this process, with a path filter when `path` is given;
/// an operation the library does not know answers -1.
pub fn ask(pid: libc::pid_t, operation: &CStr, path: Option<&CStr>) -> c_int {
    match path {
        // SAFETY: the operation is a NUL-terminated text; with `FILTER_NONE` no variadic
        // argument is read.
        None => unsafe { sandbox_check(pid, operation.as_ptr(), FILTER_NONE | NO_REPORT) },
        // SAFETY: as above, and with `FILTER_PATH` the one variadic argument is the
        // NUL-terminated path, which outlives the call.
        Some(path) => unsafe {
            sandbox_check(
                pid,
                operation.as_ptr(),
                FILTER_PATH | NO_REPORT,
                path.as_ptr(),
            )
        },
    }
}

/// The label of the home directory among the places.
const HOME: &str = "(home)";

/// The places of the write probes, each with its label and its resolved path: the fixed ones,
/// the home directory of the user's account (its passwd entry, not `HOME`) and the user's
/// temporary directory (`confstr`, not `TMPDIR`). `sandbox_check` answers "denied" for a path
/// that does not exist, whatever the profile, so each must resolve.
fn places() -> Result<Vec<(&'static str, CString)>, SandboxError> {
    let mut places = Vec::new();
    for place in FIXED_PLACES {
        places.push((place, resolve(place, PathBuf::from(place))?));
    }
    places.push((HOME, resolve("the home directory", home()?)?));
    let temporary = "the temporary directory";
    places.push(("(temporary directory)", resolve(temporary, user_temp()?)?));
    Ok(places)
}

/// `path` resolved, as a C string.
fn resolve(place: &'static str, path: PathBuf) -> Result<CString, SandboxError> {
    let real =
        std::fs::canonicalize(path).map_err(|e| SandboxError::Place(place, e.to_string()))?;
    CString::new(real.into_os_string().into_vec())
        .map_err(|_| SandboxError::Place(place, "the path holds a NUL byte".to_owned()))
}

/// The home directory of the user's account, from its passwd entry.
fn home() -> Result<PathBuf, SandboxError> {
    let missing = |why: &str| SandboxError::Place("the home directory", why.to_owned());
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buffer = vec![0_u8; 16 * 1024];
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: `entry` and `buffer` are valid for writes of their sizes, `found` for a pointer;
    // the entry's strings point into `buffer`, which outlives their use below.
    let status = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            entry.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &raw mut found,
        )
    };
    if status != 0 || found.is_null() {
        return Err(missing("the account has no passwd entry"));
    }
    // SAFETY: on success `found` points at `entry`, which the call filled.
    let dir = unsafe { (*found).pw_dir };
    if dir.is_null() {
        return Err(missing("the passwd entry has no home directory"));
    }
    // SAFETY: `pw_dir` is a NUL-terminated text inside `buffer`.
    let bytes = unsafe { CStr::from_ptr(dir) }.to_bytes().to_vec();
    Ok(PathBuf::from(OsString::from_vec(bytes)))
}

/// The user's temporary directory, from `confstr`.
fn user_temp() -> Result<PathBuf, SandboxError> {
    let mut buffer = vec![0_u8; 1025];
    // SAFETY: `buffer` is valid for writes of its length, which the call is given.
    let length = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    let text = (length > 0 && length <= buffer.len())
        .then(|| CStr::from_bytes_until_nul(&buffer).ok())
        .flatten()
        .ok_or_else(|| {
            SandboxError::Place("the temporary directory", "confstr failed".to_owned())
        })?;
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(text.to_bytes())))
}

/// What the network probes need: this process's audit token, `sandbox_check_network`, and one
/// socket of each kind, closed when dropped.
struct Network {
    token: [u32; 8],
    check: CheckNetwork,
    sockets: [(&'static str, c_int, OwnedFd); 4],
}

impl Network {
    fn new(pid: libc::pid_t) -> Result<Network, SandboxError> {
        let unable = || SandboxError::Check("network-outbound tcp 127.0.0.1".to_owned());
        let mut token = [0_u32; 8];
        let mut count: libc::mach_msg_type_number_t = 8;
        // SAFETY: `token` is valid for the eight words the flavor writes, and `count` says so.
        let status = unsafe {
            libc::task_info(
                mach_task_self_,
                TASK_AUDIT_TOKEN,
                token.as_mut_ptr().cast(),
                &raw mut count,
            )
        };
        // The token's sixth word is the process id: a check against another token would
        // answer for another process.
        if status != libc::KERN_SUCCESS
            || count != 8
            || token.get(5) != u32::try_from(pid).ok().as_ref()
        {
            return Err(unable());
        }
        // SAFETY: `dlsym` with the default handle and a NUL-terminated name only looks up.
        let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"sandbox_check_network".as_ptr()) };
        if symbol.is_null() {
            return Err(unable());
        }
        // SAFETY: the symbol is the library's `sandbox_check_network`, of the type
        // `CheckNetwork` (read from the library on macOS 26: an audit token, attributes, an
        // operation, a socket and an address); a different signature would answer -1 or
        // nonsense, which the probes treat as an error or as allowed, failing closed.
        let check = unsafe { std::mem::transmute::<*mut c_void, CheckNetwork>(symbol) };
        let open = |family: c_int, kind: c_int| {
            // SAFETY: `socket` with integer arguments; a valid result is owned below.
            let fd = unsafe { libc::socket(family, kind, 0) };
            if fd < 0 {
                return Err(unable());
            }
            // SAFETY: `fd` is a fresh descriptor that nothing else owns.
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        };
        let sockets = [
            (
                "tcp",
                libc::AF_INET,
                open(libc::AF_INET, libc::SOCK_STREAM)?,
            ),
            (
                "tcp",
                libc::AF_INET6,
                open(libc::AF_INET6, libc::SOCK_STREAM)?,
            ),
            ("udp", libc::AF_INET, open(libc::AF_INET, libc::SOCK_DGRAM)?),
            (
                "udp",
                libc::AF_INET6,
                open(libc::AF_INET6, libc::SOCK_DGRAM)?,
            ),
        ];
        Ok(Network {
            token,
            check,
            sockets,
        })
    }

    /// The protocol, family and descriptor of each probe socket.
    fn sockets(&self) -> impl Iterator<Item = (&'static str, c_int, c_int)> + '_ {
        self.sockets
            .iter()
            .map(|(protocol, family, fd)| (*protocol, *family, fd.as_raw_fd()))
    }

    /// The answer for `operation` on `socket` toward the loopback address of `family` at
    /// `port`.
    fn ask(&self, operation: &CStr, socket: c_int, family: c_int, port: u16) -> c_int {
        let family_byte = u8::try_from(family).unwrap_or(0);
        let port = port.to_be();
        let v4 = libc::sockaddr_in {
            sin_len: u8::try_from(size_of::<libc::sockaddr_in>()).unwrap_or(0),
            sin_family: family_byte,
            sin_port: port,
            sin_addr: libc::in_addr {
                s_addr: u32::from_ne_bytes([127, 0, 0, 1]),
            },
            sin_zero: [0; 8],
        };
        let mut loopback = [0_u8; 16];
        if let Some(last) = loopback.last_mut() {
            *last = 1;
        }
        let v6 = libc::sockaddr_in6 {
            sin6_len: u8::try_from(size_of::<libc::sockaddr_in6>()).unwrap_or(0),
            sin6_family: family_byte,
            sin6_port: port,
            sin6_flowinfo: 0,
            sin6_addr: libc::in6_addr { s6_addr: loopback },
            sin6_scope_id: 0,
        };
        let address: *const libc::sockaddr = if family == libc::AF_INET {
            (&raw const v4).cast()
        } else {
            (&raw const v6).cast()
        };
        // SAFETY: the token, the operation and the address are valid for the call, which only
        // reads them; the socket is open; null attributes ask without a report.
        unsafe {
            (self.check)(
                &raw const self.token,
                std::ptr::null(),
                operation.as_ptr(),
                socket,
                address,
            )
        }
    }
}

/// The most bytes kept of what `f` writes on stderr.
const STDERR_ASIDE_BYTES: usize = 4096;

/// `dup2`, by which [`with_stderr_aside`] puts stderr back (a parameter, so that a test can
/// make it fail).
fn restore(from: c_int, to: c_int) -> c_int {
    // SAFETY: `dup2` between two descriptors; it touches no memory.
    unsafe { libc::dup2(from, to) }
}

/// Runs `f` with descriptor 2 on a pipe of its own, then puts the process's stderr back with
/// `put_back` and gives `f`'s result with what it wrote there, at most
/// [`STDERR_ASIDE_BYTES`]. The pipe's write end does not block, so a writer past its capacity
/// loses the rest instead of hanging the child; no descriptor above 2 is left open, whatever
/// fails, except the duplicate of stderr that [`SandboxError::Restore`] keeps.
///
/// # Errors
///
/// [`SandboxError::Stderr`] for a call that fails before `f` runs, and
/// [`SandboxError::Restore`], with the process's stderr, when it cannot be put back. One that
/// fails after `f` ran leaves the result unused: the caller fails closed.
fn with_stderr_aside<T>(
    f: impl FnOnce() -> T,
    put_back: fn(c_int, c_int) -> c_int,
) -> Result<(T, Vec<u8>), SandboxError> {
    use std::io::{Error, Read};

    let aside = |e: Error| SandboxError::Stderr(e.to_string());
    let (mut reader, writer) = std::io::pipe().map_err(aside)?;
    for fd in [reader.as_raw_fd(), writer.as_raw_fd()] {
        // SAFETY: `fcntl` on a descriptor this function owns, with integer arguments.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        // SAFETY: as above.
        if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
        {
            return Err(aside(Error::last_os_error()));
        }
    }
    // SAFETY: `dup` of the standard error descriptor; the new one is owned below.
    let saved = unsafe { libc::dup(libc::STDERR_FILENO) };
    if saved == -1 {
        return Err(aside(Error::last_os_error()));
    }
    // SAFETY: `saved` is a fresh descriptor that nothing else owns.
    let saved = unsafe { OwnedFd::from_raw_fd(saved) };
    // SAFETY: `dup2` between two open descriptors; it touches no memory.
    if unsafe { libc::dup2(writer.as_raw_fd(), libc::STDERR_FILENO) } == -1 {
        return Err(aside(Error::last_os_error()));
    }
    drop(writer);
    let result = f();
    if put_back(saved.as_raw_fd(), libc::STDERR_FILENO) == -1 {
        return Err(SandboxError::Restore {
            error: Error::last_os_error().to_string(),
            stderr: saved,
        });
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
            Err(e) => return Err(aside(e)),
        }
    }
    Ok((result, written))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(name: &str, required: bool, answer: c_int) -> Answer {
        Answer {
            name: name.to_owned(),
            required,
            answer,
        }
    }

    #[test]
    fn only_a_denial_counts_as_denied() {
        // #1364 M2: an answer of -1, or any other than 0 and 1, is an error, never a denial,
        // for a required probe and a recorded one alike.
        for required in [true, false] {
            for bad in [-1, 2, c_int::MIN] {
                let found = judge(&[answer("a", true, 1), answer("b", required, bad)]);
                assert!(matches!(&found, Err(SandboxError::Check(name)) if name == "b"));
            }
        }
        assert!(matches!(
            judge(&[answer("a", true, 1), answer("b", true, 0)]),
            Err(SandboxError::TooPermissive(name)) if name == "b"
        ));
        assert_eq!(
            judge(&[
                answer("a", true, 1),
                answer("b", false, 0),
                answer("c", false, 1)
            ])
            .ok(),
            Some(vec!["a".to_owned(), "c".to_owned()])
        );
    }

    #[test]
    fn a_failed_restore_keeps_the_stderr_for_the_reason() {
        // #1364 L2: the reason of a restore that fails can still be written on the process's
        // own stderr, which the error keeps; this test then puts descriptor 2 back itself.
        let original = std::fs::metadata("/dev/fd/2").ok().map(|m| {
            use std::os::unix::fs::MetadataExt;
            (m.dev(), m.ino())
        });
        let error = with_stderr_aside(|| 7, |_, _| -1).err();
        assert!(error.is_some(), "a failed restore must be an error");
        let Some(error) = error else { return };
        let kept = error.original_stderr().map(|fd| fd.as_raw_fd());
        let restored = kept.map(|fd| restore(fd, libc::STDERR_FILENO));
        assert_eq!(restored, Some(libc::STDERR_FILENO));
        let now = std::fs::metadata("/dev/fd/2").ok().map(|m| {
            use std::os::unix::fs::MetadataExt;
            (m.dev(), m.ino())
        });
        assert_eq!(now, original);
        assert!(error.to_string().starts_with("cannot put stderr back"));
    }
}
