//! The system calls that the host makes for `serve:` grants before `main` (ADR 0123 rules 7, 9
//! and 19, `spec/31 §2` rules 7 and 8): the run's effective user and groups, which the check of
//! a TLS key file compares with the file's owner and group; the limit of open descriptors; and
//! one listening socket per grant, with `IPV6_V6ONLY` on IPv6 and without `SO_REUSEPORT`. The
//! standard library exposes none of them (ADR 0058, amendment of 2026-10-19).
//!
//! The calls go through the `libc` crate, Rust declarations of the system interface only, with
//! no C source built (ADR 0116 Appendix A).
#![allow(unsafe_code)]

use std::io;
use std::net::{SocketAddr, TcpListener};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// The flags that the host adds to `O_RDONLY` and `O_CLOEXEC` when it opens a TLS file: an open
/// never blocks on a FIFO, and never makes a terminal the controlling one.
pub const OPEN_FLAGS: i32 = libc::O_NONBLOCK | libc::O_NOCTTY;

/// The effective user and groups of the process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The effective user.
    pub uid: u32,
    /// The effective group.
    pub gid: u32,
    /// The supplementary groups.
    pub groups: Vec<u32>,
}

/// The effective user, the effective group and the supplementary groups of the process.
pub fn identity() -> io::Result<Identity> {
    // SAFETY: `geteuid` and `getegid` take no argument, touch no memory and cannot fail.
    let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
    // SAFETY: with a count of 0, `getgroups` writes nothing and returns the number of groups.
    let count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    let Ok(size) = usize::try_from(count) else {
        return Err(io::Error::last_os_error());
    };
    let mut groups: Vec<libc::gid_t> = vec![0; size];
    // SAFETY: `groups` holds `count` writable `gid_t`s, and `getgroups` writes at most `count`.
    let written = unsafe { libc::getgroups(count, groups.as_mut_ptr()) };
    let Ok(written) = usize::try_from(written) else {
        return Err(io::Error::last_os_error());
    };
    groups.truncate(written);
    Ok(Identity { uid, gid, groups })
}

/// Raises the soft limit of open descriptors to `wanted`, within the hard limit, and returns
/// the soft limit in force afterwards, which is below `wanted` when the system refuses.
pub fn raise_descriptor_limit(wanted: u64) -> io::Result<u64> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid `rlimit` that `getrlimit` fills.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if limit.rlim_cur >= wanted {
        return Ok(limit.rlim_cur);
    }
    let raised = libc::rlimit {
        rlim_cur: wanted.min(limit.rlim_max),
        rlim_max: limit.rlim_max,
    };
    // SAFETY: `raised` is a valid `rlimit` that `setrlimit` only reads.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const raised) } != 0 {
        // macOS refuses a soft limit above its own maximum even under an infinite hard limit:
        // the limit stays as it was, and the caller compares it with what it needs.
        return Ok(limit.rlim_cur);
    }
    Ok(raised.rlim_cur)
}

