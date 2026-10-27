//! The physical footprint of a child on macOS (ADR 0124 rule 6): macOS does not enforce
//! `RLIMIT_AS`, so the parent reads the child's footprint, which counts compressed memory,
//! every 10 ms, and kills the child past the limit plus 1 GiB. The standard library has no
//! interface to another process's memory use.
#![allow(unsafe_code)]

use std::io;

/// The physical footprint of process `pid`, in bytes (`ri_phys_footprint`).
///
/// # Errors
///
/// The error of `proc_pid_rusage`: a process that is gone, or not this user's.
pub fn physical_footprint(pid: libc::pid_t) -> io::Result<u64> {
    // SAFETY: zero is a valid, if empty, `rusage_info_v4`, which the call fills in whole.
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a valid `rusage_info_v4` for the flavor `RUSAGE_INFO_V4`, which is what
    // the call writes through the pointer.
    let read = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V4,
            (&raw mut info).cast::<libc::rusage_info_t>(),
        )
    };
    if read == 0 {
        Ok(info.ri_phys_footprint)
    } else {
        Err(io::Error::last_os_error())
    }
}
