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

use registry::{NOW, TestRegistry, hash_hex, keypair, key_text};
use una_project::client::{Floors, LoggedRelease, OpenError, Point, Registry, owner_of, shown};
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
    for refused in [
        "mirrors/acme",
        "/srv/mirror",
        "file:///srv/a%20b",
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
    registry.register("acme", &key);
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
            registry
                .files
                .keys()
                .find(|path| path.starts_with("map/0/"))
                .unwrap()
                .clone(),
            "map tile",
        ),
        ("checkpoint/latest".to_owned(), "checkpoint"),
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
        String::from_utf8(registry.files[&format!("checkpoint/{size}")].clone()).unwrap();
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
    registry.owner_event(
        "acme",
        &format!("add {}", key_text(&second.1)),
        2,
        &[&first_again, &second],
    );
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&first_again]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    assert!(open(&memory, &registry).unwrap().releases("acme/ledger").is_err());

    let mut registry = TestRegistry::new("acme");
    let first = first;
    registry.register("acme", &first);
    registry.owner_event(
        "acme",
        &format!("add {}", key_text(&second.1)),
        1,
        &[&first, &second],
    );
    let (good, _) = registry.release("acme/ledger", "0.3.1", "none", "none", &[&first]);
    let (late, _) = registry.release("acme/ledger", "0.3.2", "none", "none", &[&first]);
    registry.owner_event(
        "acme",
        &format!("revoke {} since {good}", first.1.name),
        1,
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
        registry.register("acme", &lost);
        registry.checkpoint(NOW - 30 * DAY);
        let request = registry.recover("acme", "recover", &recovered.1, &[]);
        registry.checkpoint(NOW - 20 * DAY);
        if cancel {
            registry.owner_event("acme", &format!("cancel {request}"), 1, &[&lost]);
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
    registry.register("acme", &lost);
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
    let other = keypair(61, "acme.desktop");
    let good = format!(
        "ita-owner/1\nsource acme\nowner acme\nprevious none\nthreshold 1\nkey {}\n",
        key_text(&key.1)
    );
    let read = owner::parse_entry(&good).unwrap();
    assert_eq!(read.threshold, 1);
    assert!(matches!(read.event, owner::Event::Register { cosigners: None, .. }));
    let previous = "previous 4 sha256:aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04";
    for bad in [
        good.replace("threshold 1", "threshold 2"),
        good.replace("threshold 1", "threshold 0"),
        good.replace("previous none", previous),
        good.replace("ita-owner/1", "ita-owner/2"),
        good.replace("owner acme\n", ""),
        good.replace("\nkey ", "\nkeys "),
        format!("{good}key {}\n", key_text(&key.1)),
        format!("{good}cosigner {}\n", key_text(&other.1)),
        format!("{good}cosigners 1\n"),
        format!("{good}extra line\n"),
        format!("{}{}", good, "x".repeat(70_000)),
    ] {
        assert!(owner::parse_entry(&bad).is_err(), "{bad}");
    }
    let with_cosigners = format!("{good}cosigner {}\ncosigners 1\n", key_text(&other.1));
    let read = owner::parse_entry(&with_cosigners).unwrap();
    assert!(matches!(read.event, owner::Event::Register { cosigners: Some((1, _)), .. }));
    let event = |action: &str, threshold: &str| {
        format!("ita-key/1\nsource acme\nowner acme\n{previous}\n{action}\nthreshold {threshold}\n")
    };
    for action in [
        format!("add {}", key_text(&other.1)),
        "revoke acme.laptop".to_owned(),
        "revoke acme.laptop since 3".to_owned(),
        format!("cosigners 1 {}", key_text(&other.1)),
        "cosigners none".to_owned(),
        format!("recover {}", key_text(&other.1)),
        "cancel 5".to_owned(),
        "overrule 6".to_owned(),
        format!("dispute {}", key_text(&other.1)),
    ] {
        assert!(owner::parse_entry(&event(&action, "1")).is_ok(), "{action}");
    }
    for (action, threshold) in [
        ("rotate acme.laptop".to_owned(), "1"),
        ("revoke acme.laptop since x".to_owned(), "1"),
        (format!("cosigners 2 {}", key_text(&other.1)), "1"),
        ("cosigners 1".to_owned(), "1"),
        ("cancel 05".to_owned(), "1"),
        ("revoke acme.laptop".to_owned(), "0"),
    ] {
        assert!(owner::parse_entry(&event(&action, threshold)).is_err(), "{action}");
    }
    // A key event is never a chain's first entry, and has one action.
    let first = event("revoke acme.laptop", "1").replace(previous, "previous none");
    assert!(owner::parse_entry(&first).is_err());
    let two = event("revoke acme.laptop", "1").replace("threshold 1\n", "cancel 3\nthreshold 1\n");
    assert!(owner::parse_entry(&two).is_err());
    assert_eq!(owner::chain_name("acme"), "owner:acme");
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

/// The map's tiles give the proof the in-memory map gives, for a name and for an absent one,
/// and a changed tile only fails.
#[test]
fn map_tiles_prove_what_the_map_proves() {
    use una_project::tlog::map::{Head, Map, build_tiles, prove_from_tiles, verify};
    let head = |i: u64| Head {
        index: i,
        leaf: leaf_hash(&i.to_be_bytes()),
        count: i + 1,
    };
    let mut map = Map::default();
    let mut heads: Vec<(String, Head)> = Vec::new();
    for i in 0..300u64 {
        let name = format!("owner{}/pkg{i}", i % 13);
        map.set(&name, &head(i));
        heads.push((name, head(i)));
    }
    let (root, tiles) = build_tiles(&heads);
    assert_eq!(root, map.root());
    let files: BTreeMap<String, Vec<u8>> = tiles.into_iter().collect();
    let mut fetch = |path: &str| Ok(files.get(path).cloned());
    for i in [0u64, 7, 299] {
        let name = format!("owner{}/pkg{i}", i % 13);
        let (found, proof) = prove_from_tiles(&root, &name, &mut fetch).unwrap();
        assert_eq!(found, Some(head(i)));
        assert_eq!(proof, map.prove(&name));
        assert!(verify(&root, &name, Some(&head(i)), &proof));
    }
    let (found, proof) = prove_from_tiles(&root, "nobody/here", &mut fetch).unwrap();
    assert_eq!(found, None);
    assert!(verify(&root, "nobody/here", None, &proof));
    // A tile changed, or missing: refused, never a different head.
    for path in files.keys().filter(|p| p.starts_with("map/0/") || p.starts_with("map/31/")) {
        let mut changed = files.clone();
        let bytes = changed.get_mut(path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let mut fetch = |p: &str| Ok(changed.get(p).cloned());
        let name = "owner0/pkg0";
        if let Ok((found, proof)) = prove_from_tiles(&root, name, &mut fetch) {
            // A tile off the name's path does not matter.
            assert_eq!(found, Some(head(0)));
            assert!(verify(&root, name, found.as_ref(), &proof));
        }
    }
    let mut empty = |_: &str| Ok(None);
    assert!(prove_from_tiles(&root, "owner0/pkg0", &mut empty).is_err());
    // The empty map has no tile.
    let (empty_root, none) = build_tiles(&[]);
    assert!(none.is_empty());
    let (found, _) = prove_from_tiles(&empty_root, "a/b", &mut empty).unwrap();
    assert_eq!(found, None);
}

#[test]
fn map_tiles_are_read_strictly() {
    use una_project::tlog::map::{MAX_TILE, decode_tile};
    assert!(decode_tile(&[0u8; 31], 0, &[]).is_err(), "short of its bitmap");
    assert!(decode_tile(&[0u8; 32], 0, &[]).is_err(), "no node");
    let mut one = vec![0u8; 32];
    one[0] = 1;
    assert!(decode_tile(&one, 0, &[]).is_err(), "cut inside a hash");
    one.extend_from_slice(&[0u8; 32]);
    assert!(decode_tile(&one, 0, &[]).is_err(), "an empty subtree listed");
    one.truncate(32);
    one.extend_from_slice(&[7u8; 32]);
    assert!(decode_tile(&one, 0, &[]).is_ok());
    one.push(0);
    assert!(decode_tile(&one, 0, &[]).is_err(), "trailing bytes");
    assert!(decode_tile(&vec![0u8; MAX_TILE + 1], 0, &[]).is_err());
    assert!(decode_tile(&one, 3, &[]).is_err(), "a prefix of the wrong length");
}

#[test]
fn mirrors_una_lists_https_and_file_locations_only() {
    use una_project::mirrors::parse;
    let (mirrors, errors) = parse(
        "mirrors\n  itaty\n    \"https://ita-mirror.example.net\"\n    \"file:///srv/ita-mirror\"\n  acme\n    \"file:///srv/acme\"\n",
    );
    assert_eq!(errors, []);
    assert_eq!(mirrors.of("itaty").len(), 2);
    assert_eq!(
        mirrors.of("acme")[0].location,
        Location::Directory(PathBuf::from("/srv/acme"))
    );
    assert!(mirrors.of("other").is_empty());
    for bad in [
        "mirrors\n  itaty\n    \"http://ita-mirror.example.net\"\n",
        "mirrors\n  itaty\n    \"ftp://ita-mirror.example.net\"\n",
        "mirrors\n  itaty\n    \"/srv/ita-mirror\"\n",
        "mirrors\n  itaty\n    \"file:///a\"\n    \"file:///a\"\n",
        "mirrors\n  itaty\n  itaty\n",
        "sources\n",
        "mirrors\n  Itaty\n",
        "mirrors\n  itaty\n    file\n",
        "",
    ] {
        let (_, errors) = parse(bad);
        assert!(!errors.is_empty(), "{bad:?}");
    }
}

/// A registry is read from the first place whose copy verifies: a mirror that serves a bad copy
/// is skipped and named, one with nothing is unavailable, and the source's own is tried last.
#[test]
fn the_first_place_whose_copy_verifies_is_taken() {
    use una_project::mirrors::{Attempt, first_verified};
    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    registry.checkpoint(NOW);
    let good = Memory {
        files: registry.files.clone(),
    };
    let mut bad_files = registry.files.clone();
    let checkpoint = bad_files.get_mut("checkpoint").unwrap();
    let at = checkpoint.len() / 2;
    checkpoint[at] ^= 1;
    let bad = Memory { files: bad_files };
    let empty = Memory {
        files: BTreeMap::new(),
    };
    let places: Vec<(String, Result<&dyn Transport, String>)> = vec![
        ("https://ita.acme.example".to_owned(), Err("no https transport".to_owned())),
        ("file:///bad".to_owned(), Ok(&bad)),
        ("file:///empty".to_owned(), Ok(&empty)),
        ("file:///good".to_owned(), Ok(&good)),
    ];
    let pin = registry.pin();
    let fetched = first_verified(&places, &mut |transport| {
        Registry::open_with(
            "acme",
            transport,
            &pin,
            &Point::Latest { now: NOW + 60 },
            &Floors::default(),
        )
        .map(|registry| registry.point.checkpoint.size)
        .map_err(|error| match error {
            OpenError::Unavailable(why) => Attempt::Unavailable(why),
            OpenError::Refused(why) => Attempt::Refused(why),
        })
    });
    assert_eq!(fetched.value, Some((2, "file:///good".to_owned())));
    assert_eq!(fetched.refused.len(), 1);
    assert_eq!(fetched.refused[0].from, "file:///bad");
    assert_eq!(fetched.unavailable.len(), 2);
}

/// A back-dating witness, and a place that leaves an honest witness's cosignature out of the
/// checkpoint that first includes a recovery, cannot start its wait earlier: the start is the
/// k-th earliest cosignature, so the recovery is not in effect before seven days.
#[test]
fn a_recovery_wait_starts_at_its_logged_time() {
    let lost = keypair(60, "acme.laptop");
    let recovered = keypair(63, "acme.new");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &lost);
    // The first witness lies about time; it does not cosign the registration's checkpoint.
    registry.checkpoint_with(&[None, Some(NOW - 30 * DAY), Some(NOW - 30 * DAY)]);
    registry.recover("acme", "recover", &recovered.1, &[]);
    // The liar back-dates by a month; the third witness's cosignature is left out.
    registry.checkpoint_with(&[Some(NOW - 60 * DAY), Some(NOW - 20 * DAY), None]);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 15 * DAY);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&recovered]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    // Logged at the second earliest of the witnesses' earliest cosignatures: twenty days ago.
    assert_eq!(client.logged_at(1).unwrap(), NOW - 20 * DAY);
    // Seven days after that is reached only at the last checkpoint, after the release.
    assert_eq!(client.recovery_effect(1).unwrap(), Some(4));
    assert!(client.releases("acme/ledger").is_err());
}

