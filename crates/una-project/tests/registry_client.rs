//! The registry client (ADR 0122 rules 26 and 34 to 39), against a test registry read as a
//! mirror directory: tiles, entries and proofs; root metadata, its chain and expiry; checkpoints,
//! witness time, freshness and floors; name chains and the map; owners' keys, revocations,
//! recoveries and cancellations; release statements against their archives; rule 26; and a
//! mirror whose files were tampered with, which verifies nothing.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "support/registry.rs"]
mod registry;

use std::collections::BTreeMap;
use std::path::PathBuf;

use registry::{NOW, TestRegistry, fingerprint_of, hash_hex, keypair, key_text};
use una_project::client::{LoggedRelease, Point, Registry, owner_of};
use una_project::provenance::{Expected, signed_name, verify};
use una_project::tlog::merkle::{
    Hash, consistency_proof, inclusion_proof, leaf_hash, node_hash, root as tree_root,
};
use una_project::tlog::owner;
use una_project::tlog::release::check_archive;
use una_project::tlog::tile::{Tiles, index_path, parse_entry_bundle, tile_path};
use una_project::transport::{Directory, Location, Transport, Unavailable};
use una_project::tree::VerifiedTree;
use una_project::version::{Line, Version};

const DAY: u64 = 24 * 3600;

