//! The solver child's resource limits (ADR 0124 rule 6): processor time as a backstop of the
//! parent's wall-clock guard, the address space as a backstop of Z3's own memory limit on
//! Linux, no core dump, no file written, at most 16 open descriptors. Each is set with
//! `setrlimit`, hard and soft alike, so the child cannot raise it again (the seccomp filter
//! also refuses the write through `prlimit64`, and the child never runs as root, where the
//! hard limits would not bind). The standard library has no interface to the process's limits.
#![allow(unsafe_code)]

use std::io;

/// The limits of one solver child, from the parent's flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The processor-time limit in seconds: the wall-clock guard plus one (`RLIMIT_CPU`); a
    /// child that reaches it ends by `SIGXCPU`, which the parent reports as `cpu`.
    pub cpu_seconds: u64,
    /// The memory limit in MiB (`--solver-memory`), which Z3's own `memory_max_size` holds;
    /// on Linux the address space is limited to it plus 1 GiB as a backstop.
    pub memory_mib: u64,
    /// Whether to set the address-space backstop at all: false in a build instrumented by
    /// AddressSanitizer, whose shadow memory it would break. macOS enforces no address-space
    /// limit, and the parent reads the footprint instead ([`crate::solver::footprint`]).
    pub address_space_backstop: bool,
}

/// The margin above the memory limit that the address space may use: 1 GiB, in MiB.
const ADDRESS_SPACE_MARGIN_MIB: u64 = 1024;

/// The most descriptors the child may have open: its three pipes and what Z3 opens, which is
/// nothing (rule 7 forbids it).
const OPEN_FILES: u64 = 16;

/// The resource type of `setrlimit`, which the C libraries declare differently.
#[cfg(target_os = "linux")]
type Resource = libc::__rlimit_resource_t;
#[cfg(target_os = "macos")]
type Resource = libc::c_int;

/// Sets every limit of rule 6.
///
/// # Errors
///
/// The name of the first resource whose limit could not be set, with the error: a hard limit
/// already below the value, or a system that refuses it. The caller fails closed.
pub fn apply(limits: &Limits) -> Result<(), (&'static str, io::Error)> {
    // The soft limit sends `SIGXCPU` and the hard one, a second later, `SIGKILL`: with the two
    // equal, Linux sends only the kill, and the parent could not tell `cpu` from a crash.
    set(
        libc::RLIMIT_CPU,
        limits.cpu_seconds,
        limits.cpu_seconds.saturating_add(1),
        "processor time",
    )?;
    set(libc::RLIMIT_CORE, 0, 0, "core dump")?;
    set(libc::RLIMIT_FSIZE, 0, 0, "file size")?;
    set(libc::RLIMIT_NOFILE, OPEN_FILES, OPEN_FILES, "open files")?;
    #[cfg(target_os = "linux")]
    if limits.address_space_backstop {
        let bytes = limits
            .memory_mib
            .saturating_add(ADDRESS_SPACE_MARGIN_MIB)
            .saturating_mul(1024 * 1024);
        set(libc::RLIMIT_AS, bytes, bytes, "address space")?;
    }
    Ok(())
}

/// Sets `resource`'s soft and hard limits.
fn set(
    resource: Resource,
    soft: u64,
    hard: u64,
    name: &'static str,
) -> Result<(), (&'static str, io::Error)> {
    let limit = libc::rlimit {
        rlim_cur: libc::rlim_t::try_from(soft).unwrap_or(libc::rlim_t::MAX),
        rlim_max: libc::rlim_t::try_from(hard).unwrap_or(libc::rlim_t::MAX),
    };
    // SAFETY: `limit` is a fully initialized `rlimit` that the call only reads.
    if unsafe { libc::setrlimit(resource, &raw const limit) } == 0 {
        Ok(())
    } else {
        Err((name, io::Error::last_os_error()))
    }
}

/// The current soft and hard limits of `resource`, for the tests.
///
/// # Errors
///
/// The error of `getrlimit`.
pub fn current(resource: Resource) -> io::Result<(u64, u64)> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid `rlimit` that the call writes.
    if unsafe { libc::getrlimit(resource, &raw mut limit) } == 0 {
        Ok((u64::from(limit.rlim_cur), u64::from(limit.rlim_max)))
    } else {
        Err(io::Error::last_os_error())
    }
}
