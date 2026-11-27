//! The solver child's confinement, limits, spawn attributes and footprint reading (ADR 0124
//! rules 5 to 7 and 35), behind the feature `solver`, which only `una-cli` and `una-z3` enable.
//! `una __solver` calls [`confine`] before it reads the query, in this order: the process
//! blocks `SIGXCPU` ([`processor_time`]); it refuses to go on as root, whose capabilities no
//! filter removes; the working directory
//! becomes `/`; every descriptor above the three standard ones is closed ([`descriptors`]);
//! the limits of rule 6 are set ([`limits`]); on Linux the personality bit that would make
//! every mapping executable is cleared and what the C library reads from files for the
//! processor count is read once; the process confines itself (the seccomp filter of
//! [`seccomp`] on Linux, the sandbox profile of [`sandbox`] on macOS); the thread that ends the
//! process by `SIGXCPU` starts; and only then is the
//! confinement recorded, which `una-z3` requires before it calls Z3's front end. A step that
//! fails ends the attempt: there is no unconfined fallback. The parent uses [`spawn`] to start
//! the child with an empty environment and only its three pipes, and on macOS [`footprint`]
//! to read the child's physical footprint.
//!
//! The bindings are through the `libc` crate's declarations, and four of macOS's that it
//! lacks (`sandbox_init`, `sandbox_free_error`, `sandbox_check`, `mach_task_self_`), each bound
//! once here; `sandbox_check_network` is looked up at run time ([`sandbox`]).
#![allow(unsafe_code)]

pub mod descriptors;
#[cfg(target_os = "macos")]
pub mod footprint;
pub mod limits;
pub mod processor_time;
#[cfg(target_os = "macos")]
pub mod sandbox;
#[cfg(target_os = "linux")]
pub mod seccomp;
pub mod spawn;

use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

pub use limits::Limits;

/// Set once this process has confined itself; never cleared. One flag for the whole process,
/// since the confinement is the process's.
static CONFINED: AtomicBool = AtomicBool::new(false);

/// Records that this process confined itself: every step of rule 7 succeeded. [`confine`]
/// calls it as its last step, and nothing else can.
fn record_confined() {
    CONFINED.store(true, Ordering::Release);
}

/// Whether this process recorded a successful confinement.
pub fn confined() -> bool {
    CONFINED.load(Ordering::Acquire)
}

/// How the child is confined: by its own profile or filter, or, on macOS, by an outer sandbox
/// that already confines `una` and denies at least what rule 7 requires (Question 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confinement {
    /// The child confined itself.
    Own,
    /// An outer sandbox confines it; `denied` lists, in rule 7's order, the operations of the
    /// fixed list that the outer profile denies.
    Outer {
        /// The denied probes, by their names in the report.
        denied: Vec<String>,
    },
}

/// Why the child could not confine itself. Each is U5003 in the parent, "the solver could not
/// be confined", with this text.
#[derive(Debug)]
pub enum ConfineError {
    /// The process runs as root: no filter or sandbox removes a root process's capabilities,
    /// which would lift its limits and widen what it reads.
    Root,
    /// The working directory could not be changed to `/`.
    WorkingDirectory(io::Error),
    /// A descriptor above the standard three could not be closed.
    Descriptors(io::Error),
    /// A limit of rule 6 could not be set; the name is the resource's.
    Limit(&'static str, io::Error),
    /// `SIGXCPU` could not be blocked, or its waiting thread not started
    /// ([`processor_time`]).
    ProcessorTime(io::Error),
    /// The personality could not be read or cleared (Linux).
    #[cfg(target_os = "linux")]
    Personality(io::Error),
    /// The seccomp filter could not be installed.
    #[cfg(target_os = "linux")]
    Seccomp(seccomp::InstallError),
    /// The sandbox could not be applied, or the outer one is too permissive.
    #[cfg(target_os = "macos")]
    Sandbox(sandbox::SandboxError),
}

impl fmt::Display for ConfineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root => f.write_str("the solver must not run as root"),
            Self::WorkingDirectory(e) => write!(f, "cannot make `/` the working directory: {e}"),
            Self::Descriptors(e) => write!(f, "cannot close the inherited descriptors: {e}"),
            Self::Limit(name, e) => write!(f, "cannot set the {name} limit: {e}"),
            Self::ProcessorTime(e) => {
                write!(f, "cannot take over the processor-time signal: {e}")
            }
            #[cfg(target_os = "linux")]
            Self::Personality(e) => write!(f, "cannot clear the execution personality: {e}"),
            #[cfg(target_os = "linux")]
            Self::Seccomp(e) => write!(f, "{e}"),
            #[cfg(target_os = "macos")]
            Self::Sandbox(e) => write!(f, "{e}"),
        }
    }
}

