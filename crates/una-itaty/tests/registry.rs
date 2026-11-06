//! The registry's checks without HTTP (ADR 0122 rules 36, 38 and 44 to 45): owner keys and
//! their events, chains, witnesses' cosignatures, and a store changed by hand.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::sync::Arc;

use una_itaty::registry::{Clock, Config, Refusal, Registry, WITNESS_NEXT, checkpoint_path};
use una_itaty::sign::{Signer, note};
use una_itaty::store::{MemoryStore, Store};
use una_itaty::{digest_string, entry::owner_chain};
use una_project::tlog::merkle::leaf_hash;
use una_project::tlog::note::{encode_base64, parse_checkpoint, parse_note};
use una_project::tlog::root::digest as root_digest;

struct Fixed(u64);

impl Clock for Fixed {
    fn now(&self) -> u64 {
        self.0
    }
}

fn signer(name: &str, seed: u8) -> Signer {
    Signer::from_seed(name, &[seed; 32])
}

fn key_line(s: &Signer) -> String {
    format!("{} {}", s.key().name, encode_base64(&s.key().public))
}

fn open() -> (Arc<MemoryStore>, Registry, Config, Vec<Signer>) {
    let root = signer("root.0", 1);
    let log = signer("itaty.test/log", 2);
    let witnesses = vec![signer("witness.0", 3), signer("witness.1", 4)];
    let security = signer("security.0", 5);
    let itauna = signer("itauna.0", 6);
    let text = format!(
        "ita-root/1\nsource itaty\nversion 1\nexpires 4000000000\nroot 1\nkey {}\nlog {}\nwitnesses 2\nkey {}\nkey {}\nsecurity 1\nkey {}\nitauna 1\nkey {}\n",
        key_line(&root),
        key_line(&log),
        key_line(&witnesses[0]),
        key_line(&witnesses[1]),
        key_line(&security),
        key_line(&itauna)
    );
    let document = note(&text, &[&root]);
    let store = Arc::new(MemoryStore::new());
    store.put("root/1", document.as_bytes()).unwrap();
    let config = Config {
        pin: root_digest(&document),
        log_seed: [2; 32],
    };
    let shared: Arc<dyn Store> = store.clone();
    let registry = Registry::open(shared, &config, Arc::new(Fixed(1_000))).unwrap();
    (store, registry, config, witnesses)
}

fn register(registry: &mut Registry, owner: &str, keys: &[&Signer], threshold: usize) -> [u8; 32] {
    let mut text =
        format!("ita-owner/1\nsource itaty\nowner {owner}\nprevious none\nthreshold {threshold}\n");
    for k in keys {
        text.push_str(&format!("key {}\n", key_line(k)));
    }
    let document = note(&text, keys);
    registry.register(document.as_bytes(), false).unwrap();
    leaf_hash(document.as_bytes())
}

#[test]
fn key_events_follow_the_owners_threshold() {
    let (_, mut registry, _, _) = open();
    let a = signer("acme.a", 10);
    let b = signer("acme.b", 11);
    let c = signer("acme.c", 12);
    let leaf = register(&mut registry, "acme", &[&a], 1);
    // A key added must sign its own event, besides the threshold.
    let add = |prev: u64, leaf: [u8; 32], key: &Signer, threshold: usize| {
        format!(
            "ita-key/1\nsource itaty\nowner acme\nprevious {prev} {}\nadd {}\nthreshold {threshold}\n",
            digest_string(&leaf),
            key_line(key)
        )
    };
    let text = add(0, leaf, &b, 2);
    assert!(matches!(
        registry.submit(note(&text, &[&a]).as_bytes()),
        Err(Refusal::Invalid(_))
    ));
    // A threshold above the keys the owner would hold is refused.
    let too_high = add(0, leaf, &b, 3);
    assert!(
        registry
            .submit(note(&too_high, &[&a, &b]).as_bytes())
            .is_err()
    );
    let document = note(&text, &[&a, &b]);
    let receipt = registry.submit(document.as_bytes()).unwrap();
    assert_eq!(receipt.index, 1);
    // Now two keys must sign: one is not enough.
    let text = add(1, receipt.leaf, &c, 2);
    assert!(registry.submit(note(&text, &[&a, &c]).as_bytes()).is_err());
    // A revocation by the threshold; the revoked key then counts for nothing.
    let revoke = format!(
        "ita-key/1\nsource itaty\nowner acme\nprevious 1 {}\nrevoke acme.b since 1\nthreshold 1\n",
        digest_string(&receipt.leaf)
    );
    let receipt = registry
        .submit(note(&revoke, &[&a, &b]).as_bytes())
        .unwrap();
    let text = add(2, receipt.leaf, &c, 1);
    assert!(registry.submit(note(&text, &[&b, &c]).as_bytes()).is_err());
    assert!(registry.submit(note(&text, &[&a, &c]).as_bytes()).is_ok());
    // Another owner cannot register a key that is acme's.
    let mut theirs =
        "ita-owner/1\nsource itaty\nowner zenith\nprevious none\nthreshold 1\n".to_owned();
    theirs.push_str(&format!(
        "key zenith.a {}\n",
        encode_base64(&a.key().public)
    ));
    let stolen = signer("zenith.a", 10);
    assert!(
        registry
            .register(note(&theirs, &[&stolen]).as_bytes(), false)
            .is_err()
    );
}

