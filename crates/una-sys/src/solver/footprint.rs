//! The physical footprint of a child on macOS (ADR 0124 rule 6): macOS does not enforce
//! `RLIMIT_AS`, so the parent reads the child's footprint, which counts compressed memory,
//! every 10 ms, and kills the child past the limit plus 1 GiB; and the machine's physical
//! memory, which bounds how many children run at once. The standard library has no interface
//! to another process's memory use, nor to the machine's memory.
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

/// The machine's physical memory in bytes (`hw.memsize`).
///
/// # Errors
///
/// The error of `sysctlbyname`.
pub fn physical_memory() -> io::Result<u64> {
    let mut bytes: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: the name is a NUL-terminated text, `bytes` a valid place of `len` bytes that the
    // call writes, and `len` a valid place it updates; no new value is set.
    let read = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&raw mut bytes).cast::<libc::c_void>(),
            &raw mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if read == 0 {
        Ok(bytes)
    } else {
        Err(io::Error::last_os_error())
    }
}