/// Any entry of an owner's chain at or after a point is a key change, and a pending recovery
/// stays pending.
#[test]
fn every_owner_entry_after_a_point_is_a_key_change() {
    let key = keypair(60, "acme.laptop");
    let trustee = keypair(70, "trustee.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &key);
    registry.checkpoint(NOW - DAY);
    let point = registry.entries.len() as u64;
    registry.owner_event(
        "acme",
        &format!("cosigners 1 {}", key_text(&trustee.1)),
        1,
        &[&key],
    );
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert!(history.changed_after(point), "a cosigners entry");
    assert!(!history.changed_after(point + 1));
    // A dispute pending at the point is a pending key change.
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &key);
    registry.recover("acme", "dispute", &keypair(64, "acme.disputed").1, &[]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.pending.len(), 1);
    assert!(matches!(history.pending[0].change, owner::Change::Dispute(_)));
    assert!(history.changed_after(1));
}

/// A key compromised since a position voids what it alone signed after it, and what depends
/// on that: a key it added signs nothing.
#[test]
fn compromise_cascades_to_the_entries_it_signed() {
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let b = keypair(62, "acme.b");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &d], 1, &[], 0);
    // At 1, A adds B; at 2, D revokes A as compromised since 0.
    let added = registry.owner_event("acme", &format!("add {}", key_text(&b.1)), 1, &[&a, &b]);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&d]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&b]);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&d]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.voided, vec![added]);
    assert_eq!(history.void_keys, vec![b.1.clone()]);
    // What B signed is skipped as compromised; the honest 0.3.2 stands.
    let releases = client.releases("acme/ledger").unwrap();
    let marks: Vec<(String, bool)> = releases
        .iter()
        .map(|r| (r.statement.version.to_string(), r.compromised))
        .collect();
    assert_eq!(
        marks,
        vec![("0.3.1".to_owned(), true), ("0.3.2".to_owned(), false)]
    );
    // Without the compromise, B signs.
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &d], 1, &[], 0);
    registry.owner_event("acme", &format!("add {}", key_text(&b.1)), 1, &[&a, &b]);
    registry.owner_event("acme", "revoke acme.a", 1, &[&d]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&b]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    assert_eq!(open(&memory, &registry).unwrap().releases("acme/ledger").unwrap().len(), 1);
}

