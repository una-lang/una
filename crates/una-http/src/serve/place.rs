//! Which serve thread owns a connection (ADR 0123 rule 19, `docs/plan/0123-s4-design.md` §4).
//!
//! A connection belongs to serve thread `H("una.serve.place", listener, peer address, peer
//! port) mod M`, with `H` SipHash-2-4 under a 128-bit key drawn for the run from the operating
//! system's random source. The owner depends only on the connection's own addresses, which are
//! input: not on the accept order, nor on which thread accepted, nor on how busy any thread
//! was. Without the key a peer could choose source ports that put every connection on one
//! thread; with it, it cannot predict the thread, and throwaway connections cannot move later
//! ones, since each place is computed alone. The key serves only this and the peer table's
//! shards, under distinct tags, and is shown nowhere.

use std::net::{IpAddr, SocketAddr};

use una_hash::{SipKey, sip24};

/// The thread that owns the connection from `peer` to the listener numbered `listener`, among
/// `threads` serve threads. One SipHash of at most 38 bytes: O(1).
pub fn owner(key: SipKey, threads: usize, listener: usize, peer: SocketAddr) -> usize {
    let mut bytes = Vec::with_capacity(40);
    bytes.extend_from_slice(b"una.serve.place\0");
    bytes.extend_from_slice(&(listener as u64).to_le_bytes());
    match peer.ip() {
        IpAddr::V4(v4) => {
            bytes.push(4);
            bytes.extend_from_slice(&v4.octets());
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => {
                bytes.push(4);
                bytes.extend_from_slice(&v4.octets());
            }
            None => {
                bytes.push(6);
                bytes.extend_from_slice(&v6.octets());
            }
        },
    }
    bytes.extend_from_slice(&peer.port().to_le_bytes());
    let hash = sip24(key, &bytes);
    (hash % threads.max(1) as u64) as usize
}

/// The placement probe of the oracle `serve_owner_spread` (#1660, decision D-24), with the
/// `test-hooks` feature only, which no shipped `una` or `libuna_rt.a` has
/// (`scripts/check-release.sh` refuses the feature's marker, which [`probe::install`] keeps):
/// a run can write the serve thread that placement chose for each connection it accepted, one
/// line `<peer address> <owner>` each, written before the peer's admission, so a connection that
/// admission then refuses has a line too, and can place by `port mod M` with no key, the placement
/// the key exists to prevent, as the oracle's positive control. A process installs it at
/// most once, before its serve threads start; without it placement is [`owner`] alone.
#[cfg(feature = "test-hooks")]
pub mod probe {
    use std::fs::File;
    use std::io::Write as _;
    use std::net::SocketAddr;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    use una_hash::SipKey;

    struct Probe {
        unkeyed: bool,
        log: Option<Mutex<File>>,
    }

    static PROBE: OnceLock<Probe> = OnceLock::new();

    /// Installs the probe: with `unkeyed`, placement is `port mod M`; with `log`, every
    /// placement is written to that file, created afresh. A second install changes nothing,
    /// and does not touch its file.
    pub fn install(unkeyed: bool, log: Option<&Path>) -> std::io::Result<()> {
        let _ = std::hint::black_box(&crate::resolve::UNA_HTTP_TEST_HOOKS_MARKER);
        if PROBE.get().is_some() {
            return Ok(());
        }
        let log = match log {
            Some(path) => Some(Mutex::new(File::create(path)?)),
            None => None,
        };
        let _ = PROBE.set(Probe { unkeyed, log });
        Ok(())
    }

    /// The owner [`super::owner`] names, or `port mod threads` under an unkeyed probe.
    pub(crate) fn placed(key: SipKey, threads: usize, listener: usize, peer: SocketAddr) -> usize {
        match PROBE.get() {
            Some(probe) if probe.unkeyed => usize::from(peer.port()) % threads.max(1),
            _ => super::owner(key, threads, listener, peer),
        }
    }

    /// Writes that `owner` serves the connection from `peer`, when the probe logs.
    pub(crate) fn record(peer: SocketAddr, owner: usize) {
        if let Some(log) = PROBE.get().and_then(|probe| probe.log.as_ref()) {
            let mut file = log
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _ = writeln!(file, "{peer} {owner}");
        }
    }

    /// Writes that the connection from `peer` closed with `left` bytes of its reply not
    /// written, for `cause`, when the probe logs: one line `cut <peer> <left> <cause>`, which
    /// the oracle prints when a probe's reply is cut short (#2273). Its first word is no
    /// address, so a reader of the placements passes over it.
    pub(crate) fn cut(peer: SocketAddr, left: u64, cause: &str) {
        if let Some(log) = PROBE.get().and_then(|probe| probe.log.as_ref()) {
            let mut file = log
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _ = writeln!(file, "cut {peer} {left} {cause}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 0.999 quantile of the chi-square distribution for 1, 6 and 63 degrees of freedom.
    fn quantile(degrees: usize) -> f64 {
        match degrees {
            1 => 10.83,
            6 => 22.46,
            _ => 103.4,
        }
    }

    fn chi_square(counts: &[usize]) -> f64 {
        let total: usize = counts.iter().sum();
        let expected = total as f64 / counts.len() as f64;
        counts
            .iter()
            .map(|&c| {
                let diff = c as f64 - expected;
                diff * diff / expected
            })
            .sum()
    }

    fn spread(threads: usize, peers: impl Iterator<Item = SocketAddr>) {
        let key = SipKey::from_bytes([0x5a; 16]);
        let mut counts = vec![0usize; threads];
        let mut total = 0usize;
        for peer in peers {
            if let Some(count) = counts.get_mut(owner(key, threads, 0, peer)) {
                *count += 1;
            }
            total += 1;
        }
        let mean = total as f64 / threads as f64;
        let largest = counts.iter().copied().max().unwrap_or(0) as f64;
        assert!(largest < 1.2 * mean, "{threads}: {counts:?}");
        assert!(chi_square(&counts) < quantile(threads - 1), "{counts:?}");
    }

    #[test]
    fn source_ports_of_one_address_spread_over_the_threads() {
        for threads in [2, 7, 64] {
            spread(
                threads,
                (1..=65_535u16).map(|port| SocketAddr::from(([192, 0, 2, 1], port))),
            );
        }
    }

    #[test]
    fn addresses_of_one_prefix_spread_over_the_threads() {
        for threads in [2, 7, 64] {
            spread(
                threads,
                (1..=65_535u32).map(|n| {
                    let low = u128::from(n);
                    let base: u128 = 0x2001_0db8_0000_0001_0000_0000_0000_0000;
                    SocketAddr::new(IpAddr::V6((base | low).into()), 4000)
                }),
            );
        }
    }

    #[test]
    fn the_owner_depends_on_the_connection_alone() {
        let key = SipKey::from_bytes([1; 16]);
        let peer = SocketAddr::from(([192, 0, 2, 9], 5555));
        let owner_a = owner(key, 8, 0, peer);
        for _ in 0..100 {
            assert_eq!(owner(key, 8, 0, peer), owner_a);
        }
        let mapped: SocketAddr = "[::ffff:192.0.2.9]:5555".parse().unwrap();
        assert_eq!(owner(key, 8, 0, mapped), owner_a);
        let other = SipKey::from_bytes([2; 16]);
        let differs = (1..=64u16)
            .filter(|&p| {
                let peer = SocketAddr::from(([192, 0, 2, 9], p));
                owner(key, 8, 0, peer) != owner(other, 8, 0, peer)
            })
            .count();
        assert!(differs > 32, "a new key moves most connections");
    }
}