/// A fresh directory for one test.
fn scratch(name: &str) -> PathBuf {
    let base = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    let dir = base.join(format!("una-registry-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A transport over files in memory, counting its reads.
struct Memory {
    files: BTreeMap<String, Vec<u8>>,
}

impl Transport for Memory {
    fn get(&self, path: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
        match self.files.get(path) {
            Some(bytes) if bytes.len() as u64 > limit => Err(format!("`{path}` is too long")),
            Some(bytes) => Ok(Some(bytes.clone())),
            None => Ok(None),
        }
    }

    fn describe(&self) -> String {
        "memory".to_owned()
    }
}

/// A log of `n` entries, served as tiles.
fn served(n: usize) -> (TestRegistry, Vec<Hash>) {
    let mut registry = TestRegistry::new("acme");
    for i in 0..n {
        registry.append(&format!("name{}", i % 7), &format!("entry {i}"));
    }
    registry.checkpoint(NOW);
    let leaves = registry.entries.iter().map(|e| leaf_hash(e)).collect();
    (registry, leaves)
}

#[test]
fn tile_paths_follow_the_c2sp_layout() {
    assert_eq!(index_path(0), "000");
    assert_eq!(index_path(67), "067");
    assert_eq!(index_path(1000), "x001/000");
    assert_eq!(index_path(1_234_067), "x001/x234/067");
    assert_eq!(tile_path(0, 1_234_067, 256), "tile/0/x001/x234/067");
    assert_eq!(tile_path(1, 0, 5), "tile/1/000.p/5");
}

/// Proofs computed from the tiles are the ones computed from the leaves, for sizes across tile
/// boundaries and at the second tile level.
#[test]
fn proofs_from_tiles_are_the_trees_proofs() {
    for n in [1usize, 2, 3, 255, 256, 257, 300, 513, 600] {
        let (registry, leaves) = served(n);
        let memory = Memory {
            files: registry.files.clone(),
        };
        let mut tiles = Tiles::new(&memory, n as u64);
        assert_eq!(tiles.range(0, n as u64).unwrap(), tree_root(&leaves), "{n}");
        for index in [0, n / 2, n - 1] {
            assert_eq!(
                tiles.inclusion(index as u64).unwrap(),
                inclusion_proof(index, &leaves),
                "{index} of {n}"
            );
            assert_eq!(tiles.entry(index as u64).unwrap(), registry.entries[index]);
        }
        for first in [1usize, n / 3, n - 1] {
            if first == 0 || first >= n {
                continue;
            }
            assert_eq!(
                tiles.consistency(first as u64).unwrap(),
                consistency_proof(first, &leaves),
                "{first} -> {n}"
            );
        }
    }
}

/// A tree of identical leaves, its tiles served by formula: every node of level l has the hash
/// h_l, so any size can be served without building it.
struct Uniform {
    levels: Vec<Hash>,
}

impl Transport for Uniform {
    fn get(&self, path: &str, _limit: u64) -> Result<Option<Vec<u8>>, String> {
        let rest = path.strip_prefix("tile/").unwrap();
        let (level, rest) = rest.split_once('/').unwrap();
        let level: usize = level.parse().unwrap();
        let width = match rest.split_once(".p/") {
            Some((_, width)) => width.parse::<usize>().unwrap(),
            None => 256,
        };
        let hash = self.levels[8 * level];
        Ok(Some(hash.repeat(width)))
    }

    fn describe(&self) -> String {
        "uniform".to_owned()
    }
}

/// The work of a proof from tiles grows with the logarithm of the log: a log a million times
/// larger reads a few more tiles, never a number proportional to its size.
#[test]
fn tile_reads_grow_with_the_logarithm_of_the_log() {
    let mut levels = vec![leaf_hash(b"x")];
    for l in 1..64 {
        let below = levels[l - 1];
        levels.push(node_hash(&below, &below));
    }
    let uniform = Uniform { levels };
    let mut reads = Vec::new();
    for bits in [10u32, 20, 30, 40] {
        let size = (1u64 << bits) + 12_345;
        let mut tiles = Tiles::new(&uniform, size);
        let proof = tiles.inclusion(size / 3).unwrap();
        assert!(proof.len() <= bits as usize + 2);
        let mut consistency = Tiles::new(&uniform, size);
        consistency.consistency(size / 2 + 1).unwrap();
        reads.push(tiles.reads() + consistency.reads());
    }
    // From 2^10 to 2^40 entries: at most a constant per factor of 256 more.
    for pair in reads.windows(2) {
        assert!(pair[1] <= pair[0] + 24, "{reads:?}");
    }
    assert!(reads[3] < 120, "{reads:?}");
}

#[test]
fn entry_bundles_are_read_strictly() {
    let bundle = [0u8, 2, b'h', b'i', 0, 0];
    assert_eq!(
        parse_entry_bundle(&bundle, 2, false).unwrap(),
        vec![b"hi".to_vec(), Vec::new()]
    );
    assert!(parse_entry_bundle(&bundle, 3, false).is_err(), "too few");
    assert!(parse_entry_bundle(&[0, 2, b'h'], 1, false).is_err(), "cut");
    assert!(parse_entry_bundle(&[0, 0, 9], 1, false).is_err(), "trailing");
}

#[test]
fn locations_never_use_plain_http() {
    assert_eq!(
        Location::parse("https://ita.acme.example/").unwrap(),
        Location::Https("https://ita.acme.example".to_owned())
    );
    assert_eq!(
        Location::parse("file:///srv/mirror").unwrap(),
        Location::Directory(PathBuf::from("/srv/mirror"))
    );
    assert_eq!(
        Location::parse("mirrors/acme").unwrap(),
        Location::Directory(PathBuf::from("mirrors/acme"))
    );
    for refused in [
        "http://ita.acme.example",
        "HTTP://ita.acme.example",
        "ftp://ita.acme.example",
        "http:ita.acme.example",
        "file://host/srv",
        "https://",
        "https://user@host",
        "",
    ] {
        assert!(Location::parse(refused).is_err(), "{refused}");
    }
    // An https location parses, and reading from it says that there is no https transport.
    let unavailable = Unavailable {
        url: "https://ita.acme.example".to_owned(),
    };
    let refusal = unavailable.get("checkpoint", 100).unwrap_err();
    assert!(refusal.contains("no https transport"), "{refusal}");
}

/// The owner `acme`, registered with its first key, and the key itself.
fn acme(registry: &mut TestRegistry) -> (ed25519_dalek::SigningKey, una_project::tlog::note::Key) {
    let key = keypair(60, "acme.laptop");
    registry.register("acme", &key.1);
    key
}

fn open<'t>(transport: &'t dyn Transport, registry: &TestRegistry) -> Result<Registry<'t>, String> {
    Registry::open(
        "acme",
        transport,
        &registry.pin(),
        &Point::Latest { now: NOW + 60 },
        None,
    )
}

#[test]
fn a_release_verifies_from_its_root_to_its_archive() {
    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    let (index, content) =
        registry.release("acme/ledger", "0.3.1", "Http[get]", "none", &[&key]);
    registry.checkpoint(NOW);
    let dir = scratch("end-to-end");
    registry.write(&dir);
    let mirror = Directory::new(&dir);
    let mut client = open(&mirror, &registry).unwrap();
    assert_eq!(client.point.time, NOW);
    let releases = client.releases("acme/ledger").unwrap();
    assert_eq!(releases.len(), 1);
    let release = &releases[0];
    assert_eq!(release.index, index);
    assert!(!release.compromised);
    assert_eq!(release.signer().unwrap().public, key.1.public);
    let state = client.state("acme/ledger", Line::Zero(3)).unwrap();
    assert_eq!(state[0].version, Version::parse_full("0.3.1").unwrap());
    assert_eq!(state[0].logged_at, NOW);
    // The archive agrees with the statement (rule 37).
    let bytes = client.archive(&content).unwrap();
    let tree = VerifiedTree::from_archive(&bytes).unwrap();
    let mut authority = BTreeMap::new();
    authority.insert(
        "Http".to_owned(),
        ["get".to_owned()].into_iter().collect(),
    );
    check_archive(&release.statement, &tree, Some(&authority)).unwrap();
    authority.insert("Files".to_owned(), ["read".to_owned()].into_iter().collect());
    let refusal = check_archive(&release.statement, &tree, Some(&authority)).unwrap_err();
    assert!(refusal.contains("Files[read]"), "{refusal}");
    // Its provenance verifies offline, as every load verifies it.
    let provenance = client.provenance(release, &key.1).unwrap();
    let expected = Expected {
        source: "acme".to_owned(),
        name: "acme/ledger".to_owned(),
        version: "0.3.1".to_owned(),
        digest: content.clone(),
        signed: signed_name("acme/ledger", &key.1.public),
        logged: index,
        root: registry.pin(),
        checkpoint_size: client.point.checkpoint.size,
        checkpoint_hash: hash_hex(&client.point.checkpoint.root),
    };
    verify(&provenance, &expected).unwrap();
    assert_eq!(owner_of("acme/ledger"), "acme");
    assert_eq!(owner_of("json"), owner::ITAUNA);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every byte a mirror serves is verified: a changed entry, tile, map proof, checkpoint or
/// archive is refused, and never taken because it comes from a local directory.
#[test]
fn a_tampered_mirror_is_refused() {
    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    let (_, content) = registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    registry.checkpoint(NOW);
    let dir = scratch("tampered");
    let hex = content.strip_prefix("sha256:").unwrap().to_owned();
    let size = registry.entries.len();
    let cases: Vec<(String, &str)> = vec![
        (format!("tile/entries/000.p/{size}"), "entry"),
        (format!("tile/0/000.p/{size}"), "tile"),
        (
            format!("map/{size}/{}", una_project::tlog::map::key_hex("acme/ledger")),
            "map",
        ),
        ("checkpoint".to_owned(), "checkpoint"),
        (format!("blobs/sha256/{hex}"), "archive"),
    ];
    for (path, what) in cases {
        let _ = std::fs::remove_dir_all(&dir);
        registry.write(&dir);
        let file = dir.join(&path);
        let mut bytes = std::fs::read(&file).unwrap();
        // Change one byte well inside the file.
        let at = bytes.len() / 2;
        bytes[at] ^= 0x01;
        std::fs::write(&file, &bytes).unwrap();
        let mirror = Directory::new(&dir);
        let result = open(&mirror, &registry).and_then(|mut client| {
            let releases = client.releases("acme/ledger")?;
            let bytes = client.archive(&content)?;
            let tree = VerifiedTree::from_archive(&bytes)?;
            check_archive(&releases[0].statement, &tree, None)
        });
        assert!(result.is_err(), "a tampered {what} was accepted");
    }
    // A link in the mirror is never followed.
    let _ = std::fs::remove_dir_all(&dir);
    registry.write(&dir);
    #[cfg(unix)]
    {
        let entries = dir.join(format!("tile/entries/000.p/{size}"));
        let elsewhere = dir.join("elsewhere");
        std::fs::rename(&entries, &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &entries).unwrap();
        let mirror = Directory::new(&dir);
        let mut client = open(&mirror, &registry).unwrap();
        assert!(client.releases("acme/ledger").is_err());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn checkpoints_need_witnesses_freshness_and_their_floor() {
    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    // One witness of a threshold of two.
    registry.checkpoint_with(&[Some(NOW), None, None]);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let refusal = open(&memory, &registry).err().unwrap();
    assert!(refusal.contains("fewer than its threshold"), "{refusal}");
    // A forward-dating witness does not make an old checkpoint fresh.
    registry.checkpoint_with(&[Some(NOW - 2 * DAY), Some(NOW - 2 * DAY), Some(NOW + 10 * DAY)]);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let refusal = open(&memory, &registry).err().unwrap();
    assert!(refusal.contains("more than 24 hours"), "{refusal}");
    // A local clock behind the witnesses is clock skew.
    registry.checkpoint(NOW + 3 * 3600);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let refusal = Registry::open(
        "acme",
        &memory,
        &registry.pin(),
        &Point::Latest { now: NOW },
        None,
    )
    .err()
    .unwrap();
    assert!(refusal.contains("clock"), "{refusal}");
    // A fresh one is accepted; a stored floor of the same log is consistent with it.
    let floor = String::from_utf8(registry.files["checkpoint"].clone()).unwrap();
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&key]);
    registry.checkpoint(NOW + 3 * 3600);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let opened = Registry::open(
        "acme",
        &memory,
        &registry.pin(),
        &Point::Latest { now: NOW + 4 * 3600 },
        Some(&floor),
    )
    .unwrap();
    assert!(!opened.below(&floor));
    // A fork: the stored floor is another log of the same size.
    let mut other = TestRegistry::new("acme");
    let other_key = acme(&mut other);
    other.release("acme/ledger", "0.3.9", "none", "none", &[&other_key]);
    other.checkpoint(NOW + 3 * 3600);
    let forked = String::from_utf8(other.files["checkpoint"].clone()).unwrap();
    let refusal = Registry::open(
        "acme",
        &memory,
        &registry.pin(),
        &Point::Latest { now: NOW + 4 * 3600 },
        Some(&forked),
    )
    .err()
    .unwrap();
    assert!(refusal.contains("forked"), "{refusal}");
    // A lock's resolution point is read by size and hash, with no freshness.
    let size = 2u64;
    let document =
        String::from_utf8(registry.files[&format!("checkpoints/{size}")].clone()).unwrap();
    let signed = una_project::tlog::note::parse_note(&document).unwrap();
    let checkpoint = una_project::tlog::note::parse_checkpoint(&signed.text).unwrap();
    let at = Point::At {
        size,
        hash: hash_hex(&checkpoint.root),
    };
    Registry::open("acme", &memory, &registry.pin(), &at, None).unwrap();
    let wrong = Point::At {
        size,
        hash: hash_hex(&[7u8; 32]),
    };
    assert!(Registry::open("acme", &memory, &registry.pin(), &wrong, None).is_err());
}

#[test]
fn the_root_chain_rotates_and_expires() {
    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    registry.checkpoint(NOW);
    // Version 2 rotates the third root key, signed by two keys of version 1 and of its own,
    // and expires before the checkpoint's witness time.
    let new_root = keypair(13, "acme.root.13");
    let roots = [&registry.keys.roots[0].1, &registry.keys.roots[1].1, &new_root.1];
    let text = registry::root_text("acme", 2, NOW - 1, &registry.keys, &roots);
    let document = registry::signed(&text, &[&registry.keys.roots[0], &registry.keys.roots[1]]);
    registry
        .files
        .insert("root/2".to_owned(), document.into_bytes());
    let memory = Memory {
        files: registry.files.clone(),
    };
    let refusal = open(&memory, &registry).err().unwrap();
    assert!(refusal.contains("expired"), "{refusal}");
    // Signed by one root key of version 1 only: refused.
    let text = registry::root_text("acme", 2, registry::EXPIRES, &registry.keys, &roots);
    let document = registry::signed(&text, &[&registry.keys.roots[0], &new_root]);
    registry
        .files
        .insert("root/2".to_owned(), document.into_bytes());
    let memory = Memory {
        files: registry.files.clone(),
    };
    assert!(open(&memory, &registry).is_err());
    let document = registry::signed(&text, &[&registry.keys.roots[0], &registry.keys.roots[1]]);
    registry
        .files
        .insert("root/2".to_owned(), document.into_bytes());
    let memory = Memory {
        files: registry.files.clone(),
    };
    let client = open(&memory, &registry).unwrap();
    assert_eq!(client.root.version, 2);
    assert_eq!(client.roots.len(), 2);
}

/// A map that hides an entry: the map counts one entry fewer than the chain holds, and the walk
/// catches it. A chain whose `previous` skips an entry is caught the same way.
#[test]
fn a_map_or_chain_that_hides_an_entry_is_refused() {
    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&key]);
    let mut head = registry.heads["acme/ledger"];
    head.count -= 1;
    registry.map.set("acme/ledger", &head);
    registry.heads.insert("acme/ledger".to_owned(), head);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let refusal = client.releases("acme/ledger").unwrap_err();
    assert!(refusal.contains("chain"), "{refusal}");
}

