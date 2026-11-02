//! The state a grant's serve threads share: the counters of its limits, its rate, its handshake
//! buckets, its peers and its reply pressure (ADR 0123 rules 12, 13, 17 and 19,
//! `docs/plan/0123-s4-design.md` §6).
//!
//! Counters are atomics, each on its own cache line, and a request costs a few atomic
//! operations on them and no lock. Sizes and counts are `u64`, compared with checked or
//! saturating arithmetic that never wraps (rule 7). A reservation that fails gives back what it
//! took before it, so every path leaves the counters where it found them.

use std::net::IpAddr;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use una_core::http::serve::{Hosts, Limits, ServeGrant};
use una_hash::SipKey;

use super::gcra::Gcra;
use super::peer::{Peer, PeerKey, PeerTable};
use super::pressure::{Pressure, Registry};

/// An atomic on a cache line of its own, so counters that different threads write do not share
/// one.
#[derive(Debug, Default)]
#[repr(align(128))]
pub struct Padded(AtomicU64);

impl Deref for Padded {
    type Target = AtomicU64;

    fn deref(&self) -> &AtomicU64 {
        &self.0
    }
}

/// Adds `n` to `counter` unless the sum would pass `cap`. Never wraps.
pub fn add_below(counter: &AtomicU64, n: u64, cap: u64) -> bool {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_add(n).filter(|sum| *sum <= cap)
        })
        .is_ok()
}

/// Takes `n` from `counter`, down to 0 and no further.
pub fn sub(counter: &AtomicU64, n: u64) {
    let _ = counter.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
        Some(value.saturating_sub(n))
    });
}

/// Locks a mutex whose protected data stays consistent at every step, even after a panic.
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Why a connection is not admitted at accept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcceptRefusal {
    /// The grant is at `connections`.
    Full,
    /// The peer is at `peer`.
    PeerLimit,
    /// The grant's `handshakes` rate is spent this second.
    Handshakes,
    /// The peer's share of the handshake rate is spent this second.
    PeerHandshakes,
    /// Every serve thread ended: none is left to own the connection. Reachable only when the
    /// caller is not itself a serve thread (the simulator): a serve thread that accepts lives.
    NoThread,
    /// A fault of the host while the connection was accepted: contained, and what it took was
    /// given back.
    Fault,
}

/// A connection refused at accept: why, and when its peer's entry may be forgotten if it is
/// kept until then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused {
    /// The reason.
    pub reason: AcceptRefusal,
    /// The peer and the time at which its handshake bucket is full, when its entry stays.
    pub expiry: Option<(PeerKey, u64)>,
}

/// Why a request waits for delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocked {
    /// `delivered` has no room for the request's bytes.
    Delivered,
    /// `/once` open exchanges are open.
    Once,
    /// The rate has no token before this time (nanoseconds).
    Rate(u64),
}

/// A snapshot of a grant's counters, for tests and the report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Open connections.
    pub connections: u64,
    /// Requests read and waiting for delivery.
    pub waiting: u64,
    /// Open exchanges.
    pub once: u64,
    /// Bytes of request bodies held.
    pub buffers: u64,
    /// Bytes of the requests of open exchanges.
    pub delivered: u64,
    /// Bytes of replies handed over and not yet written or freed.
    pub replies: u64,
}

/// A grant, as the serve threads of a run share it.
#[derive(Debug)]
pub struct GrantState {
    /// The grant's position among the run's grants.
    pub index: usize,
    /// The grant's address and port, as it writes them.
    pub listener: String,
    /// Whether the grant is `https`.
    pub tls: bool,
    /// The grant's limits, every default applied.
    pub limits: Limits,
    /// The names `Host` may carry.
    pub hosts: Hosts,
    open: Padded,
    waiting: Padded,
    once: Padded,
    buffers: Padded,
    delivered: Padded,
    pub(crate) replies: Padded,
    rate: Option<Gcra>,
    handshakes: Gcra,
    /// The grant's peers.
    pub peers: PeerTable,
    pub(crate) pressure_on: AtomicBool,
    pub(crate) pressure: Mutex<Pressure>,
    pub(crate) registry: Registry,
    next_connection: AtomicU64,
}

impl GrantState {
    /// The state of grant number `index`, whose peer table is keyed by `key`.
    pub fn new(index: usize, grant: &ServeGrant, key: SipKey) -> GrantState {
        let limits = grant.limits;
        GrantState {
            index,
            listener: grant.listener(),
            tls: grant.tls,
            limits,
            hosts: grant.hosts.clone(),
            open: Padded::default(),
            waiting: Padded::default(),
            once: Padded::default(),
            buffers: Padded::default(),
            delivered: Padded::default(),
            replies: Padded::default(),
            rate: limits
                .rate
                .map(|rate| Gcra::new(rate.interval_nanos(), rate.count)),
            handshakes: Gcra::per_second(limits.handshakes),
            peers: PeerTable::new(key, limits.peer_handshakes()),
            pressure_on: AtomicBool::new(false),
            pressure: Mutex::new(Pressure::default()),
            registry: Registry::default(),
            next_connection: AtomicU64::new(0),
        }
    }