/// A recovery cancelled twice stands again only when both cancellations are overruled.
#[test]
fn an_overrule_lifts_one_cancellation() {
    let lost = keypair(60, "acme.a");
    let other = keypair(61, "acme.b");
    let trustee = keypair(70, "trustee.a");
    let recovered = keypair(63, "acme.new");
    let build = |overrules: usize| -> TestRegistry {
        let mut registry = TestRegistry::new("acme");
        registry.register_keys("acme", &[&lost, &other], 1, &[&trustee], 1);
        registry.checkpoint(NOW - 30 * DAY);
        let request = registry.recover("acme", "recover", &recovered.1, &[&trustee]);
        let first = registry.owner_event("acme", &format!("cancel {request}"), 1, &[&lost]);
        let second = registry.owner_event("acme", &format!("cancel {request}"), 1, &[&other]);
        for cancel in [first, second].into_iter().take(overrules) {
            registry.owner_event("acme", &format!("overrule {cancel}"), 1, &[&trustee]);
        }
        registry.checkpoint(NOW - 20 * DAY);
        registry.append("other", "filler");
        registry.checkpoint(NOW - 10 * DAY);
        registry.release("acme/ledger", "0.3.1", "none", "none", &[&recovered]);
        registry.checkpoint(NOW);
        registry
    };
    for (overrules, stands) in [(0, false), (1, false), (2, true)] {
        let registry = build(overrules);
        let memory = Memory {
            files: registry.files.clone(),
        };
        let releases = open(&memory, &registry).unwrap().releases("acme/ledger");
        assert_eq!(releases.is_ok(), stands, "{overrules} overrules");
    }
}

