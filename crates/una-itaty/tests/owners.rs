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
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ed25519_dalek::SigningKey;
use una_itaty::digest_string;
use una_itaty::entry::owner_chain;
use una_itaty::http::{ServerConfig, Service};
use una_itaty::notify::Recording;
use una_itaty::registry::{
    Cause, Clock, Config, Receipt, Refusal, Registry, Rotation, checkpoint_path,
};
use una_itaty::store::{FileStore, Store};
use una_itaty::timing::Obligation;
use una_project::client::{Point, Registry as Client};
use una_project::tlog::note::{Key, parse_note};
use una_project::tlog::owner::{History, RECOVERY_DELAY, fingerprint};
use una_project::tlog::root::digest as root_digest;
use una_project::transport::Directory;

use support::{EXPIRES, Keys, NOW, cosign_line, key_text, keypair, root_text_with, sign_line};

type Pair = (SigningKey, Key);

struct TestClock(AtomicU64);

impl Clock for TestClock {
    fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A signed note of `text`, which ends with a line feed, by each of `signers`.
fn note_of(text: &str, signers: &[&Pair]) -> String {
    let mut document = format!("{text}\n");
    for (pair, key) in signers {
        document.push_str(&sign_line(pair, key, text));
    }
    document
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
        let document = note_of(&text, &[&keys.roots[0], &keys.roots[1]]);
        store.put("root/1", document.as_bytes()).unwrap();
        let pin = root_digest(&document);
        let clock = Arc::new(TestClock(AtomicU64::new(NOW)));
        let shared: Arc<dyn Store> = store.clone();
        let mut registry = Registry::open(
            shared,
            &Config {
                pin: pin.clone(),
                log_seed: [20; 32],
            },
            clock.clone(),
        )
        .unwrap();
        let recording = Arc::new(Recording::new());
        registry.set_notifier(recording.clone());
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
        let shared: Arc<dyn Store> = self.store.clone();
        Registry::open(
            shared,
            &Config {
                pin: self.pin.clone(),
                log_seed: [20; 32],
            },
            self.clock.clone(),
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
            .register(note_of(&text, keys).as_bytes(), false)
            .unwrap();
        self.logged(&owner_chain(owner), receipt)
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
        let receipt = self.registry.submit(note_of(&text, signers).as_bytes())?;
        Ok(self.logged(&owner_chain(owner), receipt))
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
        .request(note_of(&recover, &[&c]).as_bytes())
        .unwrap();
    let recovery = f.logged(&owner_chain("acme"), receipt);
    assert_eq!(recovery, 2);
    // Its start checkpoint must carry the threshold before anything more is logged.
    let dispute = f.event("zenith", &format!("dispute {}", key_text(&d.1)), 1);
    let refusal = f.registry.request(dispute.as_bytes()).unwrap_err();
    assert!(matches!(
        refusal,
        Refusal::Wait(Cause::Obligation(Obligation::Start(2)), _)
    ));
    f.cosign(&[0, 1], t0 + 20);

    // zenith has no cosigners: the operator logs a dispute, which only the log's key signs.
    let receipt = f.registry.request(dispute.as_bytes()).unwrap();
    let disputed = f.logged(&owner_chain("zenith"), receipt);
    assert_eq!(disputed, 3);
    f.cosign(&[0, 1], t0 + 30);

    // Before the deadline, nothing signed by the new keys is logged: it waits.
    let early = f
        .submit("acme", &format!("add {}", key_text(&s.1)), 1, &[&r, &s])
        .unwrap_err();
    assert_eq!(early.cause(), Some(&Cause::ReliesOnPending(2)), "{early:?}");

    // Seven days of witness time later, the next checkpoint the log signs is the deadline of
    // both requests. A late cosignature of the checkpoint before it is fine while fewer than
    // the threshold are late.
    let later = t0 + 30 + RECOVERY_DELAY + 100;
    f.set_time(later);
    f.cosign_at(4, 2, later).unwrap();
    assert_eq!(f.register("orbit", &[&o], &[]), 4);
    assert_eq!(
        (
            f.registry.deadline_of(recovery),
            f.registry.deadline_of(disputed)
        ),
        (Some(5), Some(5))
    );
    // The deadline checkpoint takes no cosignature before the target, and must carry the
    // threshold before the log grows: until then, every submission waits.
    let early = f.cosign_at(5, 0, t0 + 20 + RECOVERY_DELAY).unwrap_err();
    assert_eq!(early.cause(), Some(&Cause::CrossesTarget), "{early:?}");
    let waiting = f
        .submit("acme", &format!("add {}", key_text(&s.1)), 1, &[&r, &s])
        .unwrap_err();
    assert_eq!(
        waiting.cause(),
        Some(&Cause::Obligation(Obligation::Deadline(2))),
        "{waiting:?}"
    );
    f.cosign(&[0, 1], later);

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
    let document = note_of(&text, &[&f.keys.roots[0], &f.keys.roots[1]]);
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
    assert!(f.checkpoint_text(8).contains("ita-root 2 sha256:"));
    assert!(f.checkpoint_text(5).contains("ita-root 1 sha256:"));
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
        f.registry.request(itauna.as_bytes()).unwrap_err().cause(),
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
        Registry::open(shared.clone(), &config, f.clock.clone()).unwrap(),
        ServerConfig::default(),
    )
    .unwrap_err();
    assert!(refused.contains("notifier"), "{refused}");
    let outbox = temp_dir("outbox-file");
    let service = Service::new(
        shared.clone(),
        Registry::open(shared, &config, f.clock.clone()).unwrap(),
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
        .register(note_of(&registration, &[&a]).as_bytes(), false)
        .unwrap();
    let event = format!(
        "ita-key/1\nsource itaty\nowner acme\nprevious 0 {}\ncancel 0\nthreshold 1\n",
        digest_string(&receipt.leaf)
    );
    registry.submit(note_of(&event, &[&a]).as_bytes()).unwrap();
    drop(registry);
    let lines = std::fs::read_to_string(&outbox).unwrap();
    assert_eq!(lines.lines().count(), 1, "{lines}");
    assert!(
        lines.starts_with("ita-notice/1 itaty acme 1 cancel sha256:"),
        "{lines}"
    );
    let _ = std::fs::remove_file(&outbox);
}

/// The logged recovery of `acme` by its cosigner, its start cosigned at `start` by `witnesses`,
/// then the registration of `zenith` and, at `at`, its dispute, whose start checkpoint (size 4)
/// is signed just before the recovery's target. Returns the recovery's target.
fn dispute_just_before_a_target(f: &mut Fixture, witnesses: &[usize]) -> u64 {
    let a = keypair(60, "acme.a");
    let c = keypair(61, "trustee.c");
    let z = keypair(62, "zenith.z");
    let r = keypair(63, "acme.r");
    let d = keypair(64, "zenith.d");
    f.register("acme", &[&a], &[&c]);
    let recover = f.event("acme", &format!("recover {}", key_text(&r.1)), 1);
    let receipt = f
        .registry
        .request(note_of(&recover, &[&c]).as_bytes())
        .unwrap();
    assert_eq!(f.logged(&owner_chain("acme"), receipt), 1);
    f.cosign(witnesses, NOW + 10);
    let target = NOW + 10 + RECOVERY_DELAY;
    f.register("zenith", &[&z], &[]);
    f.set_time(target - 60);
    let dispute = f.event("zenith", &format!("dispute {}", key_text(&d.1)), 1);
    let receipt = f.registry.request(dispute.as_bytes()).unwrap();
    assert_eq!(f.logged(&owner_chain("zenith"), receipt), 3);
    // The dispute's start checkpoint owes the threshold: until then, the log waits.
    assert_eq!(
        f.registry.owed().map(|(obligation, _)| obligation),
        Some(Obligation::Start(3))
    );
    target
}

#[test]
fn a_start_checkpoint_signed_just_before_a_target_never_freezes_the_log() {
    // With a threshold of one, no cosignature at or after the recovery's target may land on a
    // checkpoint signed before it, so the dispute's start checkpoint can never be cosigned
    // once the target passes.
    let mut f = Fixture::with_k("freeze-k1", 1);
    let target = dispute_just_before_a_target(&mut f, &[0]);
    let o = keypair(67, "orbit.o");
    let orbit = f.registration("orbit", &[&o], 1, &[]);
    assert_eq!(
        f.registry
            .register(note_of(&orbit, &[&o]).as_bytes(), false)
            .unwrap_err()
            .cause(),
        Some(&Cause::Obligation(Obligation::Start(3)))
    );
    f.set_time(target + 10);
    assert_eq!(
        f.cosign_at(4, 0, target + 10).unwrap_err().cause(),
        Some(&Cause::CrossesTarget)
    );
    // Uncompletable: the log grows, and its next checkpoint is the recovery's deadline.
    assert_eq!(f.registry.owed(), None);
    assert_eq!(f.register("orbit", &[&o], &[]), 4);
    assert_eq!(f.registry.deadline_of(1), Some(5));
    // The dispute's start never carried the threshold: it never takes effect here, and every
    // client finds its start an omission.
    assert_eq!(f.registry.deadline_of(3), None);
}

#[test]
fn an_outage_across_a_target_lets_the_log_grow_past_an_uncompletable_checkpoint() {
    let mut f = Fixture::new("outage");
    let target = dispute_just_before_a_target(&mut f, &[0, 1]);
    // While the target has not passed, the log keeps waiting for its witnesses.
    f.set_time(target - 1);
    assert!(f.registry.owed().is_some());
    // Past it, with no cosignature before it, at most one more can come: never two.
    f.set_time(target + 10);
    assert_eq!(f.registry.owed(), None);
    let o = keypair(67, "orbit.o");
    assert_eq!(f.register("orbit", &[&o], &[]), 4);
    assert_eq!(f.registry.deadline_of(1), Some(5));
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
        .request(note_of(&recover, &[&c]).as_bytes())
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
    let document = note_of(&text, &[&f.keys.roots[0], &f.keys.roots[1]]);
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
    assert_eq!(f.registry.deadline_of(recovery), Some(5));
    f.cosign(&[0, 2], later);

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