    /// A snapshot of the counters.
    pub fn counters(&self) -> Counters {
        Counters {
            connections: self.open.load(Ordering::Acquire),
            waiting: self.waiting.load(Ordering::Acquire),
            once: self.once.load(Ordering::Acquire),
            buffers: self.buffers.load(Ordering::Acquire),
            delivered: self.delivered.load(Ordering::Acquire),
            replies: self.replies.load(Ordering::Acquire),
        }
    }

    /// Whether every counter is 0: after every connection is closed, nothing is held.
    pub fn is_quiet(&self) -> bool {
        self.counters() == Counters::default()
    }

    /// A number for the next connection, unique in the grant.
    pub fn next_connection_id(&self) -> u64 {
        self.next_connection.fetch_add(1, Ordering::Relaxed)
    }

    /// The least `connections` slots a grant may open at once.
    pub fn connection_cap(&self) -> u64 {
        self.limits.connections
    }

    /// Takes a slot of `connections`, before `accept` is called, so that every descriptor the
    /// server accepts holds one (`docs/plan/0123-s4-design.md` §4). O(1).
    pub fn take_slot(&self) -> bool {
        add_below(&self.open, 1, self.limits.connections)
    }

    /// Gives a slot back.
    pub fn give_slot(&self) {
        sub(&self.open, 1);
    }

    /// Open connections.
    pub fn open_connections(&self) -> u64 {
        self.open.load(Ordering::Acquire)
    }

    /// The checks at accept after the slot is taken, in the order of the design: the peer's
    /// `peer` count; for `https`, the peer's and then the grant's handshake rates (the peer's
    /// first, so one peer over its share cannot spend the grant's tokens). On success the
    /// peer's entry is held by the connection. On a refusal nothing stays taken but the slot,
    /// which the caller gives back, and the time at which the peer's entry may be forgotten,
    /// when it is kept until then. O(1) expected.
    pub fn admit(&self, address: IpAddr, now: u64) -> Result<Arc<Peer>, Refused> {
        let peer = self
            .peers
            .open(PeerKey::of(address), self.limits.peer)
            .map_err(|super::peer::Refused::TooMany| Refused {
                reason: AcceptRefusal::PeerLimit,
                expiry: None,
            })?;
        if self.tls {
            let refusal = if peer.handshakes.take(now).is_err() {
                Some(AcceptRefusal::PeerHandshakes)
            } else if self.handshakes.take(now).is_err() {
                Some(AcceptRefusal::Handshakes)
            } else {
                None
            };
            if let Some(refusal) = refusal {
                let expiry = self.peers.close(&peer, now).map(|at| (peer.key, at));
                return Err(Refused {
                    reason: refusal,
                    expiry,
                });
            }
        }
        Ok(peer)
    }

    /// Reserves `n` bytes of a request body for `peer`: the grant's `buffers` first, then the
    /// peer's `peer_buffers`; a failure of the second gives the first back (rule 13).
    pub fn reserve_body(&self, peer: &Peer, n: u64) -> bool {
        if !add_below(&self.buffers, n, self.limits.buffers) {
            return false;
        }
        let share = self.limits.peer_buffers.unwrap_or(u64::MAX);
        if !add_below(&peer.bodies, n, share) {
            sub(&self.buffers, n);
            return false;
        }
        true
    }

    /// Gives back `n` bytes of a body reservation.
    pub fn release_body(&self, peer: &Peer, n: u64) {
        sub(&peer.bodies, n);
        sub(&self.buffers, n);
    }

    /// Counts a request read and waiting; `false` at `waiting` (rule 14: `503`).
    pub fn queue_waiting(&self) -> bool {
        add_below(&self.waiting, 1, self.limits.waiting)
    }

    /// Gives back a place among the waiting requests.
    pub fn unqueue_waiting(&self) {
        sub(&self.waiting, 1);
    }

    /// Tries to start an exchange of `bytes` bytes (rule 17.4): room in `delivered`, a `/once`
    /// slot and a token of the rate, in this order, each undone if a later one fails. O(1).
    pub fn begin_exchange(&self, bytes: u64, now: u64) -> Result<(), Blocked> {
        if !add_below(&self.delivered, bytes, self.limits.delivered) {
            return Err(Blocked::Delivered);
        }
        if !add_below(&self.once, 1, self.limits.once) {
            sub(&self.delivered, bytes);
            return Err(Blocked::Once);
        }
        if let Some(rate) = &self.rate
            && let Err(at) = rate.take(now)
        {
            sub(&self.once, 1);
            sub(&self.delivered, bytes);
            return Err(Blocked::Rate(at));
        }
        Ok(())
    }

    /// Whether `begin_exchange` could succeed now, taking nothing: used to avoid taking a
    /// request off its queue to put it back.
    pub fn exchange_blocked(&self, bytes: u64, now: u64) -> Option<Blocked> {
        if self
            .delivered
            .load(Ordering::Acquire)
            .checked_add(bytes)
            .is_none_or(|sum| sum > self.limits.delivered)
        {
            return Some(Blocked::Delivered);
        }
        if self.once.load(Ordering::Acquire) >= self.limits.once {
            return Some(Blocked::Once);
        }
        match &self.rate {
            Some(rate) => rate.peek(now).err().map(Blocked::Rate),
            None => None,
        }
    }

    /// Ends an exchange of `bytes` bytes: its `/once` slot and its `delivered` bytes.
    pub fn end_exchange(&self, bytes: u64) {
        sub(&self.once, 1);
        sub(&self.delivered, bytes);
    }
}
