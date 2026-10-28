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

/// The soft and hard limits of open descriptors (`RLIMIT_NOFILE`): the crate's one binding of
/// `getrlimit`, which [`descriptor_limit`] and the `serve:` grants of `server.rs` read (ADR 0058,
/// amendment of 2026-10-25: each function of the C library is bound once). `rlim_t` is `u64` on
/// the 64-bit targets the runtime serves.
pub(crate) fn descriptor_limits() -> std::io::Result<libc::rlimit> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid `rlimit` that `getrlimit` fills in place.
    let read = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) };
    if read == 0 {
        Ok(limit)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// The soft limit of open descriptors (`RLIMIT_NOFILE`), or `None` when it cannot be read.
pub fn descriptor_limit() -> Option<u64> {
    descriptor_limits().ok().map(|limit| limit.rlim_cur)
}
