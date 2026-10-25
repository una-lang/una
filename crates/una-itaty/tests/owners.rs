//! Owners' chains end to end (ADR 0122 rules 36.4, 38 and 38.1, `spec/30 §14` rules 1 and 5):
//! Itaty logs owner chains with cosigners, a recovery, a dispute and a witness rotation on a
//! file store, and the registry client, reading that store as a mirror directory, verifies it
//! with its own code and reaches the same keys; then each logging rule's refusal.
//!
//! Formats come from the client and the shared test builder of `una-project`
//! (`tests/support/registry.rs`): its keys, its root metadata, its signature and cosignature
//! lines; every entry Itaty reads is read by the client's reader.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

#[path = "../../una-project/tests/support/registry.rs"]
mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ed25519_dalek::SigningKey;
use una_itaty::digest_string;
use una_itaty::entry::owner_chain;
use una_itaty::http::{ServerConfig, Service, StartError};
use una_itaty::notify::{Notice, Notifier, Recording};
use una_itaty::registry::{
    Cause, Clock, Config, Receipt, Refusal, Registry, Rotation, checkpoint_path,
};
use una_itaty::store::{FileStore, Store};
use una_itaty::timing::Obligation;
use una_project::client::{Failure, Point, Registry as Client};
use una_project::tlog::note::{Key, parse_note};
use una_project::tlog::owner::{History, RECOVERY_DELAY, fingerprint};
use una_project::tlog::root::digest as root_digest;
use una_project::transport::Directory;
use una_project::transport::Fallback;

use support::{EXPIRES, Keys, NOW, cosign_line, key_text, keypair, root_text_with, signed};

type Pair = (SigningKey, Key);

struct TestClock(AtomicU64);

impl Clock for TestClock {
    fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "una-itaty-owners-{name}-{}-{nanos}",
        std::process::id()
    ))
}

