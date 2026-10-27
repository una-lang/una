//! Closing every descriptor above the three standard ones in the solver child (ADR 0124 rule
//! 5): on Linux by `close_range`, or, on a kernel without it, by closing the descriptors that
//! `/proc/self/fd` lists; on macOS, where the parent spawned the child with
//! `POSIX_SPAWN_CLOEXEC_DEFAULT` ([`crate::solver::spawn`]), by the listing of `/dev/fd`, as a
//! check that costs nothing. Never a loop up to the descriptor limit. The standard library
//! cannot close descriptors it does not own, which is why this is here.
#![allow(unsafe_code)]

use std::io;

/// Closes every descriptor above 2.
///
/// # Errors
///
/// The error of `close_range`, or of reading the listing, when neither way works; a single
/// `close` that fails with `EBADF` (the listing's own descriptor, closed when the listing
/// ended) is not an error.
pub fn close_above_stdio() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `close_range` takes three integers and touches no memory of the process; a
        // kernel without it (before 5.9) answers `ENOSYS`, and the listing below serves.
        let closed = unsafe {
            libc::syscall(
                libc::SYS_close_range,
                3 as libc::c_uint,
                libc::c_uint::MAX,
                0 as libc::c_uint,
            )
        };
        if closed == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ENOSYS) {
            return Err(error);
        }
        close_listed("/proc/self/fd")
    }
    #[cfg(target_os = "macos")]
    {
        close_listed("/dev/fd")
    }
}

/// Closes the descriptors above 2 that the directory `listing` names. The listing is read in
/// full and dropped before the first close, so the descriptor the reading itself holds is
/// closed by its drop, and its number is then an `EBADF`, which is ignored.
fn close_listed(listing: &str) -> io::Result<()> {
    let numbers: Vec<libc::c_int> = std::fs::read_dir(listing)?
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .filter(|number| *number > 2)
        .collect();
    for number in numbers {
        // SAFETY: `close` on a descriptor number this process listed as its own; a number no
        // longer open, the listing's, gives `EBADF` and changes nothing else.
        unsafe { libc::close(number) };
    }
    Ok(())
}
