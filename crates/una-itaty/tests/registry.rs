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

/// Three keys of `owner`, `<owner>.a` to `<owner>.c`, from the seeds `3 * seed` to
/// `3 * seed + 2`: an owner at the key floor.
fn trio(owner: &str, seed: u8) -> Vec<Signer> {
    ["a", "b", "c"]
        .iter()
        .zip(seed * 3..)
        .map(|(n, s)| signer(&format!("{owner}.{n}"), s))
        .collect()
}

/// The registration text of `owner` with `keys` at `threshold`.
fn registration(owner: &str, keys: &[&Signer], threshold: usize) -> String {
    let mut text =
        format!("ita-owner/1\nsource itaty\nowner {owner}\nprevious none\nthreshold {threshold}\n");
    for k in keys {
        text.push_str(&format!("key {}\n", key_line(k)));
    }
    text
}

/// Registers `owner` with `keys` at threshold 2, signed by each.
fn register(registry: &mut Registry, owner: &str, keys: &[Signer]) -> [u8; 32] {
    let keys: Vec<&Signer> = keys.iter().collect();
    let document = note(&registration(owner, &keys, 2), &keys);
    registry.register(document.as_bytes(), false).unwrap();
    leaf_hash(document.as_bytes())
}

#[test]
fn key_events_follow_the_clients_acceptance() {
    let (_, mut registry, _, _) = open();
    let [a, b, c] = <[Signer; 3]>::try_from(trio("acme", 10)).ok().unwrap();
    let d = signer("acme.d", 40);
    let e = signer("acme.e", 41);
    // A registration below the key floor is refused: one key, two keys, a threshold of 1, or
    // no key more than the threshold.
    for (keys, threshold) in [
        (vec![&a], 1),
        (vec![&a, &b], 2),
        (vec![&a, &b, &c], 1),
        (vec![&a, &b, &c], 3),
    ] {
        let text = registration("acme", &keys, threshold);
        assert!(
            matches!(
                registry.register(note(&text, &keys).as_bytes(), false),
                Err(Refusal::Invalid(_))
            ),
            "{threshold} of {}",
            keys.len()
        );
    }
    let keys = [&a, &b, &c];
    let document = note(&registration("acme", &keys, 2), &keys);
    registry.register(document.as_bytes(), false).unwrap();
    let leaf = leaf_hash(document.as_bytes());
    let event = |prev: u64, leaf: [u8; 32], action: &str, threshold: usize| {
        format!(
            "ita-key/1\nsource itaty\nowner acme\nprevious {prev} {}\n{action}\nthreshold {threshold}\n",
            digest_string(&leaf)
        )
    };
    let add_d = |threshold| event(0, leaf, &format!("add {}", key_line(&d)), threshold);
    // A key added signs its own event, besides the owner's threshold; one key of the owner is
    // not enough, a threshold of 1 or above the keys after it is refused.
    for (text, signers) in [
        (add_d(2), vec![&a, &b]),
        (add_d(2), vec![&a, &d]),
        (add_d(1), vec![&a, &b, &d]),
        (add_d(5), vec![&a, &b, &d]),
    ] {
        assert!(
            matches!(
                registry.submit(note(&text, &signers).as_bytes()),
                Err(Refusal::Invalid(_))
            ),
            "{text}"
        );
    }
    let receipt = registry
        .submit(note(&add_d(2), &[&a, &b, &d]).as_bytes())
        .unwrap();
    assert_eq!(receipt.index, 1);
    // A claim of compromise signed by the key it names, or by one key, is refused; by two other
    // keys it is logged, and the claimed key then counts for nothing. Since 0 it would void the
    // addition of `acme.d`, which `acme.b` signed, and leave two keys: below the floor.
    let since_0 = event(1, receipt.leaf, "revoke acme.b since 0", 2);
    assert!(
        registry
            .submit(note(&since_0, &[&a, &c]).as_bytes())
            .is_err()
    );
    let claim = event(1, receipt.leaf, "revoke acme.b since 1", 2);
    assert!(registry.submit(note(&claim, &[&a, &b]).as_bytes()).is_err());
    assert!(registry.submit(note(&claim, &[&a]).as_bytes()).is_err());
    let receipt = registry.submit(note(&claim, &[&a, &c]).as_bytes()).unwrap();
    let add_e = event(2, receipt.leaf, &format!("add {}", key_line(&e)), 2);
    assert!(
        registry
            .submit(note(&add_e, &[&b, &c, &e]).as_bytes())
            .is_err()
    );
    let receipt = registry
        .submit(note(&add_e, &[&a, &c, &e]).as_bytes())
        .unwrap();
    // A revocation that would leave the owner below the floor is refused: four keys at a
    // threshold of 2 keep three, then a threshold of 3 would leave no key more than it.
    let revoke = event(3, receipt.leaf, "revoke acme.d", 3);
    assert!(
        registry
            .submit(note(&revoke, &[&a, &c]).as_bytes())
            .is_err()
    );
    // A request kind is not logged: no client of this release replays it.
    for action in [
        format!("recover {}", key_line(&signer("acme.r", 42))),
        "cancel 1".to_owned(),
    ] {
        let text = event(3, receipt.leaf, &action, 2);
        assert!(registry.submit(note(&text, &[&a, &c]).as_bytes()).is_err());
    }
    // Another owner cannot register a key that is acme's.
    let mut theirs = registration("zenith", &[], 2);
    theirs.push_str(&format!(
        "key zenith.a {}\n",
        encode_base64(&a.key().public)
    ));
    let [y1, y2, _] = <[Signer; 3]>::try_from(trio("zenith", 20)).ok().unwrap();
    theirs.push_str(&format!("key {}\nkey {}\n", key_line(&y1), key_line(&y2)));
    let stolen = signer("zenith.a", 30);
    assert!(
        registry
            .register(note(&theirs, &[&stolen, &y1, &y2]).as_bytes(), false)
            .is_err()
    );
}

