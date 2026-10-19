//! The OS bindings for `serve:` grants (ADR 0123 rules 9 and 19): each wrapper against what
//! the system shows.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6, TcpListener, TcpStream};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use una_sys::server::{OPEN_FLAGS, identity, listen, raise_descriptor_limit};

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
    let path = std::env::temp_dir().join(format!("una-sys-fifo-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let made = std::process::Command::new("mkfifo").arg(&path).status();
    if !made.is_ok_and(|status| status.success()) {
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
    // Far above any hard limit: the limit in force is returned, below what was asked.
    assert!(raise_descriptor_limit(u64::MAX / 2).unwrap() < u64::MAX / 2);
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

#[test]
fn an_ipv6_listener_is_v6_only() {
    let any = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0));
    let Ok(six) = listen(any, 16) else {
        // A machine without IPv6 has nothing to check.
        return;
    };
    let port = six.local_addr().unwrap().port();
    // With `IPV6_V6ONLY`, the IPv4 wildcard on the same port stays free.
    let four = listen(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)), 16);
    assert!(four.is_ok(), "{four:?}");
}