#[test]
fn a_second_statement_of_a_version_and_a_breach_of_rule_26_are_refused() {
    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/ledger", "0.3.1", "Http[get]", "none", &[&key]);
    registry.release("acme/ledger", "0.3.1", "Http[get]", "none", &[&key]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let refusal = open(&memory, &registry).unwrap().releases("acme/ledger").unwrap_err();
    assert!(refusal.contains("immutable"), "{refusal}");

    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/mailer", "2.1.0", "Http[get]", "none", &[&key]);
    registry.release("acme/mailer", "2.2.0", "Http[get, post]", "none", &[&key]);
    // A new major may need more.
    registry.release("acme/mailer", "3.0.0", "Http[get, post]", "modules mailer.keys", &[&key]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let refusal = open(&memory, &registry).unwrap().releases("acme/mailer").unwrap_err();
    assert!(refusal.contains("Http[post]") && refusal.contains("rule 26"), "{refusal}");

    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/mailer", "2.1.0", "Http[get, post]", "none", &[&key]);
    registry.release("acme/mailer", "2.2.0", "Http[get]", "none", &[&key]);
    registry.release("acme/mailer", "3.0.0", "Http[get, post]", "modules mailer.keys", &[&key]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    assert_eq!(
        open(&memory, &registry).unwrap().releases("acme/mailer").unwrap().len(),
        3
    );
}

#[test]
fn statements_need_the_owners_keys_valid_at_their_position() {
    let mut registry = TestRegistry::new("acme");
    let first = acme(&mut registry);
    let second = keypair(61, "acme.desktop");
    let stranger = keypair(62, "acme.laptop");
    // A key that is not the owner's, with the name of one.
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&stranger]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    assert!(open(&memory, &registry).unwrap().releases("acme/ledger").is_err());

    // Threshold 2: one key is not enough; then the first key is revoked as compromised since
    // a position, and the release it signed after that is skipped as yanked.
    let mut registry = TestRegistry::new("acme");
    let first_again = acme(&mut registry);
    registry.owner_event("acme", "add", &format!("key {}", key_text(&second.1)), &[&first_again]);
    registry.owner_event("acme", "threshold", "threshold 2", &[&first_again]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&first_again]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    assert!(open(&memory, &registry).unwrap().releases("acme/ledger").is_err());

    let mut registry = TestRegistry::new("acme");
    let first = first;
    registry.register("acme", &first.1);
    registry.owner_event("acme", "add", &format!("key {}", key_text(&second.1)), &[&first]);
    let (good, _) = registry.release("acme/ledger", "0.3.1", "none", "none", &[&first]);
    let (late, _) = registry.release("acme/ledger", "0.3.2", "none", "none", &[&first]);
    registry.owner_event(
        "acme",
        "revoke",
        &format!("fingerprint {}\ncompromised {good}", fingerprint_of(&first.1)),
        &[&second],
    );
    registry.release("acme/ledger", "0.3.3", "none", "none", &[&second]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let releases = client.releases("acme/ledger").unwrap();
    let compromised: Vec<(u64, bool)> = releases.iter().map(|r| (r.index, r.compromised)).collect();
    assert!(compromised.contains(&(good, false)), "{compromised:?}");
    assert!(compromised.contains(&(late, true)), "{compromised:?}");
    let state = client.state("acme/ledger", Line::Zero(3)).unwrap();
    assert!(state.iter().any(|r| r.yanked));
    // The revoked key signs nothing after its revocation.
    let mut after = registry;
    after.release("acme/ledger", "0.3.4", "none", "none", &[&first]);
    after.checkpoint(NOW);
    let memory = Memory {
        files: after.files.clone(),
    };
    assert!(open(&memory, &after).unwrap().releases("acme/ledger").is_err());
}

/// A recovery takes effect seven days of witness time after the checkpoint that first included
/// it, and governs the entries logged after the checkpoint where it does, unless a current key
/// cancels it first.
#[test]
fn recoveries_wait_seven_days_of_witness_time_and_can_be_cancelled() {
    let lost = keypair(60, "acme.laptop");
    let recovered = keypair(63, "acme.new");
    let build = |gap: u64, cancel: bool| -> TestRegistry {
        let mut registry = TestRegistry::new("acme");
        registry.register("acme", &lost.1);
        registry.checkpoint(NOW - 30 * DAY);
        let request = registry.recover("acme", "recover", &recovered.1, &[]);
        registry.checkpoint(NOW - 20 * DAY);
        if cancel {
            registry.owner_event("acme", "cancel", &format!("recovery {request}"), &[&lost]);
            registry.checkpoint(NOW - 19 * DAY);
        }
        // Another name's entry, so the next checkpoint is a new size.
        registry.append("other", "filler");
        registry.checkpoint(NOW - 20 * DAY + gap * DAY);
        registry.release("acme/ledger", "0.3.1", "none", "none", &[&recovered]);
        registry.checkpoint(NOW);
        registry
    };
    let opened = |registry: &TestRegistry| -> Result<Vec<LoggedRelease>, String> {
        let memory = Memory {
            files: registry.files.clone(),
        };
        let mut client = open(&memory, registry)?;
        client.releases("acme/ledger")
    };
    // Eight days: in effect before the release, which the new key signs.
    let registry = build(8, false);
    assert_eq!(opened(&registry).unwrap().len(), 1);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert!(history.pending.is_empty());
    assert!(history.changed_after(1));
    // Six days: in effect only at the last checkpoint, after the release was logged.
    assert!(opened(&build(6, false)).is_err());
    // Cancelled by the lost key, a current key: never in effect.
    assert!(opened(&build(8, true)).is_err());
}

#[test]
fn a_recovery_not_yet_in_effect_is_pending() {
    let lost = keypair(60, "acme.laptop");
    let recovered = keypair(63, "acme.new");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &lost.1);
    registry.recover("acme", "recover", &recovered.1, &[]);
    registry.checkpoint(NOW - 6 * DAY);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&recovered]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.pending.len(), 1);
    assert!(client.releases("acme/ledger").is_err());
}