impl ConfineError {
    /// The process's own stderr when the confinement left descriptor 2 elsewhere (macOS: it
    /// could not be put back after the sandbox), so that the reason can still be written there.
    pub fn original_stderr(&self) -> Option<std::os::fd::BorrowedFd<'_>> {
        match self {
            #[cfg(target_os = "macos")]
            Self::Sandbox(e) => e.original_stderr(),
            _ => None,
        }
    }
}

impl std::error::Error for ConfineError {}

/// Confines this process as the solver child (rule 7), in the order the module describes, and
/// records it.
///
/// # Errors
///
/// The first step that fails, with no step taken back: a process that reaches an error here
/// writes no handshake and exits.
pub fn confine(limits: &Limits) -> Result<Confinement, ConfineError> {
    // First, while the process has one thread and before the processor-time limit exists:
    // no handler of `SIGXCPU` may ever run in the child (#2268).
    processor_time::hold().map_err(ConfineError::ProcessorTime)?;
    // SAFETY: `geteuid` has no arguments and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        return Err(ConfineError::Root);
    }
    std::env::set_current_dir("/").map_err(ConfineError::WorkingDirectory)?;
    descriptors::close_above_stdio().map_err(ConfineError::Descriptors)?;
    limits::apply(limits).map_err(|(name, e)| ConfineError::Limit(name, e))?;
    let confinement = platform_confine()?;
    // The thread that ends the child by `SIGXCPU`, started under the confinement.
    processor_time::watch().map_err(ConfineError::ProcessorTime)?;
    record_confined();
    Ok(confinement)
}

#[cfg(target_os = "linux")]
fn platform_confine() -> Result<Confinement, ConfineError> {
    // An inherited `READ_IMPLIES_EXEC` personality would make every readable mapping
    // executable behind the filter's `PROT_EXEC` check: it is cleared, not tolerated.
    // SAFETY: `personality` with the query value reads the current personality and changes
    // nothing; with another value it sets it; neither touches memory.
    let current = unsafe { libc::personality(0xffff_ffff) };
    if current == -1 {
        return Err(ConfineError::Personality(io::Error::last_os_error()));
    }
    let exec_bit = libc::READ_IMPLIES_EXEC as libc::c_int;
    if current & exec_bit != 0 {
        let cleared = libc::c_ulong::from((current & !exec_bit).cast_unsigned());
        // SAFETY: as above: `personality` with a value other than the query value sets the
        // execution domain of the calling thread to it (the solver child is still
        // single-threaded here) and touches no memory.
        if unsafe { libc::personality(cleared) } == -1 {
            return Err(ConfineError::Personality(io::Error::last_os_error()));
        }
    }
    // What the C library would otherwise open a file for during the query (rule 7): the
    // number of processors, read from `/sys` once here. glibc 2.39 reads it again on each
    // count; under the filter that open, and its next try, `/proc/stat`, fail with `EACCES`,
    // and glibc falls back to `sched_getaffinity`, which the filter allows (probe P6, #1455).
    let _ = std::thread::available_parallelism();
    // SAFETY: `sysconf` reads a value of the system and touches no memory of the process.
    let _ = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    seccomp::install(seccomp::Mode::Enforce).map_err(ConfineError::Seccomp)?;
    Ok(Confinement::Own)
}

#[cfg(target_os = "macos")]
fn platform_confine() -> Result<Confinement, ConfineError> {
    sandbox::confine().map_err(ConfineError::Sandbox)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_is_unset_until_recorded_and_then_stays() {
        assert!(!confined());
        record_confined();
        assert!(confined());
        record_confined();
        assert!(confined());
    }
}
