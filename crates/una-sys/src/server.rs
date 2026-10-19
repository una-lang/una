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
/// Std has no `geteuid`, `getegid` or `getgroups`.
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

/// Raises the soft limit of open descriptors to `wanted` within the hard limit, and returns
/// the soft limit in force, below `wanted` when the system refuses.
/// Std has no `getrlimit` or `setrlimit`.
pub fn raise_descriptor_limit(wanted: u64) -> io::Result<u64> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid `rlimit` that `getrlimit` fills.
    ok(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) })?;
    if limit.rlim_cur >= wanted {
        return Ok(limit.rlim_cur);
    }
    let raised = libc::rlimit {
        rlim_cur: wanted.min(limit.rlim_max),
        rlim_max: limit.rlim_max,
    };
    // SAFETY: `raised` is a valid `rlimit` that `setrlimit` only reads. macOS refuses a soft
    // limit above its own maximum; the limit then stays as it was.
    let set = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const raised) };
    Ok(if set == 0 { raised.rlim_cur } else { limit.rlim_cur })
}

/// A blocking, close-on-exec socket listening on `address`: `IPV6_V6ONLY` on IPv6,
/// `SO_REUSEADDR` on Linux only, never `SO_REUSEPORT` (`spec/31 §2` rule 7).
/// Std's `TcpListener::bind` sets `SO_REUSEADDR` everywhere and cannot set `IPV6_V6ONLY`.
pub fn listen(address: SocketAddr, backlog: u32) -> io::Result<TcpListener> {
    let six = address.is_ipv6();
    let family = if six { libc::AF_INET6 } else { libc::AF_INET };
    // SAFETY: `socket` takes integers only and returns a new descriptor, or -1.
    let fd = ok(unsafe { libc::socket(family, libc::SOCK_STREAM, 0) })?;
    let fd = libc::c_int::try_from(fd).unwrap_or(-1);
    // SAFETY: `fd` is a new open descriptor that nothing else owns; `socket` closes it on
    // every path from here.
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    let raw = socket.as_raw_fd();
    let size = |bytes: usize| libc::socklen_t::try_from(bytes).unwrap_or(0);
    let one: libc::c_int = 1;
    let set = |level, option| {
        let at = (&raw const one).cast();
        // SAFETY: `raw` is open; `one` is a `c_int` alive across the call, of the size given.
        ok(unsafe { libc::setsockopt(raw, level, option, at, size(size_of::<libc::c_int>())) })
    };
    // SAFETY: `raw` is open; `F_SETFD` sets its descriptor flags. The runtime starts no
    // process, so no exec comes between `socket` and this call.
    ok(unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) })?;
    // On macOS and the BSDs no `SO_REUSEADDR`: a restart waits out `TIME_WAIT`.
    #[cfg(target_os = "linux")]
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
    // SAFETY: `raw` is an open, bound stream socket; `listen` takes integers only.
    ok(unsafe { libc::listen(raw, backlog) })?;
    Ok(TcpListener::from(socket))
}
