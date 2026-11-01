//! Replies waiting to be written, and what the host does when they pass the grant's `replies`
//! (ADR 0123 rule 13, `docs/plan/0123-s4-design.md` §6.5).
//!
//! A pending reply is a queue of segments of at most [`SEGMENT`] bytes. The owner writes from
//! the front segment and drops each segment as soon as its last byte is written, giving its
//! bytes back to the grant's and the peer's counters at that moment, so the counters always
//! equal the bytes of segments allocated and not yet dropped.
//!
//! `reply` reserves its size first, never waits and never fails for it. When the grant is then
//! over `replies`, the same thread, before it allocates, frees victims until the grant is back
//! under or no other reply is pending: the connection with the most bytes left of the peer
//! furthest over its `peer_replies` share, or, when no peer is over its share, the connection
//! with the most bytes left of all, ties to the lower connection number. A victim's pending
//! segments are dropped at once and its connection is closed by its owner later.
//!
//! **Costs.** Outside pressure there are no heaps: a reply touches atomics, the connection's
//! own outbox and one shard of the registry. When a grant first goes over `replies`, three
//! kinds of max-heaps are built from the registry in O(c): the peers ordered by how far they
//! are over their share, each peer's connections by bytes left, and all connections by bytes
//! left. A key is an upper bound that only ever decreases as segments are dropped, so a pick
//! refreshes the top and puts it back lower when it no longer wins; each refresh lowers a key,
//! so picks cost O(log c) amortized. A key that can grow (a peer's, by a new reservation) is
//! pushed again at the reservation, and the older entry is dropped when it reaches the top.
//! The heaps are kept while the grant is above three quarters of `replies` and dropped below,
//! so the O(c) build is amortized over a quarter of `replies` written or freed.

use std::cmp::{Ordering as Cmp, Reverse};
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use super::grant::{GrantState, lock, sub};
use super::peer::{Peer, PeerKey};

/// The most bytes of one segment of a pending reply.
pub const SEGMENT: usize = 16 * 1024;

/// Shards of the registry of connections with a reply pending.
const REGISTRY_SHARDS: usize = 16;

/// What a connection shares with the threads that reply to it and the threads that free it: its
/// pending reply and the flags of its end.
#[derive(Debug)]
pub struct ConnShared {
    /// The connection's number in its grant.
    pub id: u64,
    /// The grant.
    pub grant: Arc<GrantState>,
    /// The peer, held from accept to close.
    pub peer: Arc<Peer>,
    /// The serve thread that owns the connection.
    pub owner: usize,
    /// The peer's address and port.
    pub address: SocketAddr,
    /// The connection's place in its owner's table, set when the owner adopts it.
    slot: AtomicU32,
    /// The generation of that place.
    generation: AtomicU32,
    outbox: Mutex<Outbox>,
    bytes_left: AtomicU64,
    dead: AtomicBool,
}

/// The pending reply of a connection.
#[derive(Debug, Default)]
struct Outbox {
    segments: VecDeque<Vec<u8>>,
    /// Bytes of the front segment already written.
    offset: usize,
    discarded: bool,
    registered: bool,
}

/// What one write of a pending reply did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing is pending.
    Empty,
    /// The reply was freed under pressure, or the connection closed.
    Discarded,
    /// `written` bytes went out; `finished` says whether the reply is now fully written.
    Wrote {
        /// Bytes accepted by the socket.
        written: usize,
        /// Whether nothing is pending any more.
        finished: bool,
    },
}

impl ConnShared {
    /// A connection's shared part.
    pub fn new(
        grant: Arc<GrantState>,
        peer: Arc<Peer>,
        owner: usize,
        address: SocketAddr,
    ) -> Arc<ConnShared> {
        Arc::new(ConnShared {
            id: grant.next_connection_id(),
            grant,
            peer,
            owner,
            address,
            slot: AtomicU32::new(0),
            generation: AtomicU32::new(0),
            outbox: Mutex::new(Outbox::default()),
            bytes_left: AtomicU64::new(0),
            dead: AtomicBool::new(false),
        })
    }

    /// Records the place the owner gave the connection.
    pub fn place(&self, slot: u32, generation: u32) {
        self.slot.store(slot, Ordering::Release);
        self.generation.store(generation, Ordering::Release);
    }

    /// The place the owner gave the connection: its slot and the slot's generation.
    pub fn placed(&self) -> (u32, u32) {
        (
            self.slot.load(Ordering::Acquire),
            self.generation.load(Ordering::Acquire),
        )
    }

