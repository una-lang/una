//! The OS bindings for `serve:` grants (ADR 0123 rules 9 and 19): each wrapper against what
//! the system shows.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6, TcpListener, TcpStream};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use una_sys::server::{OPEN_FLAGS, identity, listen, raise_descriptor_limit};

/// Held by the tests that start a child process and by the test that rebinds a port. On macOS
/// close-on-exec is set apart from `socket` and `accept` (as in std), so a child started by a
/// parallel test in between keeps a copy of a socket, and a rebind then finds the port held.
static CHILDREN: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn the_identity_owns_what_the_process_creates() {
    let me = identity().unwrap();
    let path = std::env::temp_dir().join(format!("una-sys-identity-{}", std::process::id()));
    std::fs::write(&path, b"x").unwrap();
    let metadata = std::fs::metadata(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(metadata.uid(), me.uid);
    // A new file's group is the effective group on Linux, the directory's on macOS; either
    // way the process belongs to it.
    #[cfg(target_os = "linux")]
    assert_eq!(metadata.gid(), me.gid);
}

#[test]
fn the_open_flags_keep_a_fifo_from_blocking() {
    let _children = CHILDREN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let path = std::env::temp_dir().join(format!("una-sys-fifo-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let made = std::process::Command::new("mkfifo").arg(&path).status();
    if !made.is_ok_and(|status| status.success()) {
        eprintln!("note: `mkfifo` is not available here; the FIFO case is not checked");
        return;
    }
    // Without `O_NONBLOCK`, this open would wait for a writer forever.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_FLAGS)
        .open(&path)
        .unwrap();
    assert!(!file.metadata().unwrap().file_type().is_file());
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn the_descriptor_limit_rises_or_says_where_it_stays() {
    let now = raise_descriptor_limit(0).unwrap();
    assert!(now > 0);
    assert!(raise_descriptor_limit(now + 1).unwrap() >= now);
    // Far above any reachable limit, yet below `RLIM_INFINITY` (`u64::MAX / 2` on macOS):
    // the limit read back is returned, at least what it was and below what was asked; on
    // macOS, the reachable `OPEN_MAX` is tried.
    let far = 1 << 40;
    let got = raise_descriptor_limit(far).unwrap();
    assert!(got >= now && got < far, "{now} {got}");
    #[cfg(target_os = "macos")]
    assert!(got <= 10_240 || got == now, "{now} {got}");
}

#[test]
fn a_listener_is_not_inherited_by_a_child() {
    let _children = CHILDREN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    use std::os::fd::AsRawFd;
    let listener = listen(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), 16).unwrap();
    let fd = listener.as_raw_fd();
    let open_in_child = |fd: i32| {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("[ -e /dev/fd/{fd} ]"))
            .status()
            .unwrap()
            .success()
    };
    // The control: the child's own standard input is seen.
    assert!(open_in_child(0));
    // `FD_CLOEXEC` is set, so the listener is closed in the child.
    assert!(!open_in_child(fd), "descriptor {fd} was inherited");
}

#[test]
fn a_listener_accepts_connections() {
    let first = listen(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), 16).unwrap();
    let bound = first.local_addr().unwrap();
    assert!(TcpStream::connect(bound).is_ok());
}

/// A restart rebinds its port while connections of the previous listener are in `TIME_WAIT`,
/// which `SO_REUSEADDR` allows (`spec/31 §2` rule 7).
#[test]
fn a_restart_rebinds_while_old_connections_are_in_time_wait() {
    let _children = CHILDREN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    use std::io::Read;
    let first = listen(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), 16).unwrap();
    let bound = first.local_addr().unwrap();
    let client = TcpStream::connect(bound).unwrap();
    let (served, _) = first.accept().unwrap();
    // The server side closes first, so its end of the connection enters `TIME_WAIT`.
    drop(served);
    let mut rest = Vec::new();
    let _ = (&client).read_to_end(&mut rest);
    drop(client);
    drop(first);
    let again = listen(bound, 16);
    assert!(again.is_ok(), "{again:?}");
}

/// A second listener on the same exact address and port is refused while the first listens,
/// whatever options it sets: no `SO_REUSEPORT` is ever set.
#[test]
fn a_second_listener_on_the_same_address_and_port_is_refused() {
    let first = listen(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), 16).unwrap();
    let bound = first.local_addr().unwrap();
    let ours = listen(bound, 16).unwrap_err();
    assert_eq!(ours.kind(), std::io::ErrorKind::AddrInUse);
    // `TcpListener::bind` sets `SO_REUSEADDR` on its own socket, and is refused as well.
    let std = TcpListener::bind(bound).unwrap_err();
    assert_eq!(std.kind(), std::io::ErrorKind::AddrInUse);
}

/// The listener is bound to the address and port asked for, in both families (#1483): the
/// port of a first listener, freed, is asked for by number.
#[test]
fn a_listener_is_bound_to_the_address_and_port_asked_for() {
    let _children = CHILDREN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let free = listen(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), 16).unwrap();
    let four = free.local_addr().unwrap();
    assert_eq!(four.ip(), Ipv4Addr::LOCALHOST);
    drop(free);
    assert_eq!(listen(four, 16).unwrap().local_addr().unwrap(), four);
    let free = match listen(SocketAddr::from((Ipv6Addr::LOCALHOST, 0)), 16) {
        // Only a machine without IPv6 skips; any other error fails.
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::EAFNOSUPPORT | libc::EADDRNOTAVAIL)
            ) =>
        {
            eprintln!("note: IPv6 is not available here ({error}); the case is not checked");
            return;
        }
        other => other.unwrap(),
    };
    let six = free.local_addr().unwrap();
    assert_eq!(six.ip(), Ipv6Addr::LOCALHOST);
    drop(free);
    assert_eq!(listen(six, 16).unwrap().local_addr().unwrap(), six);
}

#[test]
fn an_ipv6_listener_is_v6_only() {
    let any = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0));
    let six = match listen(any, 16) {
        // Only a machine without IPv6 skips; any other error fails.
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::EAFNOSUPPORT | libc::EADDRNOTAVAIL)
            ) =>
        {
            eprintln!("note: IPv6 is not available here ({error}); the case is not checked");
            return;
        }
        other => other.unwrap(),
    };
    let port = six.local_addr().unwrap().port();
    // With `IPV6_V6ONLY`, the IPv4 wildcard on the same port stays free.
    let four = listen(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)), 16);
    assert!(four.is_ok(), "{four:?}");
}