#[test]
fn registrations_follow_the_name_rules_and_the_chain() {
    let (_, mut registry, _, _) = open();
    register(&mut registry, "acme", &trio("acme", 10));
    let z = trio("x", 20);
    let zs: Vec<&Signer> = z.iter().collect();
    for owner in ["acme", "4cme", "std", "itauna", "a"] {
        let text = registration(owner, &zs, 2);
        assert!(
            matches!(
                registry.register(note(&text, &zs).as_bytes(), false),
                Err(Refusal::Invalid(_))
            ),
            "{owner}"
        );
    }
    let near = registration("acmes", &zs, 2);
    assert!(matches!(
        registry.register(note(&near, &zs).as_bytes(), false),
        Err(Refusal::Held(_))
    ));
    assert!(registry.register(note(&near, &zs).as_bytes(), true).is_ok());
    // A key that does not sign its registration is refused.
    let y = trio("y", 21);
    let ys: Vec<&Signer> = y.iter().collect();
    let text = registration("zenith", &ys, 2);
    assert!(
        registry
            .register(note(&text, &ys[..2]).as_bytes(), false)
            .is_err()
    );
    // Another source's entry is refused.
    let other = text.replace("source itaty", "source acme");
    assert!(
        registry
            .register(note(&other, &ys).as_bytes(), false)
            .is_err()
    );
    assert_eq!(registry.size(), 2);
    assert_ne!(owner_chain("acme"), "acme");
}

#[test]
fn cosignatures_are_verified_and_monotonic() {
    let (store, mut registry, _, witnesses) = open();
    register(&mut registry, "acme", &trio("acme", 10));
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
    register(&mut registry, "zenith", &trio("zenith", 11));
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
    register(&mut registry, "acme", &trio("acme", 10));
    register(&mut registry, "zenith", &trio("zenith", 11));
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
    register(&mut registry, "acme", &trio("acme", 10));
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
    register(&mut registry, "zenith", &trio("zenith", 11));
    assert_eq!(root_line(&store, 2), format!("ita-root 2 {second}"));
    assert_eq!(root_line(&store, 1), format!("ita-root 1 {first}"));
    drop(registry);
    // The old log key no longer opens the store.
    assert!(Registry::open(shared, &config, Arc::new(Fixed(1_000))).is_err());
}

#[test]
fn a_size_first_signed_after_a_rotation_names_the_new_root() {
    let (store, mut registry, _, _) = open();
    register(&mut registry, "acme", &trio("acme", 10));
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
    register(&mut registry, "acme", &trio("acme", 10));
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
        // The log's exact text under version 1, signed by another key: the signature only.
        note(&text, &[&signer("itaty.test/log", 9)]),
        // Version 1's line and key, with one more extension line: the text only.
        note(&format!("{text}ita-extra x\n"), &[&old_log]),
    ];
    for document in bad {
        store.put(&path, document.as_bytes()).unwrap();
        assert!(Registry::open(shared.clone(), &rotated, Arc::new(Fixed(1_000))).is_err());
    }
}