#[test]
fn registrations_follow_the_name_rules_and_the_chain() {
    let (_, mut registry, _, _) = open();
    let a = signer("acme.a", 10);
    register(&mut registry, "acme", &[&a], 1);
    let z = signer("x.a", 20);
    for owner in ["acme", "4cme", "std", "itauna", "a"] {
        let text = format!(
            "ita-owner/1\nsource itaty\nowner {owner}\nprevious none\nthreshold 1\nkey {}\n",
            key_line(&z)
        );
        assert!(
            matches!(
                registry.register(note(&text, &[&z]).as_bytes(), false),
                Err(Refusal::Invalid(_))
            ),
            "{owner}"
        );
    }
    let near = format!(
        "ita-owner/1\nsource itaty\nowner acmes\nprevious none\nthreshold 1\nkey {}\n",
        key_line(&z)
    );
    assert!(matches!(
        registry.register(note(&near, &[&z]).as_bytes(), false),
        Err(Refusal::Held(_))
    ));
    assert!(
        registry
            .register(note(&near, &[&z]).as_bytes(), true)
            .is_ok()
    );
    // A key that does not sign its registration is refused.
    let y = signer("y.a", 21);
    let text = format!(
        "ita-owner/1\nsource itaty\nowner zenith\nprevious none\nthreshold 1\nkey {}\n",
        key_line(&y)
    );
    assert!(
        registry
            .register(note(&text, &[&z]).as_bytes(), false)
            .is_err()
    );
    // Another source's entry is refused.
    let other = text.replace("source itaty", "source acme");
    assert!(
        registry
            .register(note(&other, &[&y]).as_bytes(), false)
            .is_err()
    );
    assert_eq!(registry.size(), 2);
    assert_ne!(owner_chain("acme"), "acme");
}