    /// Bytes of segments held for this connection.
    pub fn bytes_left(&self) -> u64 {
        self.bytes_left.load(Ordering::Acquire)
    }

    /// Whether the connection was discarded: closed, or freed under pressure.
    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::Acquire)
    }

    /// Gives `n` bytes of segments back to the counters, without ending pressure.
    fn release_raw(&self, n: u64) {
        sub(&self.grant.replies, n);
        sub(&self.peer.replies, n);
        sub(&self.bytes_left, n);
    }

    /// Hands over a reply whose size was reserved by [`GrantState::reserve_reply`]: cuts it
    /// into segments and queues them. `false` when the connection was discarded, in which case
    /// the reservation is given back.
    pub fn push(self: &Arc<ConnShared>, reply: Vec<u8>) -> bool {
        let size = reply.len() as u64;
        let queued = {
            let mut outbox = lock(&self.outbox);
            if outbox.discarded {
                false
            } else {
                for chunk in reply.chunks(SEGMENT) {
                    outbox.segments.push_back(chunk.to_vec());
                }
                self.bytes_left.fetch_add(size, Ordering::AcqRel);
                if !outbox.registered {
                    outbox.registered = true;
                    self.grant.registry.insert(self);
                }
                true
            }
        };
        if !queued {
            self.release_raw(size);
            self.grant.after_release();
            return false;
        }
        if self.grant.pressure_on.load(Ordering::Acquire) {
            let mut pressure = lock(&self.grant.pressure);
            if self.grant.pressure_on.load(Ordering::Acquire) {
                pressure.insert(self, &self.grant);
            }
        }
        true
    }

    /// One write of the front segment through `write`, which returns how many bytes it took.
    /// A segment fully written is dropped and its bytes are given back at once.
    pub fn write_front(&self, write: impl FnOnce(&[u8]) -> io::Result<usize>) -> io::Result<Step> {
        let step = {
            let mut outbox = lock(&self.outbox);
            if outbox.discarded {
                return Ok(Step::Discarded);
            }
            let offset = outbox.offset;
            let Some(front) = outbox.segments.front() else {
                return Ok(Step::Empty);
            };
            let rest = front.get(offset..).unwrap_or_default();
            let written = write(rest)?.min(rest.len());
            let length = front.len();
            if offset + written >= length {
                outbox.segments.pop_front();
                outbox.offset = 0;
                self.release_raw(length as u64);
            } else {
                outbox.offset = offset + written;
            }
            let finished = outbox.segments.is_empty();
            if finished && outbox.registered {
                outbox.registered = false;
                self.grant.registry.remove(self.id);
            }
            Step::Wrote { written, finished }
        };
        self.grant.after_release();
        Ok(step)
    }

    /// Drops everything pending and marks the connection dead; returns the bytes freed. The
    /// reply is never written, which the program never sees (rule 14). Idempotent.
    pub fn discard(&self) -> u64 {
        let freed = {
            let mut outbox = lock(&self.outbox);
            outbox.discarded = true;
            self.dead.store(true, Ordering::Release);
            let freed: usize = outbox.segments.iter().map(Vec::len).sum();
            outbox.segments.clear();
            outbox.offset = 0;
            if outbox.registered {
                outbox.registered = false;
                self.grant.registry.remove(self.id);
            }
            freed as u64
        };
        self.release_raw(freed);
        freed
    }

    /// [`ConnShared::discard`], then the end of pressure if the grant is back below three
    /// quarters of `replies`.
    pub fn close(&self) -> u64 {
        let freed = self.discard();
        self.grant.after_release();
        freed
    }
}

/// The connections of a grant that have a reply pending, found in O(c) when pressure begins.
/// Held weakly: a connection that is gone is simply not there.
#[derive(Debug)]
pub struct Registry {
    shards: Vec<Mutex<HashMap<u64, Weak<ConnShared>>>>,
}

impl Default for Registry {
    fn default() -> Registry {
        Registry {
            shards: (0..REGISTRY_SHARDS)
                .map(|_| Mutex::new(HashMap::new()))
                .collect(),
        }
    }
}

impl Registry {
    fn shard(&self, id: u64) -> Option<&Mutex<HashMap<u64, Weak<ConnShared>>>> {
        self.shards.get((id % REGISTRY_SHARDS as u64) as usize)
    }

    /// Adds a connection. O(1) expected.
    pub fn insert(&self, conn: &Arc<ConnShared>) {
        if let Some(shard) = self.shard(conn.id) {
            lock(shard).insert(conn.id, Arc::downgrade(conn));
        }
    }