#[test]
fn owner_entries_are_read_strictly() {
    let key = keypair(60, "acme.laptop");
    let good = format!(
        "ita-owner/1\nsource acme\nowner acme\nprevious none\nevent register\nkey {}\n",
        key_text(&key.1)
    );
    assert!(owner::parse_entry(&good).is_ok());
    for (from, to) in [
        ("event register", "event adopt"),
        ("previous none", "previous 01 sha256:00"),
        ("ita-owner/1", "ita-owner/2"),
        ("\nkey ", "\nkeys "),
    ] {
        assert!(owner::parse_entry(&good.replacen(from, to, 1)).is_err(), "{to}");
    }
    assert!(owner::parse_entry(&format!("{good}extra\n")).is_err());
    let cosigners = format!(
        "ita-owner/1\nsource acme\nowner acme\nprevious none\nevent cosigners\nthreshold 2\nkey {}\n",
        key_text(&key.1)
    );
    assert!(owner::parse_entry(&cosigners).is_err(), "threshold above the keys");
}

#[test]
fn back_dating_one_witness_moves_no_time() {
    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    // Two honest witnesses at NOW - 1 day; one back-dates to long ago.
    registry.checkpoint_with(&[Some(NOW - DAY), Some(NOW - DAY), Some(1)]);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = Registry::open(
        "acme",
        &memory,
        &registry.pin(),
        &Point::Latest { now: NOW },
        None,
    )
    .unwrap();
    let state = client.state("acme/ledger", Line::Zero(3)).unwrap();
    assert_eq!(state[0].logged_at, NOW - DAY);
}

#[test]
fn a_witness_whose_time_decreases_is_refused() {
    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    registry.checkpoint(NOW);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&key]);
    registry.checkpoint_with(&[Some(NOW + 60), Some(NOW + 60), Some(NOW - DAY)]);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = Registry::open(
        "acme",
        &memory,
        &registry.pin(),
        &Point::Latest { now: NOW + 120 },
        None,
    )
    .unwrap();
    let refusal = client.state("acme/ledger", Line::Zero(3)).unwrap_err();
    assert!(refusal.contains("decrease"), "{refusal}");
}