/// A place that hides a newer root version than one already accepted is refused, and so is
/// a point that does not extend the lock's.
#[test]
fn floors_refuse_an_older_root_and_a_point_off_the_lock() {
    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let floors = Floors {
        checkpoint: None,
        root_version: Some(2),
    };
    let refused = Registry::open_with(
        "acme",
        &memory,
        &registry.pin(),
        &Point::Latest { now: NOW + 60 },
        &floors,
    );
    assert!(matches!(refused, Err(OpenError::Refused(why)) if why.contains("root version")));
    let mut client = open(&memory, &registry).unwrap();
    let size = client.point.checkpoint.size;
    let own = hash_hex(&client.point.checkpoint.root);
    assert!(client.extends(size, &own).is_ok());
    assert!(client.extends(size, &hash_hex(&[9u8; 32])).is_err(), "another log");
    assert!(client.extends(size + 1, &own).is_err(), "a point below the lock's");
    let prefix = hash_hex(&tree_root(
        &registry.entries[..1]
            .iter()
            .map(|e| leaf_hash(e))
            .collect::<Vec<_>>(),
    ));
    assert!(client.extends(1, &prefix).is_ok());
    // A place that does not serve is unavailable, one that serves a bad copy refused.
    let empty = Memory {
        files: BTreeMap::new(),
    };
    let missing = Registry::open_with(
        "acme",
        &empty,
        &registry.pin(),
        &Point::Latest { now: NOW + 60 },
        &Floors::default(),
    );
    assert!(matches!(missing, Err(OpenError::Unavailable(_))));
}

#[test]
fn notes_bound_their_signature_lines() {
    use una_project::tlog::note::{MAX_SIGNATURES, parse_note};
    let line = "\u{2014} k AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\n";
    assert!(parse_note(&format!("text\n\n{line}")).is_ok());
    assert!(parse_note(&format!("text\n\n{line}{line}")).is_err(), "one line per key");
    let mut many = "text\n\n".to_owned();
    for i in 0..=MAX_SIGNATURES {
        many.push('\u{2014}');
        many.push_str(&format!(
            " k{i} AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\n"
        ));
    }
    assert!(parse_note(&many).is_err(), "too many lines");
}

#[test]
fn sizes_at_the_top_of_the_range_end() {
    use una_project::tlog::note::parse_checkpoint;
    use una_project::tlog::tile::split;
    assert_eq!(split(2), 1);
    assert_eq!(split(5), 4);
    assert_eq!(split(8), 4);
    assert_eq!(split(u64::MAX), 1 << 63);
    let root = "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=";
    assert!(parse_checkpoint(&format!("log\n9223372036854775807\n{root}\n")).is_ok());
    assert!(parse_checkpoint(&format!("log\n9223372036854775808\n{root}\n")).is_err());
}

#[test]
fn messages_escape_and_bound_what_a_place_served() {
    let served = format!(
        "abc\u{202e}def\u{7}\u{200b}\u{feff}\u{e0041}\u{2028}{}",
        "x".repeat(1000)
    );
    let message = shown(&served);
    assert!(
        message.starts_with("abc\\u{202e}def\\u{7}\\u{200b}\\u{feff}\\u{e0041}\\u{2028}"),
        "{message}"
    );
    assert!(message.chars().count() < 450);
    assert!(!message.chars().any(char::is_control));
    // The reason before what was served always survives: a long served root is cut, not the
    // sentence that says what failed.
    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    registry.checkpoint(NOW);
    let mut files = registry.files.clone();
    files.insert("root/2".to_owned(), "y".repeat(5000).into_bytes());
    let memory = Memory { files };
    let refusal = open(&memory, &registry).err().unwrap();
    assert!(refusal.starts_with("the registry `acme` (memory): "), "{refusal}");
    assert!(refusal.len() < 600, "{refusal}");
}

