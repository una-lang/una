//! Reply pressure (ADR 0123 rule 13): the victims the lazy heaps pick against a brute-force
//! maximum over random histories, the bound on pending bytes, and the closing of the slowest
//! connections while a fresh client is answered.

use std::net::SocketAddr;
use std::sync::Arc;

use una_core::http::serve::ServeGrant;
use una_hash::SipKey;
use una_http::reply::Reply;
use una_http::serve::grant::GrantState;
use una_http::serve::peer::PeerKey;
use una_http::serve::pressure::{ConnShared, Reserve, SEGMENT, Step};
use una_http::serve::sim::{Reads, Sim};

const MIB: u64 = 1024 * 1024;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A connection of the model.
struct Model {
    shared: Arc<ConnShared>,
    peer: u32,
    size: u64,
    written: u64,
    pending: bool,
}

impl Model {
    fn left(&self) -> u64 {
        if !self.pending || self.written >= self.size {
            0
        } else {
            self.size - (self.written / SEGMENT as u64) * SEGMENT as u64
        }
    }
}

fn state(spec: &str) -> Arc<GrantState> {
    let grant = ServeGrant::parse(spec).unwrap();
    Arc::new(GrantState::new(0, &grant, SipKey::from_bytes([3; 16])))
}

fn connect(state: &Arc<GrantState>, peer: u32, n: u64) -> Model {
    let entry = state.peers.open(PeerKey::V4(peer), None).unwrap();
    let address: SocketAddr = format!("10.0.0.{}:{}", peer + 1, 1000 + n).parse().unwrap();
    Model {
        shared: ConnShared::new(Arc::clone(state), entry, 0, address),
        peer,
        size: 0,
        written: 0,
        pending: false,
    }
}

/// The connections the spec says to free, one after another, for a grant over `cap` once
/// `new_size` is reserved for a connection of `new_peer`.
fn expected(models: &[Model], new_peer: u32, new_size: u64, cap: u64, share: u64) -> Vec<u64> {
    let mut left: Vec<(u64, u32, u64)> = models
        .iter()
        .filter(|m| m.pending && m.left() > 0)
        .map(|m| (m.shared.id, m.peer, m.left()))
        .collect();
    let mut total: u64 = left.iter().map(|l| l.2).sum::<u64>() + new_size;
    let mut out = Vec::new();
    while total > cap {
        let peer_total = |peer: u32, left: &[(u64, u32, u64)]| -> u64 {
            left.iter()
                .filter(|l| l.1 == peer)
                .map(|l| l.2)
                .sum::<u64>()
                + if peer == new_peer { new_size } else { 0 }
        };
        let mut peers: Vec<u32> = left.iter().map(|l| l.1).collect();
        peers.sort_unstable();
        peers.dedup();
        let mut best_peer: Option<(u64, u32)> = None;
        for peer in peers {
            let over = peer_total(peer, &left).saturating_sub(share);
            if over > 0 && best_peer.is_none_or(|(o, _)| over > o) {
                best_peer = Some((over, peer));
            }
        }
        let victim = match best_peer {
            Some((_, peer)) => left
                .iter()
                .filter(|l| l.1 == peer)
                .max_by_key(|l| (l.2, std::cmp::Reverse(l.0)))
                .copied(),
            None => left
                .iter()
                .max_by_key(|l| (l.2, std::cmp::Reverse(l.0)))
                .copied(),
        };
        let Some((id, _, bytes)) = victim else { break };
        out.push(id);
        total -= bytes;
        left.retain(|l| l.0 != id);
    }
    out
}

