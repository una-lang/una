//! A zero-timeout readiness check of an output descriptor (ADR 0117 rule 3, amendment of
//! 2026-10-19): a worker writes a line itself only when the write will not wait.
#![allow(unsafe_code)]

/// Whether `fd` can take a write without waiting, by `poll` with a zero timeout; `false` on an
/// error, so that the caller hands the write to a helper.
pub fn writable(fd: i32) -> bool {
    let mut entry = libc::pollfd {
        fd,
        events: libc::POLLOUT,
        revents: 0,
    };
    // SAFETY: `entry` is one valid, writable `pollfd`; a timeout of 0 returns at once.
    let ready = unsafe { libc::poll(&raw mut entry, 1, 0) };
    ready == 1 && entry.revents & libc::POLLOUT != 0
}