/// Two writers at once, each through its own handle of the floor lock, storing different
/// sizes many times: the floor ends at the larger, and a reader never sees it decrease. A lock
/// file left by a holder that ended blocks nothing.
#[test]
fn floors_never_move_backwards() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use una_project::itaoca::{Itaoca, registry_id};
    use una_project::tlog::note::{parse_checkpoint, parse_note};
    let dir = scratch("floors");
    let itaoca = Itaoca { root: dir.clone() };
    let id = registry_id("sha256:0101010101010101010101010101010101010101010101010101010101010101");
    // A lock file a holder that ended left behind: it exists, and nobody holds its lock.
    std::fs::create_dir_all(dir.join("registry").join(&id)).unwrap();
    std::fs::write(dir.join("registry").join(&id).join("floor.lock"), b"").unwrap();
    let checkpoint = |size: u64| {
        format!(
            "log\n{size}\n47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=\n\n{dash} log AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\n",
            dash = '\u{2014}'
        )
    };
    let size_of = |text: &str| parse_checkpoint(&parse_note(text).unwrap().text).unwrap().size;
    itaoca
        .store_checkpoint(&id, &checkpoint(1), &mut |_| Ok(()))
        .unwrap();
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let writers: Vec<_> = [(1u64, 40u64), (2, 39)]
            .into_iter()
            .map(|(low, high)| {
                let itaoca = &itaoca;
                let id = &id;
                let checkpoint = &checkpoint;
                scope.spawn(move || {
                    for round in 0..30u64 {
                        let size = if round % 2 == 0 { low + round } else { high - round };
                        itaoca
                            .store_checkpoint(id, &checkpoint(size.max(1)), &mut |_| Ok(()))
                            .unwrap();
                    }
                })
            })
            .collect();
        let itaoca = &itaoca;
        let id = &id;
        let done = &done;
        let size_of = &size_of;
        let reader = scope.spawn(move || {
            let mut last = 0;
            while !done.load(Ordering::Relaxed) {
                if let Some(text) = itaoca.load_checkpoint(id).unwrap() {
                    let size = size_of(&text);
                    assert!(size >= last, "the floor went from {last} to {size}");
                    last = size;
                }
            }
        });
        for writer in writers {
            writer.join().unwrap();
        }
        done.store(true, Ordering::Relaxed);
        reader.join().unwrap();
    });
    // The largest any writer stored: 40 - 1, at the first writer's second round.
    let stored = itaoca.load_checkpoint(&id).unwrap().unwrap();
    assert_eq!(size_of(&stored), 39);
    // A smaller one, or one the consistency check refuses, is never stored.
    itaoca
        .store_checkpoint(&id, &checkpoint(3), &mut |_| Ok(()))
        .unwrap();
    assert!(
        itaoca
            .store_checkpoint(&id, &checkpoint(50), &mut |_| Err("another log".to_owned()))
            .is_err()
    );
    assert_eq!(size_of(&itaoca.load_checkpoint(&id).unwrap().unwrap()), 39);
    itaoca.store_root_version(&id, 3).unwrap();
    itaoca.store_root_version(&id, 2).unwrap();
    assert_eq!(itaoca.load_root_version(&id).unwrap(), Some(3));
    // The lock file stays; its existence means nothing.
    assert!(dir.join("registry").join(&id).join("floor.lock").exists());
    // A registry's floors are its own: another pin is another identity, and the identity is
    // the pin's alone, whatever name `sources` gives it.
    let other = registry_id("sha256:0202020202020202020202020202020202020202020202020202020202020202");
    assert_ne!(id, other);
    assert_eq!(itaoca.load_checkpoint(&other).unwrap(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A key revoked as compromised since before it was registered is compromised since its
/// registration only: the owner keeps its other keys, and their releases stand.
#[test]
fn a_compromise_never_undoes_a_registration() {
    let x = keypair(60, "acme.x");
    let y = keypair(61, "acme.y");
    let mut registry = TestRegistry::new("acme");
    registry.append("other", "one");
    registry.append("other", "two");
    registry.register_keys("acme", &[&x, &y], 1, &[], 0);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&y]);
    registry.owner_event("acme", "revoke acme.x since 0", 1, &[&y]);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&y]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.compromised.get(&x.1.public), Some(&2), "clamped to the registration");
    assert_eq!(history.at(10).unwrap().keys, vec![y.1.clone()]);
    let releases = client.releases("acme/ledger").unwrap();
    assert_eq!(releases.len(), 2);
    assert!(releases.iter().all(|r| !r.compromised));
}

/// A thief holding key A revokes the owner's other key D; the owner, with D, revokes A as
/// compromised since before that. The owner wins: the thief's revocation is void.
#[test]
fn the_thiefs_revocation_does_not_stand() {
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &d], 1, &[], 0);
    let thief = registry.owner_event("acme", "revoke acme.d", 1, &[&a]);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&d]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&d]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.voided, vec![thief]);
    assert_eq!(history.at(10).unwrap().keys, vec![d.1.clone()]);
    assert_eq!(client.releases("acme/ledger").unwrap().len(), 1);
}

/// Two keys each revoke the other as compromised since before the other's revocation: both
/// claims stand, so both keys are compromised, the owner has no sound key, and what either
/// signed after its point is skipped; the chain is not refused.
#[test]
fn revocations_that_cancel_each_other_compromise_both_keys() {
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &d], 1, &[], 0);
    registry.owner_event("acme", "revoke acme.d since 0", 1, &[&a]);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&d]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.compromised.len(), 2);
    assert_eq!(history.voided, vec![1, 2]);
    let releases = client.releases("acme/ledger").unwrap();
    assert!(releases[0].compromised);
}

/// After revocations that cancel each other, a recovery, which needs no owner key, takes effect
/// seven days on and cures the chain: the new key's releases count.
#[test]
fn a_recovery_cures_revocations_that_cancel_each_other() {
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let recovered = keypair(63, "acme.new");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &d], 1, &[], 0);
    registry.owner_event("acme", "revoke acme.d since 0", 1, &[&a]);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&d]);
    registry.recover("acme", "recover", &recovered.1, &[]);
    registry.checkpoint(NOW - 20 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 10 * DAY);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&recovered]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let releases = client.releases("acme/ledger").unwrap();
    assert_eq!(releases.len(), 1);
    assert!(!releases[0].compromised);
}