/// A listening socket on `address`: close-on-exec, with `SO_REUSEADDR`, never `SO_REUSEPORT`,
/// and `IPV6_V6ONLY` on IPv6, bound and listening with `backlog`. The socket is blocking; the
/// reactor sets what it needs when it takes the socket over.
pub fn listen(address: SocketAddr, backlog: u32) -> io::Result<TcpListener> {
    let family = match address {
        SocketAddr::V4(_) => libc::AF_INET,
        SocketAddr::V6(_) => libc::AF_INET6,
    };
    #[cfg(target_os = "linux")]
    let kind = libc::SOCK_STREAM | libc::SOCK_CLOEXEC;
    #[cfg(target_os = "macos")]
    let kind = libc::SOCK_STREAM;
    // SAFETY: `socket` takes integers only and returns a new descriptor, or -1.
    let fd = unsafe { libc::socket(family, kind, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is a new open descriptor that nothing else owns; from here `socket` closes
    // it on every path, the error paths included.
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    // macOS has no `SOCK_CLOEXEC`. The runtime starts no process, so no exec can come between
    // the two calls.
    #[cfg(target_os = "macos")]
    {
        // SAFETY: `fd` is open and owned by `socket`; `F_SETFD` sets its descriptor flags and
        // touches no memory.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    enable(&socket, libc::SOL_SOCKET, libc::SO_REUSEADDR)?;
    if address.is_ipv6() {
        enable(&socket, libc::IPPROTO_IPV6, libc::IPV6_V6ONLY)?;
    }
    bind(&socket, address)?;
    let backlog = libc::c_int::try_from(backlog).unwrap_or(libc::c_int::MAX);
    // SAFETY: `socket` is an open, bound stream socket; `listen` takes integers only.
    if unsafe { libc::listen(socket.as_raw_fd(), backlog) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(TcpListener::from(socket))
}

/// Sets the integer option `option` of `level` to 1.
fn enable(socket: &OwnedFd, level: libc::c_int, option: libc::c_int) -> io::Result<()> {
    let one: libc::c_int = 1;
    let size = libc::socklen_t::try_from(size_of::<libc::c_int>()).unwrap_or(0);
    // SAFETY: `socket` is open; `one` is a `c_int` that lives across the call, and `size` is its
    // size, so `setsockopt` reads exactly that value.
    let done = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            level,
            option,
            (&raw const one).cast(),
            size,
        )
    };
    if done == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Binds `socket` to `address`.
fn bind(socket: &OwnedFd, address: SocketAddr) -> io::Result<()> {
    let done = match address {
        SocketAddr::V4(v4) => {
            // SAFETY: zero is a valid `sockaddr_in`; the fields that matter are set below.
            let mut raw: libc::sockaddr_in = unsafe { std::mem::zeroed() };
            raw.sin_family = libc::sa_family_t::try_from(libc::AF_INET).unwrap_or(0);
            raw.sin_port = v4.port().to_be();
            raw.sin_addr = libc::in_addr {
                s_addr: u32::from_ne_bytes(v4.ip().octets()),
            };
            #[cfg(target_os = "macos")]
            {
                raw.sin_len = u8::try_from(size_of::<libc::sockaddr_in>()).unwrap_or(0);
            }
            let size = libc::socklen_t::try_from(size_of::<libc::sockaddr_in>()).unwrap_or(0);
            // SAFETY: `socket` is open; `raw` is an initialized `sockaddr_in` that lives across
            // the call, and `size` is its size.
            unsafe { libc::bind(socket.as_raw_fd(), (&raw const raw).cast(), size) }
        }
        SocketAddr::V6(v6) => {
            // SAFETY: zero is a valid `sockaddr_in6`; the fields that matter are set below, and
            // the flow label and scope stay 0.
            let mut raw: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
            raw.sin6_family = libc::sa_family_t::try_from(libc::AF_INET6).unwrap_or(0);
            raw.sin6_port = v6.port().to_be();
            raw.sin6_addr = libc::in6_addr {
                s6_addr: v6.ip().octets(),
            };
            #[cfg(target_os = "macos")]
            {
                raw.sin6_len = u8::try_from(size_of::<libc::sockaddr_in6>()).unwrap_or(0);
            }
            let size = libc::socklen_t::try_from(size_of::<libc::sockaddr_in6>()).unwrap_or(0);
            // SAFETY: `socket` is open; `raw` is an initialized `sockaddr_in6` that lives across
            // the call, and `size` is its size.
            unsafe { libc::bind(socket.as_raw_fd(), (&raw const raw).cast(), size) }
        }
    };
    if done == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV6, TcpStream};

    #[test]
    fn the_identity_is_the_effective_user() {
        let me = identity().unwrap();
        // A file the process creates is owned by its effective user.
        let dir = std::env::temp_dir().join(format!("una-sys-identity-{}", std::process::id()));
        std::fs::write(&dir, b"x").unwrap();
        let owner = std::os::unix::fs::MetadataExt::uid(&std::fs::metadata(&dir).unwrap());
        std::fs::remove_file(&dir).unwrap();
        assert_eq!(owner, me.uid);
    }

    #[test]
    fn the_descriptor_limit_rises_to_what_is_asked_or_says_where_it_stays() {
        let now = raise_descriptor_limit(0).unwrap();
        assert!(now > 0);
        assert!(raise_descriptor_limit(now).unwrap() >= now);
    }

    #[test]
    fn listeners_accept_and_never_share_their_port() {
        let first = listen(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), 16).unwrap();
        let bound = first.local_addr().unwrap();
        assert!(TcpStream::connect(bound).is_ok());
        // A second socket on the same address and port is refused while the first listens.
        assert!(listen(bound, 16).is_err());
    }

    #[test]
    fn ipv6_listeners_are_v6_only() {
        let Ok(six) = listen(SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0)), 16) else {
            // A machine without IPv6 has nothing to check.
            return;
        };
        let port = six.local_addr().unwrap().port();
        // With `IPV6_V6ONLY`, the IPv4 wildcard on the same port is still free.
        let four = listen(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)), 16);
        assert!(four.is_ok(), "{four:?}");
    }
}
