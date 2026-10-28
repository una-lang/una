//! OS bindings for `serve:` grants before `main` (ADR 0123 rules 9 and 19), each a safe wrapper
//! of what the standard library cannot do. The checks of a TLS file's open descriptor use
//! `File::metadata` in `una-http`, which is safe.
#![allow(unsafe_code)]

use std::io;
use std::net::{SocketAddr, TcpListener};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// Flags added to a TLS file's open: no blocking on a FIFO, no controlling terminal.
/// Std names no `O_*` constant, and `OpenOptionsExt::custom_flags` takes raw flags.
pub const OPEN_FLAGS: i32 = libc::O_NONBLOCK | libc::O_NOCTTY;

/// The effective user and group, and the supplementary groups, of the process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The effective user.
    pub uid: u32,
    /// The effective group.
    pub gid: u32,
    /// The supplementary groups.
    pub groups: Vec<u32>,
}

/// The result of a call that returns -1 on failure.
fn ok(result: libc::c_int) -> io::Result<usize> {
    usize::try_from(result).map_err(|_| io::Error::last_os_error())
}

/// The process's [`Identity`], which the check of a key file's owner and group needs.
/// Std has no `geteuid`, `getegid` or `getgroups`. On macOS, the legacy `getgroups` bound here
/// reads at most 16 groups: a key readable only through a group past those is refused, which
/// fails closed; the extended symbol is not bound.
pub fn identity() -> io::Result<Identity> {
    // SAFETY: `geteuid` and `getegid` take no argument, touch no memory and cannot fail; with a
    // count of 0, `getgroups` writes nothing and returns the number of groups.
    let (uid, gid, count) = unsafe {
        let count = libc::getgroups(0, std::ptr::null_mut());
        (libc::geteuid(), libc::getegid(), count)
    };
    let mut groups = vec![0; ok(count)?];
    // SAFETY: `groups` holds `count` writable `gid_t`s, and `getgroups` writes at most `count`.
    let written = ok(unsafe { libc::getgroups(count, groups.as_mut_ptr()) })?;
    groups.truncate(written);
    Ok(Identity { uid, gid, groups })
}

/// macOS's `OPEN_MAX`, the largest soft limit of descriptors it accepts (`<sys/syslimits.h>`).
#[cfg(target_os = "macos")]
const OPEN_MAX: u64 = 10_240;

/// The limit of open descriptors, by the crate's one binding of `getrlimit` (`io.rs`).
fn descriptor_limit() -> io::Result<libc::rlimit> {
    crate::io::descriptor_limits()
}

/// Sets the soft limit of open descriptors to `soft`, keeping the hard limit `hard`.
fn set_descriptor_limit(soft: u64, hard: u64) -> io::Result<usize> {
    let limit = libc::rlimit {
        rlim_cur: soft,
        rlim_max: hard,
    };
    // SAFETY: `limit` is a valid `rlimit` that `setrlimit` only reads.
    ok(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit) })
}

/// Raises the soft limit of open descriptors to `wanted` within the hard limit, and returns
/// the soft limit in force afterwards, as read back, below `wanted` when the system refuses.
/// Std has no `getrlimit` or `setrlimit`.
pub fn raise_descriptor_limit(wanted: u64) -> io::Result<u64> {
    let limit = descriptor_limit()?;
    if limit.rlim_cur >= wanted {
        return Ok(limit.rlim_cur);
    }
    let soft = wanted.min(limit.rlim_max);
    // macOS may accept a soft limit above `OPEN_MAX` and still never open that many: the raise
    // stops there, unless the limit in force is already above it, so the caller's check of
    // what it needs fails closed.
    #[cfg(target_os = "macos")]
    let soft = soft.min(OPEN_MAX.max(limit.rlim_cur));
    let set = set_descriptor_limit(soft, limit.rlim_max);
    // macOS refuses a soft limit above `OPEN_MAX` with `EINVAL`, even under an infinite hard
    // limit: the reachable limit is tried, so the caller's message names it.
    #[cfg(target_os = "macos")]
    if set
        .as_ref()
        .is_err_and(|e| e.raw_os_error() == Some(libc::EINVAL))
        && soft > OPEN_MAX
        && OPEN_MAX > limit.rlim_cur
    {
        let _ = set_descriptor_limit(OPEN_MAX, limit.rlim_max);
    }
    let _ = set;
    Ok(descriptor_limit()?.rlim_cur)
}

