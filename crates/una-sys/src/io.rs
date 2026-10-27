//! A zero-timeout readiness check of an output descriptor (ADR 0117 rule 3, amendment of
//! 2026-10-19), and the read of the limit of open descriptors that caps the reactor's sockets.
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

/// The soft limit of open descriptors (`RLIMIT_NOFILE`), or `None` when it cannot be read. The
/// one read of it until the binding of #1005 (`server.rs`) serves the reactor too.
pub fn descriptor_limit() -> Option<u64> {
    // SAFETY: zero is a valid `rlimit`, which `getrlimit` fills in place.
    let (read, limit) = unsafe {
        let mut limit: libc::rlimit = std::mem::zeroed();
        (libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit), limit)
    };
    (read == 0).then_some(limit.rlim_cur)
}