/// After a recovery, the owner revokes its former key as compromised since a position: the keys
/// held do not change, and what the former key signed after that position is skipped.
#[test]
fn a_former_key_can_be_revoked_as_compromised() {
    let a = keypair(60, "acme.a");
    let recovered = keypair(63, "acme.new");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 30 * DAY);
    // The key is stolen: the thief publishes with it, and the owner asks a recovery.
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&a]);
    registry.recover("acme", "recover", &recovered.1, &[]);
    registry.checkpoint(NOW - 20 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 10 * DAY);
    registry.owner_event("acme", "revoke acme.a since 1", 1, &[&recovered]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.compromised.get(&a.1.public), Some(&1));
    assert_eq!(history.at(10).unwrap().keys, vec![recovered.1.clone()]);
    let marks: Vec<(String, bool)> = client
        .releases("acme/ledger")
        .unwrap()
        .iter()
        .map(|r| (r.statement.version.to_string(), r.compromised))
        .collect();
    assert_eq!(
        marks,
        vec![("0.3.1".to_owned(), false), ("0.3.2".to_owned(), true)]
    );
}

/// A revocation names an owner key, never a recovery cosigner of the same name.
#[test]
fn a_claim_never_names_a_cosigner() {
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let namesake = keypair(70, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &d], 1, &[&namesake], 1);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&d]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.compromised.get(&a.1.public), Some(&0));
    assert!(!history.compromised.contains_key(&namesake.1.public));
}

/// The replay's work grows linearly with a chain padded with void entries: the keys the chain
/// names are checked only for an entry not signed as required after a void one.
#[test]
fn replay_stays_linear_with_void_entries() {
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let mut per_entry = Vec::new();
    for n in [50u64, 200, 800] {
        let mut registry = TestRegistry::new("acme");
        registry.register_keys("acme", &[&a, &d], 1, &[], 0);
        for _ in 0..n {
            registry.owner_event("acme", "cosigners none", 1, &[&a]);
        }
        registry.owner_event("acme", "revoke acme.a since 0", 1, &[&d]);
        registry.checkpoint(NOW);
        let memory = Memory {
            files: registry.files.clone(),
        };
        let mut client = open(&memory, &registry).unwrap();
        let history = client.history("acme").unwrap();
        assert_eq!(history.voided.len() as u64, n);
        per_entry.push(history.checks as f64 / n as f64);
    }
    assert!(per_entry[2] <= per_entry[0] * 2.0, "{per_entry:?}");
}

/// After a void entry, an entry signed by no key the chain names still refuses the chain.
#[test]
fn an_entry_signed_by_no_known_key_refuses_even_after_a_void_one() {
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let b = keypair(62, "acme.b");
    let stranger = keypair(69, "acme.stranger");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &d], 1, &[], 0);
    registry.owner_event("acme", &format!("add {}", key_text(&b.1)), 1, &[&a, &b]);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&d]);
    registry.owner_event("acme", "cosigners none", 1, &[&stranger]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    assert!(client.history("acme").is_err());
}

/// A thief holding A from position 1 claims D compromised; the owner recovers to N and claims
/// A since 1. The thief's lone claim does not stand: D's releases count.
#[test]
fn a_thiefs_lone_claim_does_not_stand() {
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &d], 1, &[], 0);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&d]);
    registry.checkpoint(NOW - 40 * DAY);
    registry.owner_event("acme", "revoke acme.d since 0", 1, &[&a]);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&d]);
    registry.recover("acme", "recover", &n.1, &[]);
    registry.checkpoint(NOW - 30 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 20 * DAY);
    registry.owner_event("acme", "revoke acme.a since 1", 1, &[&n]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert!(!history.compromised.contains_key(&d.1.public));
    assert_eq!(history.compromised.get(&a.1.public), Some(&1));
    let releases = client.releases("acme/ledger").unwrap();
    assert_eq!(releases.len(), 2);
    assert!(releases.iter().all(|r| !r.compromised));
}

/// A thief steals N, the key a recovery brought in, and claims the former keys compromised;
/// the owner recovers again to M and claims N. The former keys' releases stand.
#[test]
fn a_thief_of_a_recovered_key_cannot_compromise_the_former_keys() {
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let n = keypair(63, "acme.n");
    let m = keypair(64, "acme.m");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &d], 1, &[], 0);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&d]);
    registry.recover("acme", "recover", &n.1, &[]);
    registry.checkpoint(NOW - 40 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 30 * DAY);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&n]);
    registry.owner_event("acme", "revoke acme.d since 0", 1, &[&n]);
    registry.recover("acme", "recover", &m.1, &[]);
    registry.checkpoint(NOW - 20 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 10 * DAY);
    registry.owner_event("acme", "revoke acme.n since 4", 1, &[&m]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.compromised.len(), 1, "{:?}", history.compromised);
    assert_eq!(history.compromised.get(&n.1.public), Some(&4));
    let releases = client.releases("acme/ledger").unwrap();
    assert!(releases.iter().all(|r| !r.compromised));
}