#[test]
fn victims_are_the_slowest_of_the_peer_furthest_over_its_share() {
    let spec = "serve:http://127.0.0.1:8080;replies=8;peer_replies=1";
    let (cap, share) = (8 * MIB, MIB);
    for seed in 1..=20u64 {
        let state = state(spec);
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut models: Vec<Model> = (0..24)
            .map(|n| connect(&state, (n % 6) as u32, n))
            .collect();
        let mut next_conn = 24u64;
        let mut victims_seen = 0usize;
        for _ in 0..1500 {
            let index = rng.below(models.len() as u64) as usize;
            let busy = models.get(index).is_some_and(|m| m.pending);
            if !busy {
                let size = 1 + rng.below(MIB + MIB / 2);
                let peer = models[index].peer;
                let want = expected(&models, peer, size, cap, share);
                let reserve = state.reserve_reply(&models[index].shared, size);
                let Reserve::Reserved(victims) = reserve else {
                    panic!("a reply under the cap is never refused");
                };
                let got: Vec<u64> = victims.iter().map(|v| v.id).collect();
                assert_eq!(got, want, "seed {seed}");
                victims_seen += got.len();
                for id in &got {
                    // A freed connection reconnects as a new one.
                    if let Some(at) = models.iter().position(|m| m.shared.id == *id) {
                        let peer = models[at].peer;
                        models[at] = connect(&state, peer, next_conn);
                        next_conn += 1;
                    }
                }
                let model = &mut models[index];
                if model.shared.is_dead() {
                    continue;
                }
                model.size = size;
                model.written = 0;
                model.pending = model.shared.push(vec![7; size as usize]);
            } else {
                let model = &mut models[index];
                let mut remaining = 1 + rng.below(40_000);
                while remaining > 0 && model.pending {
                    let step = model
                        .shared
                        .write_front(|bytes| Ok(bytes.len().min(remaining as usize)))
                        .unwrap();
                    match step {
                        Step::Wrote { written, finished } => {
                            model.written += written as u64;
                            remaining -= written as u64;
                            if finished {
                                model.pending = false;
                            }
                        }
                        Step::Empty | Step::Discarded => model.pending = false,
                    }
                }
            }
            let held: u64 = models.iter().map(Model::left).sum();
            assert_eq!(state.counters().replies, held, "seed {seed}");
            assert!(held <= cap, "seed {seed}: {held} pending");
            for m in &models {
                assert_eq!(m.shared.bytes_left(), m.left());
            }
        }
        assert!(victims_seen > 0, "seed {seed} never reached pressure");
    }
}

#[test]
fn a_reply_larger_than_replies_is_kept_alone_or_its_connection_closes() {
    let state = state("serve:http://127.0.0.1:8080;replies=1");
    let big = connect(&state, 1, 0);
    let other = connect(&state, 2, 1);
    // Alone, it is kept.
    assert!(matches!(
        state.reserve_reply(&big.shared, 2 * MIB),
        Reserve::Reserved(v) if v.is_empty()
    ));
    assert!(big.shared.push(vec![0; (2 * MIB) as usize]));
    // With another pending, the later large reply's connection closes at once.
    assert!(matches!(
        state.reserve_reply(&other.shared, 3 * MIB),
        Reserve::CloseSelf
    ));
    assert_eq!(state.counters().replies, 2 * MIB);
    // A small reply then frees the large one.
    let third = connect(&state, 3, 2);
    let Reserve::Reserved(victims) = state.reserve_reply(&third.shared, 10) else {
        panic!("refused")
    };
    assert_eq!(victims.len(), 1);
    assert!(big.shared.is_dead());
    assert_eq!(state.counters().replies, 10);
}

fn reply_of(bytes: usize) -> Reply {
    Reply {
        status: 200,
        headers: Vec::new(),
        body: "x".repeat(bytes),
    }
}

#[test]
fn slow_readers_are_closed_while_a_fresh_client_is_answered() {
    // 64 peers that never read their 1 MiB replies fill `replies` exactly; more close the
    // slowest, and a fresh client's request is answered within its own time.
    let mut sim = Sim::new(
        &["serve:http://127.0.0.1:8080;replies=64;connections=256;peer=none;1000/second"],
        1,
    );
    let mut pending = Vec::new();
    for k in 0..96u64 {
        let from = format!("192.0.2.{}:{}", (k % 200) + 1, 20_000 + k);
        assert!(sim.connect(k, 0, &from, Reads::Paced));
        sim.send(k, b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
        pending.push(sim.next().expect("a request"));
    }
    let grant = Arc::clone(&sim.shared.grants[0]);
    let mut max = 0;
    for (k, mut exchange) in pending.into_iter().enumerate() {
        exchange.reply(&reply_of(MIB as usize - 200)).unwrap();
        sim.flush();
        let held = grant.counters().replies;
        max = max.max(held);
        assert!(held <= 64 * MIB, "{held} bytes pending after reply {k}");
    }
    assert!(max > 60 * MIB, "pressure was reached: {max}");
    let closed = (0..96u64).filter(|k| sim.closed(*k)).count();
    assert!(
        (30..=34).contains(&closed),
        "{closed} connections were freed"
    );
    // The oldest connections (lowest numbers) were the victims, not the later ones.
    assert!(sim.closed(0) && !sim.closed(95));
    // A fresh client is answered.
    assert!(sim.connect(500, 0, "192.0.2.250:30000", Reads::Eager));
    sim.send(500, b"GET /fresh HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let mut fresh = sim.next().expect("the fresh request");
    assert_eq!(fresh.request.target, "/fresh");
    fresh.reply(&reply_of(5)).unwrap();
    sim.flush();
    assert!(String::from_utf8_lossy(&sim.received(500)).ends_with("xxxxx"));
}
