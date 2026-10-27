//! The solver child's confinement, limits, spawn attributes and footprint reading (ADR 0124
//! rules 5 to 7 and 35), behind the feature `solver`, which only `una-cli` and `una-z3` enable.
//! `una __solver` calls [`confine`] before it reads the query, in this order: the working
//! directory becomes `/`, every descriptor above the three standard ones is closed
//! ([`descriptors`]), the limits of rule 6 are set ([`limits`]), the process confines itself
//! (the seccomp filter of [`seccomp`] on Linux, the sandbox profile of [`sandbox`] on macOS),
//! and only then is the confinement recorded ([`record_confined`]), which `una-z3` requires
//! before it calls Z3's front end. A step that fails ends the attempt: there is no unconfined
//! fallback. The parent uses [`spawn`] to start the child with an empty environment and only
//! its three pipes, and on macOS [`footprint`] to read the child's physical footprint.
//!
//! The bindings are through the `libc` crate's declarations, and three of macOS's that it
//! lacks (`sandbox_init`, `sandbox_free_error`, `sandbox_check`), each bound once here.

pub mod descriptors;
#[cfg(target_os = "macos")]
pub mod footprint;
pub mod limits;
#[cfg(target_os = "macos")]
pub mod sandbox;
#[cfg(target_os = "linux")]
pub mod seccomp;
pub mod spawn;

use std::ffi::CStr;
use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

pub use limits::Limits;

/// Set once this process has confined itself; never cleared.
static CONFINED: AtomicBool = AtomicBool::new(false);

/// Records that this process confined itself: every step of rule 7 succeeded. [`confine`]
/// calls it as its last step; nothing else should.
pub fn record_confined() {
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
        /// The denied operations, by their names in the report.
        denied: Vec<&'static str>,
    },
}

/// Why the child could not confine itself. Each is U5003 in the parent, "the solver could not
/// be confined", with this text.
#[derive(Debug)]
pub enum ConfineError {
    /// The working directory could not be changed to `/`.
    WorkingDirectory(io::Error),
    /// A descriptor above the standard three could not be closed.
    Descriptors(io::Error),
    /// A limit of rule 6 could not be set; the name is the resource's.
    Limit(&'static str, io::Error),
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
            Self::WorkingDirectory(e) => write!(f, "cannot make `/` the working directory: {e}"),
            Self::Descriptors(e) => write!(f, "cannot close the inherited descriptors: {e}"),
            Self::Limit(name, e) => write!(f, "cannot set the {name} limit: {e}"),
            #[cfg(target_os = "linux")]
            Self::Seccomp(e) => write!(f, "{e}"),
            #[cfg(target_os = "macos")]
            Self::Sandbox(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ConfineError {}

/// Confines this process as the solver child (rule 7), in the order the module describes, and
/// records it. `outside_temp` is a path outside the temporary directory, passed by the parent,
/// at which the outer-sandbox check of macOS tests writes and reads; Linux ignores it.
///
/// # Errors
///
/// The first step that fails, with no step taken back: a process that reaches an error here
/// writes no handshake and exits.
pub fn confine(limits: &Limits, outside_temp: &CStr) -> Result<Confinement, ConfineError> {
    std::env::set_current_dir("/").map_err(ConfineError::WorkingDirectory)?;
    descriptors::close_above_stdio().map_err(ConfineError::Descriptors)?;
    limits::apply(limits).map_err(|(name, e)| ConfineError::Limit(name, e))?;
    let confinement = platform_confine(outside_temp)?;
    record_confined();
    Ok(confinement)
}

#[cfg(target_os = "linux")]
fn platform_confine(_: &CStr) -> Result<Confinement, ConfineError> {
    seccomp::install(seccomp::Mode::Enforce).map_err(ConfineError::Seccomp)?;
    Ok(Confinement::Own)
}

#[cfg(target_os = "macos")]
fn platform_confine(outside_temp: &CStr) -> Result<Confinement, ConfineError> {
    sandbox::confine(outside_temp).map_err(ConfineError::Sandbox)
}