    /// Removes a connection. O(1) expected.
    pub fn remove(&self, id: u64) {
        if let Some(shard) = self.shard(id) {
            lock(shard).remove(&id);
        }
    }

    /// Every connection with a reply pending. O(c).
    pub fn all(&self) -> Vec<Arc<ConnShared>> {
        let mut out = Vec::new();
        for shard in &self.shards {
            out.extend(lock(shard).values().filter_map(Weak::upgrade));
        }
        out
    }

    /// The number of connections with a reply pending.
    pub fn len(&self) -> usize {
        self.shards.iter().map(|shard| lock(shard).len()).sum()
    }

    /// Whether no connection has a reply pending.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// An entry of a heap of connections: an upper bound of the bytes left.
#[derive(Debug)]
struct Entry {
    key: u64,
    id: u64,
    conn: Weak<ConnShared>,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Entry) -> bool {
        self.cmp(other) == Cmp::Equal
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Entry) -> Option<Cmp> {
        Some(self.cmp(other))
    }
}
impl Ord for Entry {
    /// The most bytes first, then the lower connection number.
    fn cmp(&self, other: &Entry) -> Cmp {
        (self.key, Reverse(self.id)).cmp(&(other.key, Reverse(other.id)))
    }
}

/// An entry of the heap of peers: an upper bound of how far the peer is over its share.
#[derive(Debug)]
struct PeerEntry {
    over: u64,
    peer: Arc<Peer>,
}

impl PartialEq for PeerEntry {
    fn eq(&self, other: &PeerEntry) -> bool {
        self.cmp(other) == Cmp::Equal
    }
}
impl Eq for PeerEntry {}
impl PartialOrd for PeerEntry {
    fn partial_cmp(&self, other: &PeerEntry) -> Option<Cmp> {
        Some(self.cmp(other))
    }
}
impl Ord for PeerEntry {
    fn cmp(&self, other: &PeerEntry) -> Cmp {
        (self.over, Reverse(self.peer.key)).cmp(&(other.over, Reverse(other.peer.key)))
    }
}

/// The heaps of one pressure episode of a grant.
#[derive(Debug, Default)]
pub struct Pressure {
    all: BinaryHeap<Entry>,
    by_peer: HashMap<PeerKey, BinaryHeap<Entry>>,
    peers: BinaryHeap<PeerEntry>,
}

/// How far a peer is over its share of `peer_replies`; 0 under it or with no share.
fn over(peer: &Peer, share: u64) -> u64 {
    peer.replies.load(Ordering::Acquire).saturating_sub(share)
}

impl Pressure {
    /// Adds a connection with a reply pending, and its peer's current excess. O(log c).
    pub fn insert(&mut self, conn: &Arc<ConnShared>, grant: &GrantState) {
        let entry = || Entry {
            key: conn.bytes_left(),
            id: conn.id,
            conn: Arc::downgrade(conn),
        };
        self.all.push(entry());
        self.by_peer.entry(conn.peer.key).or_default().push(entry());
        let share = grant.limits.peer_replies.unwrap_or(u64::MAX);
        self.peers.push(PeerEntry {
            over: over(&conn.peer, share),
            peer: Arc::clone(&conn.peer),
        });
    }

    /// A reservation raised the peer's excess over its share: the peer enters the heap again
    /// with its new key, and the older entry is dropped when it reaches the top. O(log c).
    fn raise(&mut self, peer: &Arc<Peer>, share: u64) {
        self.peers.push(PeerEntry {
            over: over(peer, share),
            peer: Arc::clone(peer),
        });
    }

    /// Builds the heaps from every connection with a reply pending. O(c).
    fn build(&mut self, grant: &GrantState) {
        *self = Pressure::default();
        let mut seen = HashSet::new();
        let share = grant.limits.peer_replies.unwrap_or(u64::MAX);
        for conn in grant.registry.all() {
            self.all.push(Entry {
                key: conn.bytes_left(),
                id: conn.id,
                conn: Arc::downgrade(&conn),
            });
            self.by_peer.entry(conn.peer.key).or_default().push(Entry {
                key: conn.bytes_left(),
                id: conn.id,
                conn: Arc::downgrade(&conn),
            });
            if seen.insert(conn.peer.key) {
                self.peers.push(PeerEntry {
                    over: over(&conn.peer, share),
                    peer: Arc::clone(&conn.peer),
                });
            }
        }
    }