/// A thief adds more keys than the chain records; the owner claims the stolen key and
/// recovers: the chain is not refused, and the owner's names resolve with the new key.
#[test]
fn past_the_known_cap_entries_are_void_never_a_refusal() {
    use ed25519_dalek::SigningKey;
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let recovered = keypair(63, "acme.new");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &d], 1, &[], 0);
    registry.checkpoint(NOW - 40 * DAY);
    for i in 0..1100u32 {
        let mut seed = [7u8; 32];
        seed[..4].copy_from_slice(&i.to_be_bytes());
        let signing = SigningKey::from_bytes(&seed);
        let key = una_project::tlog::note::Key {
            name: format!("acme.t{i}"),
            public: signing.verifying_key().to_bytes(),
        };
        let pair = (signing, key);
        registry.owner_event("acme", &format!("add {}", key_text(&pair.1)), 1, &[&a, &pair]);
    }
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&d]);
    registry.recover("acme", "recover", &recovered.1, &[]);
    registry.checkpoint(NOW - 20 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 10 * DAY);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&recovered]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.voided.len(), 1100);
    let releases = client.releases("acme/ledger").unwrap();
    assert_eq!(releases.len(), 1);
    assert!(!releases[0].compromised);
}

/// The thief adds T with the stolen A; the owner revokes T and claims A since before the add,
/// in either order: the revocation of T, a key only a void entry brought in, changes nothing,
/// and the chain stands.
#[test]
fn revoking_a_key_whose_addition_is_void_changes_nothing() {
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let t = keypair(62, "acme.t");
    for claim_first in [false, true] {
        for since in ["", " since 0"] {
            let mut registry = TestRegistry::new("acme");
            registry.register_keys("acme", &[&a, &d], 1, &[], 0);
            registry.owner_event("acme", &format!("add {}", key_text(&t.1)), 1, &[&a, &t]);
            let revoke_t = format!("revoke acme.t{since}");
            if claim_first {
                registry.owner_event("acme", "revoke acme.a since 0", 1, &[&d]);
                registry.owner_event("acme", &revoke_t, 1, &[&d]);
            } else {
                registry.owner_event("acme", &revoke_t, 1, &[&d]);
                registry.owner_event("acme", "revoke acme.a since 0", 1, &[&d]);
            }
            registry.release("acme/ledger", "0.3.1", "none", "none", &[&d]);
            registry.checkpoint(NOW);
            let memory = Memory {
                files: registry.files.clone(),
            };
            let mut client = open(&memory, &registry).unwrap();
            let history = client.history("acme");
            assert!(history.is_ok(), "{claim_first} {since:?}: {:?}", history.err());
            let releases = client.releases("acme/ledger").unwrap();
            assert!(!releases[0].compromised);
        }
    }
}

/// The history of `acme` read from `registry` at its latest checkpoint.
fn history_of(registry: &TestRegistry) -> Result<owner::History, String> {
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, registry)?;
    client.history("acme")
}

/// The chain that locked a single-key owner out: a thief holding its only key designates
/// cosigners of its own, which a recovery would need and which bar a dispute. The designation
/// waits seven days, so the cosigners in effect are still none and the owner's dispute is
/// logged; once it takes effect, the owner claims the stolen key. The designation is void, the
/// thief's release is skipped, and the new key signs what counts.
#[test]
fn a_designation_after_a_theft_no_longer_blocks_the_owners_dispute() {
    let a = keypair(60, "acme.a");
    let thiefs = keypair(71, "thief.cosigner");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.checkpoint(NOW - 30 * DAY);
    let designation = registry.owner_event(
        "acme",
        &format!("cosigners 1 {}", key_text(&thiefs.1)),
        1,
        &[&a],
    );
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 29 * DAY);
    let dispute = registry.recover("acme", "dispute", &n.1, &[]);
    registry.checkpoint(NOW - 28 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 20 * DAY);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&n]);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&n]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert!(history.voided.contains(&designation));
    assert!(history.took_effect.contains_key(&dispute));
    assert_eq!(history.cosigners, None);
    assert_eq!(history.frozen, None);
    assert_eq!(history.compromised.get(&a.1.public), Some(&0));
    let releases = client.releases("acme/ledger").unwrap();
    assert_eq!(releases.len(), 2);
    assert!(releases[0].compromised);
    assert!(!releases[1].compromised);
    assert_eq!(releases[1].signers, vec![n.1.clone()]);
}

/// A co-signer change waits seven days; any one current key of the owner cancels it meanwhile,
/// and then it never takes effect. Uncancelled, it takes effect.
#[test]
fn a_pending_designation_is_cancelled_by_any_current_key() {
    let a = keypair(60, "acme.a");
    let b = keypair(61, "acme.b");
    let z = keypair(71, "thief.cosigner");
    let build = |cancel: bool| -> (TestRegistry, u64, Option<u64>) {
        let mut registry = TestRegistry::new("acme");
        registry.register_keys("acme", &[&a, &b], 1, &[], 0);
        registry.checkpoint(NOW - 30 * DAY);
        let designation = registry.owner_event(
            "acme",
            &format!("cosigners 1 {}", key_text(&z.1)),
            1,
            &[&a],
        );
        registry.checkpoint(NOW - 29 * DAY);
        let cancelled = cancel.then(|| {
            registry.owner_event("acme", &format!("cancel {designation}"), 1, &[&b])
        });
        registry.append("other", "filler");
        registry.checkpoint(NOW);
        (registry, designation, cancelled)
    };
    let (registry, designation, cancelled) = build(true);
    let history = history_of(&registry).unwrap();
    assert_eq!(history.cosigners, None);
    assert!(history.took_effect.is_empty());
    assert!(history.pending.is_empty());
    let cancel = cancelled.unwrap();
    assert!(
        history
            .cancellations
            .get(&designation)
            .is_some_and(|cancels| cancels.contains(&cancel)),
        "{:?}",
        history.cancellations
    );
    let (registry, designation, _) = build(false);
    let history = history_of(&registry).unwrap();
    assert_eq!(history.cosigners, Some((1, vec![z.1.clone()])));
    assert!(history.took_effect.contains_key(&designation));
}