#[test]
fn cosignatures_are_verified_and_monotonic() {
    let (store, mut registry, _, witnesses) = open();
    let a = signer("acme.a", 10);
    register(&mut registry, "acme", &[&a], 1);
    let next = String::from_utf8(store.get(WITNESS_NEXT, 1 << 20).unwrap().unwrap()).unwrap();
    let text = parse_note(&next).unwrap().text;
    // A key that is not a witness, a time from the future, a bad signature: refused.
    let stranger = signer("witness.9", 9);
    assert!(
        registry
            .cosign(1, &stranger.cosignature(&text, 1_000))
            .is_err()
    );
    assert!(
        registry
            .cosign(1, &witnesses[0].cosignature(&text, 1_000_000))
            .is_err()
    );
    assert!(
        registry
            .cosign(1, &witnesses[0].cosignature("other\n1\nAAAA\n", 1_000))
            .is_err()
    );
    assert!(
        registry
            .cosign(9, &witnesses[0].cosignature(&text, 1_000))
            .is_err()
    );
    registry
        .cosign(1, &witnesses[0].cosignature(&text, 1_000))
        .unwrap();
    // The same witness twice on one checkpoint, and below its own time later: refused.
    assert!(
        registry
            .cosign(1, &witnesses[0].cosignature(&text, 1_001))
            .is_err()
    );
    assert!(
        store.get("checkpoint", 1 << 20).unwrap().is_none(),
        "one of two witnesses"
    );
    registry
        .cosign(1, &witnesses[1].cosignature(&text, 1_000))
        .unwrap();
    let served = String::from_utf8(store.get("checkpoint", 1 << 20).unwrap().unwrap()).unwrap();
    assert_eq!(
        parse_checkpoint(&parse_note(&served).unwrap().text)
            .unwrap()
            .size,
        1
    );
    let b = signer("zenith.a", 11);
    register(&mut registry, "zenith", &[&b], 1);
    let next = String::from_utf8(store.get(WITNESS_NEXT, 1 << 20).unwrap().unwrap()).unwrap();
    let text = parse_note(&next).unwrap().text;
    assert!(
        registry
            .cosign(2, &witnesses[0].cosignature(&text, 999))
            .is_err()
    );
    // A smaller checkpoint after a larger one: refused.
    registry
        .cosign(2, &witnesses[0].cosignature(&text, 1_002))
        .unwrap();
    let empty =
        String::from_utf8(store.get(&checkpoint_path(0), 1 << 20).unwrap().unwrap()).unwrap();
    let empty = parse_note(&empty).unwrap().text;
    assert!(
        registry
            .cosign(0, &witnesses[0].cosignature(&empty, 1_003))
            .is_err()
    );
}

#[test]
fn a_store_changed_by_hand_refuses_to_open() {
    let (store, mut registry, config, _) = open();
    let a = signer("acme.a", 10);
    register(&mut registry, "acme", &[&a], 1);
    let b = signer("zenith.a", 11);
    register(&mut registry, "zenith", &[&b], 1);
    let shared: Arc<dyn Store> = store.clone();
    let reopened = Registry::open(shared.clone(), &config, Arc::new(Fixed(1_000))).unwrap();
    assert_eq!(
        (reopened.size(), reopened.map_root()),
        (registry.size(), registry.map_root())
    );
    // Swap the two entries of the bundle: the chains still hold, the checkpoint does not.
    let bundle = store.get("tile/entries/000.p/2", 1 << 20).unwrap().unwrap();
    let first_len = usize::from(u16::from_be_bytes([bundle[0], bundle[1]])) + 2;
    let mut swapped = bundle[first_len..].to_vec();
    swapped.extend_from_slice(&bundle[..first_len]);
    store.put("tile/entries/000.p/2", &swapped).unwrap();
    assert!(Registry::open(shared, &config, Arc::new(Fixed(1_000))).is_err());
}

/// Root version 2 of the store `open` makes: signed by its root key, with the log's key
/// rotated to `itaty.test/log2` (seed 7).
fn rotate(store: &MemoryStore) -> (String, Config) {
    let root = signer("root.0", 1);
    let log = signer("itaty.test/log2", 7);
    let text = format!(
        "ita-root/1\nsource itaty\nversion 2\nexpires 4000000000\nroot 1\nkey {}\nlog {}\nwitnesses 2\nkey {}\nkey {}\nsecurity 1\nkey {}\nitauna 1\nkey {}\n",
        key_line(&root),
        key_line(&log),
        key_line(&signer("witness.0", 3)),
        key_line(&signer("witness.1", 4)),
        key_line(&signer("security.0", 5)),
        key_line(&signer("itauna.0", 6))
    );
    let document = note(&text, &[&root]);
    store.put("root/2", document.as_bytes()).unwrap();
    let first = String::from_utf8(store.get("root/1", 1 << 20).unwrap().unwrap()).unwrap();
    (
        root_digest(&document),
        Config {
            pin: root_digest(&first),
            log_seed: [7; 32],
        },
    )
}