/// A blocking, close-on-exec socket listening on `address`: `SO_REUSEADDR`, `IPV6_V6ONLY` on
/// IPv6, never `SO_REUSEPORT`. On macOS a local process can still bind a more specific address
/// on a port whose grant binds the wildcard (`spec/31 §2` rule 7).
/// Std is not enough: `TcpListener::bind` cannot set `IPV6_V6ONLY` before it binds, nor take
/// the caller's backlog, and one code path serves both families.
pub fn listen(address: SocketAddr, backlog: u32) -> io::Result<TcpListener> {
    let six = address.is_ipv6();
    let family = if six { libc::AF_INET6 } else { libc::AF_INET };
    #[cfg(target_os = "linux")]
    let kind = libc::SOCK_STREAM | libc::SOCK_CLOEXEC;
    #[cfg(target_os = "macos")]
    let kind = libc::SOCK_STREAM;
    // SAFETY: `socket` takes integers only and returns a new descriptor, or -1.
    let fd = unsafe { libc::socket(family, kind, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is a new open descriptor that nothing else owns; `socket` closes it on
    // every path from here.
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    let raw = socket.as_raw_fd();
    // macOS has no `SOCK_CLOEXEC`: close-on-exec is a second, non-atomic step, as in std's
    // macOS path, so a process started by another thread in between would inherit the socket.
    #[cfg(target_os = "macos")]
    // SAFETY: `raw` is open and owned by `socket`; `F_SETFD` sets its descriptor flags only.
    ok(unsafe { crate::signals::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) })?;
    let size = |bytes: usize| libc::socklen_t::try_from(bytes).unwrap_or(0);
    let one: libc::c_int = 1;
    let set = |level, option| {
        let at = (&raw const one).cast();
        // SAFETY: `raw` is open; `one` is a `c_int` alive across the call, of the size given.
        ok(unsafe { libc::setsockopt(raw, level, option, at, size(size_of::<libc::c_int>())) })
    };
    set(libc::SOL_SOCKET, libc::SO_REUSEADDR)?;
    if six {
        set(libc::IPPROTO_IPV6, libc::IPV6_V6ONLY)?;
    }
    // SAFETY: zero is a valid `sockaddr_storage`, large enough for both address kinds.
    let mut storage: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let length = match address {
        SocketAddr::V4(v4) => {
            // SAFETY: `sockaddr_storage` is larger and more aligned than `sockaddr_in`.
            let a = unsafe { &mut *(&raw mut storage).cast::<libc::sockaddr_in>() };
            a.sin_family = libc::sa_family_t::try_from(libc::AF_INET).unwrap_or(0);
            a.sin_port = v4.port().to_be();
            a.sin_addr.s_addr = u32::from_ne_bytes(v4.ip().octets());
            size_of::<libc::sockaddr_in>()
        }
        SocketAddr::V6(v6) => {
            // SAFETY: `sockaddr_storage` is larger and more aligned than `sockaddr_in6`.
            let a = unsafe { &mut *(&raw mut storage).cast::<libc::sockaddr_in6>() };
            a.sin6_family = libc::sa_family_t::try_from(libc::AF_INET6).unwrap_or(0);
            a.sin6_port = v6.port().to_be();
            a.sin6_addr.s6_addr = v6.ip().octets();
            size_of::<libc::sockaddr_in6>()
        }
    };
    #[cfg(target_os = "macos")]
    {
        storage.ss_len = u8::try_from(length).unwrap_or(0);
    }
    // SAFETY: `raw` is open; `storage` holds an initialized address of `length` bytes.
    ok(unsafe { libc::bind(raw, (&raw const storage).cast(), size(length)) })?;
    let backlog = libc::c_int::try_from(backlog).unwrap_or(libc::c_int::MAX);
    // Between `bind` and `listen` the port is held and refuses connections, as with std.
    // SAFETY: `raw` is an open, bound stream socket; `listen` takes integers only.
    ok(unsafe { libc::listen(raw, backlog) })?;
    Ok(TcpListener::from(socket))
}
