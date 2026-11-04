//! The peers of a grant: who has connections open, how much of the shared budgets each holds,
//! and when each may next start a TLS handshake (ADR 0123 rules 12 and 13,
//! `docs/plan/0123-s4-design.md` §6.3).
//!
//! A peer is an IPv4 address, or the /64 prefix of an IPv6 address; an IPv4-mapped IPv6 address
//! counts as its IPv4 address. The table is 64 shards of hash maps behind their own mutexes,
//! the shard chosen by SipHash-2-4 under the run's placement key with the tag `una.serve.peer`,
//! so a peer cannot choose addresses that collide in one shard. A lock is taken at accept, at
//! close and at expiry only, never per request: a connection holds its [`Peer`] by reference
//! and its shares are atomics of the entry. Every operation is O(1) expected under any choice
//! of addresses.
//!
//! **The bound.** An entry lives while it has an open connection, or while its handshake bucket
//! is not yet full. A bucket that is not full was drained by handshakes the grant's own rate
//! admitted, at most `handshakes` per second, and refills within a second (its burst is one
//! second's worth). So a table holds at most `connections + handshakes` entries.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use una_hash::{SipKey, sip24};

use super::fault::{Health, hold};
use super::gcra::Gcra;

/// Shards of a peer table.
pub const SHARDS: usize = 64;

/// The key of a peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PeerKey {
    /// An IPv4 address, or the IPv4 address an IPv4-mapped IPv6 address maps.
    V4(u32),
    /// The /64 prefix of an IPv6 address.
    V6(u64),
}

impl PeerKey {
    /// The peer of an address.
    pub fn of(address: IpAddr) -> PeerKey {
        match address {
            IpAddr::V4(v4) => PeerKey::V4(u32::from(v4)),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => PeerKey::V4(u32::from(v4)),
                None => PeerKey::V6(prefix(v6)),
            },
        }
    }

    /// The key's bytes, with a tag for its family, for hashing.
    fn bytes(self) -> [u8; 9] {
        let mut out = [0u8; 9];
        match self {
            PeerKey::V4(v4) => {
                out[0] = 4;
                for (to, from) in out.iter_mut().skip(1).zip(v4.to_be_bytes()) {
                    *to = from;
                }
            }
            PeerKey::V6(v6) => {
                out[0] = 6;
                for (to, from) in out.iter_mut().skip(1).zip(v6.to_be_bytes()) {
                    *to = from;
                }
            }
        }
        out
    }
}

/// The first 64 bits of an IPv6 address.
fn prefix(address: Ipv6Addr) -> u64 {
    let octets = address.octets();
    let mut high = [0u8; 8];
    for (to, from) in high.iter_mut().zip(octets) {
        *to = from;
    }
    u64::from_be_bytes(high)
}

/// What a peer holds, and when it may start a handshake. The counters are atomics so that a
/// connection touches them without the table's locks.
#[derive(Debug)]
pub struct Peer {
    /// The peer's key.
    pub key: PeerKey,
    /// Open connections, `peer` at most.
    pub open: AtomicU64,
    /// Bytes of request bodies admitted and not yet delivered or refused, `peer_buffers` at most.
    pub bodies: AtomicU64,
    /// Bytes of replies handed over and not yet written or freed.
    pub replies: AtomicU64,
    /// The peer's handshake bucket: `max(4, handshakes / 16)` per second.
    pub handshakes: Gcra,
}

/// Why a connection from a peer is not admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The peer already has `peer` connections open.
    TooMany,
}

/// The peers of a grant.
#[derive(Debug)]
pub struct PeerTable {
    key: SipKey,
    shards: Vec<Mutex<HashMap<PeerKey, Arc<Peer>>>>,
    /// Handshakes one peer may start per second.
    handshake_rate: u64,
    /// The run's record of faults: a panic with a shard locked stops the run.
    health: Arc<Health>,
}

impl PeerTable {
    /// An empty table whose shards are chosen under `key`, and whose peers may start
    /// `handshake_rate` handshakes per second.
    pub fn new(key: SipKey, handshake_rate: u64) -> PeerTable {
        PeerTable::with_health(key, handshake_rate, Arc::default())
    }