fn root_line(store: &MemoryStore, size: u64) -> String {
    let document =
        String::from_utf8(store.get(&checkpoint_path(size), 1 << 20).unwrap().unwrap()).unwrap();
    let text = parse_note(&document).unwrap().text;
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("ita-root "))
        .collect();
    assert_eq!(lines.len(), 1, "one `ita-root` line in {text}");
    lines[0].to_owned()
}

#[test]
fn a_size_signed_again_after_a_rotation_keeps_its_first_root() {
    let (store, mut registry, config, _) = open();
    let first = String::from_utf8(store.get("root/1", 1 << 20).unwrap().unwrap()).unwrap();
    let first = root_digest(&first);
    let a = signer("acme.a", 10);
    register(&mut registry, "acme", &[&a], 1);
    assert_eq!(root_line(&store, 1), format!("ita-root 1 {first}"));
    let stored = store.get(&checkpoint_path(1), 1 << 20).unwrap().unwrap();
    drop(registry);
    // The root rotates, the log's key with it, and the service restarts: size 1 keeps the
    // checkpoint signed under version 1, byte for byte, and witnesses are served it.
    let (second, rotated) = rotate(&store);
    let shared: Arc<dyn Store> = store.clone();
    let mut registry = Registry::open(shared.clone(), &rotated, Arc::new(Fixed(1_000))).unwrap();
    assert_eq!(registry.root().version, 2);
    assert_eq!(
        store.get(&checkpoint_path(1), 1 << 20).unwrap().unwrap(),
        stored
    );
    assert_eq!(store.get(WITNESS_NEXT, 1 << 20).unwrap().unwrap(), stored);
    // The rotation takes effect from the checkpoint of the next entry.
    let b = signer("zenith.a", 11);
    register(&mut registry, "zenith", &[&b], 1);
    assert_eq!(root_line(&store, 2), format!("ita-root 2 {second}"));
    assert_eq!(root_line(&store, 1), format!("ita-root 1 {first}"));
    drop(registry);
    // The old log key no longer opens the store.
    assert!(Registry::open(shared, &config, Arc::new(Fixed(1_000))).is_err());
}

#[test]
fn a_size_first_signed_after_a_rotation_names_the_new_root() {
    let (store, mut registry, _, _) = open();
    let a = signer("acme.a", 10);
    register(&mut registry, "acme", &[&a], 1);
    drop(registry);
    // A crash before the checkpoint of size 1 was stored: it was never signed, so it is
    // signed under the version in force when the service opens.
    store.remove(&checkpoint_path(1)).unwrap();
    let (second, rotated) = rotate(&store);
    let shared: Arc<dyn Store> = store.clone();
    Registry::open(shared, &rotated, Arc::new(Fixed(1_000))).unwrap();
    assert_eq!(root_line(&store, 1), format!("ita-root 2 {second}"));
}

#[test]
fn a_stored_checkpoint_naming_another_root_refuses_to_open() {
    let (store, mut registry, _, _) = open();
    let a = signer("acme.a", 10);
    register(&mut registry, "acme", &[&a], 1);
    drop(registry);
    let (second, rotated) = rotate(&store);
    let path = checkpoint_path(1);
    let document = String::from_utf8(store.get(&path, 1 << 20).unwrap().unwrap()).unwrap();
    let text = parse_note(&document).unwrap().text;
    let line = root_line(&store, 1);
    let shared: Arc<dyn Store> = store.clone();
    // Version 2's line under the old log key, a version past the chain, a wrong digest, a
    // line removed: each is a store changed by hand.
    let old_log = signer("itaty.test/log", 2);
    let new_log = signer("itaty.test/log2", 7);
    let bad = [
        note(
            &text.replace(&line, &format!("ita-root 2 {second}")),
            &[&old_log],
        ),
        note(
            &text.replace(&line, &format!("ita-root 3 {second}")),
            &[&new_log],
        ),
        note(
            &text.replace(&line, &format!("ita-root 1 sha256:{}", "0".repeat(64))),
            &[&old_log],
        ),
        note(&text.replace(&format!("{line}\n"), ""), &[&old_log]),
    ];
    for document in bad {
        store.put(&path, document.as_bytes()).unwrap();
        assert!(Registry::open(shared.clone(), &rotated, Arc::new(Fixed(1_000))).is_err());
    }
}