/// A registry of the source `itaty` on a file store, under the shared builder's keys: three
/// witnesses, two of which must cosign.
struct Fixture {
    dir: PathBuf,
    store: Arc<FileStore>,
    registry: Registry,
    keys: Keys,
    clock: Arc<TestClock>,
    recording: Arc<Recording>,
    pin: String,
    heads: BTreeMap<String, (u64, [u8; 32])>,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        Fixture::with_k(name, 2)
    }

    /// A fixture whose root asks `k` of the three witnesses.
    fn with_k(name: &str, k: usize) -> Fixture {
        let dir = temp_dir(name);
        let store = Arc::new(FileStore::open(&dir).unwrap());
        let keys = Keys::new("itaty");
        let roots: Vec<&Key> = keys.roots.iter().map(|(_, key)| key).collect();
        let text = format!(
            "{}\n",
            root_text_with("itaty", 1, EXPIRES, &keys, &roots, k, &[])
        );
        let document = signed(&text, &[&keys.roots[0], &keys.roots[1]]);
        store.put("root/1", document.as_bytes()).unwrap();
        let pin = root_digest(&document);
        let clock = Arc::new(TestClock(AtomicU64::new(NOW)));
        let shared: Arc<dyn Store> = store.clone();
        let recording = Arc::new(Recording::new());
        let registry = Registry::open(
            shared,
            &Config {
                pin: pin.clone(),
                log_seed: [20; 32],
            },
            clock.clone(),
            recording.clone(),
        )
        .unwrap();
        Fixture {
            dir,
            store,
            registry,
            keys,
            clock,
            recording,
            pin,
            heads: BTreeMap::new(),
        }
    }

    /// The registry opened again on the same store, as after a restart.
    fn reopen(&self) -> Registry {
        self.reopen_with(Arc::new(Recording::new()))
    }

    /// [`Fixture::reopen`], its notices going to `notifier`.
    fn reopen_with(&self, notifier: Arc<dyn Notifier>) -> Registry {
        let shared: Arc<dyn Store> = self.store.clone();
        Registry::open(
            shared,
            &Config {
                pin: self.pin.clone(),
                log_seed: [20; 32],
            },
            self.clock.clone(),
            notifier,
        )
        .unwrap()
    }

    fn set_time(&self, time: u64) {
        self.clock.0.store(time, Ordering::SeqCst);
    }

    fn previous(&self, chain: &str) -> String {
        match self.heads.get(chain) {
            Some((index, leaf)) => format!("previous {index} {}", digest_string(leaf)),
            None => "previous none".to_owned(),
        }
    }

    fn logged(&mut self, chain: &str, receipt: Receipt) -> u64 {
        self.heads
            .insert(chain.to_owned(), (receipt.index, receipt.leaf));
        receipt.index
    }

    /// The text of a registration of `owner` with `keys`, `threshold`, and the cosigners
    /// `cosigners` with threshold one when there are any.
    fn registration(
        &self,
        owner: &str,
        keys: &[&Pair],
        threshold: usize,
        cosigners: &[&Pair],
    ) -> String {
        let mut text = format!(
            "ita-owner/1\nsource itaty\nowner {owner}\n{}\nthreshold {threshold}\n",
            self.previous(&owner_chain(owner))
        );
        for (_, key) in keys {
            text.push_str(&format!("key {}\n", key_text(key)));
        }
        for (_, key) in cosigners {
            text.push_str(&format!("cosigner {}\n", key_text(key)));
        }
        if !cosigners.is_empty() {
            text.push_str("cosigners 1\n");
        }
        text
    }

    fn register(&mut self, owner: &str, keys: &[&Pair], cosigners: &[&Pair]) -> u64 {
        let text = self.registration(owner, keys, 1, cosigners);
        let receipt = self
            .registry
            .register(signed(&text, keys).as_bytes(), false)
            .unwrap();
        self.logged(&owner_chain(owner), receipt)
    }

    /// [`Fixture::register`] as the operator does once it reviewed a name held for its
    /// distance to a popular owner.
    fn register_reviewed(&mut self, owner: &str, key: &Pair) -> u64 {
        let text = self.registration(owner, &[key], 1, &[]);
        let receipt = self
            .registry
            .register(signed(&text, &[key]).as_bytes(), true)
            .unwrap();
        self.logged(&owner_chain(owner), receipt)
    }

    /// The operator logs a dispute of `owner` naming `key`.
    fn dispute(&mut self, owner: &str, key: &Pair) -> Result<u64, Refusal> {
        let text = self.event(owner, &format!("dispute {}", key_text(&key.1)), 1);
        let receipt = self.registry.request(text.as_bytes(), false)?;
        Ok(self.logged(&owner_chain(owner), receipt))
    }

    /// The text of a key event of `owner`.
    fn event(&self, owner: &str, action: &str, threshold: usize) -> String {
        format!(
            "ita-key/1\nsource itaty\nowner {owner}\n{}\n{action}\nthreshold {threshold}\n",
            self.previous(&owner_chain(owner))
        )
    }

    fn submit(
        &mut self,
        owner: &str,
        action: &str,
        threshold: usize,
        signers: &[&Pair],
    ) -> Result<u64, Refusal> {
        let text = self.event(owner, action, threshold);
        let receipt = self.registry.submit(signed(&text, signers).as_bytes())?;
        Ok(self.logged(&owner_chain(owner), receipt))
    }

    /// The stored checkpoint of `size`, as served.
    fn checkpoint_note(&self, size: u64) -> una_project::tlog::note::Note {
        let bytes = self
            .store
            .get(&checkpoint_path(size), 1 << 20)
            .unwrap()
            .unwrap();
        parse_note(&String::from_utf8(bytes).unwrap()).unwrap()
    }

    /// The text of the newest checkpoint, or of the one of `size`.
    fn checkpoint_text(&self, size: u64) -> String {
        let bytes = self
            .store
            .get(&checkpoint_path(size), 1 << 20)
            .unwrap()
            .unwrap();
        parse_note(&String::from_utf8(bytes).unwrap()).unwrap().text
    }

    fn cosign_at(&mut self, size: u64, witness: usize, time: u64) -> Result<(), Refusal> {
        let text = self.checkpoint_text(size);
        let (pair, key) = &self.keys.witnesses[witness];
        let line = cosign_line(pair, key, &text, time);
        self.registry.cosign(size, &line)
    }

    /// The witnesses of `which` cosign the newest checkpoint at `time`.
    fn cosign(&mut self, which: &[usize], time: u64) {
        let size = self.registry.size();
        for witness in which {
            self.cosign_at(size, *witness, time).unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The parts of a history the client and the service must agree on.
fn verdicts(history: &History) -> impl PartialEq + std::fmt::Debug {
    (
        history.snapshots.clone(),
        history.compromised.clone(),
        history.pending.clone(),
        history.voided.clone(),
        history.requests.clone(),
        history.took_effect.clone(),
        history.cosigners.clone(),
        history.frozen,
    )
}

#[test]
fn the_client_verifies_the_owner_chains_itaty_serves() {
    let mut f = Fixture::new("round-trip");
    let a = keypair(60, "acme.a");
    let c = keypair(61, "trustee.c");
    let z = keypair(62, "zenith.z");
    let r = keypair(63, "acme.r");
    let d = keypair(64, "zenith.d");
    let s = keypair(65, "acme.s");
    let e = keypair(66, "zenith.e");
    let o = keypair(67, "orbit.o");
    let t0 = NOW;

    // acme registers with a recovery cosigner; zenith with none.
    assert_eq!(f.register("acme", &[&a], &[&c]), 0);
    f.cosign(&[0, 1, 2], t0);
    assert_eq!(f.register("zenith", &[&z], &[]), 1);
    f.cosign(&[0, 1], t0 + 10);

    // acme's cosigner asks a recovery; the operator logs it with the log's key.
    let recover = f.event("acme", &format!("recover {}", key_text(&r.1)), 1);
    let receipt = f
        .registry
        .request(signed(&recover, &[&c]).as_bytes(), false)
        .unwrap();
    let recovery = f.logged(&owner_chain("acme"), receipt);
    assert_eq!(recovery, 2);
    // Its start checkpoint must carry the threshold before anything more is logged.
    let dispute = f.event("zenith", &format!("dispute {}", key_text(&d.1)), 1);
    let refusal = f.registry.request(dispute.as_bytes(), false).unwrap_err();
    assert!(matches!(
        refusal,
        Refusal::Wait(Cause::Obligation(Obligation::Start(2)), _)
    ));
    f.cosign(&[0, 1], t0 + 20);

    // zenith has no cosigners: the operator logs a dispute, which only the log's key signs.
    let receipt = f.registry.request(dispute.as_bytes(), false).unwrap();
    let disputed = f.logged(&owner_chain("zenith"), receipt);
    assert_eq!(disputed, 3);
    f.cosign(&[0, 1], t0 + 30);

    // Before the deadline, nothing signed by the new keys is logged: it waits.
    let early = f
        .submit("acme", &format!("add {}", key_text(&s.1)), 1, &[&r, &s])
        .unwrap_err();
    assert_eq!(early.cause(), Some(&Cause::ReliesOnPending(2)), "{early:?}");

    // The early cosignatures of the dispute's start checkpoint were held back, since it may
    // have become the recovery's deadline; they count for its obligation.
    assert!(
        f.checkpoint_note(4)
            .cosignature(&f.keys.witnesses[0].1)
            .is_none()
    );
    // Seven days of witness time later, a late cosignature of that checkpoint is taken.
    let later = t0 + 30 + RECOVERY_DELAY + 100;
    f.set_time(later);
    f.cosign_at(4, 2, later).unwrap();
    assert_eq!(f.register("orbit", &[&o], &[]), 4);
    // The log grew past it: its held lines are published, and the dispute's start is fixed.
    assert!(
        f.checkpoint_note(4)
            .cosignature(&f.keys.witnesses[0].1)
            .is_some()
    );
    assert_eq!(
        f.registry
            .request_timing(disputed)
            .and_then(|timed| timed.start),
        Some(t0 + 30)
    );
    // The newest checkpoint may now be both deadlines, or the one before them: it carries the
    // threshold before the log grows.
    assert_eq!(
        (
            f.registry.deadline_of(recovery),
            f.registry.deadline_of(disputed)
        ),
        (None, None)
    );
    let waiting = f.submit("orbit", "cancel 4", 1, &[&o]).unwrap_err();
    assert_eq!(
        waiting.cause(),
        Some(&Cause::Obligation(Obligation::BeforeDeadline(2))),
        "{waiting:?}"
    );
    // The threshold of late cosignatures makes it the deadline of both, as they arrive.
    f.cosign(&[0, 1], later);
    assert_eq!(
        (
            f.registry.deadline_of(recovery),
            f.registry.deadline_of(disputed)
        ),
        (Some(5), Some(5))
    );

    // From the deadline on, the new keys govern.
    assert_eq!(
        f.submit("acme", &format!("add {}", key_text(&s.1)), 1, &[&r, &s])
            .unwrap(),
        5
    );
    assert_eq!(
        f.submit("zenith", &format!("add {}", key_text(&e.1)), 1, &[&d, &e])
            .unwrap(),
        6
    );

    // The witness of seed 32 is rotated out as compromised, its key listed as revoked.
    let rotated = keypair(95, "witness.rotated.2");
    let compromised = f.keys.witnesses[2].1.clone();
    let rotation = Rotation::new(
        EXPIRES,
        vec![
            f.keys.witnesses[0].1.clone(),
            f.keys.witnesses[1].1.clone(),
            rotated.1.clone(),
        ],
        2,
    )
    .compromise(vec![compromised.clone()]);
    let text = f.registry.rotation_text(&rotation).unwrap();
    assert!(text.contains(&format!(
        "witnesses-revoked {}\n",
        fingerprint(&compromised.public)
    )));
    // Rotating a compromised witness out unlisted needs the operator's confirmation.
    let mut unlisted = rotation.clone();
    unlisted.list_compromised = false;
    assert_eq!(
        f.registry.rotation_text(&unlisted).unwrap_err().cause(),
        Some(&Cause::CompromisedUnlisted)
    );
    unlisted.confirmed_unlisted = true;
    assert!(
        !f.registry
            .rotation_text(&unlisted)
            .unwrap()
            .contains("witnesses-revoked")
    );
    // Raising the threshold to three would leave kept checkpoints short: refused.
    let mut raised = rotation.clone();
    raised.threshold = 3;
    let short = f.registry.rotation_text(&raised).unwrap_err();
    assert_eq!(short.cause(), Some(&Cause::ThresholdShort), "{short:?}");
    let document = signed(&text, &[&f.keys.roots[0], &f.keys.roots[1]]);
    f.registry.install_root(&document, false).unwrap();
    f.keys.witnesses[2] = rotated;
    // A revoked key is never a witness again.
    let back = Rotation::new(
        EXPIRES,
        vec![f.keys.witnesses[0].1.clone(), compromised.clone()],
        1,
    );
    assert_eq!(
        f.registry.rotation_text(&back).unwrap_err().cause(),
        Some(&Cause::RevokedWitness)
    );
    // The next size is signed under the new root; the older checkpoints are kept as they are.
    assert_eq!(f.submit("orbit", "cancel 4", 1, &[&o]).unwrap(), 7);
    let named = |text: &str| {
        let checkpoint = una_project::tlog::note::parse_checkpoint(text).unwrap();
        una_project::tlog::root::named(&checkpoint.extensions)
            .unwrap()
            .0
    };
    assert_eq!(named(&f.checkpoint_text(8)), 2);
    assert_eq!(named(&f.checkpoint_text(5)), 1);
    f.cosign(&[0, 1, 2], later + 10);

    // The client reads Itaty's store as a mirror and verifies everything with its own code.
    let directory = Directory::new(&f.dir);
    let mut client = Client::open("itaty", &directory, &f.pin, &Point::Served, None).unwrap();
    assert_eq!(client.point.checkpoint.size, 8);
    assert_eq!(client.recovery_effect(recovery).unwrap(), Some(5));
    assert_eq!(client.earliest_effect(recovery).unwrap(), Some(5));
    assert_eq!(client.recovery_effect(disputed).unwrap(), Some(5));
    for owner in ["acme", "zenith", "orbit"] {
        let theirs = client.history(owner).unwrap();
        let ours = f.registry.owner_history(owner).unwrap();
        assert_eq!(verdicts(&theirs), verdicts(&ours), "{owner}");
    }
    // The model's verdicts: each new key governs from the deadline, alone, then with the key
    // it added; acme keeps the cosigners in effect when its recovery was logged, and zenith's
    // dispute leaves none.
    let acme = client.history("acme").unwrap();
    let keys_at = |history: &History, at: u64| -> Vec<String> {
        history
            .at(at)
            .unwrap()
            .keys
            .iter()
            .map(|key| key.name.clone())
            .collect()
    };
    assert_eq!(keys_at(&acme, 4), vec!["acme.a"]);
    assert_eq!(keys_at(&acme, 5), vec!["acme.r"]);
    assert_eq!(keys_at(&acme, 6), vec!["acme.r", "acme.s"]);
    assert_eq!(
        acme.cosigners.as_ref().map(|(k, list)| (*k, list.len())),
        Some((1, 1))
    );
    assert!(acme.pending.is_empty() && acme.frozen.is_none());
    let zenith = client.history("zenith").unwrap();
    assert_eq!(keys_at(&zenith, 5), vec!["zenith.d"]);
    assert_eq!(keys_at(&zenith, 7), vec!["zenith.d", "zenith.e"]);
    assert!(zenith.cosigners.is_none());
    assert!(client.misbehavior().is_empty());

    // Every key event, recovery and dispute gave exactly one notice; registrations none.
    let notices = f.recording.notices();
    let kinds: Vec<(u64, &str)> = notices
        .iter()
        .map(|notice| (notice.index, notice.kind.as_str()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (2, "recover"),
            (3, "dispute"),
            (5, "add"),
            (6, "add"),
            (7, "cancel")
        ]
    );
    assert!(notices.iter().all(|notice| notice.source == "itaty"));
    assert_eq!(notices[0].owner, "acme");
}

#[test]
fn entries_the_clients_replay_refuses_are_never_logged() {
    let mut f = Fixture::new("refusals");
    let a = keypair(60, "acme.a");
    let t = keypair(66, "acme.t");
    let x = keypair(67, "acme.x");
    f.register("acme", &[&a], &[]);

    // Not signed as its kind requires: a key added signs, and so does the owner's threshold.
    assert_eq!(
        f.submit("acme", &format!("add {}", key_text(&x.1)), 1, &[&x])
            .unwrap_err()
            .cause(),
        Some(&Cause::ReplayRefused)
    );
    // Fixed thresholds: a cancellation states the current one.
    assert_eq!(
        f.submit("acme", "cancel 0", 2, &[&a]).unwrap_err().cause(),
        Some(&Cause::StatedThreshold)
    );
    // A key's name is never used twice.
    let renamed = keypair(68, "acme.a");
    assert_eq!(
        f.submit(
            "acme",
            &format!("add {}", key_text(&renamed.1)),
            1,
            &[&a, &renamed]
        )
        .unwrap_err()
        .cause(),
        Some(&Cause::KeyReused)
    );
    // A recovery or a dispute goes through the operator, never `submit`.
    assert_eq!(
        f.submit("acme", &format!("dispute {}", key_text(&x.1)), 1, &[&a])
            .unwrap_err()
            .cause(),
        Some(&Cause::OperatorOnly)
    );

    // A chain that would not settle: `T`, added by `A`, alone claims `A` compromised since 0.
    f.submit("acme", &format!("add {}", key_text(&t.1)), 1, &[&a, &t])
        .unwrap();
    let unsettled = f
        .submit("acme", "revoke acme.a since 0", 1, &[&t])
        .unwrap_err();
    assert_eq!(unsettled.cause(), Some(&Cause::Unsettled), "{unsettled:?}");
}

#[test]
fn void_entries_are_never_logged() {
    let mut f = Fixture::new("void");
    let a = keypair(60, "acme.a");
    let c = keypair(61, "trustee.c");
    let x = keypair(67, "acme.x");
    f.register("acme", &[&a], &[&c]);
    // The cosigners' claim on the owner's last key: what it signs after 0 does not count.
    f.submit("acme", "revoke acme.a since 0", 1, &[&c]).unwrap();
    // So an addition signed by it alone would be void at its position: refused.
    let void = f
        .submit("acme", &format!("add {}", key_text(&x.1)), 1, &[&a, &x])
        .unwrap_err();
    assert_eq!(void.cause(), Some(&Cause::Void), "{void:?}");
    // Itaúna has no chain, and so no recovery or dispute.
    let itauna = format!(
        "ita-key/1\nsource itaty\nowner itauna\nprevious 0 {}\ndispute {}\nthreshold 1\n",
        digest_string(&[0u8; 32]),
        key_text(&x.1)
    );
    assert_eq!(
        f.registry
            .request(itauna.as_bytes(), false)
            .unwrap_err()
            .cause(),
        Some(&Cause::ItaunaHasNoChain)
    );
}

#[test]
fn every_multiple_of_64_carries_the_threshold_and_every_event_one_notice() {
    let mut f = Fixture::new("cadence");
    let a = keypair(60, "acme.a");
    f.register("acme", &[&a], &[]);
    for _ in 1..64 {
        f.submit("acme", "cancel 0", 1, &[&a]).unwrap();
    }
    assert_eq!(f.registry.size(), 64);
    let waiting = f.submit("acme", "cancel 0", 1, &[&a]).unwrap_err();
    assert_eq!(
        waiting.cause(),
        Some(&Cause::Obligation(Obligation::Cadence)),
        "{waiting:?}"
    );
    assert!(f.registry.waiting().is_some());
    // One witness is not enough; the second completes the threshold.
    f.cosign(&[0], NOW);
    assert_eq!(
        f.submit("acme", "cancel 0", 1, &[&a]).unwrap_err().cause(),
        Some(&Cause::Obligation(Obligation::Cadence))
    );
    f.cosign(&[1], NOW);
    assert!(f.registry.waiting().is_none());
    f.submit("acme", "cancel 0", 1, &[&a]).unwrap();
    // Each key event gave exactly one notice, each of its own index.
    let indices: Vec<u64> = f.recording.notices().iter().map(|n| n.index).collect();
    assert_eq!(indices, (1..=64).collect::<Vec<u64>>());
}

#[test]
fn a_service_starts_only_with_a_named_outbox() {
    let f = Fixture::new("outbox");
    let shared: Arc<dyn Store> = f.store.clone();
    let config = Config {
        pin: f.pin.clone(),
        log_seed: [20; 32],
    };
    let refused = Service::new(
        shared.clone(),
        Registry::open(
            shared.clone(),
            &config,
            f.clock.clone(),
            f.recording.clone(),
        )
        .unwrap(),
        ServerConfig::default(),
    )
    .unwrap_err();
    assert_eq!(refused, StartError::NoNotifier);
    let outbox = temp_dir("outbox-file");
    let service = Service::new(
        shared.clone(),
        Registry::open(shared, &config, f.clock.clone(), f.recording.clone()).unwrap(),
        ServerConfig {
            outbox: Some(outbox.clone()),
            ..ServerConfig::default()
        },
    )
    .unwrap();
    let a = keypair(60, "acme.a");
    let registration = f.registration("acme", &[&a], 1, &[]);
    let mut registry = service.registry().lock().unwrap();
    let receipt = registry
        .register(signed(&registration, &[&a]).as_bytes(), false)
        .unwrap();
    let event = format!(
        "ita-key/1\nsource itaty\nowner acme\nprevious 0 {}\ncancel 0\nthreshold 1\n",
        digest_string(&receipt.leaf)
    );
    registry.submit(signed(&event, &[&a]).as_bytes()).unwrap();
    // The service delivers notices outside the registry's lock; held here, flush them.
    registry.flush_notices();
    drop(registry);
    let lines = std::fs::read_to_string(&outbox).unwrap();
    assert_eq!(lines.lines().count(), 1, "{lines}");
    assert!(
        lines.starts_with("ita-notice/1 itaty acme 1 cancel sha256:"),
        "{lines}"
    );
    let _ = std::fs::remove_file(&outbox);
}

#[test]
fn a_rotation_that_moves_a_waiting_start_needs_confirmation_and_keeps_deadlines() {
    let mut f = Fixture::new("rotation-start");
    let a = keypair(60, "acme.a");
    let c = keypair(61, "trustee.c");
    let r = keypair(63, "acme.r");
    let z = keypair(62, "zenith.z");
    let o = keypair(67, "orbit.o");
    let t0 = NOW;
    f.register("acme", &[&a], &[&c]);
    let recover = f.event("acme", &format!("recover {}", key_text(&r.1)), 1);
    let receipt = f
        .registry
        .request(signed(&recover, &[&c]).as_bytes(), false)
        .unwrap();
    let recovery = f.logged(&owner_chain("acme"), receipt);
    // Its start, the second earliest of three cosignatures, is t0 + 20.
    f.cosign_at(2, 0, t0 + 10).unwrap();
    f.cosign_at(2, 1, t0 + 20).unwrap();
    f.cosign_at(2, 2, t0 + 30).unwrap();
    assert_eq!(
        f.registry
            .request_timing(recovery)
            .and_then(|timed| timed.start),
        Some(t0 + 20)
    );
    f.register("zenith", &[&z], &[]);

    // Revoking the witness of seed 31 leaves t0 + 10 and t0 + 30: the start moves to t0 + 30.
    let compromised = f.keys.witnesses[1].1.clone();
    let incoming = keypair(96, "witness.rotated.1");
    let rotation = Rotation::new(
        EXPIRES,
        vec![
            f.keys.witnesses[0].1.clone(),
            f.keys.witnesses[2].1.clone(),
            incoming.1.clone(),
        ],
        2,
    )
    .compromise(vec![compromised]);
    let text = f.registry.rotation_text(&rotation).unwrap();
    let document = signed(&text, &[&f.keys.roots[0], &f.keys.roots[1]]);
    assert_eq!(
        f.registry
            .install_root(&document, false)
            .unwrap_err()
            .cause(),
        Some(&Cause::StartMoves(recovery))
    );
    f.registry.install_root(&document, true).unwrap();
    f.keys.witnesses[1] = incoming;
    assert_eq!(
        f.registry
            .request_timing(recovery)
            .and_then(|timed| timed.start),
        Some(t0 + 30)
    );

    // Between the old target and the new one, no deadline is fixed.
    let between = t0 + 25 + RECOVERY_DELAY;
    f.set_time(between);
    f.register("orbit", &[&o], &[]);
    assert_eq!(f.registry.deadline_of(recovery), None);
    f.cosign(&[0, 2], between);
    let later = t0 + 30 + RECOVERY_DELAY + 100;
    f.set_time(later);
    f.submit("orbit", "cancel 3", 1, &[&o]).unwrap();
    assert_eq!(f.registry.deadline_of(recovery), None);
    f.cosign(&[0, 2], later);
    assert_eq!(f.registry.deadline_of(recovery), Some(5));

    // Opened again, the registry keeps its starts and deadlines; the client agrees.
    let reopened = f.reopen();
    assert_eq!(
        reopened.request_timing(recovery),
        f.registry.request_timing(recovery)
    );
    let directory = Directory::new(&f.dir);
    let mut client = Client::open("itaty", &directory, &f.pin, &Point::Served, None).unwrap();
    assert_eq!(client.recovery_effect(recovery).unwrap(), Some(5));
    assert_eq!(client.earliest_effect(recovery).unwrap(), Some(5));
    assert!(client.misbehavior().is_empty());
}

/// A notifier that takes nothing, as an outbox that cannot be written.
struct Failing;

impl Notifier for Failing {
    fn notify(&self, _: &Notice) -> Result<(), String> {
        Err("the outbox cannot be written".to_owned())
    }
}

#[test]
fn notices_not_taken_before_a_crash_are_sent_again_on_reopening() {
    let mut f = Fixture::new("crash");
    let a = keypair(60, "acme.a");
    f.register("acme", &[&a], &[]);
    f.submit("acme", "cancel 0", 1, &[&a]).unwrap();
    assert_eq!(f.registry.notice_status().notified, Some(1));
    // The next two entries are logged, but their notices are not taken.
    f.registry.set_notifier(Arc::new(Failing));
    f.submit("acme", "cancel 0", 1, &[&a]).unwrap();
    f.submit("acme", "cancel 0", 1, &[&a]).unwrap();
    let status = f.registry.notice_status().clone();
    assert_eq!(
        (status.notified, status.failures, status.behind),
        (Some(1), 2, true)
    );
    assert!(status.last_error.is_some());
    // Opened again, as after a crash: each notice after the cursor is sent, once more.
    let recording = Arc::new(Recording::new());
    let mut reopened = f.reopen_with(recording.clone());
    // Opening sends nothing; the notices are sent once the notifier that serves is set.
    assert!(recording.notices().is_empty());
    reopened.resend_notices();
    let indices: Vec<u64> = recording
        .notices()
        .iter()
        .map(|notice| notice.index)
        .collect();
    assert_eq!(indices, vec![2, 3]);
    let status = reopened.notice_status();
    assert_eq!((status.notified, status.failures), (Some(3), 2));
}

#[test]
fn a_request_that_would_freeze_the_chain_needs_the_operators_confirmation() {
    let mut f = Fixture::new("competing");
    let z = keypair(62, "zenith.z");
    let d = keypair(64, "zenith.d");
    let e = keypair(66, "zenith.e");
    f.register("zenith", &[&z], &[]);
    let first = f.event("zenith", &format!("dispute {}", key_text(&d.1)), 1);
    let receipt = f.registry.request(first.as_bytes(), false).unwrap();
    f.logged(&owner_chain("zenith"), receipt);
    f.cosign(&[0, 1], NOW);
    // A second dispute while the first waits competes with it: the chain would freeze.
    let second = f.event("zenith", &format!("dispute {}", key_text(&e.1)), 1);
    assert_eq!(
        f.registry
            .request(second.as_bytes(), false)
            .unwrap_err()
            .cause(),
        Some(&Cause::CompetingRequest)
    );
    assert_eq!(f.registry.size(), 2);
    // With the operator's explicit confirmation, it is logged, and the chain is frozen there.
    let receipt = f.registry.request(second.as_bytes(), true).unwrap();
    assert_eq!(receipt.index, 2);
    assert_eq!(f.registry.owner_history("zenith").unwrap().frozen, Some(2));
}

#[test]
fn one_co_signer_change_waits_at_a_time_and_only_real_cancellations_are_timed() {
    let mut f = Fixture::new("cosigners");
    let a = keypair(60, "acme.a");
    let x = keypair(70, "trustee.x");
    let y = keypair(71, "trustee.y");
    f.register("acme", &[&a], &[]);
    // A cancellation of nothing waiting is not timed.
    let nothing = f.submit("acme", "cancel 0", 1, &[&a]).unwrap();
    assert!(f.registry.request_timing(nothing).is_none());
    let change = f
        .submit("acme", &format!("cosigners 1 {}", key_text(&x.1)), 1, &[&a])
        .unwrap();
    assert!(f.registry.request_timing(change).is_some());
    f.cosign(&[0, 1], NOW);
    // A second co-signer change while the first waits is refused.
    assert_eq!(
        f.submit("acme", &format!("cosigners 1 {}", key_text(&y.1)), 1, &[&a])
            .unwrap_err()
            .cause(),
        Some(&Cause::PendingCosignerChange)
    );
    // A cancellation of the waiting change is timed: its seven days bound an overruling.
    let cancel = f
        .submit("acme", &format!("cancel {change}"), 1, &[&a])
        .unwrap();
    assert!(f.registry.request_timing(cancel).is_some());
    f.cosign(&[0, 1], NOW);
    // Reopened, the same cancellations are timed.
    let reopened = f.reopen();
    assert!(reopened.request_timing(cancel).is_some());
    assert!(reopened.request_timing(nothing).is_none());
    // Once cancelled, a new co-signer change may wait.
    f.submit("acme", &format!("cosigners 1 {}", key_text(&y.1)), 1, &[&a])
        .unwrap();
}

#[test]
fn settled_requests_leave_the_scans() {
    let mut f = Fixture::new("prune");
    let z = keypair(62, "zenith.z");
    let d = keypair(64, "zenith.d");
    let o = keypair(67, "orbit.o");
    f.register("zenith", &[&z], &[]);
    let dispute = f.event("zenith", &format!("dispute {}", key_text(&d.1)), 1);
    let receipt = f.registry.request(dispute.as_bytes(), false).unwrap();
    let disputed = f.logged(&owner_chain("zenith"), receipt);
    f.cosign(&[0, 1], NOW);
    f.register("orbit", &[&o], &[]);
    assert_eq!(f.registry.active_requests_count(), 1);
    let later = NOW + RECOVERY_DELAY + 100;
    f.set_time(later);
    // The newest checkpoint reaches the target with late cosignatures: it is the deadline.
    f.cosign(&[0, 1], later);
    let deadline = f.registry.deadline_of(disputed).unwrap();
    assert_eq!(deadline, 3);
    f.submit("orbit", "cancel 2", 1, &[&o]).unwrap();
    // The deadline is fixed, but its pinned multiple of 64 is still ahead: it stays active.
    assert_eq!(f.registry.active_requests_count(), 1);
    while f.registry.size() < 64 {
        f.submit("orbit", "cancel 2", 1, &[&o]).unwrap();
    }
    assert!(deadline < 64);
    f.cosign(&[0, 1], later);
    f.submit("orbit", "cancel 2", 1, &[&o]).unwrap();
    assert_eq!(f.registry.active_requests_count(), 0);
}

#[cfg(unix)]
#[test]
fn an_outbox_is_never_a_link() {
    let target = temp_dir("outbox-target");
    std::fs::write(&target, b"").unwrap();
    let link = temp_dir("outbox-link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(una_itaty::notify::Outbox::open(&link).is_err());
    let directory = temp_dir("outbox-dir");
    std::fs::create_dir_all(&directory).unwrap();
    assert!(una_itaty::notify::Outbox::open(&directory).is_err());
    let _ = std::fs::remove_file(&link);
    let _ = std::fs::remove_file(&target);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_store_has_one_writer() {
    let f = Fixture::new("lock");
    // The fixture's registry holds the store: a second writer is refused.
    assert!(matches!(
        FileStore::open(&f.dir),
        Err(una_itaty::store::OpenError::Locked)
    ));
}

#[test]
fn a_notice_names_its_entry_by_index_and_the_digest_of_its_bytes() {
    use sha2::{Digest, Sha256};
    let mut f = Fixture::new("notice-link");
    let a = keypair(60, "acme.a");
    f.register("acme", &[&a], &[]);
    let document = signed(&f.event("acme", "cancel 0", 1), &[&a]);
    let receipt = f.registry.submit(document.as_bytes()).unwrap();
    let notice = f.recording.notices().pop().unwrap();
    assert_eq!(notice.index, receipt.index);
    assert_eq!(
        notice.digest,
        digest_string(&Sha256::digest(document.as_bytes()).into())
    );
    assert_eq!(notice.leaf, digest_string(&receipt.leaf));
    assert_eq!(notice.link, "tile/entries/000");
}

#[test]
fn owner_checks_refuse_by_their_cause() {
    let mut f = Fixture::new("owner-checks");
    let a = keypair(60, "acme.a");
    let c = keypair(61, "trustee.c");
    let d = keypair(64, "acme.d");
    f.register("acme", &[&a], &[&c]);
    // An owner with recovery cosigners in effect has no dispute.
    let dispute = f.event("acme", &format!("dispute {}", key_text(&d.1)), 1);
    assert_eq!(
        f.registry
            .request(dispute.as_bytes(), false)
            .unwrap_err()
            .cause(),
        Some(&Cause::DisputeWithCosigners)
    );
    // `since` at the entry's own position, and past it.
    for since in [1, 2] {
        assert_eq!(
            f.submit("acme", &format!("revoke acme.a since {since}"), 1, &[&a])
                .unwrap_err()
                .cause(),
            Some(&Cause::SinceNotReached),
            "{since}"
        );
    }
    // Another owner's key, under another name.
    let stolen = keypair(60, "zenith.a");
    let theirs = f.registration("zenith", &[&stolen], 1, &[]);
    assert_eq!(
        f.registry
            .register(signed(&theirs, &[&stolen]).as_bytes(), false)
            .unwrap_err()
            .cause(),
        Some(&Cause::KeyOfAnotherOwner)
    );
    // A registration naming one of its own keys as a cosigner.
    let o = keypair(67, "orbit.o");
    let own = f.registration("orbit", &[&o], 1, &[&o]);
    assert_eq!(
        f.registry
            .register(signed(&own, &[&o]).as_bytes(), false)
            .unwrap_err()
            .cause(),
        Some(&Cause::CosignerIsOwnKey)
    );
    assert_eq!(f.registry.size(), 1);
}

/// A key of its own for each `n`, beyond the shared builder's seeds.
fn keypair_n(n: u32, name: &str) -> Pair {
    let mut seed = [7u8; 32];
    seed[..4].copy_from_slice(&n.to_be_bytes());
    let pair = SigningKey::from_bytes(&seed);
    let public = pair.verifying_key().to_bytes();
    (
        pair,
        Key {
            name: name.to_owned(),
            public,
        },
    )
}

/// An owner name for `n`, of letters no skeleton confuses.
fn owner_name(n: usize) -> String {
    let letters: Vec<char> = "acdefhjkpx".chars().collect();
    let mut name = String::from("t");
    let mut rest = n;
    for _ in 0..3 {
        name.push(letters[rest % letters.len()]);
        rest /= letters.len();
    }
    name
}

/// A deterministic generator of numbers below a bound, for schedules.
struct Schedule(u64);

impl Schedule {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound.max(1)
    }
}

/// The keys an owner holds at each of `positions`.
fn keys_at(history: &History, positions: &[u64]) -> Vec<Vec<String>> {
    positions
        .iter()
        .map(|at| {
            history
                .at(*at)
                .map(|snapshot| snapshot.keys.iter().map(|key| key.name.clone()).collect())
                .unwrap_or_default()
        })
        .collect()
}

/// A request of `zenith` (a dispute) and a publisher, `orbit`: the dispute's start cosigned
/// at `NOW` by every witness. Returns the dispute's index and its target.
fn disputed_zenith(f: &mut Fixture) -> (u64, u64) {
    let z = keypair(62, "zenith.z");
    let d = keypair(64, "zenith.d");
    let o = keypair(67, "orbit.o");
    f.register("zenith", &[&z], &[]);
    let disputed = f.dispute("zenith", &d).unwrap();
    f.cosign(&[0, 1, 2], NOW);
    f.register("orbit", &[&o], &[]);
    (disputed, NOW + RECOVERY_DELAY)
}

/// A cadence checkpoint signed one second before a target, with every witness cosigning one
/// second after it, by clocks `skew` from the registry's, on a registry whose clock is
/// `behind` the witnesses': it reaches the threshold, it is the deadline, and the log grows.
fn cadence_just_before_a_target(k: usize, skew: u64, behind: u64) {
    let mut f = Fixture::with_k(&format!("cadence-{k}-{skew}-{behind}"), k);
    let o = keypair(67, "orbit.o");
    let (disputed, target) = disputed_zenith(&mut f);
    while f.registry.size() < 63 {
        f.submit("orbit", "cancel 2", 1, &[&o]).unwrap();
    }
    f.set_time(target - 1);
    f.submit("orbit", "cancel 2", 1, &[&o]).unwrap();
    assert_eq!(f.registry.size(), 64);
    f.set_time(target + 1 - behind);
    f.cosign(&[0, 1, 2], target + 1 + skew);
    assert_eq!(f.registry.deadline_of(disputed), Some(64));
    assert_eq!(f.registry.owed(), None);
    f.submit("orbit", "cancel 2", 1, &[&o]).unwrap();
}

#[test]
fn a_cadence_checkpoint_just_before_a_target_never_freezes_the_log() {
    for k in [1, 2] {
        cadence_just_before_a_target(k, 0, 0);
    }
    // A witness clock an hour fast; the registry's clock an hour behind.
    cadence_just_before_a_target(2, 3600, 0);
    cadence_just_before_a_target(2, 0, 3600);
}

#[test]
fn a_start_checkpoint_just_before_a_target_never_freezes_the_log() {
    for k in [1, 2] {
        let mut f = Fixture::with_k(&format!("start-{k}"), k);
        let (disputed, target) = disputed_zenith(&mut f);
        // Another request's start checkpoint is signed one second before the target.
        f.set_time(target - 1);
        let other = f.dispute("orbit", &keypair(68, "orbit.d")).unwrap();
        assert_eq!(
            f.registry.owed().map(|(obligation, _)| obligation),
            Some(Obligation::Start(other))
        );
        // Cosigned one second after the target, it reaches it: the first request's deadline,
        // and the second's start.
        f.set_time(target + 1);
        f.cosign(&[0, 1, 2], target + 1);
        assert_eq!(f.registry.deadline_of(disputed), Some(other + 1));
        assert_eq!(
            f.registry
                .request_timing(other)
                .and_then(|timed| timed.start),
            Some(target + 1)
        );
        assert_eq!(f.registry.owed(), None);
        let p = keypair(69, "papa.p");
        f.register("papa", &[&p], &[]);
    }
}

#[test]
fn no_schedule_freezes_the_log() {
    let mut schedule = Schedule(0x1144);
    for round in 0..6u64 {
        let k = 1 + (round % 3) as usize;
        let mut f = Fixture::with_k(&format!("property-{round}"), k);
        let o = keypair(67, "orbit.o");
        disputed_zenith(&mut f);
        let mut now = NOW;
        let mut last = [NOW; 3];
        for _ in 0..60 {
            now += schedule.below(6 * 3600);
            f.set_time(now);
            // Witnesses cosign the newest checkpoint by their own clocks, an hour fast or slow.
            let size = f.registry.size();
            for (witness, seen) in last.iter_mut().enumerate() {
                let skew = schedule.below(7200);
                let at = (now + skew).saturating_sub(3600).max(*seen);
                if at <= now + 3600 && f.cosign_at(size, witness, at).is_ok() {
                    *seen = at;
                }
            }
            // The log tries to grow: only an obligation owed may refuse it.
            if let Err(refusal) = f.submit("orbit", "cancel 2", 1, &[&o]) {
                assert!(
                    matches!(refusal.cause(), Some(Cause::Obligation(_))),
                    "{refusal:?}"
                );
                // With the witnesses' clocks caught up, two rounds always complete it.
                for _ in 0..2 {
                    now += 2 * 3600;
                    f.set_time(now);
                    let size = f.registry.size();
                    for (witness, seen) in last.iter_mut().enumerate() {
                        let at = now.max(*seen);
                        if f.cosign_at(size, witness, at).is_ok() {
                            *seen = at;
                        }
                    }
                }
                assert_eq!(f.registry.owed(), None, "round {round}");
            }
        }
    }
}

#[test]
fn an_early_line_on_a_checkpoint_that_may_become_the_deadline_is_not_served() {
    let mut f = Fixture::new("held");
    let o = keypair(67, "orbit.o");
    let (disputed, target) = disputed_zenith(&mut f);
    f.set_time(target - 10);
    // The newest checkpoint is above the dispute, whose deadline is not fixed: an early line
    // is taken, but neither published nor served.
    f.cosign_at(3, 0, target - 10).unwrap();
    assert!(
        f.checkpoint_note(3)
            .cosignature(&f.keys.witnesses[0].1)
            .is_none()
    );
    // A late line is published, and with one more, the checkpoint would be the deadline; the
    // early line counts for its obligations.
    f.set_time(target + 10);
    f.cosign_at(3, 1, target + 10).unwrap();
    assert!(
        f.checkpoint_note(3)
            .cosignature(&f.keys.witnesses[1].1)
            .is_some()
    );
    assert_eq!(f.registry.owed(), None);
    // The log grows: the checkpoint is not the deadline, and its held line is published.
    f.submit("orbit", "cancel 2", 1, &[&o]).unwrap();
    assert!(
        f.checkpoint_note(3)
            .cosignature(&f.keys.witnesses[0].1)
            .is_some()
    );
    assert_eq!(f.registry.deadline_of(disputed), None);
    // It carries an early line now: rule 1 refuses the late line that would make it reach
    // the target.
    assert_eq!(
        f.cosign_at(3, 2, target + 10).unwrap_err().cause(),
        Some(&Cause::CrossesTarget(
            una_itaty::timing::Crossing::EarlyThenLate
        ))
    );
}

#[test]
fn no_owner_stalls_the_registry_and_the_bucket_spares_owners_own_requests() {
    let mut f = Fixture::new("stall");
    let o = keypair(67, "orbit.o");
    let trustee = keypair(70, "trustee.x");
    let orbit = f.register("orbit", &[&o], &[]);
    let owners: Vec<(String, Pair)> = (0..209)
        .map(|n| {
            let name = owner_name(n);
            let key = keypair_n(n as u32, &format!("{name}.a"));
            (name, key)
        })
        .collect();
    for (name, key) in &owners {
        f.register_reviewed(name, key);
    }
    let change = format!("cosigners 1 {}", key_text(&trustee.1));
    let mut now = NOW;
    let mut changes: Vec<u64> = Vec::new();
    // 200 owners each ask a co-signer change, a minute apart, between a publisher's entries.
    for (name, key) in owners.iter().take(200) {
        now += 60;
        f.set_time(now);
        let index = f.submit(name, &change, 1, &[key]).unwrap();
        changes.push(index);
        // The publisher waits only for the newest checkpoint's own obligation.
        let refusal = f
            .submit("orbit", &format!("cancel {orbit}"), 1, &[&o])
            .unwrap_err();
        assert_eq!(
            refusal.cause(),
            Some(&Cause::Obligation(Obligation::Start(index)))
        );
        f.cosign(&[0, 1], now);
        f.submit("orbit", &format!("cancel {orbit}"), 1, &[&o])
            .unwrap();
        f.cosign(&[0, 1], now);
    }
    // At once, 8 more co-signer changes drain the bucket; the ninth waits, its sender alone.
    for (name, key) in owners.iter().skip(200).take(8) {
        f.submit(name, &change, 1, &[key]).unwrap();
        f.cosign(&[0, 1], now);
    }
    let (name, key) = &owners[208];
    assert_eq!(
        f.submit(name, &change, 1, &[key]).unwrap_err().cause(),
        Some(&Cause::RateLimited)
    );
    f.submit("orbit", &format!("cancel {orbit}"), 1, &[&o])
        .unwrap();
    f.cosign(&[0, 1], now);
    // The bucket drained, owners still get a dispute and a recovery through the operator.
    let disputed = f
        .dispute(&owners[0].0, &keypair_n(1_000, "t.disputed"))
        .unwrap();
    f.cosign(&[0, 1], now);
    let recover = f.event(
        &owners[1].0,
        &format!("recover {}", key_text(&keypair_n(1_001, "t.recovered").1)),
        1,
    );
    let receipt = f.registry.request(recover.as_bytes(), false).unwrap();
    f.logged(&owner_chain(&owners[1].0), receipt);
    f.cosign(&[0, 1], now);
    assert!(f.registry.request_timing(disputed).is_some());
    // Two hours before the first change's target, its owner's entries wait; no other's do.
    let first = f
        .registry
        .request_timing(changes[0])
        .and_then(|timed| timed.target())
        .unwrap();
    f.set_time(first - 3600);
    let (name, key) = &owners[0];
    assert_eq!(
        f.submit(name, &format!("cancel {}", changes[0]), 1, &[key])
            .unwrap_err()
            .cause(),
        Some(&Cause::OwnerHold(changes[0]))
    );
    f.submit("orbit", &format!("cancel {orbit}"), 1, &[&o])
        .unwrap();
}

/// Copies the files of a store directory.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Leaves only the log's signature on the checkpoint of `size` in the store directory `dir`.
fn strip(dir: &Path, size: u64) {
    let path = dir.join("checkpoint").join(size.to_string());
    let document = std::fs::read_to_string(&path).unwrap();
    let (text, lines) = document.split_once("\n\n").unwrap();
    let log = lines.lines().next().unwrap();
    std::fs::write(&path, format!("{text}\n\n{log}\n")).unwrap();
}

#[test]
fn itaty_and_the_client_agree_on_every_deadline() {
    let mut schedule = Schedule(0x0d0e);
    for k in 1..=3usize {
        let mut f = Fixture::with_k(&format!("agree-{k}"), k);
        let o = keypair(67, "orbit.o");
        let names = ["bravo", "delta", "hotel", "kilo"];
        let keys: Vec<Pair> = (0..4u32)
            .map(|n| keypair_n(2_000 + n, &format!("{}.a", names[n as usize])))
            .collect();
        for (name, key) in names.iter().zip(&keys) {
            f.register(name, &[key], &[]);
        }
        let orbit = f.register("orbit", &[&o], &[]);
        let mut disputes: Vec<(usize, u64)> = Vec::new();
        let mut now = NOW;
        let mut last = [NOW; 3];
        let mut step = 0;
        while disputes.len() < names.len()
            || disputes
                .iter()
                .any(|(_, index)| f.registry.deadline_of(*index).is_none())
        {
            step += 1;
            assert!(step < 400, "the schedule does not end");
            now += schedule.below(12 * 3600);
            f.set_time(now);
            let logged = if disputes.len() < names.len() && schedule.below(3) == 0 {
                let at = disputes.len();
                let key = keypair_n(3_000 + at as u32, &format!("{}.d", names[at]));
                f.dispute(names[at], &key)
                    .map(|index| disputes.push((at, index)))
            } else {
                f.submit("orbit", &format!("cancel {orbit}"), 1, &[&o])
                    .map(|_| ())
            };
            if let Err(refusal) = logged {
                assert!(
                    matches!(refusal.cause(), Some(Cause::Obligation(_))),
                    "{refusal:?}"
                );
            }
            let size = f.registry.size();
            for (witness, seen) in last.iter_mut().enumerate() {
                let at = (now + schedule.below(1200)).saturating_sub(600).max(*seen);
                if f.cosign_at(size, witness, at).is_ok() {
                    *seen = at;
                }
            }
            // Reopened mid-schedule, the registry keeps its starts and deadlines.
            if step == 40 {
                let before: Vec<_> = disputes
                    .iter()
                    .map(|(_, index)| f.registry.request_timing(*index).cloned())
                    .collect();
                f.registry = f.reopen();
                let after: Vec<_> = disputes
                    .iter()
                    .map(|(_, index)| f.registry.request_timing(*index).cloned())
                    .collect();
                assert_eq!(before, after);
            }
        }
        // One more entry and its checkpoint, so every deadline is below the point.
        loop {
            now += 3600;
            f.set_time(now);
            if f.submit("orbit", &format!("cancel {orbit}"), 1, &[&o])
                .is_ok()
            {
                break;
            }
            let size = f.registry.size();
            for (witness, seen) in last.iter_mut().enumerate() {
                let at = now.max(*seen);
                if f.cosign_at(size, witness, at).is_ok() {
                    *seen = at;
                }
            }
        }
        let size = f.registry.size();
        for (witness, seen) in last.iter_mut().enumerate() {
            let at = now.max(*seen);
            f.cosign_at(size, witness, at).unwrap();
            *seen = at;
        }
        let directory = Directory::new(&f.dir);
        let mut client = Client::open("itaty", &directory, &f.pin, &Point::Served, None).unwrap();
        for (_, index) in &disputes {
            let ours = f.registry.deadline_of(*index);
            assert_eq!(
                client.recovery_effect(*index).unwrap(),
                ours,
                "k {k}, {index}"
            );
            assert_eq!(
                client.earliest_effect(*index).unwrap(),
                ours,
                "k {k}, {index}"
            );
        }
        for name in names {
            let theirs = client.history(name).unwrap();
            let ours = f.registry.owner_history(name).unwrap();
            assert_eq!(verdicts(&theirs), verdicts(&ours), "k {k}, {name}");
        }
        assert!(client.misbehavior().is_empty());

        // A mirror that strips every line from each deadline and the first pinned checkpoint
        // after it: read beside the registry, the same verdicts; alone, the same keys, or
        // unavailable.
        let stripped = temp_dir(&format!("stripped-{k}"));
        copy_dir(&f.dir, &stripped);
        for (_, index) in &disputes {
            let deadline = f.registry.deadline_of(*index).unwrap();
            strip(&stripped, deadline);
            let pinned = deadline.div_ceil(64) * 64;
            if pinned < size {
                strip(&stripped, pinned);
            }
        }
        let beside = Fallback::new(
            Box::new(Directory::new(&stripped)),
            vec![Box::new(Directory::new(&f.dir))],
        );
        let mut merged = Client::open("itaty", &beside, &f.pin, &Point::Served, None).unwrap();
        for (_, index) in &disputes {
            assert_eq!(
                merged.recovery_effect(*index).unwrap(),
                f.registry.deadline_of(*index)
            );
        }
        let alone = Directory::new(&stripped);
        let mut lone = Client::open("itaty", &alone, &f.pin, &Point::Served, None).unwrap();
        for name in names {
            let ours = f.registry.owner_history(name).unwrap();
            match lone.history(name) {
                Ok(theirs) => assert_eq!(
                    keys_at(&theirs, &[size]),
                    keys_at(&ours, &[size]),
                    "k {k}, {name}"
                ),
                Err(_) => assert_eq!(lone.take_failure(), Some(Failure::Unserved)),
            }
        }
        let _ = std::fs::remove_dir_all(&stripped);
    }
}

/// The size of the checkpoint served as `checkpoint`.
fn served_size(f: &Fixture) -> u64 {
    let bytes = f.store.get("checkpoint", 1 << 20).unwrap().unwrap();
    let note = parse_note(&String::from_utf8(bytes).unwrap()).unwrap();
    una_project::tlog::note::parse_checkpoint(&note.text)
        .unwrap()
        .size
}

#[test]
fn lines_late_for_one_request_and_early_for_another_fix_its_deadline_and_are_served() {
    let mut f = Fixture::new("two-targets");
    let o = keypair(67, "orbit.o");
    let (first, target) = disputed_zenith(&mut f);
    // A second request starts three days later: its target is four days after the first's.
    let second_start = NOW + 3 * 24 * 3600;
    f.set_time(second_start);
    let second = f.dispute("orbit", &keypair(68, "orbit.d")).unwrap();
    f.cosign(&[0, 1, 2], second_start);
    f.submit("orbit", "cancel 2", 1, &[&o]).unwrap();
    assert_eq!(
        f.registry
            .request_timing(second)
            .and_then(|timed| timed.start),
        Some(second_start)
    );
    f.cosign(&[0, 1, 2], second_start);
    // Between the two targets, every witness cosigns the newest checkpoint: late for the
    // first request, early for the second, so held; they fix the first's deadline.
    f.set_time(target + 10);
    f.submit("orbit", "cancel 2", 1, &[&o]).unwrap();
    let size = f.registry.size();
    f.cosign(&[0, 1, 2], target + 10);
    assert_eq!(f.registry.deadline_of(first), Some(size));
    assert!(
        f.checkpoint_note(size)
            .cosignature(&f.keys.witnesses[0].1)
            .is_none()
    );
    // The log grows: the deadline is served with every line, and the point advances.
    f.submit("orbit", "cancel 2", 1, &[&o]).unwrap();
    let served = f.checkpoint_note(size);
    assert!(
        f.keys
            .witnesses
            .iter()
            .all(|(_, key)| served.cosignature(key).is_some())
    );
    assert_eq!(served_size(&f), size);
    let directory = Directory::new(&f.dir);
    let mut client = Client::open("itaty", &directory, &f.pin, &Point::Served, None).unwrap();
    assert_eq!(client.recovery_effect(first).unwrap(), Some(size));
    assert_eq!(client.earliest_effect(first).unwrap(), Some(size));
    assert!(client.misbehavior().is_empty());
}

#[test]
fn concurrent_requests_keep_the_point_advancing() {
    let mut schedule = Schedule(0x0c0c);
    for k in 1..=3usize {
        let mut f = Fixture::with_k(&format!("concurrent-{k}"), k);
        let o = keypair(67, "orbit.o");
        let names = ["bravo", "delta", "hotel", "kilo", "lima"];
        let keys: Vec<Pair> = (0..5u32)
            .map(|n| keypair_n(4_000 + n, &format!("{}.a", names[n as usize])))
            .collect();
        for (name, key) in names.iter().zip(&keys) {
            f.register(name, &[key], &[]);
        }
        let orbit = f.register("orbit", &[&o], &[]);
        let mut disputes: Vec<u64> = Vec::new();
        let mut now = NOW;
        let mut last = [NOW; 3];
        let mut step = 0;
        while disputes.len() < names.len()
            || disputes
                .iter()
                .any(|index| f.registry.deadline_of(*index).is_none())
        {
            step += 1;
            assert!(step < 500, "the schedule does not end");
            now += schedule.below(10 * 3600);
            f.set_time(now);
            let logged = if disputes.len() < names.len() && schedule.below(3) == 0 {
                let at = disputes.len();
                let key = keypair_n(5_000 + at as u32, &format!("{}.d", names[at]));
                f.dispute(names[at], &key).map(|index| disputes.push(index))
            } else {
                f.submit("orbit", &format!("cancel {orbit}"), 1, &[&o])
                    .map(|_| ())
            };
            if let Err(refusal) = logged {
                assert!(
                    matches!(refusal.cause(), Some(Cause::Obligation(_))),
                    "{refusal:?}"
                );
            }
            let size = f.registry.size();
            for (witness, seen) in last.iter_mut().enumerate() {
                let at = now.max(*seen);
                if f.cosign_at(size, witness, at).is_ok() {
                    *seen = at;
                }
            }
            // The point never falls more than one checkpoint behind the newest.
            assert!(
                served_size(&f) + 1 >= f.registry.size(),
                "k {k}, step {step}"
            );
        }
        loop {
            now += 3600;
            f.set_time(now);
            let grown = f
                .submit("orbit", &format!("cancel {orbit}"), 1, &[&o])
                .is_ok();
            let size = f.registry.size();
            for (witness, seen) in last.iter_mut().enumerate() {
                let at = now.max(*seen);
                if f.cosign_at(size, witness, at).is_ok() {
                    *seen = at;
                }
            }
            if grown {
                break;
            }
        }
        let directory = Directory::new(&f.dir);
        let mut client = Client::open("itaty", &directory, &f.pin, &Point::Served, None).unwrap();
        for index in &disputes {
            let ours = f.registry.deadline_of(*index);
            assert_eq!(
                client.recovery_effect(*index).unwrap(),
                ours,
                "k {k}, {index}"
            );
            assert_eq!(
                client.earliest_effect(*index).unwrap(),
                ours,
                "k {k}, {index}"
            );
        }
        assert!(client.misbehavior().is_empty());
    }
}

#[test]
fn a_release_cut_short_by_a_crash_is_done_again_once() {
    let mut f = Fixture::new("release-crash");
    let o = keypair(67, "orbit.o");
    let (_, target) = disputed_zenith(&mut f);
    f.set_time(target - 10);
    f.cosign_at(3, 0, target - 10).unwrap();
    f.cosign_at(3, 1, target - 5).unwrap();
    let held = f.store.get("state/held", 1 << 20).unwrap().unwrap();
    f.submit("orbit", "cancel 2", 1, &[&o]).unwrap();
    // A crash after the checkpoint was written and before `state/held` was: put it back.
    f.store.put("state/held", &held).unwrap();
    let reopened = f.reopen();
    let note = f.checkpoint_note(3);
    // The log's signature and one line of each witness, no second line of one key.
    assert_eq!(note.signatures.len(), 3);
    assert!(
        f.store
            .get("state/held", 1 << 20)
            .unwrap()
            .is_none_or(|bytes| bytes.is_empty())
    );
    assert_eq!(reopened.size(), 4);
}

#[test]
fn notices_a_crash_left_reach_the_outbox_once_served() {
    let mut f = Fixture::new("crash-serve");
    let a = keypair(60, "acme.a");
    f.register("acme", &[&a], &[]);
    f.registry.set_notifier(Arc::new(Failing));
    f.submit("acme", "cancel 0", 1, &[&a]).unwrap();
    f.submit("acme", "cancel 0", 1, &[&a]).unwrap();
    // Opened again, as after a crash, with a stand-in notifier, then served with its outbox.
    let stand_in = Arc::new(Recording::new());
    let reopened = f.reopen_with(stand_in.clone());
    let outbox = temp_dir("crash-serve-outbox");
    let _service = Service::new(
        f.store.clone(),
        reopened,
        ServerConfig {
            outbox: Some(outbox.clone()),
            ..ServerConfig::default()
        },
    )
    .unwrap();
    assert!(stand_in.notices().is_empty());
    let lines = std::fs::read_to_string(&outbox).unwrap();
    let indices: Vec<&str> = lines
        .lines()
        .filter_map(|line| line.split(' ').nth(3))
        .collect();
    assert_eq!(indices, vec!["1", "2"], "{lines}");
    let _ = std::fs::remove_file(&outbox);
}

#[test]
fn one_owner_asks_two_co_signer_changes_a_week_and_a_cancelled_recovery_never_waits() {
    let mut f = Fixture::new("cap");
    let a = keypair(60, "acme.a");
    let c = keypair(61, "trustee.c");
    let x = keypair(70, "trustee.x");
    let r = keypair(63, "acme.r");
    f.register("acme", &[&a], &[&c]);
    let change = format!("cosigners 1 {}", key_text(&x.1));
    // One owner loops a co-signer change and its cancellation: the third change is refused.
    for _ in 0..2 {
        let asked = f.submit("acme", &change, 1, &[&a]).unwrap();
        f.cosign(&[0, 1], NOW);
        f.submit("acme", &format!("cancel {asked}"), 1, &[&a])
            .unwrap();
        f.cosign(&[0, 1], NOW);
    }
    assert_eq!(
        f.submit("acme", &change, 1, &[&a]).unwrap_err().cause(),
        Some(&Cause::CosignerChangeCap)
    );
    // Others drain the bucket.
    let mut drained = false;
    for n in 0..16u32 {
        let name = owner_name(300 + n as usize);
        let key = keypair_n(6_000 + n, &format!("{name}.a"));
        f.register_reviewed(&name, &key);
        match f.submit(&name, &change, 1, &[&key]) {
            Ok(_) => f.cosign(&[0, 1], NOW),
            Err(refusal) => {
                assert_eq!(refusal.cause(), Some(&Cause::RateLimited));
                drained = true;
                break;
            }
        }
    }
    assert!(drained);
    // A thief's recovery of acme, by its cosigner's key, is cancelled by acme with the bucket
    // drained.
    let recover = f.event("acme", &format!("recover {}", key_text(&r.1)), 1);
    let receipt = f
        .registry
        .request(signed(&recover, &[&c]).as_bytes(), false)
        .unwrap();
    let recovery = f.logged(&owner_chain("acme"), receipt);
    f.cosign(&[0, 1], NOW);
    let cancel = f
        .submit("acme", &format!("cancel {recovery}"), 1, &[&a])
        .unwrap();
    assert!(f.registry.request_timing(cancel).is_some());
}

#[test]
fn opening_recomputes_a_deadline_and_drops_a_phantom_cancellation() {
    let mut f = Fixture::new("reopen-state");
    let a = keypair(60, "acme.a");
    let (disputed, target) = disputed_zenith(&mut f);
    f.set_time(target + 10);
    f.cosign(&[0, 1], target + 10);
    assert_eq!(f.registry.deadline_of(disputed), Some(3));
    // A crash after the cosignature and before `state/deadlines`: opening fixes D again.
    f.store.put("state/deadlines", b"").unwrap();
    f.registry = f.reopen();
    assert_eq!(f.registry.deadline_of(disputed), Some(3));
    // A crash after `state/timed-cancels` named the next index and before `state/size`.
    f.register("acme", &[&a], &[]);
    let next = f.registry.size();
    f.store
        .put("state/timed-cancels", format!("{next}\n").as_bytes())
        .unwrap();
    f.registry = f.reopen();
    let cancel = f.submit("acme", "cancel 3", 1, &[&a]).unwrap();
    assert_eq!(cancel, next);
    assert!(f.registry.request_timing(cancel).is_none());
}