#[test]
fn a_store_whose_earlier_checkpoint_names_a_root_no_longer_served_refuses_to_open() {
    let (store, mut registry, _, _) = open();
    register(&mut registry, "acme", &trio("acme", 10));
    drop(registry);
    let (_, rotated) = rotate(&store);
    let shared: Arc<dyn Store> = store.clone();
    let mut registry = Registry::open(shared.clone(), &rotated, Arc::new(Fixed(1_000))).unwrap();
    register(&mut registry, "zenith", &trio("zenith", 11));
    register(&mut registry, "nadir", &trio("nadir", 12));
    drop(registry);
    // A crash before checkpoint/3 was stored, and root/2 restored away: checkpoint/2 names a
    // version the chain no longer has, so the service must not sign size 3 under version 1.
    store.remove(&checkpoint_path(3)).unwrap();
    let document_2 = store.get("root/2", 1 << 20).unwrap().unwrap();
    store.remove("root/2").unwrap();
    let first = String::from_utf8(store.get("root/1", 1 << 20).unwrap().unwrap()).unwrap();
    let back = Config {
        pin: root_digest(&first),
        log_seed: [2; 32],
    };
    assert!(Registry::open(shared.clone(), &back, Arc::new(Fixed(1_000))).is_err());
    assert!(store.get(&checkpoint_path(3), 1 << 20).unwrap().is_none());
    // With root/2 back, it opens and signs size 3 under version 2.
    store.put("root/2", &document_2).unwrap();
    Registry::open(shared, &rotated, Arc::new(Fixed(1_000))).unwrap();
    assert!(root_line(&store, 3).starts_with("ita-root 2 "));
}

#[test]
fn stored_checkpoints_never_name_a_lower_version_as_the_size_grows() {
    let (store, mut registry, _, _) = open();
    register(&mut registry, "acme", &trio("acme", 10));
    drop(registry);
    let (_, rotated) = rotate(&store);
    let shared: Arc<dyn Store> = store.clone();
    let mut registry = Registry::open(shared.clone(), &rotated, Arc::new(Fixed(1_000))).unwrap();
    register(&mut registry, "zenith", &trio("zenith", 11));
    register(&mut registry, "nadir", &trio("nadir", 12));
    drop(registry);
    // checkpoint/3 replaced by the log's own text of size 3 under version 1, signed by version
    // 1's log key: a valid checkpoint of the chain, below the version 2 of size 2.
    let first = String::from_utf8(store.get("root/1", 1 << 20).unwrap().unwrap()).unwrap();
    let path = checkpoint_path(3);
    let document = String::from_utf8(store.get(&path, 1 << 20).unwrap().unwrap()).unwrap();
    let text = parse_note(&document).unwrap().text;
    let line = root_line(&store, 3);
    let lowered = text
        .replace("itaty.test/log2\n", "itaty.test/log\n")
        .replace(&line, &format!("ita-root 1 {}", root_digest(&first)));
    store
        .put(
            &path,
            note(&lowered, &[&signer("itaty.test/log", 2)]).as_bytes(),
        )
        .unwrap();
    let refused = Registry::open(shared.clone(), &rotated, Arc::new(Fixed(1_000))).unwrap_err();
    assert!(refused.contains("below"), "{refused}");
    // An earlier checkpoint missing: refused too.
    store.put(&path, document.as_bytes()).unwrap();
    Registry::open(shared.clone(), &rotated, Arc::new(Fixed(1_000))).unwrap();
    store.remove(&checkpoint_path(1)).unwrap();
    assert!(Registry::open(shared, &rotated, Arc::new(Fixed(1_000))).is_err());
}

#[test]
fn a_store_written_before_checkpoints_named_their_root_refuses_to_open() {
    let (store, mut registry, config, _) = open();
    register(&mut registry, "acme", &trio("acme", 10));
    drop(registry);
    let path = checkpoint_path(1);
    let line = root_line(&store, 1);
    let document = String::from_utf8(store.get(&path, 1 << 20).unwrap().unwrap()).unwrap();
    let text = parse_note(&document)
        .unwrap()
        .text
        .replace(&format!("{line}\n"), "");
    store
        .put(
            &path,
            note(&text, &[&signer("itaty.test/log", 2)]).as_bytes(),
        )
        .unwrap();
    let shared: Arc<dyn Store> = store.clone();
    let refused = Registry::open(shared, &config, Arc::new(Fixed(1_000))).unwrap_err();
    assert!(refused.contains("names no root version"), "{refused}");
}