    /// [`PeerTable::new`] in a run whose record of faults is `health`. Each shard is locked
    /// through [`hold`], so a panic with a shard locked stops the run.
    pub fn with_health(key: SipKey, handshake_rate: u64, health: Arc<Health>) -> PeerTable {
        PeerTable {
            key,
            shards: (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect(),
            handshake_rate,
            health,
        }
    }

    /// The shard of a peer.
    fn shard(&self, peer: PeerKey) -> Option<&Mutex<HashMap<PeerKey, Arc<Peer>>>> {
        let mut bytes = [0u8; 9 + 14];
        for (to, from) in bytes.iter_mut().zip(b"una.serve.peer\0") {
            *to = *from;
        }
        for (to, from) in bytes.iter_mut().skip(14).zip(peer.bytes()) {
            *to = from;
        }
        let hash = sip24(self.key, &bytes);
        self.shards.get((hash % SHARDS as u64) as usize)
    }

    /// Opens a connection of `peer`: the entry is made when it is new, and `open` is raised
    /// unless it is already at `cap`. O(1) expected.
    pub fn open(&self, peer: PeerKey, cap: Option<u64>) -> Result<Arc<Peer>, Refused> {
        let Some(shard) = self.shard(peer) else {
            return Err(Refused::TooMany);
        };
        let mut map = hold(&self.health, shard);
        let entry = Arc::clone(map.entry(peer).or_insert_with(|| {
            Arc::new(Peer {
                key: peer,
                open: AtomicU64::new(0),
                bodies: AtomicU64::new(0),
                replies: AtomicU64::new(0),
                handshakes: Gcra::per_second(self.handshake_rate),
            })
        }));
        // `open` is written only with the shard locked, so this read and write are one step.
        let open = entry.open.load(Ordering::Acquire);
        if cap.is_some_and(|cap| open >= cap) {
            return Err(Refused::TooMany);
        }
        entry.open.store(open + 1, Ordering::Release);
        Ok(entry)
    }

    /// Closes one connection of `peer` (or gives back one that was refused after `open`).
    /// When the entry then holds no connection, it goes at once if its handshake bucket is
    /// full; otherwise the time at which it is full is returned, for the caller to arm a timer
    /// that calls [`PeerTable::expire`]. O(1) expected.
    pub fn close(&self, peer: &Peer, now: u64) -> Option<u64> {
        let shard = self.shard(peer.key)?;
        let mut map = hold(&self.health, shard);
        let left = peer.open.load(Ordering::Acquire).saturating_sub(1);
        peer.open.store(left, Ordering::Release);
        Self::drop_if_idle(&mut map, peer.key, now)
    }

    /// Removes the entry of `peer` if it holds no connection and its bucket is full at `now`;
    /// otherwise the time at which it will be, when it holds no connection.
    pub fn expire(&self, peer: PeerKey, now: u64) -> Option<u64> {
        let shard = self.shard(peer)?;
        let mut map = hold(&self.health, shard);
        Self::drop_if_idle(&mut map, peer, now)
    }

    /// The rule of the entry's life, with the shard locked.
    fn drop_if_idle(map: &mut HashMap<PeerKey, Arc<Peer>>, key: PeerKey, now: u64) -> Option<u64> {
        let entry = map.get(&key)?;
        if entry.open.load(Ordering::Acquire) > 0 {
            return None;
        }
        if entry.handshakes.is_full(now) {
            map.remove(&key);
            return None;
        }
        Some(entry.handshakes.full_at())
    }

    /// The entries in the table. O(shards).
    pub fn len(&self) -> usize {
        self.shards
            .iter()
            .map(|shard| hold(&self.health, shard).len())
            .sum()
    }

    /// Whether the table has no entry.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use super::*;

    fn key() -> SipKey {
        SipKey::from_bytes([7; 16])
    }

    #[test]
    fn peers_are_addresses_prefixes_and_mapped_addresses() {
        let v4 = PeerKey::of(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)));
        let mapped = PeerKey::of(IpAddr::V6(Ipv6Addr::new(
            0, 0, 0, 0, 0, 0xffff, 0xc000, 0x201,
        )));
        assert_eq!(v4, mapped);
        let a = PeerKey::of("2001:db8:1:2::1".parse().unwrap());
        let b = PeerKey::of("2001:db8:1:2:ffff:ffff:ffff:ffff".parse().unwrap());
        let c = PeerKey::of("2001:db8:1:3::1".parse().unwrap());
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, v4);
    }

    #[test]
    fn the_cap_counts_open_connections_and_closing_frees_the_entry() {
        let table = PeerTable::new(key(), 512);
        let peer = PeerKey::V4(1);
        let one = table.open(peer, Some(2)).unwrap();
        let _two = table.open(peer, Some(2)).unwrap();
        assert_eq!(table.open(peer, Some(2)).unwrap_err(), Refused::TooMany);
        assert_eq!(one.open.load(Ordering::Relaxed), 2);
        assert_eq!(table.close(&one, 0), None);
        let again = table.open(peer, Some(2)).unwrap();
        assert_eq!(table.close(&again, 0), None);
        assert_eq!(table.close(&one, 0), None);
        assert!(table.is_empty());
    }

    #[test]
    fn an_entry_with_a_drained_bucket_waits_for_it_to_refill() {
        let table = PeerTable::new(key(), 4);
        let peer = PeerKey::V4(9);
        let entry = table.open(peer, None).unwrap();
        for _ in 0..4 {
            assert!(entry.handshakes.take(0).is_ok());
        }
        let full_at = table.close(&entry, 0).unwrap();
        assert_eq!(table.len(), 1);
        assert_eq!(table.expire(peer, full_at - 1), Some(full_at));
        assert_eq!(table.expire(peer, full_at), None);
        assert!(table.is_empty());
    }

    #[test]
    fn shards_spread_the_peers() {
        let table = PeerTable::new(key(), 512);
        let mut counts = [0usize; SHARDS];
        for n in 0..6400u32 {
            let peer = table.open(PeerKey::V4(n), None).unwrap();
            let shard = table.shard(peer.key).unwrap();
            let at = table
                .shards
                .iter()
                .position(|s| std::ptr::eq(s, shard))
                .unwrap();
            counts[at] += 1;
        }
        assert!(
            counts.iter().all(|&c| (40..=170).contains(&c)),
            "{counts:?}"
        );
    }
}