    /// The connection to free: the one with the most bytes left of the peer furthest over its
    /// share, else the one with the most bytes left of all. O(log c) amortized.
    fn pick(&mut self, share: u64) -> Option<Arc<ConnShared>> {
        while let Some(top) = self.peers.pop() {
            let now = over(&top.peer, share);
            if now == 0 || now > top.over {
                // Under its share, or a fresher entry of this peer is in the heap.
                continue;
            }
            if now < top.over {
                self.peers.push(PeerEntry {
                    over: now,
                    peer: top.peer,
                });
                continue;
            }
            let victim = self.by_peer.get_mut(&top.peer.key).and_then(valid_top);
            if let Some(victim) = victim {
                self.peers.push(top);
                return Some(victim);
            }
        }
        valid_top(&mut self.all)
    }

    fn clear(&mut self) {
        *self = Pressure::default();
    }
}

/// The connection with the most bytes left in `heap`, after refreshing stale keys.
fn valid_top(heap: &mut BinaryHeap<Entry>) -> Option<Arc<ConnShared>> {
    while let Some(entry) = heap.pop() {
        let Some(conn) = entry.conn.upgrade() else {
            continue;
        };
        if conn.is_dead() {
            continue;
        }
        let now = conn.bytes_left();
        if now == 0 || now > entry.key {
            // Nothing pending, or a fresher entry of this connection is in the heap.
            continue;
        }
        if now < entry.key {
            heap.push(Entry {
                key: now,
                id: entry.id,
                conn: entry.conn,
            });
            continue;
        }
        return Some(conn);
    }
    None
}

/// What reserving a reply decided.
#[derive(Debug)]
pub enum Reserve {
    /// The reply may be queued. These connections were freed under pressure: their pending
    /// segments are dropped, and their owners are to close them.
    Reserved(Vec<Arc<ConnShared>>),
    /// The reply is larger than `replies` while others are pending: it is not kept, and its
    /// connection closes at once (rule 13).
    CloseSelf,
}

impl GrantState {
    /// Reserves `size` bytes of reply for `conn` on the grant's `replies` and the peer's share,
    /// and frees victims while the grant is over `replies` (rule 13). Never waits and never
    /// fails for the reservation. O(1) outside pressure; O(log c) per victim under it.
    pub fn reserve_reply(&self, conn: &ConnShared, size: u64) -> Reserve {
        let cap = self.limits.replies;
        let before = self
            .replies
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
                Some(v.saturating_add(size))
            })
            .unwrap_or(0);
        let _ = conn
            .peer
            .replies
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
                Some(v.saturating_add(size))
            });
        if size > cap {
            if before > 0 {
                sub(&self.replies, size);
                sub(&conn.peer.replies, size);
                return Reserve::CloseSelf;
            }
            return Reserve::Reserved(Vec::new());
        }
        let mut victims = Vec::new();
        if self.replies.load(Ordering::Acquire) > cap || self.pressure_on.load(Ordering::Acquire) {
            let share = self.limits.peer_replies.unwrap_or(u64::MAX);
            let mut pressure = lock(&self.pressure);
            if self.pressure_on.load(Ordering::Acquire) {
                // Every reservation made while the heaps exist updates its peer's key.
                pressure.raise(&conn.peer, share);
            } else if self.replies.load(Ordering::Acquire) > cap {
                pressure.build(self);
                self.pressure_on.store(true, Ordering::Release);
            }
            while self.replies.load(Ordering::Acquire) > cap {
                let Some(victim) = pressure.pick(share) else {
                    break;
                };
                if victim.id == conn.id {
                    continue;
                }
                victim.discard();
                victims.push(victim);
            }
        }
        Reserve::Reserved(victims)
    }

    /// Ends pressure when the grant is back below three quarters of `replies`: the heaps are
    /// dropped. A new episode begins only after a quarter of `replies` was written or freed.
    pub fn after_release(&self) {
        if !self.pressure_on.load(Ordering::Acquire) {
            return;
        }
        let low = self.limits.replies / 4 * 3;
        if self.replies.load(Ordering::Acquire) <= low {
            let mut pressure = lock(&self.pressure);
            if self.replies.load(Ordering::Acquire) <= low {
                pressure.clear();
                self.pressure_on.store(false, Ordering::Release);
            }
        }
    }

    /// Whether pressure heaps exist.
    pub fn under_pressure(&self) -> bool {
        self.pressure_on.load(Ordering::Acquire)
    }
}