/// A thief's designation is pending, so reported, for seven days, and a dispute can be logged
/// meanwhile; once its seven days pass with nobody cancelling it, it is in effect, and a dispute
/// logged after it refuses the chain, as one of an owner with cosigners does.
#[test]
fn a_thiefs_designation_takes_effect_only_after_seven_days_uncancelled() {
    let a = keypair(60, "acme.a");
    let z = keypair(71, "thief.cosigner");
    let n = keypair(63, "acme.n");
    // The designation logged `days` ago, a checkpoint two days ago, then a dispute if asked.
    let build = |days: u64, dispute: bool| -> (TestRegistry, u64) {
        let mut registry = TestRegistry::new("acme");
        registry.register("acme", &a);
        registry.checkpoint(NOW - 40 * DAY);
        let designation = registry.owner_event(
            "acme",
            &format!("cosigners 1 {}", key_text(&z.1)),
            1,
            &[&a],
        );
        registry.checkpoint(NOW - days * DAY);
        registry.append("other", "filler");
        registry.checkpoint(NOW - 2 * DAY);
        if dispute {
            registry.recover("acme", "dispute", &n.1, &[]);
        } else {
            registry.append("other", "another filler");
        }
        registry.checkpoint(NOW);
        (registry, designation)
    };
    // Logged three days ago: pending, and reported as such.
    let (registry, designation) = build(3, false);
    let history = history_of(&registry).unwrap();
    assert_eq!(history.cosigners, None);
    assert_eq!(
        history.pending,
        vec![owner::Pending {
            index: designation,
            change: owner::Change::Cosigners(Some((1, vec![z.1.clone()]))),
        }]
    );
    assert!(history.took_effect.is_empty());
    // A dispute logged while it is pending is one of an owner with no cosigners in effect.
    let (registry, _) = build(3, true);
    let history = history_of(&registry).unwrap();
    assert_eq!(history.pending.len(), 2);
    // Logged ten days ago: in effect from the checkpoint two days ago.
    let (registry, designation) = build(10, false);
    let history = history_of(&registry).unwrap();
    assert_eq!(history.cosigners, Some((1, vec![z.1.clone()])));
    assert!(history.took_effect.contains_key(&designation));
    assert!(history.pending.is_empty());
    // A dispute logged after it took effect is one of an owner with cosigners: refused.
    let (registry, _) = build(10, true);
    assert!(history_of(&registry).is_err());
}

/// Two disputes whose waits overlap: none takes effect, and the chain is frozen from the
/// second. The release logged before stands; those logged after are skipped, whoever signs
/// them, and the name is not resolved.
#[test]
fn competing_disputes_freeze_the_chain() {
    let a = keypair(60, "acme.a");
    let m = keypair(72, "acme.m");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 30 * DAY);
    registry.recover("acme", "dispute", &m.1, &[]);
    registry.checkpoint(NOW - 29 * DAY);
    let second = registry.recover("acme", "dispute", &n.1, &[]);
    registry.checkpoint(NOW - 28 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 20 * DAY);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&m]);
    let last = registry.release("acme/ledger", "0.3.3", "none", "none", &[&n]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.frozen, Some(second));
    assert!(history.took_effect.is_empty());
    assert!(history.pending.is_empty());
    assert_eq!(history.at(last).unwrap().keys, vec![a.1.clone()]);
    let releases = client.releases("acme/ledger").unwrap();
    assert_eq!(releases.len(), 3);
    assert!(!releases[0].compromised);
    assert!(releases[1].compromised && releases[2].compromised);
    let refusal = client.state("acme/ledger", Line::Zero(3)).unwrap_err();
    assert!(refusal.contains("frozen"), "{refusal}");
}

/// A dispute logged once the chain is frozen changes nothing, nor does a recovery or a key
/// event; a revocation with `since` still makes its claim.
#[test]
fn a_dispute_after_the_freeze_changes_nothing() {
    let a = keypair(60, "acme.a");
    let m = keypair(72, "acme.m");
    let n = keypair(63, "acme.n");
    let p = keypair(64, "acme.p");
    let b = keypair(61, "acme.b");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 40 * DAY);
    registry.recover("acme", "dispute", &m.1, &[]);
    let second = registry.recover("acme", "dispute", &n.1, &[]);
    registry.checkpoint(NOW - 39 * DAY);
    let third = registry.recover("acme", "dispute", &p.1, &[]);
    registry.owner_event("acme", &format!("add {}", key_text(&b.1)), 1, &[&a, &b]);
    registry.owner_event("acme", "revoke acme.a since 1", 1, &[&a]);
    registry.checkpoint(NOW - 38 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.frozen, Some(second));
    assert!(history.requests.contains_key(&second));
    assert!(!history.requests.contains_key(&third));
    assert!(history.took_effect.is_empty());
    assert!(history.pending.is_empty());
    let keys = &history.snapshots.last().unwrap().keys;
    assert_eq!(keys, &vec![a.1.clone()]);
    assert_eq!(history.compromised.get(&a.1.public), Some(&1));
    let releases = client.releases("acme/ledger").unwrap();
    assert_eq!(releases.len(), 1);
    assert!(!releases[0].compromised);
}
