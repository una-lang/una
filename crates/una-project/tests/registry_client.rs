//! The registry client (ADR 0122 rules 26 and 34 to 39), against a test registry read as a
//! mirror directory: tiles, entries and proofs; root metadata, its chain and expiry; checkpoints,
//! witness time, freshness and floors; name chains and the map; owners' keys, revocations and
//! claims at the key floor, and the kinds of owner entries this release does not replay;
//! release statements against their archives; rule 26; and a mirror whose files were tampered
//! with, which verifies nothing.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "support/registry.rs"]
mod registry;

use std::collections::BTreeMap;
use std::path::PathBuf;

use registry::{NOW, TestRegistry, hash_hex, key_text, keypair, walked, walked_root};
use una_project::client::{
    Failure, Floors, Misbehavior, Omission, OpenError, Point, Registry, owner_of, shown,
};
use una_project::itaoca::FloorDigest;
use una_project::provenance::{Expected, Floor, signed_name, verify};
use una_project::tlog::merkle::{
    Hash, consistency_proof, inclusion_proof, leaf_hash, node_hash, root as tree_root,
};
use una_project::tlog::owner;
use una_project::tlog::owner::MAX_CLAIMS;
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
        // RFC 9162 §2.1.3: an inclusion proof holds at most ceil(log2 size) hashes.
        let height = (size - 1).ilog2() as usize + 1;
        assert!(
            proof.len() <= height,
            "{} hashes for {size} leaves",
            proof.len()
        );
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
    assert!(
        parse_entry_bundle(&[0, 0, 9], 1, false).is_err(),
        "trailing"
    );
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
    let (index, content) = registry.release("acme/ledger", "0.3.1", "Http[get]", "none", &[&key]);
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
    // The key and its companion, the owner's threshold of two; the lock names the first by
    // fingerprint.
    assert_eq!(release.signers.len(), 2);
    assert!(release.signers.iter().any(|s| s.public == key.1.public));
    assert_eq!(release.signer(), release.signers.first());
    let state = client.state("acme/ledger", Line::Zero(3)).unwrap();
    assert_eq!(state[0].version, Version::parse_full("0.3.1").unwrap());
    assert_eq!(state[0].logged_at, NOW);
    // The archive agrees with the statement (rule 37).
    let bytes = client.archive(&content).unwrap();
    let tree = VerifiedTree::from_archive(&bytes).unwrap();
    let mut authority = BTreeMap::new();
    authority.insert("Http".to_owned(), ["get".to_owned()].into_iter().collect());
    check_archive(&release.statement, &tree, Some(&authority)).unwrap();
    authority.insert(
        "Files".to_owned(),
        ["read".to_owned()].into_iter().collect(),
    );
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
        last_root: client.last_root().unwrap().0,
        last_root_digest: client.last_root().unwrap().1.to_owned(),
    };
    verify(&provenance, &expected, &Floor::None).unwrap();
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
        // A file the mirror cannot read is one it does not serve: an omission (U7013), never
        // evidence against the registry.
        assert_eq!(client.take_failure(), Some(Failure::Unserved));
        let path = format!("tile/entries/000.p/{size}");
        assert!(client.take_omitted().contains(&Omission::Document(path)));
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
    registry.checkpoint_with(&[
        Some(NOW - 2 * DAY),
        Some(NOW - 2 * DAY),
        Some(NOW + 10 * DAY),
    ]);
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
        &Point::Latest {
            now: NOW + 4 * 3600,
        },
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
        &Point::Latest {
            now: NOW + 4 * 3600,
        },
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
    // Serves `document` as root version 2 and signs the latest checkpoint under it: a
    // checkpoint names the last root version a place serves (spec/30 §11 rule 9), so serving a
    // newer root under an older checkpoint would only stand for a checkpoint withheld.
    fn serve_root_2(registry: &mut TestRegistry, document: String) {
        registry
            .files
            .insert("root/2".to_owned(), document.clone().into_bytes());
        registry.roots.truncate(1);
        registry.roots.push(document);
        registry.checkpoint(NOW);
    }
    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    registry.checkpoint(NOW);
    // Version 2 rotates the third root key, signed by two keys of version 1 and of its own,
    // and expires before the checkpoint's witness time.
    let new_root = keypair(13, "acme.root.13");
    let kept = [
        registry.keys.roots[0].1.clone(),
        registry.keys.roots[1].1.clone(),
    ];
    let roots = [&kept[0], &kept[1], &new_root.1];
    let text = registry::root_text("acme", 2, NOW - 1, &registry.keys, &roots);
    let document = registry::signed(&text, &[&registry.keys.roots[0], &registry.keys.roots[1]]);
    serve_root_2(&mut registry, document);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let refusal = open(&memory, &registry).err().unwrap();
    assert!(refusal.contains("expired"), "{refusal}");
    // Signed by one root key of version 1 only: refused.
    let text = registry::root_text("acme", 2, registry::EXPIRES, &registry.keys, &roots);
    let document = registry::signed(&text, &[&registry.keys.roots[0], &new_root]);
    serve_root_2(&mut registry, document);
    let memory = Memory {
        files: registry.files.clone(),
    };
    assert!(open(&memory, &registry).is_err());
    let document = registry::signed(&text, &[&registry.keys.roots[0], &registry.keys.roots[1]]);
    serve_root_2(&mut registry, document);
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
    let refusal = open(&memory, &registry)
        .unwrap()
        .releases("acme/ledger")
        .unwrap_err();
    assert!(refusal.contains("immutable"), "{refusal}");

    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/mailer", "2.1.0", "Http[get]", "none", &[&key]);
    registry.release("acme/mailer", "2.2.0", "Http[get, post]", "none", &[&key]);
    // A new major may need more.
    registry.release(
        "acme/mailer",
        "3.0.0",
        "Http[get, post]",
        "modules mailer.keys",
        &[&key],
    );
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let refusal = open(&memory, &registry)
        .unwrap()
        .releases("acme/mailer")
        .unwrap_err();
    assert!(
        refusal.contains("Http[post]") && refusal.contains("rule 26"),
        "{refusal}"
    );

    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/mailer", "2.1.0", "Http[get, post]", "none", &[&key]);
    registry.release("acme/mailer", "2.2.0", "Http[get]", "none", &[&key]);
    registry.release(
        "acme/mailer",
        "3.0.0",
        "Http[get, post]",
        "modules mailer.keys",
        &[&key],
    );
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    assert_eq!(
        open(&memory, &registry)
            .unwrap()
            .releases("acme/mailer")
            .unwrap()
            .len(),
        3
    );
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
    assert!(matches!(
        read.event,
        owner::Event::Register {
            cosigners: None,
            ..
        }
    ));
    let previous =
        "previous 4 sha256:aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04aa04";
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
    assert!(matches!(
        read.event,
        owner::Event::Register {
            cosigners: Some((1, _)),
            ..
        }
    ));
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
        assert!(
            owner::parse_entry(&event(&action, threshold)).is_err(),
            "{action}"
        );
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
        // Less than 24 hours after the witness time, NOW - DAY: fresh (rule 3).
        &Point::Latest { now: NOW - 60 },
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
    for path in files
        .keys()
        .filter(|p| p.starts_with("map/0/") || p.starts_with("map/31/"))
    {
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
    use una_project::tlog::map::{MAX_TILE, decode_tile, empty_at};
    assert!(
        decode_tile(&[0u8; 31], 0, &[]).is_err(),
        "short of its bitmap"
    );
    assert!(decode_tile(&[0u8; 32], 0, &[]).is_err(), "no node");
    let mut one = vec![0u8; 32];
    one[0] = 1;
    assert!(decode_tile(&one, 0, &[]).is_err(), "cut inside a hash");
    // The empty subtree at the height of a level-0 tile's bottom nodes, 256 - 8: a node there
    // that is the empty subtree is never listed.
    one.extend_from_slice(&empty_at(248));
    assert!(
        decode_tile(&one, 0, &[]).is_err(),
        "an empty subtree listed"
    );
    one.truncate(32);
    one.extend_from_slice(&[7u8; 32]);
    assert!(decode_tile(&one, 0, &[]).is_ok());
    one.push(0);
    assert!(decode_tile(&one, 0, &[]).is_err(), "trailing bytes");
    assert!(decode_tile(&vec![0u8; MAX_TILE + 1], 0, &[]).is_err());
    assert!(
        decode_tile(&one, 3, &[]).is_err(),
        "a prefix of the wrong length"
    );
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
        (
            "https://ita.acme.example".to_owned(),
            Err("no https transport".to_owned()),
        ),
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
            OpenError::Unavailable(why) | OpenError::Omitted(_, why) => Attempt::Unavailable(why),
            OpenError::Refused(why) | OpenError::Evidence(_, why) => Attempt::Refused(why),
        })
    });
    assert_eq!(fetched.value, Some((2, "file:///good".to_owned())));
    assert_eq!(fetched.refused.len(), 1);
    assert_eq!(fetched.refused[0].from, "file:///bad");
    assert_eq!(fetched.unavailable.len(), 2);
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
        root_digest: None,
    };
    let refused = Registry::open_with(
        "acme",
        &memory,
        &registry.pin(),
        &Point::Latest { now: NOW + 60 },
        &floors,
    );
    assert!(matches!(refused, Err(OpenError::Refused(why)) if why.contains("root version")));
    // ADR 0122, amendments of 2026-11-09 (L3) and of 2026-11-19: the floor's version served
    // with other root metadata than the one accepted is a fork of the root chain, refused;
    // with the same digest, of the text, or a floor of the earliest form (no digest), the chain
    // is accepted. A floor of the form of 2026-11-09 is compared by the whole note's digest, and
    // a digest of one kind never passes as the other.
    let whole = una_project::tlog::root::digest(&registry.roots[0]);
    let text = una_project::tlog::root::text_digest(&registry.roots[0]).unwrap();
    assert_ne!(whole, text);
    let zeros = format!("sha256:{}", "0".repeat(64));
    let first = FloorDigest::Text(text.clone());
    let other = FloorDigest::Text(zeros.clone());
    let at = |digest: Option<&FloorDigest>| {
        Registry::open_with(
            "acme",
            &memory,
            &registry.pin(),
            &Point::Latest { now: NOW + 60 },
            &Floors {
                checkpoint: None,
                root_version: Some(1),
                root_digest: digest,
            },
        )
    };
    assert!(
        matches!(at(Some(&other)), Err(OpenError::Refused(why)) if why.contains("forked")),
        "another digest of root version 1"
    );
    assert!(at(Some(&first)).is_ok());
    assert!(at(None).is_ok());
    assert!(at(Some(&FloorDigest::Note(whole.clone()))).is_ok());
    for kind in [
        FloorDigest::Note(zeros),
        FloorDigest::Note(text),
        FloorDigest::Text(whole),
    ] {
        assert!(
            matches!(at(Some(&kind)), Err(OpenError::Refused(why)) if why.contains("forked")),
            "{kind:?}"
        );
    }
    let mut client = open(&memory, &registry).unwrap();
    let size = client.point.checkpoint.size;
    let own = hash_hex(&client.point.checkpoint.root);
    assert!(client.extends(size, &own).is_ok());
    assert!(
        client.extends(size, &hash_hex(&[9u8; 32])).is_err(),
        "another log"
    );
    assert!(
        client.extends(size + 1, &own).is_err(),
        "a point below the lock's"
    );
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
    assert!(
        parse_note(&format!("text\n\n{line}{line}")).is_err(),
        "one line per key"
    );
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
    assert!(
        refusal.starts_with("the registry `acme` (memory): "),
        "{refusal}"
    );
    assert!(refusal.len() < 600, "{refusal}");
}

/// Two writers at once, each through its own handle of the floor lock, storing different
/// sizes many times: the floor ends at the larger, and a reader never sees it decrease. A lock
/// file left by a holder that ended blocks nothing.
#[test]
fn floors_never_move_backwards() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use una_project::itaoca::{Itaoca, RootFloor, registry_id};
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
    let size_of = |text: &str| {
        parse_checkpoint(&parse_note(text).unwrap().text)
            .unwrap()
            .size
    };
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
                        let size = if round % 2 == 0 {
                            low + round
                        } else {
                            high - round
                        };
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
    let digest = |byte: char| format!("sha256:{}", byte.to_string().repeat(64));
    itaoca
        .store_root_chain(&id, &[walked(3, &digest('a'))])
        .unwrap();
    itaoca
        .store_root_chain(&id, &[walked(2, &digest('b'))])
        .unwrap();
    assert_eq!(
        itaoca.load_root_version(&id).unwrap(),
        Some(RootFloor {
            version: 3,
            digest: Some(FloorDigest::Text(digest('a')))
        })
    );
    // The lock file stays; its existence means nothing.
    assert!(dir.join("registry").join(&id).join("floor.lock").exists());
    // A registry's floors are its own: another pin is another identity, and the identity is
    // the pin's alone, whatever name `sources` gives it.
    let other =
        registry_id("sha256:0202020202020202020202020202020202020202020202020202020202020202");
    assert_ne!(id, other);
    assert_eq!(itaoca.load_checkpoint(&other).unwrap(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The history of `acme` read from `registry` at its latest checkpoint.
fn history_of(registry: &TestRegistry) -> Result<owner::History, String> {
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, registry)?;
    client.history("acme")
}

/// A signing key and its public key.
type Pair = (ed25519_dalek::SigningKey, una_project::tlog::note::Key);

/// Keys of `acme` named `acme.<name>`, from the seeds 60 on, in order.
fn owner_keys<const N: usize>(names: [&str; N]) -> [Pair; N] {
    let mut seed = 60u8;
    names.map(|name| {
        let pair = keypair(seed, &format!("acme.{name}"));
        seed += 1;
        pair
    })
}

/// A key of `acme` named `acme.<prefix><i>`, from a seed of its own.
fn numbered_key(prefix: &str, i: u32) -> Pair {
    let mut seed = [7u8; 32];
    seed[..4].copy_from_slice(&i.to_be_bytes());
    seed[4..4 + prefix.len().min(8)].copy_from_slice(&prefix.as_bytes()[..prefix.len().min(8)]);
    let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
    let key = una_project::tlog::note::Key {
        name: format!("acme.{prefix}{i}"),
        public: signing.verifying_key().to_bytes(),
    };
    (signing, key)
}

/// Asserts that the chain of `acme` in `registry` holds at `index` an entry a registry must not
/// log (`spec/30 §14` rule 5): the owner's names are refused (U7010), the entry is recorded as
/// the registry's misbehavior, and no release of `acme/ledger` is read. Returns why.
fn assert_unacceptable(registry: &TestRegistry, index: u64) -> String {
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, registry).unwrap();
    let refusal = client.history("acme").unwrap_err();
    assert!(
        refusal.contains("which it must not log") && refusal.contains("refused"),
        "{refusal}"
    );
    assert_eq!(client.take_failure(), Some(Failure::Refused));
    let recorded = client.misbehavior();
    assert!(
        matches!(recorded, [Misbehavior::Unacceptable(_)]),
        "{recorded:?}"
    );
    let bad = recorded
        .iter()
        .find_map(|m| match m {
            Misbehavior::Unacceptable(bad) => Some(bad.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        (bad.owner.as_str(), bad.index, bad.rule),
        ("acme", index, owner::RULE),
        "{bad:?}"
    );
    let why = bad.why;
    assert!(client.releases("acme/ledger").is_err());
    why
}

/// Asserts that the chain of `acme` in `registry` holds at `index` an entry of `kind`, which this
/// release does not replay: the owner's names are unavailable (U7013), never refused, no release
/// of `acme/ledger` counts, and nothing is recorded against the registry.
fn assert_unsupported(registry: &TestRegistry, index: u64, kind: &str) {
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, registry).unwrap();
    let why = client.history("acme").unwrap_err();
    assert!(
        why.contains(&format!("holds {kind}, at log entry {index},"))
            && why.contains("unavailable"),
        "{why}"
    );
    assert_eq!(client.take_failure(), Some(Failure::Unserved));
    assert!(client.releases("acme/ledger").is_err());
    assert_eq!(client.take_failure(), Some(Failure::Unserved));
    assert!(client.misbehavior().is_empty());
}

/// The owner `acme` at the key floor ([`acme`]) with one release of `acme/ledger`, and its key.
fn released_owner() -> (TestRegistry, Pair) {
    let mut registry = TestRegistry::new("acme");
    let key = acme(&mut registry);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&key]);
    (registry, key)
}

#[test]
fn statements_need_the_owners_keys_valid_at_their_position() {
    let first = keypair(60, "acme.laptop");
    let [_, c2] = registry::companions(&first);
    let second = keypair(61, "acme.desktop");
    let stranger = keypair(62, "acme.laptop");
    // A key that is not the owner's, with the name of one: a release the owner's keys do not
    // sign, skipped and recorded against the registry, never a refusal of the name.
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &first);
    let (unsigned, _) = registry.release("acme/ledger", "0.3.1", "none", "none", &[&stranger]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let releases = client.releases("acme/ledger").unwrap();
    assert!(releases.len() == 1 && releases[0].compromised);
    assert_eq!(
        client.misbehavior(),
        &[Misbehavior::Unsigned {
            name: "acme/ledger".to_owned(),
            index: unsigned,
        }]
    );

    // Threshold 3: the first key and its companion are not enough.
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &first);
    registry.owner_event(
        "acme",
        &format!("add {}", key_text(&second.1)),
        3,
        &[&first, &second],
    );
    let (short, _) = registry.release("acme/ledger", "0.3.1", "none", "none", &[&first]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let releases = client.releases("acme/ledger").unwrap();
    assert!(releases[0].compromised);
    assert!(client.misbehavior().contains(&Misbehavior::Unsigned {
        name: "acme/ledger".to_owned(),
        index: short,
    }));

    // The first key is claimed compromised since a position: the release it signed after that
    // is skipped as yanked.
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &first);
    registry.owner_event(
        "acme",
        &format!("add {}", key_text(&second.1)),
        2,
        &[&first, &second],
    );
    let (good, _) = registry.release("acme/ledger", "0.3.1", "none", "none", &[&first]);
    let (late, _) = registry.release("acme/ledger", "0.3.2", "none", "none", &[&first]);
    registry.owner_event(
        "acme",
        &format!("revoke {} since {good}", first.1.name),
        2,
        &[&second, &c2],
    );
    registry.release("acme/ledger", "0.3.3", "none", "none", &[&second, &c2]);
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
    // The revoked key signs nothing after its revocation: its release is unsigned.
    let mut after = registry;
    let (revoked, _) = after.release("acme/ledger", "0.3.4", "none", "none", &[&first]);
    after.checkpoint(NOW);
    let memory = Memory {
        files: after.files.clone(),
    };
    let mut client = open(&memory, &after).unwrap();
    let releases = client.releases("acme/ledger").unwrap();
    assert!(releases.iter().any(|r| r.index == revoked && r.compromised));
    assert!(client.misbehavior().contains(&Misbehavior::Unsigned {
        name: "acme/ledger".to_owned(),
        index: revoked,
    }));
}

/// Any entry of an owner's chain at or after a point is a key change.
#[test]
fn every_owner_entry_after_a_point_is_a_key_change() {
    let key = keypair(60, "acme.laptop");
    let added = keypair(64, "acme.desktop");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &key);
    registry.checkpoint(NOW - DAY);
    let point = registry.entries.len() as u64;
    registry.owner_event(
        "acme",
        &format!("add {}", key_text(&added.1)),
        2,
        &[&key, &added],
    );
    registry.checkpoint(NOW);
    let history = history_of(&registry).unwrap();
    assert!(history.changed_after(point), "an addition");
    assert!(!history.changed_after(point + 1));
    assert!(history.pending.is_empty() && history.took_effect.is_empty());
}

/// A key compromised since a position voids what it signed after it without the threshold of
/// other clean keys, and what depends on that: a key it added counts for nothing.
#[test]
fn compromise_cascades_to_the_entries_it_signed() {
    let [a, c, d, e, b] = owner_keys(["a", "c", "d", "e", "b"]);
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &c, &d, &e], 2, &[], 0);
    // At 1, A and C add B; B and C sign a release; then D and E claim A since 0.
    let added = registry.owner_event("acme", &format!("add {}", key_text(&b.1)), 2, &[&a, &c, &b]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&b, &c]);
    registry.owner_event("acme", "revoke acme.a since 0", 2, &[&d, &e]);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&d, &e]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.voided, vec![added]);
    assert_eq!(
        history.snapshots.last().unwrap().keys,
        vec![c.1.clone(), d.1.clone(), e.1.clone()]
    );
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
    registry.register_keys("acme", &[&a, &c, &d, &e], 2, &[], 0);
    registry.owner_event("acme", &format!("add {}", key_text(&b.1)), 2, &[&a, &c, &b]);
    registry.owner_event("acme", "revoke acme.a", 2, &[&d, &e]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&b, &c]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let releases = open(&memory, &registry)
        .unwrap()
        .releases("acme/ledger")
        .unwrap();
    assert!(releases.len() == 1 && !releases[0].compromised);
}

/// A key revoked as compromised since before it was registered is compromised since its
/// registration only: the owner keeps its other keys, and their releases stand.
#[test]
fn a_compromise_never_undoes_a_registration() {
    let [x, y, z, w] = owner_keys(["x", "y", "z", "w"]);
    let mut registry = TestRegistry::new("acme");
    registry.append("other", "one");
    registry.append("other", "two");
    registry.register_keys("acme", &[&x, &y, &z, &w], 2, &[], 0);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&y, &z]);
    registry.owner_event("acme", "revoke acme.x since 0", 2, &[&y, &z]);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&y, &z]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(
        history.compromised.get(&x.1.public),
        Some(&2),
        "clamped to the registration"
    );
    assert_eq!(
        history.at(10).unwrap().keys,
        vec![y.1.clone(), z.1.clone(), w.1.clone()]
    );
    let releases = client.releases("acme/ledger").unwrap();
    assert_eq!(releases.len(), 2);
    assert!(releases.iter().all(|r| !r.compromised));
}

/// A thief holding the keys A and B revokes the owner's key E; the owner, with D and C, claims
/// A compromised since before that. The owner wins: the thief's revocation is void.
#[test]
fn the_thiefs_revocation_does_not_stand() {
    let [a, b, c, d, e] = owner_keys(["a", "b", "c", "d", "e"]);
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b, &c, &d, &e], 2, &[], 0);
    let thief = registry.owner_event("acme", "revoke acme.e", 2, &[&a, &b]);
    registry.owner_event("acme", "revoke acme.a since 0", 2, &[&c, &d]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&c, &d]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.voided, vec![thief]);
    assert_eq!(
        history.at(10).unwrap().keys,
        vec![b.1.clone(), c.1.clone(), d.1.clone(), e.1.clone()]
    );
    assert_eq!(client.releases("acme/ledger").unwrap().len(), 1);
}

/// The thief, with A and B, claims C compromised; the owner, with D and E, claims A
/// compromised since before that. The owner's claim voids the thief's, which does not count:
/// only A is compromised, C stays, and the release the thief signed between is skipped; the
/// chain is not refused.
#[test]
fn of_two_claims_on_each_others_signers_the_one_that_voids_the_other_stands() {
    let [a, b, c, d, e] = owner_keys(["a", "b", "c", "d", "e"]);
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b, &c, &d, &e], 2, &[], 0);
    let thiefs = registry.owner_event("acme", "revoke acme.c since 0", 2, &[&a, &b]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a, &b]);
    registry.owner_event("acme", "revoke acme.a since 0", 2, &[&d, &e]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.compromised.len(), 1);
    assert_eq!(history.compromised.get(&a.1.public), Some(&0));
    assert_eq!(history.voided, vec![thiefs]);
    assert_eq!(
        history.snapshots.last().unwrap().keys,
        vec![b.1.clone(), c.1.clone(), d.1.clone(), e.1.clone()]
    );
    let releases = client.releases("acme/ledger").unwrap();
    assert!(releases[0].compromised);
}

/// The owner revokes a stolen key, then claims it, a key held before, compromised since a
/// position: the keys held do not change, and what the former key signed after that position
/// is skipped.
#[test]
fn a_former_key_can_be_revoked_as_compromised() {
    let [a, b, c, d] = owner_keys(["a", "b", "c", "d"]);
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b, &c, &d], 2, &[], 0);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a, &b]);
    // The key is stolen: the thief publishes with it, and the owner revokes it.
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&a, &b]);
    registry.owner_event("acme", "revoke acme.a", 2, &[&c, &d]);
    registry.owner_event("acme", "revoke acme.a since 1", 2, &[&c, &d]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.compromised.get(&a.1.public), Some(&1));
    assert_eq!(
        history.at(10).unwrap().keys,
        vec![b.1.clone(), c.1.clone(), d.1.clone()]
    );
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

/// The replay's work grows linearly with a chain padded with entries a claim voids: each
/// signature of an entry by a key is checked once.
#[test]
fn replay_stays_linear_with_void_entries() {
    let [a, b, c, d] = owner_keys(["a", "b", "c", "d"]);
    let mut per_entry = Vec::new();
    for n in [25u32, 100, 400] {
        let mut registry = TestRegistry::new("acme");
        registry.register_keys("acme", &[&a, &b, &c, &d], 2, &[], 0);
        for i in 0..n {
            let t = numbered_key("t", i);
            registry.owner_event("acme", &format!("add {}", key_text(&t.1)), 2, &[&a, &b, &t]);
            registry.owner_event("acme", &format!("revoke {}", t.1.name), 2, &[&a, &b]);
        }
        registry.owner_event("acme", "revoke acme.a since 0", 2, &[&c, &d]);
        registry.checkpoint(NOW);
        let history = history_of(&registry).unwrap();
        assert_eq!(history.voided.len() as u32, 2 * n);
        per_entry.push(history.checks as f64 / f64::from(2 * n));
    }
    assert!(per_entry[2] <= per_entry[0] * 2.0, "{per_entry:?}");
}

/// After a void entry, an entry signed by no key the chain names still refuses the chain.
#[test]
fn an_entry_signed_by_no_known_key_refuses_even_after_a_void_one() {
    let [a, b, c, d, t] = owner_keys(["a", "b", "c", "d", "t"]);
    let strangers = [keypair(69, "acme.stranger"), keypair(68, "acme.other")];
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b, &c, &d], 2, &[], 0);
    registry.owner_event("acme", &format!("add {}", key_text(&t.1)), 2, &[&a, &b, &t]);
    registry.owner_event("acme", "revoke acme.a since 0", 2, &[&c, &d]);
    let index = registry.owner_event("acme", "revoke acme.b", 2, &[&strangers[0], &strangers[1]]);
    registry.checkpoint(NOW);
    let why = assert_unacceptable(&registry, index);
    assert!(why.contains("signed by 0 keys"), "{why}");
}

/// A thief holding one key, A, claims the owner's key D compromised with it alone: one key never
/// meets the threshold of two, so a registry must not log the claim, and the client refuses the
/// owner's names as the registry's misbehavior.
#[test]
fn a_thiefs_lone_claim_is_unacceptable() {
    let [a, b, c, d] = owner_keys(["a", "b", "c", "d"]);
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b, &c, &d], 2, &[], 0);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&c, &d]);
    let claim = registry.owner_event("acme", "revoke acme.d since 0", 2, &[&a]);
    registry.checkpoint(NOW);
    let why = assert_unacceptable(&registry, claim);
    assert!(why.contains("fewer than its threshold of 2"), "{why}");
}

/// The thief adds T with the stolen A and B; the owner revokes T, then claims A since before the
/// addition: the revocation of T, a key only a void entry brought in, changes nothing, and the
/// chain stands. Revoking T after the claim revokes a key the owner never held as the chain is
/// known there: a registry must not log it.
#[test]
fn revoking_a_key_whose_addition_is_void_changes_nothing() {
    let [a, b, c, d, t] = owner_keys(["a", "b", "c", "d", "t"]);
    for since in ["", " since 0"] {
        let mut registry = TestRegistry::new("acme");
        registry.register_keys("acme", &[&a, &b, &c, &d], 2, &[], 0);
        registry.owner_event("acme", &format!("add {}", key_text(&t.1)), 2, &[&a, &b, &t]);
        registry.owner_event("acme", &format!("revoke acme.t{since}"), 2, &[&c, &d]);
        registry.owner_event("acme", "revoke acme.a since 0", 2, &[&c, &d]);
        registry.release("acme/ledger", "0.3.1", "none", "none", &[&c, &d]);
        registry.checkpoint(NOW);
        let memory = Memory {
            files: registry.files.clone(),
        };
        let mut client = open(&memory, &registry).unwrap();
        let history = client.history("acme");
        assert!(history.is_ok(), "{since:?}: {:?}", history.err());
        let releases = client.releases("acme/ledger").unwrap();
        assert!(!releases[0].compromised);
    }
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b, &c, &d], 2, &[], 0);
    registry.owner_event("acme", &format!("add {}", key_text(&t.1)), 2, &[&a, &b, &t]);
    registry.owner_event("acme", "revoke acme.a since 0", 2, &[&c, &d]);
    let late = registry.owner_event("acme", "revoke acme.t", 2, &[&c, &d]);
    registry.checkpoint(NOW);
    let why = assert_unacceptable(&registry, late);
    assert!(why.contains("neither holds nor held before"), "{why}");
}

/// A claim that the key it names signs is that key's own word, which a thief holding it can
/// give: a registry must not log it. The same claim signed by other keys reaches back.
#[test]
fn a_claim_signed_by_the_key_it_names_is_unacceptable() {
    let [a, b, c, d] = owner_keys(["a", "b", "c", "d"]);
    let build = |signers: &[&Pair]| {
        let mut registry = TestRegistry::new("acme");
        registry.register_keys("acme", &[&a, &b, &c, &d], 2, &[], 0);
        registry.release("acme/ledger", "0.3.1", "none", "none", &[&a, &b]);
        registry.checkpoint(NOW - DAY);
        let claim = registry.owner_event("acme", "revoke acme.a since 0", 2, signers);
        registry.release("acme/ledger", "0.3.2", "none", "none", &[&b, &c]);
        registry.checkpoint(NOW);
        (registry, claim)
    };
    let (registry, claim) = build(&[&a, &b]);
    let why = assert_unacceptable(&registry, claim);
    assert!(why.contains("signed by `acme.a` itself"), "{why}");
    let (registry, _) = build(&[&b, &c]);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.compromised.get(&a.1.public), Some(&0));
    let releases = client.releases("acme/ledger").unwrap();
    assert!(releases[0].compromised && !releases[1].compromised);
}

/// The key an addition adds signs it, but does not count toward the owner's threshold: a thief
/// holding one key cannot add a key of its own with it.
#[test]
fn an_added_keys_own_signature_does_not_meet_the_threshold() {
    let [a, b, c, t] = owner_keys(["a", "b", "c", "t"]);
    let build = |signers: &[&Pair]| {
        let mut registry = TestRegistry::new("acme");
        registry.register_keys("acme", &[&a, &b, &c], 2, &[], 0);
        let added = registry.owner_event("acme", &format!("add {}", key_text(&t.1)), 2, signers);
        registry.checkpoint(NOW);
        (registry, added)
    };
    let (registry, added) = build(&[&a, &t]);
    let why = assert_unacceptable(&registry, added);
    assert!(why.contains("fewer than its threshold of 2"), "{why}");
    // Without the added key's own signature, neither.
    let (registry, added) = build(&[&a, &b]);
    assert_unacceptable(&registry, added);
    let (registry, _) = build(&[&a, &b, &t]);
    let history = history_of(&registry).unwrap();
    assert_eq!(history.snapshots.last().unwrap().keys.len(), 4);
}

/// An entry a registry must not log (the key floor here), fed directly: the client refuses the
/// owner's names, the same on every run and on a client of its own, and records the registry's
/// misbehavior, the entry's index and the rule it was judged by.
#[test]
fn an_unacceptable_entry_is_refused_as_registry_misbehavior() {
    let [a, b, c] = owner_keys(["a", "b", "c"]);
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b, &c], 2, &[], 0);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a, &b]);
    let bad = registry.owner_event("acme", "revoke acme.c", 2, &[&a, &b]);
    registry.checkpoint(NOW);
    let why = assert_unacceptable(&registry, bad);
    assert!(why.contains("a threshold of at least 2"), "{why}");
    let memory = Memory {
        files: registry.files.clone(),
    };
    let refusal = |client: &mut Registry<'_>| client.history("acme").unwrap_err();
    let mut client = open(&memory, &registry).unwrap();
    let first = refusal(&mut client);
    assert!(
        first.contains(&format!("logged the entry {bad} of the owner `acme`"))
            && first.contains("the owner's packages are refused"),
        "{first}"
    );
    assert_eq!(refusal(&mut client), first);
    let mut again = open(&memory, &registry).unwrap();
    assert_eq!(refusal(&mut again), first);
    assert_eq!(client.misbehavior(), again.misbehavior());
    assert_eq!(client.misbehavior().len(), 1);
    assert!(client.state("acme/ledger", Line::Zero(3)).is_err());
}

/// The key floor (`spec/30 §14` rule 5): after every entry, the registration included, an owner
/// keeps a threshold of at least 2, at least one key more than it, and at most 32 keys.
#[test]
fn every_entry_keeps_the_owner_at_the_key_floor() {
    let [a, b, c, d] = owner_keys(["a", "b", "c", "d"]);
    // A registration below the floor: threshold 1, or no key more than its threshold.
    for (pairs, threshold) in [(vec![&a, &b, &c], 1), (vec![&a, &b], 2)] {
        let mut registry = TestRegistry::new("acme");
        let index = registry.register_keys("acme", &pairs, threshold, &[], 0);
        registry.checkpoint(NOW);
        let why = assert_unacceptable(&registry, index);
        assert!(why.contains("a threshold of at least 2"), "{why}");
    }
    // An addition that lowers the threshold to 1.
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b, &c], 2, &[], 0);
    let index = registry.owner_event("acme", &format!("add {}", key_text(&d.1)), 1, &[&a, &b, &d]);
    registry.checkpoint(NOW);
    assert_unacceptable(&registry, index);
    // At the floor, every entry stands.
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b, &c], 2, &[], 0);
    registry.owner_event("acme", &format!("add {}", key_text(&d.1)), 3, &[&a, &b, &d]);
    registry.owner_event("acme", "revoke acme.d", 2, &[&a, &b, &c]);
    registry.checkpoint(NOW);
    let history = history_of(&registry).unwrap();
    assert_eq!(history.snapshots.last().unwrap().threshold, 2);
    // A 33rd key.
    let many: Vec<Pair> = (0..33).map(|i| numbered_key("k", i)).collect();
    let first: Vec<&Pair> = many.iter().take(32).collect();
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &first, 2, &[], 0);
    registry.checkpoint(NOW);
    assert!(history_of(&registry).is_ok());
    let index = registry.owner_event(
        "acme",
        &format!("add {}", key_text(&many[32].1)),
        2,
        &[&many[0], &many[1], &many[32]],
    );
    registry.checkpoint(NOW);
    assert_unacceptable(&registry, index);
}

/// The owner's claims on its former key D, each since an earlier position, `claims` of them,
/// signed by A and B, then a release they sign. Returns the registry and the last claim's index.
fn claims_on_a_former_key(claims: u64) -> (TestRegistry, u64) {
    let [a, b, c, d] = owner_keys(["a", "b", "c", "d"]);
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b, &c, &d], 2, &[], 0);
    for at in 0..2 * MAX_CLAIMS as u64 {
        registry.append("other", &format!("filler {at}"));
    }
    registry.owner_event("acme", "revoke acme.d", 2, &[&a, &b]);
    let mut last = 0;
    for at in 0..claims {
        let since = 2 * MAX_CLAIMS as u64 - at;
        last = registry.owner_event(
            "acme",
            &format!("revoke acme.d since {since}"),
            2,
            &[&a, &b],
        );
    }
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a, &b]);
    (registry, last)
}

/// The cap of claims (`spec/30 §14` rule 5): an owner's chain holds at most 64 claims. At the
/// cap the owner's names resolve; one more is an entry a registry must not log, and no release
/// of the owner counts.
#[test]
fn past_the_cap_of_claims_the_chain_is_unacceptable() {
    let (mut registry, _) = claims_on_a_former_key(MAX_CLAIMS as u64);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    client.history("acme").unwrap();
    assert!(!client.releases("acme/ledger").unwrap()[0].compromised);
    assert!(client.misbehavior().is_empty());
    let (mut registry, last) = claims_on_a_former_key(MAX_CLAIMS as u64 + 1);
    registry.checkpoint(NOW);
    let why = assert_unacceptable(&registry, last);
    assert!(
        why.contains(&format!("at most {MAX_CLAIMS} claims")),
        "{why}"
    );
}

/// A registration with recovery cosigners, a kind this release does not replay: the owner's
/// names are unavailable.
#[test]
fn a_registration_with_cosigners_is_not_replayed() {
    let [a, b, c] = owner_keys(["a", "b", "c"]);
    let x = keypair(70, "trustee.x");
    let mut registry = TestRegistry::new("acme");
    let index = registry.register_keys("acme", &[&a, &b, &c], 2, &[&x], 1);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a, &b]);
    registry.checkpoint(NOW);
    assert_unsupported(&registry, index, "a registration with recovery cosigners");
}

/// A recovery, a kind this release does not replay: the owner's names are unavailable.
#[test]
fn a_recovery_is_not_replayed() {
    let (mut registry, _) = released_owner();
    let index = registry.recover("acme", "recover", &keypair(63, "acme.new").1, &[]);
    registry.checkpoint(NOW);
    assert_unsupported(&registry, index, "a recovery");
}

/// A dispute, a kind this release does not replay: the owner's names are unavailable.
#[test]
fn a_dispute_is_not_replayed() {
    let (mut registry, _) = released_owner();
    let index = registry.recover("acme", "dispute", &keypair(63, "acme.new").1, &[]);
    registry.checkpoint(NOW);
    assert_unsupported(&registry, index, "a dispute");
}

/// A co-signer change, a kind this release does not replay: the owner's names are unavailable.
#[test]
fn a_cosigner_change_is_not_replayed() {
    let (mut registry, key) = released_owner();
    let x = keypair(70, "trustee.x");
    let index = registry.owner_event(
        "acme",
        &format!("cosigners 1 {}", key_text(&x.1)),
        2,
        &[&key],
    );
    registry.checkpoint(NOW);
    assert_unsupported(&registry, index, "a co-signer change");
}

/// A cancellation, a kind this release does not replay: the owner's names are unavailable.
#[test]
fn a_cancellation_is_not_replayed() {
    let (mut registry, key) = released_owner();
    let index = registry.owner_event("acme", "cancel 1", 2, &[&key]);
    registry.checkpoint(NOW);
    assert_unsupported(&registry, index, "a cancellation");
}

/// An overruling, a kind this release does not replay: the owner's names are unavailable.
#[test]
fn an_overruling_is_not_replayed() {
    let (mut registry, key) = released_owner();
    let index = registry.owner_event("acme", "overrule 1", 2, &[&key]);
    registry.checkpoint(NOW);
    assert_unsupported(&registry, index, "an overruling");
}

/// An entry a registry must not log before one of a kind this release does not replay: the
/// owner's names are refused as the registry's misbehavior, not merely unavailable.
#[test]
fn an_unacceptable_entry_before_an_unreplayed_one_refuses_the_names() {
    let (mut registry, key) = released_owner();
    let bad = registry.owner_event("acme", "revoke acme.laptop.c2", 2, &[&key]);
    registry.recover("acme", "recover", &keypair(63, "acme.new").1, &[]);
    registry.checkpoint(NOW);
    let why = assert_unacceptable(&registry, bad);
    assert!(why.contains("a threshold of at least 2"), "{why}");
}

/// The checkpoint `document` with the cosignature lines of every witness but `kept` removed:
/// what a place that withholds cosignatures serves.
fn withhold(document: &[u8], log: &str, kept: &str) -> Vec<u8> {
    let text = String::from_utf8(document.to_vec()).unwrap();
    let (body, lines) = text.split_once("\n\n").unwrap();
    let lines: Vec<&str> = lines
        .lines()
        .filter(|line| {
            line.starts_with(&format!("\u{2014} {log} "))
                || line.starts_with(&format!("\u{2014} {kept} "))
        })
        .collect();
    format!("{body}\n\n{}\n", lines.join("\n")).into_bytes()
}

/// More than 64 sizes with no checkpoint of the witness threshold, every place read: a cadence
/// gap, evidence against the registry, with its range.
#[test]
fn a_cadence_gap_is_recorded_as_misbehavior() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.bare = true;
    registry.register("acme", &a);
    registry.checkpoint(NOW - 30 * DAY);
    let (release, _) = registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    for at in 0..70 {
        registry.append("other", &format!("filler {at}"));
    }
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    // Never the point's time: the logged time is unavailable, an omission of the cadence.
    assert!(client.logged_at(release).is_err());
    assert_eq!(
        client.take_omitted(),
        vec![Omission::Cadence {
            from: release + 1,
            to: release + 64,
        }]
    );
    assert_eq!(
        client.misbehavior(),
        &[Misbehavior::CadenceGap {
            from: release + 1,
            to: release + 64,
            places: vec!["memory".to_owned()],
            places_agreeing: 1,
        }]
    );
    // Two places serving the same view: both agree, and it is still evidence from an absence.
    let first = Memory {
        files: registry.files.clone(),
    };
    let second = Memory {
        files: registry.files.clone(),
    };
    let both = una_project::transport::Fallback::new(Box::new(first), vec![Box::new(second)]);
    let mut client = open(&both, &registry).unwrap();
    assert!(client.logged_at(release).is_err());
    assert_eq!(
        client.misbehavior(),
        &[Misbehavior::CadenceGap {
            from: release + 1,
            to: release + 64,
            places: vec!["memory".to_owned(), "memory".to_owned()],
            places_agreeing: 2,
        }]
    );
}

/// Every checkpoint names, inside its signed text, the root version it was signed under: a
/// place that hides the newer root makes the client's result unavailable, never decided under
/// the older key set; a checkpoint without the line, or with another digest, is refused.
#[test]
fn a_checkpoint_names_its_root_and_a_hidden_newer_root_is_unavailable() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.checkpoint(NOW - DAY);
    registry.rotate_witness();
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW);
    let floors = Floors::default();
    let point = Point::Latest { now: NOW + 60 };
    let full = Memory {
        files: registry.files.clone(),
    };
    let client = Registry::open_with("acme", &full, &registry.pin(), &point, &floors).unwrap();
    assert_eq!(client.root.version, 2);
    // Root version 2 hidden: unavailable.
    let mut hiding = Memory {
        files: registry.files.clone(),
    };
    hiding.files.remove("root/2");
    let opened = Registry::open_with("acme", &hiding, &registry.pin(), &point, &floors);
    assert!(
        matches!(opened, Err(OpenError::Unavailable(_))),
        "{:?}",
        opened.err()
    );
    // The latest checkpoint re-signed without the line, and with another digest: refused.
    let document = String::from_utf8(registry.files["checkpoint"].clone()).unwrap();
    let text = document.split_once("\n\n").unwrap().0.to_owned() + "\n";
    let root_line = registry.root_line();
    let resign = |text: &str| {
        let mut out = format!(
            "{text}\n{}",
            registry::sign_line(&registry.keys.log.0, &registry.keys.log.1, text)
        );
        for (pair, key) in &registry.keys.witnesses {
            out.push_str(&registry::cosign_line(pair, key, text, NOW));
        }
        out.into_bytes()
    };
    for altered in [
        text.replace(&format!("{root_line}\n"), ""),
        text.replace(&root_line, &format!("ita-root 2 sha256:{}", "0".repeat(64))),
        text.replace(&root_line, &format!("{root_line}\n{root_line}")),
    ] {
        let mut place = Memory {
            files: registry.files.clone(),
        };
        place
            .files
            .insert("checkpoint".to_owned(), resign(&altered));
        let opened = Registry::open_with("acme", &place, &registry.pin(), &point, &floors);
        assert!(matches!(opened, Err(OpenError::Refused(_))), "{altered}");
    }
}

/// A checkpoint `text` signed again by the log, and cosigned at `time` by `witnesses`.
fn resigned(
    registry: &TestRegistry,
    text: &str,
    witnesses: &[(ed25519_dalek::SigningKey, una_project::tlog::note::Key)],
    time: u64,
) -> Vec<u8> {
    let mut out = format!(
        "{text}\n{}",
        registry::sign_line(&registry.keys.log.0, &registry.keys.log.1, text)
    );
    for (pair, key) in witnesses {
        out.push_str(&registry::cosign_line(pair, key, text, time));
    }
    out.into_bytes()
}

/// The signed text of the checkpoint document `bytes`.
fn text_of(bytes: &[u8]) -> String {
    let document = String::from_utf8(bytes.to_vec()).unwrap();
    document.split_once("\n\n").unwrap().0.to_owned() + "\n"
}

/// After a rotation, a latest checkpoint that names the older root while the newer one is
/// served stands for a newer checkpoint withheld: unavailable, never accepted.
#[test]
fn a_point_naming_an_older_root_is_unavailable() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.checkpoint(NOW - DAY);
    registry.rotate_witness();
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW);
    let older = format!(
        "ita-root 1 {}",
        una_project::tlog::root::digest(&registry.roots[0])
    );
    let text = text_of(&registry.files["checkpoint"]).replace(&registry.root_line(), &older);
    let mut place = Memory {
        files: registry.files.clone(),
    };
    let forged = resigned(&registry, &text, &registry.keys.witnesses, NOW);
    place.files.insert("checkpoint".to_owned(), forged);
    let point = Point::Latest { now: NOW + 60 };
    let opened = Registry::open_with("acme", &place, &registry.pin(), &point, &Floors::default());
    assert!(
        matches!(opened, Err(OpenError::Unavailable(_))),
        "{:?}",
        opened.err()
    );
}

/// The root version checkpoints name never decreases as the log grows: a kept checkpoint
/// naming an older version than a smaller one is refused.
#[test]
fn a_root_version_that_decreases_is_refused() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.rotate_witness();
    registry.checkpoint(NOW - 3 * DAY);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 2 * DAY);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&a]);
    registry.checkpoint(NOW);
    let older = format!(
        "ita-root 1 {}",
        una_project::tlog::root::digest(&registry.roots[0])
    );
    let text = text_of(&registry.files["checkpoint/2"]).replace(&registry.root_line(), &older);
    let mut place = Memory {
        files: registry.files.clone(),
    };
    let forged = resigned(&registry, &text, &registry.keys.witnesses, NOW - 2 * DAY);
    place.files.insert("checkpoint/2".to_owned(), forged);
    let mut client = open(&place, &registry).unwrap();
    // Size 1 names version 2; size 2, read after it, names version 1.
    client.logged_at(0).unwrap();
    assert!(client.logged_at(1).is_err());
}

/// A witness the next root revokes as it rotates it out: its back-dated cosignature on a kept
/// checkpoint is ignored, and no time moves.
#[test]
fn an_old_witness_back_dating_is_ignored() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.checkpoint(NOW - 10 * DAY);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    // The release's checkpoint, cosigned by two of the three witnesses.
    registry.checkpoint_with(&[Some(NOW - 5 * DAY), Some(NOW - 5 * DAY), None]);
    registry.rotate_witnesses(1, true);
    registry.append("other", "filler");
    registry.checkpoint(NOW);
    let honest = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&honest, &registry).unwrap();
    let logged = client.logged_at(1).unwrap();
    let earliest = client.earliest_logged(2).unwrap();
    // The rotated-out witness, seed 32, cosigns the release's checkpoint a month earlier.
    let old = keypair(32, "witness.32");
    let mut document = String::from_utf8(registry.files["checkpoint/2"].clone()).unwrap();
    let text = text_of(document.as_bytes());
    let back_dated = registry::cosign_line(&old.0, &old.1, &text, NOW - 40 * DAY);
    document.push_str(&back_dated);
    let mut place = Memory {
        files: registry.files.clone(),
    };
    place
        .files
        .insert("checkpoint/2".to_owned(), document.into_bytes());
    let mut client = open(&place, &registry).unwrap();
    assert_eq!(client.logged_at(1).unwrap(), logged);
    assert_eq!(client.earliest_logged(2).unwrap(), earliest);
}

/// The withheld-eligibility delay: a mirror withholds the cosignatures of the checkpoint that
/// makes a fix release past the cooldown. Resolution keeps the served time, so the older
/// version is the newest eligible, and the delay is observed; with the honest copy merged, the
/// fix is eligible.
#[test]
fn a_withheld_eligibility_delay_is_observed_and_merging_ends_it() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 30 * DAY);
    let (fix, _) = registry.release("acme/ledger", "0.3.2", "none", "none", &[&a]);
    let eligible = registry.checkpoint(NOW - 4 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 2 * DAY);
    registry.append("other", "another filler");
    registry.checkpoint(NOW);
    let newest_eligible = |client: &mut Registry<'_>| {
        let now = client.point.time;
        client
            .state("acme/ledger", Line::Zero(3))
            .unwrap()
            .into_iter()
            .filter(|r| r.logged_at + una_project::resolve::COOLDOWN <= now)
            .map(|r| r.version)
            .max()
            .unwrap()
    };
    let path = format!("checkpoint/{eligible}");
    let mut withholding = Memory {
        files: registry.files.clone(),
    };
    let text = text_of(&registry.files[&path]);
    let stripped = resigned(&registry, &text, &[], 0);
    withholding.files.insert(path.clone(), stripped);
    let files = withholding.files.clone();
    let mut client = open(&withholding, &registry).unwrap();
    let older = Version::parse_full("0.3.1").unwrap();
    let fixed = Version::parse_full("0.3.2").unwrap();
    assert_eq!(newest_eligible(&mut client), older);
    let observed = client.observations().to_vec();
    assert!(
        matches!(
            observed.as_slice(),
            [una_project::client::Observation::EligibilityDelay { version, served, .. }]
                if *version == fixed && *served == NOW - 2 * DAY
        ),
        "{observed:?}"
    );
    assert!(client.take_omitted().is_empty());
    // The honest copy behind it: merged, the fix is eligible, and nothing is observed.
    let honest = Memory {
        files: registry.files.clone(),
    };
    let both =
        una_project::transport::Fallback::new(Box::new(Memory { files }), vec![Box::new(honest)]);
    let mut client = open(&both, &registry).unwrap();
    assert_eq!(client.logged_at(fix).unwrap(), NOW - 4 * DAY);
    assert_eq!(newest_eligible(&mut client), fixed);
    assert!(client.observations().is_empty());
}

/// A place that withholds the cosignatures of the cadence checkpoint after a fix release does
/// not delay the fix without bound: no checkpoint of the threshold within 64 sizes makes the
/// release's logged time unavailable, never the point's time, so the older version is never
/// chosen in the fix's place.
#[test]
fn withholding_the_cadence_after_a_fix_is_unavailable() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.sparse = true;
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 30 * DAY);
    let (fix, _) = registry.release("acme/ledger", "0.3.2", "none", "none", &[&a]);
    for at in 0..100 {
        registry.append("other", &format!("filler {at}"));
    }
    registry.checkpoint(NOW);
    let honest = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&honest, &registry).unwrap();
    let now = client.point.time;
    let eligible: Vec<Version> = client
        .state("acme/ledger", Line::Zero(3))
        .unwrap()
        .into_iter()
        .filter(|r| r.logged_at + una_project::resolve::COOLDOWN <= now)
        .map(|r| r.version)
        .collect();
    assert!(eligible.contains(&Version::parse_full("0.3.2").unwrap()));
    // The cadence checkpoint of size 64 served with no cosignature.
    let path = "checkpoint/64".to_owned();
    let text = text_of(&registry.files[&path]);
    let mut withholding = Memory {
        files: registry.files.clone(),
    };
    withholding
        .files
        .insert(path, resigned(&registry, &text, &[], 0));
    let mut client = open(&withholding, &registry).unwrap();
    assert!(client.state("acme/ledger", Line::Zero(3)).is_err());
    // The older release's logged time rests on the same withheld checkpoint, and `state` stops
    // at the first release it cannot time; the fix's own logged time is unavailable too.
    assert!(client.logged_at(fix).is_err());
    assert!(
        client
            .take_omitted()
            .iter()
            .any(|o| matches!(o, Omission::Cadence { from, .. } if *from == fix + 1)),
    );
}

/// A rotation that leaves fewer than the threshold of the old witnesses: their cosignatures on
/// the checkpoints below the point, which a consistency proof binds to it, still count with the
/// threshold of the version those checkpoints name, so older history stays usable, and an
/// older lock's point verifies once proven a prefix of the latest checkpoint.
#[test]
fn retired_witnesses_keep_older_history_usable() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    let older = registry.checkpoint(NOW - 10 * DAY);
    let older_hash = hash_hex(&una_project::tlog::merkle::root(
        &registry
            .entries
            .iter()
            .map(|e| una_project::tlog::merkle::leaf_hash(e))
            .collect::<Vec<_>>(),
    ));
    registry.rotate_witnesses(2, false);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&a]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    assert_eq!(client.logged_at(1).unwrap(), NOW - 10 * DAY);
    assert_eq!(client.releases("acme/ledger").unwrap().len(), 2);
    assert!(client.history("acme").is_ok());
    // The older lock's point, signed under root 1 and cosigned mostly by retired witnesses.
    let at = Point::At {
        size: older,
        hash: older_hash,
    };
    let opened = Registry::open_with("acme", &memory, &registry.pin(), &at, &Floors::default());
    assert!(opened.is_ok(), "{:?}", opened.err());
}

/// Retired witnesses cosigning a checkpoint off the log, not a prefix of the point: rejected.
#[test]
fn retired_witnesses_cannot_cosign_a_forged_branch() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 10 * DAY);
    registry.rotate_witnesses(2, false);
    registry.append("other", "filler");
    registry.checkpoint(NOW);
    // A checkpoint of size 2 over other leaves, under root 1, cosigned by the retired ones.
    let older = format!(
        "ita-root 1 {}",
        una_project::tlog::root::digest(&registry.roots[0])
    );
    let forged_root = una_project::tlog::merkle::root(&[
        una_project::tlog::merkle::leaf_hash(b"another first entry"),
        una_project::tlog::merkle::leaf_hash(b"another second entry"),
    ]);
    let text = format!(
        "{}\n2\n{}\nita-map {}\n{older}\n",
        registry.keys.log.1.name,
        una_project::tlog::note::encode_base64(&forged_root),
        una_project::tlog::note::encode_base64(&forged_root)
    );
    let retired = [keypair(31, "witness.31"), keypair(32, "witness.32")];
    let mut place = Memory {
        files: registry.files.clone(),
    };
    place.files.insert(
        "checkpoint/2".to_owned(),
        resigned(&registry, &text, &retired, NOW - 20 * DAY),
    );
    let mut client = open(&place, &registry).unwrap();
    assert!(client.logged_at(1).is_err());
    assert!(client.take_omitted().is_empty());
}

/// A witness a later root revokes counts for nothing at any version: its back-dated line on an
/// older checkpoint is ignored, and the logged time is the one the others give.
#[test]
fn a_revoked_witness_counts_for_nothing() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint_with(&[Some(NOW - 10 * DAY), None, None]);
    registry.rotate_witnesses(1, true);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 5 * DAY);
    registry.append("other", "another filler");
    registry.checkpoint(NOW);
    let revoked = keypair(32, "witness.32");
    let mut document = String::from_utf8(registry.files["checkpoint/2"].clone()).unwrap();
    let text = text_of(document.as_bytes());
    let line = registry::cosign_line(&revoked.0, &revoked.1, &text, NOW - 20 * DAY);
    document.push_str(&line);
    let mut place = Memory {
        files: registry.files.clone(),
    };
    place
        .files
        .insert("checkpoint/2".to_owned(), document.into_bytes());
    let mut client = open(&place, &registry).unwrap();
    assert_eq!(client.logged_at(1).unwrap(), NOW - 5 * DAY);
}

/// A registry whose lock's point, the checkpoint returned with its tree hash, is signed under
/// root 1 and cosigned by every witness of it, before a rotation of the last two witnesses to
/// root 2 and a later checkpoint signed under it.
fn rotated_with_older_point(k: usize) -> (TestRegistry, Point) {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::with_witness_threshold("acme", k);
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    let older = registry.checkpoint(NOW - 10 * DAY);
    let leaves: Vec<Hash> = registry.entries.iter().map(|e| leaf_hash(e)).collect();
    let hash = hash_hex(&tree_root(&leaves));
    registry.rotate_witnesses(2, false);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&a]);
    registry.checkpoint(NOW);
    let at = Point::At { size: older, hash };
    (registry, at)
}

/// A lock's point naming an older root, cosigned by retired witnesses, whose prefix proof no
/// place lets the client complete, the latest checkpoint unserved or a tile of the proof
/// unserved: an omission of the prefix proof (U7013), never a point counted on fewer
/// cosignatures or refused.
#[test]
fn an_unserved_prefix_proof_makes_the_point_unavailable() {
    let (registry, at) = rotated_with_older_point(2);
    let Point::At { size, .. } = at else {
        panic!("the rotated registry's point is at a size")
    };
    let mut no_latest = Memory {
        files: registry.files.clone(),
    };
    no_latest.files.remove("checkpoint");
    let mut no_tiles = Memory {
        files: registry.files.clone(),
    };
    no_tiles.files.retain(|path, _| !path.starts_with("tile/"));
    for place in [&no_latest, &no_tiles] {
        match Registry::open_with("acme", place, &registry.pin(), &at, &Floors::default()) {
            Err(OpenError::Omitted(Omission::PrefixProof(at_size), why)) => {
                assert_eq!(at_size, size);
                assert!(why.contains("could not be completed"), "{why}");
            }
            Err(other) => panic!("{}", other.message()),
            Ok(_) => panic!("a point counted without its prefix proof"),
        }
    }
    // Served, the proof completes and the point verifies.
    let memory = Memory {
        files: registry.files.clone(),
    };
    let opened = Registry::open_with("acme", &memory, &registry.pin(), &at, &Floors::default());
    assert!(opened.is_ok(), "{:?}", opened.err());
}

/// A lock's point counted on current witnesses alone, its prefix proof not completed: every
/// kept checkpoint below it that carries a retired witness's cosignature is unavailable, an
/// omission of the prefix proof, never a time read from fewer cosignatures, and never
/// evidence of a cadence gap.
#[test]
fn an_unserved_prefix_proof_makes_kept_checkpoints_with_retired_witnesses_unavailable() {
    let (registry, at) = rotated_with_older_point(1);
    let Point::At { size, .. } = at else {
        panic!("the rotated registry's point is at a size")
    };
    let path = format!("checkpoint/{size}");
    let log = registry.keys.log.1.name.clone();
    let current = registry.keys.witnesses[0].1.name.clone();
    let mut place = Memory {
        files: registry.files.clone(),
    };
    place.files.remove("checkpoint");
    let withheld = withhold(&registry.files[&path], &log, &current);
    place.files.insert(path, withheld);
    let mut client =
        Registry::open_with("acme", &place, &registry.pin(), &at, &Floors::default()).unwrap();
    assert!(client.logged_at(0).is_err());
    assert!(client.take_omitted().contains(&Omission::PrefixProof(size)));
    assert!(client.misbehavior().is_empty());
}

/// A checkpoint naming an older root version is counted with the larger of its threshold and
/// the last root's: one current witness's cosignature, enough under root 1's threshold of one,
/// is too few under root 2's of two, so the point is unavailable.
#[test]
fn the_threshold_never_drops_below_the_last_roots() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::with_witness_threshold("acme", 1);
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    let older = registry.checkpoint(NOW - 10 * DAY);
    let leaves: Vec<Hash> = registry.entries.iter().map(|e| leaf_hash(e)).collect();
    let hash = hash_hex(&tree_root(&leaves));
    registry.witness_k = 2;
    registry.rotate_witnesses(0, false);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&a]);
    registry.checkpoint(NOW);
    let at = Point::At { size: older, hash };
    let path = format!("checkpoint/{older}");
    let log = registry.keys.log.1.name.clone();
    let witness = registry.keys.witnesses[0].1.name.clone();
    let mut place = Memory {
        files: registry.files.clone(),
    };
    let withheld = withhold(&registry.files[&path], &log, &witness);
    place.files.insert(path, withheld);
    let opened = Registry::open_with("acme", &place, &registry.pin(), &at, &Floors::default());
    assert!(matches!(opened, Err(OpenError::Unavailable(_))));
    let memory = Memory {
        files: registry.files.clone(),
    };
    let opened = Registry::open_with("acme", &memory, &registry.pin(), &at, &Floors::default());
    assert!(opened.is_ok(), "{:?}", opened.err());
}

/// Two places serving copies of one size's checkpoint, both signed by the log, that name
/// different root versions: two checkpoints of that size, refused (U7010) and recorded as
/// evidence, never one version taken over the other.
#[test]
fn copies_of_one_size_naming_different_roots_are_refused() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 10 * DAY);
    registry.rotate_witness();
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&a]);
    registry.checkpoint(NOW);
    let older = format!(
        "ita-root 1 {}",
        una_project::tlog::root::digest(&registry.roots[0])
    );
    let text = text_of(&registry.files["checkpoint/1"]).replace(&older, &registry.root_line());
    let witnesses = registry.keys.witnesses.clone();
    let mut other = Memory {
        files: registry.files.clone(),
    };
    other.files.insert(
        "checkpoint/1".to_owned(),
        resigned(&registry, &text, &witnesses, NOW - 10 * DAY),
    );
    let honest = Memory {
        files: registry.files.clone(),
    };
    let both = una_project::transport::Fallback::new(Box::new(honest), vec![Box::new(other)]);
    let mut client = open(&both, &registry).unwrap();
    assert!(client.logged_at(0).is_err());
    assert!(client.take_omitted().is_empty());
    assert_eq!(
        client.misbehavior(),
        &[Misbehavior::Equivocation {
            size: 1,
            versions: vec![1, 2],
            places: vec!["memory".to_owned(), "memory".to_owned()],
        }]
    );
}

/// A root version that lists as a witness a key an earlier version revoked: the chain is
/// refused (U7010), a revoked key never vouching again.
#[test]
fn a_revoked_witness_key_is_never_a_witness_again() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.checkpoint(NOW - 10 * DAY);
    registry.rotate_witnesses(1, true);
    registry.keys.witnesses[2] = keypair(32, "witness.32");
    let roots = [
        &registry.keys.roots[0].1,
        &registry.keys.roots[1].1,
        &registry.keys.roots[2].1,
    ];
    let (keys, expires) = (&registry.keys, registry::EXPIRES);
    let text = registry::root_text_with("acme", 3, expires, keys, &roots, 2, &[]);
    let document = registry::signed(&text, &[&registry.keys.roots[0], &registry.keys.roots[1]]);
    registry
        .files
        .insert("root/3".to_owned(), document.clone().into_bytes());
    registry.roots.push(document);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    match open(&memory, &registry) {
        Err(why) => assert!(why.contains("an earlier version revoked"), "{why}"),
        Ok(_) => panic!("a revoked witness key accepted again"),
    }
}

/// A file mirror lacking a tile and a kept checkpoint, with an https place behind it, which
/// this release cannot read: the https place serves nothing, so each result is unserved, an
/// omission of the document (U7013), never a refusal (U7010).
#[test]
fn a_mirror_lacking_a_document_with_an_https_place_behind_is_unserved() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW);
    let mut mirror = Memory {
        files: registry.files.clone(),
    };
    mirror.files.retain(|path, _| !path.starts_with("tile/"));
    mirror.files.remove("checkpoint/1");
    let https = Unavailable {
        url: "https://registry.example".to_owned(),
    };
    let both = una_project::transport::Fallback::new(Box::new(mirror), vec![Box::new(https)]);
    let mut client = open(&both, &registry).unwrap();
    assert!(client.entry(0).is_err());
    assert_eq!(client.take_failure(), Some(Failure::Unserved));
    let omitted = client.take_omitted();
    assert!(omitted.iter().any(is_tile), "{omitted:?}");
    assert!(client.logged_at(0).is_err());
    assert_eq!(client.take_failure(), Some(Failure::Unserved));
    assert_eq!(
        client.take_omitted(),
        vec![Omission::Document("checkpoint/1".to_owned())]
    );
}

/// An omission recorded early, then a checkpoint whose signature does not verify: the failure
/// is a refusal (U7010) by its own kind, never U7013 inferred from the earlier omission.
#[test]
fn a_bad_signature_after_an_omission_is_a_refusal() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.append("other", "filler");
    registry.append("other", "another filler");
    registry.checkpoint(NOW);
    let mut place = Memory {
        files: registry.files.clone(),
    };
    place.files.remove("checkpoint/3");
    let unsigned = withhold(&registry.files["checkpoint/2"], "nobody", "nobody");
    place.files.insert("checkpoint/2".to_owned(), unsigned);
    let mut client = open(&place, &registry).unwrap();
    assert!(client.logged_at(2).is_err());
    assert_eq!(client.take_failure(), Some(Failure::Unserved));
    assert!(client.logged_at(1).is_err());
    assert_eq!(client.take_failure(), Some(Failure::Refused));
    // The earlier omission is still recorded; the code comes from the failure's own kind.
    assert!(!client.take_omitted().is_empty());
    // The reverse: a refusal, then an omission; each failure's kind is its own, taken once.
    let mut client = open(&place, &registry).unwrap();
    assert!(client.logged_at(1).is_err());
    assert_eq!(client.take_failure(), Some(Failure::Refused));
    assert_eq!(client.take_failure(), None);
    assert!(client.logged_at(2).is_err());
    assert_eq!(client.take_failure(), Some(Failure::Unserved));
}

/// Whether `omission` is of a tile or an entry bundle.
fn is_tile(omission: &Omission) -> bool {
    matches!(omission, Omission::Document(path) if path.starts_with("tile/"))
}

/// The provenance a client writes for the release at the point `at` of `registry`, and what a
/// lock written then says of it.
fn provenance_at(
    registry: &TestRegistry,
    at: &Point,
) -> (una_project::provenance::Provenance, Expected) {
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client =
        Registry::open_with("acme", &memory, &registry.pin(), at, &Floors::default()).unwrap();
    let release = client
        .release("acme/ledger", &Version::parse_full("0.3.1").unwrap())
        .unwrap();
    let key = release.signer().unwrap().clone();
    let provenance = client.provenance(&release, &key).unwrap();
    let (last_root, last_root_digest) = client
        .last_root()
        .map(|(version, digest)| (version, digest.to_owned()))
        .unwrap();
    let expected = Expected {
        source: "acme".to_owned(),
        name: "acme/ledger".to_owned(),
        version: "0.3.1".to_owned(),
        digest: release.statement.digest.clone(),
        signed: signed_name("acme/ledger", &key.public),
        logged: release.index,
        root: registry.pin(),
        checkpoint_size: client.point.checkpoint.size,
        checkpoint_hash: hash_hex(&client.point.checkpoint.root),
        last_root,
        last_root_digest,
    };
    (provenance, expected)
}

/// Whether `result` is unavailable (U7013), with a message holding `says`.
fn unavailable(
    result: Result<una_project::tlog::release::Statement, una_project::provenance::Problem>,
    says: &str,
) -> bool {
    matches!(result, Err(una_project::provenance::Problem::Unavailable(why)) if why.contains(says))
}

/// Whether `result` is refused (U7010), with a message holding `says`.
fn refused(
    result: Result<una_project::tlog::release::Statement, una_project::provenance::Problem>,
    says: &str,
) -> bool {
    matches!(result, Err(una_project::provenance::Problem::Refused(why)) if why.contains(says))
}

/// ADR 0122, amendment of 2026-11-03, items 1 and 2: a lock's point under an older root,
/// cosigned by witnesses a later root retired, has a provenance that carries the latest
/// checkpoint under the last root and the consistency proof the client read, and it verifies
/// offline, the retired witnesses counting by that proof, at every load.
#[test]
fn a_provenance_carries_the_prefix_proof_of_an_older_point() {
    let (registry, at) = rotated_with_older_point(2);
    let (provenance, expected) = provenance_at(&registry, &at);
    assert_eq!(provenance.format, una_project::provenance::Format::V2);
    let latest = provenance
        .latest
        .as_ref()
        .expect("the prefix proof is carried");
    assert!(!latest.consistency.is_empty());
    let encoded = provenance.encode();
    assert!(encoded.starts_with(b"ita-provenance/2\n"));
    assert_eq!(
        una_project::provenance::parse(&encoded).unwrap(),
        provenance
    );
    let digests: Vec<FloorDigest> = registry
        .roots
        .iter()
        .map(|document| FloorDigest::Text(una_project::tlog::root::text_digest(document).unwrap()))
        .collect();
    let notes: Vec<FloorDigest> = registry
        .roots
        .iter()
        .map(|document| FloorDigest::Note(una_project::tlog::root::digest(document)))
        .collect();
    for floor in [
        Floor::At {
            version: 1,
            digest: Some(notes[0].clone()),
        },
        Floor::At {
            version: 2,
            digest: Some(notes[1].clone()),
        },
        Floor::None,
        Floor::At {
            version: 1,
            digest: None,
        },
        Floor::At {
            version: 2,
            digest: None,
        },
        Floor::At {
            version: 1,
            digest: Some(digests[0].clone()),
        },
        Floor::At {
            version: 2,
            digest: Some(digests[1].clone()),
        },
    ] {
        let verified = verify(&provenance, &expected, &floor);
        assert!(verified.is_ok(), "{floor:?}: {verified:?}");
    }
    // A point that names the last root carries no proof.
    let mut current = TestRegistry::new("acme");
    let a = keypair(60, "acme.a");
    current.register("acme", &a);
    current.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    let size = current.checkpoint(NOW);
    let leaves: Vec<Hash> = current.entries.iter().map(|e| leaf_hash(e)).collect();
    let point = Point::At {
        size,
        hash: hash_hex(&tree_root(&leaves)),
    };
    let (plain, plain_expected) = provenance_at(&current, &point);
    assert!(plain.latest.is_none());
    assert!(verify(&plain, &plain_expected, &Floor::None).is_ok());
    // And a proof added to it is refused: the encoding of a point has one form.
    let mut added = plain.clone();
    added.latest = provenance.latest.clone();
    assert!(refused(
        verify(&added, &plain_expected, &Floor::None),
        "names the last root version"
    ));
}

/// Item 3: without the proof, a checkpoint that carries a retired witness's cosignature is
/// unavailable (U7013), never decided on fewer cosignatures, in either format; a checkpoint
/// with no such cosignature counts on the last root's witnesses alone, as before.
#[test]
fn a_retired_witness_without_the_proof_is_unavailable() {
    let (registry, at) = rotated_with_older_point(2);
    let (provenance, expected) = provenance_at(&registry, &at);
    let mut stripped = provenance.clone();
    stripped.latest = None;
    assert!(unavailable(
        verify(&stripped, &expected, &Floor::None),
        "retired"
    ));
    let mut first = stripped.clone();
    first.format = una_project::provenance::Format::V1;
    let encoded = first.encode();
    assert!(encoded.starts_with(b"ita-provenance/1\n"));
    let read = una_project::provenance::parse(&encoded).unwrap();
    assert!(unavailable(
        verify(&read, &expected, &Floor::None),
        "retired"
    ));
    // Format 1 never carries the proof.
    let mut forged = provenance.clone();
    forged.format = una_project::provenance::Format::V1;
    assert!(
        una_project::provenance::parse(&forged.encode())
            .unwrap_err()
            .contains("carries no latest checkpoint")
    );
    // Even when the last root's witnesses alone would meet the threshold: one retired, the
    // threshold one, the point cosigned by all three.
    let (one, at_one) = rotated_with_older_point(1);
    let (provenance, expected) = provenance_at(&one, &at_one);
    let mut stripped = provenance.clone();
    stripped.latest = None;
    assert!(unavailable(
        verify(&stripped, &expected, &Floor::None),
        "retired"
    ));
    // The point cosigned by the witness the rotation kept, alone, at threshold one: no retired
    // witness's line, so it counts on the last root's witnesses, with no proof.
    let mut kept = stripped.clone();
    let text = text_of(kept.checkpoint.as_bytes());
    kept.checkpoint = String::from_utf8(resigned(
        &one,
        &text,
        &[one.keys.witnesses[0].clone()],
        NOW - 10 * DAY,
    ))
    .unwrap();
    let verified = verify(&kept, &expected, &Floor::None);
    assert!(verified.is_ok(), "{verified:?}");
}

/// A proof carried that does not verify is refused (U7010): an edited consistency hash, one
/// dropped, a latest checkpoint that names an older version, one short of cosignatures, one
/// not signed by the log, one of the point's own size, or one below it.
#[test]
fn a_carried_proof_that_does_not_verify_is_refused() {
    let (registry, at) = rotated_with_older_point(2);
    let (provenance, expected) = provenance_at(&registry, &at);
    let latest = provenance.latest.clone().unwrap();
    let with = |change: &dyn Fn(&mut una_project::provenance::Latest)| {
        let mut changed = provenance.clone();
        let mut l = latest.clone();
        change(&mut l);
        changed.latest = Some(l);
        verify(&changed, &expected, &Floor::None)
    };
    assert!(refused(with(&|l| l.consistency[0][0] ^= 1), "forked"));
    assert!(refused(
        with(&|l| {
            l.consistency.pop();
        }),
        "forked"
    ));
    assert!(refused(with(&|l| l.consistency.push([7; 32])), "forked"));
    // The point's checkpoint in place of the latest: it names root version 1.
    assert!(refused(
        with(&|l| l.checkpoint = provenance.checkpoint.clone()),
        "not the last one it stores"
    ));
    // Cosigned by one current witness, below the threshold of two.
    let text = text_of(latest.checkpoint.as_bytes());
    let one = String::from_utf8(resigned(
        &registry,
        &text,
        &[registry.keys.witnesses[0].clone()],
        NOW,
    ))
    .unwrap();
    assert!(refused(
        with(&|l| l.checkpoint = one.clone()),
        "fewer than its threshold"
    ));
    // Not signed by the log's key.
    let unsigned = latest.checkpoint.replacen(
        &format!("\u{2014} {} ", registry.keys.log.1.name),
        "\u{2014} another.log ",
        1,
    );
    assert!(with(&|l| l.checkpoint = unsigned.clone()).is_err());
    // A checkpoint of the point's size signed under root 2: two checkpoints of one size.
    let point_text = text_of(provenance.checkpoint.as_bytes());
    let older_line = format!(
        "ita-root 1 {}",
        una_project::tlog::root::digest(&registry.roots[0])
    );
    let same_size_text = point_text.replacen(&older_line, &registry.root_line(), 1);
    let same_size = String::from_utf8(resigned(
        &registry,
        &same_size_text,
        &registry.keys.witnesses,
        NOW,
    ))
    .unwrap();
    assert!(refused(
        with(&|l| {
            l.checkpoint = same_size.clone();
            l.consistency.clear();
        }),
        "two checkpoints of size"
    ));
}

/// Item 4: the load is anchored to the root-version floor this machine stores, and to the last
/// root version the lock records. A chain below the floor is refused (U7010), saying that it is
/// older than what this machine accepted and never that the log forked; a floor that exists
/// and cannot be read is unavailable (U7013); a chain without the lock's last root, or with
/// another digest for it, is refused.
#[test]
fn the_offline_load_is_anchored_to_the_floor_and_the_lock() {
    let (registry, at) = rotated_with_older_point(2);
    let (provenance, expected) = provenance_at(&registry, &at);
    assert_eq!(expected.last_root, 2);
    let below = verify(
        &provenance,
        &expected,
        &Floor::At {
            version: 3,
            digest: None,
        },
    );
    assert!(
        refused(below.clone(), "older than root version 3"),
        "{below:?}"
    );
    assert!(!format!("{below:?}").contains("forked"));
    assert!(unavailable(
        verify(
            &provenance,
            &expected,
            &Floor::Unreadable("permission denied".to_owned())
        ),
        "cannot be read"
    ));
    let mut higher = expected.clone();
    higher.last_root = 3;
    assert!(refused(
        verify(&provenance, &higher, &Floor::None),
        "which the lock records"
    ));
    let mut other = expected.clone();
    other.last_root_digest = format!("sha256:{}", "0".repeat(64));
    assert!(refused(
        verify(&provenance, &other, &Floor::None),
        "which the lock records"
    ));
    // The rollback of the amendment's context: the later root versions dropped, and the proof
    // with them, so that the retired witnesses are witnesses of the last version stored.
    let mut truncated = provenance.clone();
    truncated.roots.truncate(1);
    truncated.latest = None;
    // The lock left as written: its last root is not in the chain.
    assert!(refused(
        verify(&truncated, &expected, &Floor::None),
        "which the lock records"
    ));
    // The lock edited too, a visible change in a reviewed diff: below the floor, refused.
    let mut edited = expected.clone();
    edited.last_root = 1;
    edited.last_root_digest = una_project::tlog::root::digest(&registry.roots[0]);
    assert!(refused(
        verify(
            &truncated,
            &edited,
            &Floor::At {
                version: 2,
                digest: None,
            }
        ),
        "older than root version 2"
    ));
    // Item 5, the residual: with no floor stored, the vendored chain past the pin is trusted on
    // first use, and the truncated chain loads.
    assert!(verify(&truncated, &edited, &Floor::None).is_ok());
}

/// ADR 0122, amendment of 2026-11-09 (L3): a floor that stores the digest of its root version
/// refuses (U7010) a chain whose version of the floor has another digest, a fork of the root
/// chain, at the chain's last version and below it; a floor of the earlier form, the version
/// alone, keeps the comparison by version. A chain below the floor is refused as older, never
/// as a fork. Each check is O(R) for R root versions, done once per load.
#[test]
fn a_floor_with_a_digest_refuses_a_fork_of_the_root_chain() {
    let (registry, at) = rotated_with_older_point(2);
    let (provenance, expected) = provenance_at(&registry, &at);
    let first = una_project::tlog::root::text_digest(&registry.roots[0]).unwrap();
    let second = una_project::tlog::root::text_digest(&registry.roots[1]).unwrap();
    let other = format!("sha256:{}", "0".repeat(64));
    let at = |version: u64, digest: &str| Floor::At {
        version,
        digest: Some(FloorDigest::Text(digest.to_owned())),
    };
    // A floor of the form of 2026-11-09: by the digest of the whole signed note.
    let whole = |version: u64, digest: &str| Floor::At {
        version,
        digest: Some(FloorDigest::Note(digest.to_owned())),
    };
    let first_note = una_project::tlog::root::digest(&registry.roots[0]);
    let second_note = una_project::tlog::root::digest(&registry.roots[1]);
    assert!(verify(&provenance, &expected, &whole(2, &second_note)).is_ok());
    assert!(verify(&provenance, &expected, &whole(1, &first_note)).is_ok());
    for floor in [
        whole(2, &other),
        whole(2, &second),
        whole(1, &first),
        at(2, &second_note),
        at(1, &first_note),
    ] {
        let forked = verify(&provenance, &expected, &floor);
        assert!(refused(forked.clone(), "forked"), "{floor:?}: {forked:?}");
    }
    assert!(verify(&provenance, &expected, &at(2, &second)).is_ok());
    assert!(verify(&provenance, &expected, &at(1, &first)).is_ok());
    for floor in [at(2, &other), at(2, &first), at(1, &other), at(1, &second)] {
        let forked = verify(&provenance, &expected, &floor);
        assert!(
            refused(forked.clone(), "with another digest") && refused(forked.clone(), "forked"),
            "{floor:?}: {forked:?}"
        );
        assert!(!format!("{forked:?}").contains("older than"));
    }
    // Below the floor, whatever its digest: older, not a fork.
    let below = verify(&provenance, &expected, &at(3, &other));
    assert!(
        refused(below.clone(), "older than root version 3"),
        "{below:?}"
    );
    assert!(!format!("{below:?}").contains("forked"));
    // The earlier form: by version only.
    for version in [1, 2] {
        let floor = Floor::At {
            version,
            digest: None,
        };
        assert!(verify(&provenance, &expected, &floor).is_ok());
    }
    // Raised to a walked version: a higher one replaces the stored floor and its digest; at the
    // same version the stored digest is kept, and a floor of the earlier form takes the walked
    // one; a lower one never lowers it; an unreadable floor stays unreadable.
    assert_eq!(Floor::None.at_least(2, &second), at(2, &second));
    assert_eq!(at(1, &first).at_least(2, &second), at(2, &second));
    assert_eq!(at(2, &other).at_least(2, &second), at(2, &other));
    assert_eq!(
        Floor::At {
            version: 2,
            digest: None
        }
        .at_least(2, &second),
        at(2, &second)
    );
    assert_eq!(at(3, &other).at_least(2, &second), at(3, &other));
    assert_eq!(
        whole(2, &second_note).at_least(2, &second),
        whole(2, &second_note)
    );
    assert_eq!(whole(1, &first_note).at_least(2, &second), at(2, &second));
    assert_eq!(
        Floor::Unreadable("denied".to_owned()).at_least(2, &second),
        Floor::Unreadable("denied".to_owned())
    );
}

/// Two runs read a root-version floor at the same time and each walks a root chain; the store
/// compares again under the floor lock, with the floor as the other run left it (`spec/30 §14`,
/// Floors; #2099). The run whose chain holds the stored floor's version with another digest
/// than the one held for it: its store fails as a fork, typed, and the floor the
/// first run stored is kept byte for byte, with nothing else left in the directory. The same
/// digest from both runs, and a higher version whose chain holds the floor's version with the
/// stored digest, still pass.
#[test]
fn a_store_that_finds_another_digest_under_the_lock_is_a_fork() {
    use una_project::itaoca::{FloorRefusal, Itaoca, registry_id};

    let dir = std::env::temp_dir().join(format!("una-root-floor-fork-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let itaoca = Itaoca { root: dir.clone() };
    let id = registry_id("sha256:root-floor-fork");
    let registry = dir.join("registry").join(&id);
    let file = registry.join("root-version");
    let digest = |byte: char| format!("sha256:{}", byte.to_string().repeat(64));
    let (a, b, c, d) = (digest('a'), digest('b'), digest('c'), digest('d'));
    let entries = || {
        let mut names: Vec<String> = std::fs::read_dir(&registry)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    };
    let write = |text: &str| {
        std::fs::create_dir_all(&registry).unwrap();
        let _ = std::fs::remove_file(&file);
        std::fs::write(&file, text).unwrap();
    };

    // Both runs read the floor of the earlier form, `2`. The first stores its digest.
    write("2\n");
    itaoca.store_root_chain(&id, &[walked(2, &a)]).unwrap();
    // The second walked another root metadata of version 2: a fork, and the floor is kept.
    let fork = itaoca.store_root_chain(&id, &[walked(2, &b)]).unwrap_err();
    assert_eq!(
        fork,
        FloorRefusal::Forked {
            version: 2,
            held: FloorDigest::Text(a.clone()),
            offered: b.clone(),
        }
    );
    assert!(
        fork.message()
            .contains("another digest is held for the same root version")
            && fork.message().contains("the floor is kept"),
        "{fork:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("2 text {a}\n")
    );
    // Nothing half-written: the floor, the lock file, and no fresh directory beside them.
    assert_eq!(entries(), ["floor.lock", "root-version"]);
    // A second run with the same digest: no failure, the floor as it is.
    itaoca.store_root_chain(&id, &[walked(2, &a)]).unwrap();
    itaoca
        .store_root_chain(&id, &[walked(1, &c), walked(2, &a)])
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("2 text {a}\n")
    );

    // Both runs read no floor at all: the same comparison.
    std::fs::remove_file(&file).unwrap();
    itaoca
        .store_root_chain(&id, &[walked(1, &c), walked(2, &a)])
        .unwrap();
    let fork = itaoca
        .store_root_chain(&id, &[walked(1, &c), walked(2, &b)])
        .unwrap_err();
    assert!(
        matches!(fork, FloorRefusal::Forked { version: 2, .. }),
        "{fork:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("2 text {a}\n")
    );

    // A chain that goes higher but holds the floor's version with another digest forked below
    // its last version: refused, and the floor does not move to it.
    let fork = itaoca
        .store_root_chain(&id, &[walked(1, &c), walked(2, &b), walked(3, &d)])
        .unwrap_err();
    assert_eq!(
        fork,
        FloorRefusal::Forked {
            version: 2,
            held: FloorDigest::Text(a.clone()),
            offered: b.clone(),
        }
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("2 text {a}\n")
    );
    // A higher version whose chain holds the floor's version with the stored digest: stored.
    itaoca
        .store_root_chain(&id, &[walked(1, &c), walked(2, &a), walked(3, &d)])
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("3 text {d}\n")
    );
    // A run whose chain ends below the floor stored meanwhile is not stored, and is no fork:
    // the floor holds one digest, of a version that chain does not reach.
    itaoca
        .store_root_chain(&id, &[walked(1, &c), walked(2, &b)])
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("3 text {d}\n")
    );
    assert_eq!(entries(), ["floor.lock", "root-version"]);

    // What is not a chain is not stored, and is not a fork.
    let mut half = walked(4, &a);
    half.note = "sha256:zz".to_owned();
    for chain in [
        vec![],
        vec![walked(2, &a), walked(2, &a)],
        vec![walked(3, &d), walked(2, &a)],
        vec![walked(3, "sha256:zz"), walked(4, &a)],
        vec![half],
    ] {
        let refusal = itaoca.store_root_chain(&id, &chain).unwrap_err();
        assert!(matches!(refusal, FloorRefusal::NotStored(_)), "{refusal:?}");
    }
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("3 text {d}\n")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The race itself, with threads, each with its own handle of the lock file, as two processes
/// have: eight runs read the floor `3` of the earlier form and store, at once, four different
/// root metadata of version 3, two runs each. Whatever the order, one digest is stored, the two
/// runs that hold it succeed, the six others fail with that digest as the one held, and the
/// file is one whole floor line (#2099).
#[test]
fn stores_of_different_digests_at_once_agree_on_one() {
    use std::sync::{Arc, Barrier};
    use una_project::itaoca::{FloorRefusal, Itaoca, RootFloor, registry_id};

    let dir = std::env::temp_dir().join(format!("una-root-floor-race-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let id = registry_id("sha256:root-floor-race");
    let registry = dir.join("registry").join(&id);
    std::fs::create_dir_all(&registry).unwrap();
    let file = registry.join("root-version");
    for round in 0..8 {
        let _ = std::fs::remove_file(&file);
        std::fs::write(&file, "3\n").unwrap();
        let barrier = Arc::new(Barrier::new(8));
        let runs: Vec<_> = (0..8usize)
            .map(|run| {
                let barrier = Arc::clone(&barrier);
                let itaoca = Itaoca { root: dir.clone() };
                let id = id.clone();
                std::thread::spawn(move || {
                    let digest = format!(
                        "sha256:{}",
                        ((b'a' + (run % 4) as u8) as char).to_string().repeat(64)
                    );
                    // Each run read the floor before any stored: by version only.
                    assert_eq!(
                        itaoca.load_root_version(&id).unwrap(),
                        Some(RootFloor {
                            version: 3,
                            digest: None
                        })
                    );
                    barrier.wait();
                    let stored = itaoca.store_root_chain(&id, &[walked(3, &digest)]);
                    (digest, stored)
                })
            })
            .collect();
        let results: Vec<_> = runs.into_iter().map(|run| run.join().unwrap()).collect();
        let text = std::fs::read_to_string(&file).unwrap();
        let floor = RootFloor::parse(&text).unwrap();
        let held = floor.digest.clone().unwrap();
        assert!(matches!(held, FloorDigest::Text(_)), "round {round}");
        assert_eq!(floor.version, 3, "round {round}");
        let mut passed = 0;
        for (digest, stored) in &results {
            if *digest == held.value() {
                assert_eq!(*stored, Ok(()), "round {round}");
                passed += 1;
            } else {
                assert_eq!(
                    *stored,
                    Err(FloorRefusal::Forked {
                        version: 3,
                        held: held.clone(),
                        offered: digest.clone(),
                    }),
                    "round {round}"
                );
            }
        }
        assert_eq!(passed, 2, "round {round}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The root-version floor file (`spec/30 §14`, Floors; ADR 0122, amendments of 2026-11-09, L3,
/// and of 2026-11-19): stored as `<n> text sha256:<hex>` and a line feed, the digest of the
/// text of that root metadata, read back the same; the earlier forms, `<n> sha256:<hex>`, the
/// digest of the whole signed note, and `<n>`, are read as what they are; any other content is
/// unreadable (U7013 where a load reports it), never no floor; and a stored floor is replaced
/// only by a higher version, never by an equal one with another digest (refused as a fork), nor
/// by a lower one, while a floor of the earliest form takes the digest of its own version
/// (#1970).
#[test]
fn the_root_version_floor_file_round_trips_and_moves_only_up() {
    use una_project::itaoca::{FloorRefusal, Itaoca, RootFloor, registry_id};

    let dir = std::env::temp_dir().join(format!("una-root-floor-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let itaoca = Itaoca { root: dir.clone() };
    let id = registry_id("sha256:root-floor");
    let file = dir.join("registry").join(&id).join("root-version");
    let digest = |byte: char| format!("sha256:{}", byte.to_string().repeat(64));
    let floor = |version: u64, digest: Option<String>| {
        Some(RootFloor {
            version,
            digest: digest.map(FloorDigest::Text),
        })
    };

    assert_eq!(itaoca.load_root_version(&id).unwrap(), None);
    itaoca
        .store_root_chain(&id, &[walked(2, &digest('a'))])
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("2 text {}\n", digest('a'))
    );
    assert_eq!(
        itaoca.load_root_version(&id).unwrap(),
        floor(2, Some(digest('a')))
    );
    // Equal, with the same digest: kept as it is.
    itaoca
        .store_root_chain(&id, &[walked(2, &digest('a'))])
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("2 text {}\n", digest('a'))
    );
    // Equal, with another digest: a fork of the root chain, refused, never stored.
    let fork = itaoca
        .store_root_chain(&id, &[walked(2, &digest('b'))])
        .unwrap_err();
    assert!(matches!(fork, FloorRefusal::Forked { .. }), "{fork:?}");
    assert!(fork.message().contains("the floor is kept"), "{fork:?}");
    // Lower: never stored.
    itaoca
        .store_root_chain(&id, &[walked(1, &digest('c'))])
        .unwrap();
    assert_eq!(
        itaoca.load_root_version(&id).unwrap(),
        floor(2, Some(digest('a')))
    );
    // Higher: stored, with its digest.
    itaoca
        .store_root_chain(&id, &[walked(3, &digest('d'))])
        .unwrap();
    assert_eq!(
        itaoca.load_root_version(&id).unwrap(),
        floor(3, Some(digest('d')))
    );
    // A digest that is not one is never stored.
    for refusal in [
        itaoca
            .store_root_chain(&id, &[walked(4, "sha256:zz")])
            .unwrap_err(),
        itaoca
            .store_root_chain(&id, &[walked(0, &digest('e'))])
            .unwrap_err(),
    ] {
        assert!(matches!(refusal, FloorRefusal::NotStored(_)), "{refusal:?}");
    }
    assert_eq!(
        itaoca.load_root_version(&id).unwrap(),
        floor(3, Some(digest('d')))
    );

    // The earliest form, as builds before v0.4.0 write it: the version alone, read without a
    // digest. A lower version leaves it as it is; the equal version takes its digest, of the
    // text (#1970), after which another digest at that version is a fork, refused, and a lower
    // one is still not stored; a higher one replaces it.
    std::fs::remove_file(&file).unwrap();
    std::fs::write(&file, "3\n").unwrap();
    assert_eq!(itaoca.load_root_version(&id).unwrap(), floor(3, None));
    itaoca
        .store_root_chain(&id, &[walked(2, &digest('a'))])
        .unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "3\n");
    itaoca
        .store_root_chain(&id, &[walked(3, &digest('a'))])
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("3 text {}\n", digest('a'))
    );
    let fork = itaoca
        .store_root_chain(&id, &[walked(3, &digest('b'))])
        .unwrap_err();
    assert!(matches!(fork, FloorRefusal::Forked { .. }), "{fork:?}");
    itaoca
        .store_root_chain(&id, &[walked(2, &digest('b'))])
        .unwrap();
    assert_eq!(
        itaoca.load_root_version(&id).unwrap(),
        floor(3, Some(digest('a')))
    );
    // A digest that is not one never upgrades a floor of the earlier form.
    std::fs::remove_file(&file).unwrap();
    std::fs::write(&file, "3\n").unwrap();
    assert!(itaoca.store_root_chain(&id, &[walked(3, "")]).is_err());
    assert!(
        itaoca
            .store_root_chain(&id, &[walked(3, "sha256:zz")])
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "3\n");
    itaoca
        .store_root_chain(&id, &[walked(4, &digest('a'))])
        .unwrap();
    assert_eq!(
        itaoca.load_root_version(&id).unwrap(),
        floor(4, Some(digest('a')))
    );

    // Malformed: unreadable, never none.
    let hex = "a".repeat(64);
    for text in [
        String::new(),
        "\n".to_owned(),
        "3".to_owned(),
        "3\n\n".to_owned(),
        "03\n".to_owned(),
        "0\n".to_owned(),
        "-3\n".to_owned(),
        "3 \n".to_owned(),
        format!(" 3 sha256:{hex}\n"),
        format!("3  sha256:{hex}\n"),
        format!("3 sha256:{hex} \n"),
        format!("3 sha256:{hex}"),
        format!("3 sha256:{}\n", "A".repeat(64)),
        format!("3 sha256:{}\n", "a".repeat(63)),
        format!("3 sha256:{}\n", "a".repeat(65)),
        format!("3 sha512:{hex}\n"),
        format!("3 {hex}\n"),
        format!("3 sha256:{hex}\r\n"),
        "99999999999999999999\n".to_owned(),
        format!("3 sha256:{hex} 4\n"),
        "3 text\n".to_owned(),
        "3 text \n".to_owned(),
        format!("3 text  sha256:{hex}\n"),
        format!("3  text sha256:{hex}\n"),
        format!("3 text sha256:{hex} \n"),
        format!("3 text sha256:{hex} 4\n"),
        format!("3 text sha256:{hex}"),
        format!("3 text sha256:{}\n", "a".repeat(63)),
        format!("3 text sha512:{hex}\n"),
        format!("3 text {hex}\n"),
        format!("3 TEXT sha256:{hex}\n"),
        format!("3 note sha256:{hex}\n"),
        format!("3 text text sha256:{hex}\n"),
        format!("3 sha256:{hex} text\n"),
        format!("text sha256:{hex}\n"),
        format!("0 text sha256:{hex}\n"),
        format!("3\ttext sha256:{hex}\n"),
    ] {
        std::fs::remove_file(&file).unwrap();
        std::fs::write(&file, &text).unwrap();
        assert!(
            itaoca.load_root_version(&id).is_err(),
            "{text:?} read as a floor"
        );
        assert!(RootFloor::parse(&text).is_none(), "{text:?}");
    }
    // The format round-trips.
    for floor in [
        RootFloor {
            version: 1,
            digest: None,
        },
        RootFloor {
            version: u64::MAX,
            digest: Some(FloorDigest::Text(digest('f'))),
        },
        RootFloor {
            version: u64::MAX,
            digest: Some(FloorDigest::Note(digest('f'))),
        },
    ] {
        assert_eq!(RootFloor::parse(&floor.line().unwrap()), Some(floor));
    }
    // The three forms, each read as what it is, and the longest one within the read's bound.
    for (text, digest) in [
        ("3\n".to_owned(), None),
        (
            format!("3 sha256:{hex}\n"),
            Some(FloorDigest::Note(format!("sha256:{hex}"))),
        ),
        (
            format!("18446744073709551615 text sha256:{hex}\n"),
            Some(FloorDigest::Text(format!("sha256:{hex}"))),
        ),
    ] {
        std::fs::remove_file(&file).unwrap();
        std::fs::write(&file, &text).unwrap();
        let read = itaoca.load_root_version(&id).unwrap().unwrap();
        assert_eq!(read.digest, digest, "{text:?}");
        assert_eq!(read.line().unwrap(), text);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A registry whose lock's point, the checkpoint returned with its tree hash, is signed under
/// root 1, before root 2 rotates the root metadata with the same witnesses and a later
/// checkpoint is signed under it: the scenario of #2132.
fn rotated_under_the_same_witnesses() -> (TestRegistry, Point) {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    let older = registry.checkpoint(NOW - 10 * DAY);
    let leaves: Vec<Hash> = registry.entries.iter().map(|e| leaf_hash(e)).collect();
    let hash = hash_hex(&tree_root(&leaves));
    registry.rotate_witnesses(0, false);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&a]);
    registry.checkpoint(NOW);
    let at = Point::At { size: older, hash };
    (registry, at)
}

/// `document`, a signed note, with one more signature line: the name of a key nobody holds and
/// bytes that are no signature. Whoever serves a note can add it; it needs no key.
fn with_a_junk_line(document: &str) -> String {
    format!(
        "{document}\u{2014} nobody {}\n",
        una_project::tlog::note::encode_base64(&[7u8; 68])
    )
}

/// Opens `transport` at `point` with the floors `itaoca` stores for `registry`, and stores the
/// root chain walked as its root-version floor, as `una ita` does for every place it reads
/// (`una_cli::registry::open`): a fork found by the store is a refusal, any other failure to
/// store is a note, returned.
fn open_and_store<'t>(
    itaoca: &una_project::itaoca::Itaoca,
    transport: &'t dyn Transport,
    registry: &TestRegistry,
    point: &Point,
) -> Result<(Registry<'t>, Option<String>), OpenError> {
    use una_project::itaoca::{FloorRefusal, registry_id};

    let id = registry_id(&registry.pin());
    let stored = itaoca.load_root_version(&id).unwrap();
    let floors = Floors {
        checkpoint: None,
        root_version: stored.as_ref().map(|floor| floor.version),
        root_digest: stored.as_ref().and_then(|floor| floor.digest.as_ref()),
    };
    let client = Registry::open_with("acme", transport, &registry.pin(), point, &floors)?;
    match itaoca.store_root_chain(&id, &client.root_chain()) {
        Ok(()) => Ok((client, None)),
        Err(FloorRefusal::Forked { .. }) => Err(OpenError::Refused("forked".to_owned())),
        Err(FloorRefusal::NotStored(why)) => Ok((client, Some(why))),
    }
}

/// #2132, fixed by ADR 0122, amendment of 2026-11-19: a place that holds no key cannot poison
/// the root-version floor. The lock was made under root 1; the registry has since rotated to
/// root 2 with the same witnesses. A place serves the honest root 2 in another form, its
/// signature lines changed with no key (a junk line added, the lines in another order, or a
/// further root key's line present or dropped), and no latest checkpoint, so that no accepted
/// checkpoint names root 2. Each form verifies and is accepted, as before; the floor stored
/// holds the digest of the text, which is the honest note's too, so the honest registry is
/// accepted afterwards at the lock's point and at its latest checkpoint, and the honest
/// provenance loads offline. Before the fix the floor held the digest of the altered note,
/// and every one of those was refused as forked (U7010).
///
/// What the floor still refuses: another text of version 2, which a threshold of root keys
/// signed.
#[test]
fn a_place_with_no_key_cannot_poison_the_root_floor() {
    use una_project::itaoca::{Itaoca, RootFloor, registry_id};
    use una_project::tlog::root::{digest, text_digest};

    let (registry, at) = rotated_under_the_same_witnesses();
    let honest = Memory {
        files: registry.files.clone(),
    };
    let second = &registry.roots[1];
    let text = second.split_once("\n\n").unwrap().0.to_owned() + "\n";
    let lines: Vec<&str> = second
        .split_once("\n\n")
        .unwrap()
        .1
        .split_terminator('\n')
        .collect();
    assert_eq!(lines.len(), 2);
    let third = &registry.keys.roots[2];
    let forms = [
        ("a junk line", with_a_junk_line(second)),
        (
            "another order",
            format!("{text}\n{}\n{}\n", lines[1], lines[0]),
        ),
        (
            "a further signer",
            format!("{second}{}", registry::sign_line(&third.0, &third.1, &text)),
        ),
    ];
    let floor_of_the_text = RootFloor {
        version: 2,
        digest: Some(FloorDigest::Text(text_digest(second).unwrap())),
    };
    // The honest provenance of the lock's point, written from the honest registry, and what
    // the lock says: it was written under root 1, so it records `last root 1`.
    let (provenance, mut expected) = provenance_at(&registry, &at);
    expected.last_root = 1;
    expected.last_root_digest = registry.pin();
    for (name, altered) in &forms {
        assert_ne!(digest(altered), digest(second), "{name}");
        assert_eq!(
            text_digest(altered).unwrap(),
            text_digest(second).unwrap(),
            "{name}"
        );
        let mut hostile = Memory {
            files: registry.files.clone(),
        };
        hostile.files.remove("checkpoint");
        hostile
            .files
            .insert("root/2".to_owned(), altered.clone().into_bytes());
        let itaoca = Itaoca {
            root: scratch("root-floor-poison"),
        };
        let id = registry_id(&registry.pin());
        // What the machine stored when it locked, under root 1.
        itaoca
            .store_root_chain(&id, &[walked_root(1, &registry.roots[0])])
            .unwrap();

        // The fetch from the hostile place: accepted, with no note, and the release reads.
        let (mut client, note) = open_and_store(&itaoca, &hostile, &registry, &at).unwrap();
        assert_eq!(note, None, "{name}");
        assert_eq!(client.last_root(), Some((2, digest(altered).as_str())));
        let release = client
            .release("acme/ledger", &Version::parse_full("0.3.1").unwrap())
            .unwrap();
        // The provenance it writes holds the altered note.
        let key = release.signer().unwrap().clone();
        let from_hostile = client.provenance(&release, &key).unwrap();
        assert_eq!(from_hostile.roots[1], *altered);
        drop(client);
        // The floor holds root version 2 with the digest of the text: the honest note's.
        assert_eq!(
            itaoca.load_root_version(&id).unwrap(),
            Some(floor_of_the_text.clone()),
            "{name}"
        );

        // The honest registry is accepted from then on, whatever the point, and stores nothing
        // new.
        for point in [at.clone(), Point::Latest { now: NOW + 60 }] {
            match open_and_store(&itaoca, &honest, &registry, &point) {
                Ok((_, note)) => assert_eq!(note, None, "{name}"),
                Err(error) => panic!("{name}: {}", error.message()),
            }
        }
        assert_eq!(
            itaoca.load_root_version(&id).unwrap(),
            Some(floor_of_the_text.clone())
        );
        // Offline, the floor accepts the honest provenance and the one written from the
        // hostile place alike: one root text.
        let floor = Floor::At {
            version: 2,
            digest: floor_of_the_text.digest.clone(),
        };
        let offline = verify(&provenance, &expected, &floor);
        assert!(offline.is_ok(), "{name}: {offline:?}");
        let offline = verify(&from_hostile, &expected, &floor);
        assert!(offline.is_ok(), "{name}: {offline:?}");

        // Another text of version 2, signed by the threshold of the root keys (here it expires
        // a second later): the fork the floor is for, refused, and the floor is kept.
        let roots: Vec<&una_project::tlog::note::Key> =
            registry.keys.roots.iter().map(|(_, key)| key).collect();
        let other_text = registry::root_text_with(
            "acme",
            2,
            registry::EXPIRES + 1,
            &registry.keys,
            &roots,
            2,
            &[],
        );
        let forked = registry::signed(
            &other_text,
            &[&registry.keys.roots[0], &registry.keys.roots[1]],
        );
        let mut fork = Memory {
            files: registry.files.clone(),
        };
        fork.files.remove("checkpoint");
        fork.files
            .insert("root/2".to_owned(), forked.clone().into_bytes());
        // With no floor, that place is accepted at the lock's point, as the altered note was.
        let unanchored =
            Registry::open_with("acme", &fork, &registry.pin(), &at, &Floors::default());
        assert!(unanchored.is_ok(), "{:?}", unanchored.err());
        match open_and_store(&itaoca, &fork, &registry, &at) {
            Err(OpenError::Refused(why)) => assert!(
                why.contains("root version 2 with another digest of the text")
                    && why.contains("forked"),
                "{why}"
            ),
            Err(other) => panic!("{}", other.message()),
            Ok(_) => panic!("{name}: a fork of the root chain accepted"),
        }
        assert_eq!(
            itaoca.load_root_version(&id).unwrap(),
            Some(floor_of_the_text.clone())
        );
    }
}

/// ADR 0122, amendment of 2026-11-19, item 5: a floor of the form of 2026-11-09, the digest of
/// the whole signed note, is compared as it was stored, and is upgraded to the digest of the
/// text once, under the lock, only by a chain that holds its version with exactly that note,
/// and then to the digest of that same note's text. A note of another whole-note digest, the
/// same text in another form included, is refused as before and the floor is kept byte for
/// byte, so the upgrade accepts nothing the stored floor refused. A higher version replaces it
/// by the ordinary rule; a chain that ends below it changes nothing.
#[test]
fn a_floor_of_the_whole_note_is_upgraded_once_and_only_by_the_same_note() {
    use una_project::itaoca::{FloorRefusal, Itaoca, RootFloor, registry_id};
    use una_project::tlog::root::{digest, text_digest};

    let (registry, at) = rotated_under_the_same_witnesses();
    let (first, second) = (&registry.roots[0], &registry.roots[1]);
    let altered = with_a_junk_line(second);
    let note = digest(second);
    let text = text_digest(second).unwrap();
    let itaoca = Itaoca {
        root: scratch("root-floor-upgrade"),
    };
    let id = registry_id(&registry.pin());
    let dir = itaoca.root.join("registry").join(&id);
    let file = dir.join("root-version");
    let write = |line: &str| {
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(&file);
        std::fs::write(&file, line).unwrap();
    };
    let read = || std::fs::read_to_string(&file).unwrap();
    let old = format!("2 {note}\n");
    let new = format!("2 text {text}\n");

    // As v0.4.1 and earlier stored it.
    write(&old);
    assert_eq!(
        itaoca.load_root_version(&id).unwrap(),
        Some(RootFloor {
            version: 2,
            digest: Some(FloorDigest::Note(note.clone())),
        })
    );
    // The same text in another form: another whole-note digest, refused as it was before, and
    // nothing is upgraded.
    let fork = itaoca
        .store_root_chain(&id, &[walked_root(1, first), walked_root(2, &altered)])
        .unwrap_err();
    assert_eq!(
        fork,
        FloorRefusal::Forked {
            version: 2,
            held: FloorDigest::Note(note.clone()),
            offered: digest(&altered),
        }
    );
    assert!(fork.message().contains("a floor of the earlier form"));
    assert_eq!(read(), old);
    // Nor by a chain that goes higher past another note of version 2.
    let fork = itaoca
        .store_root_chain(
            &id,
            &[
                walked_root(1, first),
                walked_root(2, &altered),
                walked(3, &format!("sha256:{}", "3".repeat(64))),
            ],
        )
        .unwrap_err();
    assert!(
        matches!(fork, FloorRefusal::Forked { version: 2, .. }),
        "{fork:?}"
    );
    assert_eq!(read(), old);
    // A digest that is the stored one, but offered as a text's: not the same note.
    let mut posing = walked_root(2, &altered);
    posing.text.clone_from(&note);
    assert!(matches!(
        itaoca.store_root_chain(&id, &[posing]).unwrap_err(),
        FloorRefusal::Forked { version: 2, .. }
    ));
    assert_eq!(read(), old);
    // A chain that ends below the floor: not stored, nothing upgraded.
    itaoca
        .store_root_chain(&id, &[walked_root(1, first)])
        .unwrap();
    assert_eq!(read(), old);
    // The client's walk compares such a floor by the whole note too.
    let floors = Floors {
        checkpoint: None,
        root_version: Some(2),
        root_digest: Some(&FloorDigest::Note(note.clone())),
    };
    let honest = Memory {
        files: registry.files.clone(),
    };
    let mut other_form = Memory {
        files: registry.files.clone(),
    };
    other_form
        .files
        .insert("root/2".to_owned(), altered.clone().into_bytes());
    other_form.files.remove("checkpoint");
    assert!(Registry::open_with("acme", &honest, &registry.pin(), &at, &floors).is_ok());
    assert!(matches!(
        Registry::open_with("acme", &other_form, &registry.pin(), &at, &floors),
        Err(OpenError::Refused(why)) if why.contains("forked") && why.contains("earlier form")
    ));

    // The chain that holds version 2 with exactly the stored note: the floor takes the digest
    // of that note's text, at the same version.
    itaoca
        .store_root_chain(&id, &[walked_root(1, first), walked_root(2, second)])
        .unwrap();
    assert_eq!(read(), new);
    assert_eq!(
        itaoca.load_root_version(&id).unwrap(),
        Some(RootFloor {
            version: 2,
            digest: Some(FloorDigest::Text(text.clone())),
        })
    );
    // Once: the next stores of that text, in any form, write nothing.
    #[cfg(unix)]
    let written = {
        use std::os::unix::fs::MetadataExt;
        || std::fs::metadata(&file).unwrap().ino()
    };
    #[cfg(unix)]
    let before = written();
    for form in [second.as_str(), altered.as_str()] {
        itaoca
            .store_root_chain(&id, &[walked_root(1, first), walked_root(2, form)])
            .unwrap();
        assert_eq!(read(), new);
    }
    #[cfg(unix)]
    assert_eq!(written(), before, "the floor was written again");
    // And from then on the comparison is by the text: another text is a fork, whatever its
    // whole-note digest, the stored one included.
    let mut same_note_other_text = walked_root(2, second);
    same_note_other_text.text = format!("sha256:{}", "0".repeat(64));
    let fork = itaoca
        .store_root_chain(&id, &[same_note_other_text])
        .unwrap_err();
    assert_eq!(
        fork,
        FloorRefusal::Forked {
            version: 2,
            held: FloorDigest::Text(text.clone()),
            offered: format!("sha256:{}", "0".repeat(64)),
        }
    );
    assert_eq!(read(), new);

    // A higher version, from a floor of the earlier form whose note the chain holds: replaced
    // by the ordinary rule, in the new form.
    write(&old);
    let third = format!("sha256:{}", "3".repeat(64));
    itaoca
        .store_root_chain(
            &id,
            &[
                walked_root(1, first),
                walked_root(2, second),
                walked(3, &third),
            ],
        )
        .unwrap();
    assert_eq!(read(), format!("3 text {third}\n"));
}

/// L3 of the review of #2128 (card #2132; ADR 0122, amendment of 2026-11-19, item 7): the floor
/// lock is not held while a checkpoint store proves the stored checkpoint consistent with its
/// own, which reads tiles from a place. While one run's proof stalls, another run, whose root
/// chain holds the floor's version with another digest, takes the lock at once, compares, and
/// is refused as a fork. Before the fix it waited five seconds for the lock, compared nothing,
/// and got a failure to store, which a command reports as a note and exit code 0.
///
/// The stalled store still writes only against the checkpoint it proved: one that another run
/// stored meanwhile is proven again, a proof that then fails stores nothing, and a stored
/// checkpoint that keeps being replaced ends the store after a bounded number of proofs.
#[test]
fn a_stalled_checkpoint_store_does_not_hold_the_floor_lock() {
    use std::sync::mpsc::channel;
    use una_project::itaoca::{FloorRefusal, Itaoca, registry_id};
    use una_project::tlog::note::{parse_checkpoint, parse_note};

    let itaoca = Itaoca {
        root: scratch("floor-lock-stall"),
    };
    let id = registry_id("sha256:floor-lock-stall");
    let checkpoint = |size: u64| {
        format!(
            "log\n{size}\n47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=\n\n{dash} log AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\n",
            dash = '\u{2014}'
        )
    };
    let size_of = |text: &str| {
        parse_checkpoint(&parse_note(text).unwrap().text)
            .unwrap()
            .size
    };
    let stored_size = || size_of(&itaoca.load_checkpoint(&id).unwrap().unwrap());
    let digest = |byte: char| format!("sha256:{}", byte.to_string().repeat(64));
    itaoca
        .store_checkpoint(&id, &checkpoint(1), &mut |_| Ok(()))
        .unwrap();
    itaoca
        .store_root_chain(&id, &[walked(2, &digest('a'))])
        .unwrap();
    let (inside, entered) = channel::<()>();
    let (release, released) = channel::<()>();
    let proved = std::thread::scope(|scope| {
        let itaoca = &itaoca;
        let id = &id;
        // The run whose place stalls: inside its first consistency proof until released.
        let stalled = scope.spawn(move || {
            let mut proved: Vec<u64> = Vec::new();
            let stored = itaoca.store_checkpoint(id, &checkpoint(3), &mut |stored| {
                proved.push(size_of(stored));
                if proved.len() == 1 {
                    inside.send(()).unwrap();
                    released.recv().unwrap();
                }
                Ok(())
            });
            (stored, proved)
        });
        entered.recv().unwrap();
        // The other run walked another root metadata of version 2: compared under the lock,
        // which the stalled run does not hold, and refused as a fork.
        let refusal = itaoca
            .store_root_chain(id, &[walked(2, &digest('b'))])
            .unwrap_err();
        assert_eq!(
            refusal,
            FloorRefusal::Forked {
                version: 2,
                held: FloorDigest::Text(digest('a')),
                offered: digest('b'),
            }
        );
        // And a third run stores a larger checkpoint meanwhile.
        itaoca
            .store_checkpoint(id, &checkpoint(2), &mut |_| Ok(()))
            .unwrap();
        release.send(()).unwrap();
        let (stored, proved) = stalled.join().unwrap();
        stored.unwrap();
        proved
    });
    // The stalled run proved against the checkpoint of size 1, found the one of size 2 under
    // the lock, proved against that one, and only then stored its own.
    assert_eq!(proved, [1, 2]);
    assert_eq!(stored_size(), 3);

    // A proof against the checkpoint stored meanwhile that fails: nothing is stored.
    let mut calls = 0;
    let refused = itaoca.store_checkpoint(&id, &checkpoint(9), &mut |stored| {
        calls += 1;
        if size_of(stored) == 3 {
            // Another run, while this proof is made.
            itaoca.store_checkpoint(&id, &checkpoint(4), &mut |_| Ok(()))
        } else {
            Err("another log".to_owned())
        }
    });
    assert_eq!(refused, Err("another log".to_owned()));
    assert_eq!((calls, stored_size()), (2, 4));

    // A stored checkpoint replaced during every proof: the store ends, bounded, with nothing
    // of its own written. The store made inside the proof takes the lock, so it also shows
    // that the lock is not held there.
    let mut calls = 0;
    let refused = itaoca.store_checkpoint(&id, &checkpoint(99), &mut |stored| {
        calls += 1;
        itaoca.store_checkpoint(&id, &checkpoint(size_of(stored) + 1), &mut |_| Ok(()))
    });
    assert!(
        matches!(&refused, Err(why) if why.contains("replaced by other runs")),
        "{refused:?}"
    );
    assert_eq!((calls, stored_size()), (3, 7));
    // Left alone, it stores.
    itaoca
        .store_checkpoint(&id, &checkpoint(99), &mut |_| Ok(()))
        .unwrap();
    assert_eq!(stored_size(), 99);
}

/// Item 2: the last root's expiry is judged at the witness time of the latest checkpoint the
/// provenance carries, which the current witnesses cosign, never at the older point's.
#[test]
fn the_last_root_expires_by_the_latest_checkpoints_time() {
    let a = keypair(60, "acme.a");
    let mut registry = TestRegistry::with_witness_threshold("acme", 2);
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    let older = registry.checkpoint(NOW - 10 * DAY);
    let leaves: Vec<Hash> = registry.entries.iter().map(|e| leaf_hash(e)).collect();
    let hash = hash_hex(&tree_root(&leaves));
    registry.rotate_witnesses(2, false);
    // Root 2 expires between the point's witness time and the latest checkpoint's.
    let roots: Vec<&una_project::tlog::note::Key> =
        registry.keys.roots.iter().map(|(_, key)| key).collect();
    let text = registry::root_text_with("acme", 2, NOW - DAY, &registry.keys, &roots, 2, &[]);
    let document = registry::signed(&text, &[&registry.keys.roots[0], &registry.keys.roots[1]]);
    registry
        .files
        .insert("root/2".to_owned(), document.clone().into_bytes());
    registry.roots[1] = document;
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&a]);
    registry.checkpoint(NOW);
    let at = Point::At { size: older, hash };
    let (provenance, expected) = provenance_at(&registry, &at);
    assert!(provenance.latest.is_some());
    assert!(refused(
        verify(&provenance, &expected, &Floor::None),
        "expired"
    ));
}

/// The registry of [`rotated_with_older_point`] served by a place whose latest checkpoint is of
/// the lock's point's own size and names the last root, signed by the log and cosigned by the
/// current witnesses: with the point, two checkpoints of one size naming different versions.
fn same_size_latest() -> Option<(TestRegistry, Point, Memory, u64)> {
    let (registry, at) = rotated_with_older_point(2);
    let size = match &at {
        Point::At { size, .. } => *size,
        _ => return None,
    };
    let point_text = text_of(&registry.files[&format!("checkpoint/{size}")]);
    let older_line = format!(
        "ita-root 1 {}",
        una_project::tlog::root::digest(&registry.roots[0])
    );
    let same_size_text = point_text.replacen(&older_line, &registry.root_line(), 1);
    let mut place = Memory {
        files: registry.files.clone(),
    };
    place.files.insert(
        "checkpoint".to_owned(),
        resigned(&registry, &same_size_text, &registry.keys.witnesses, NOW),
    );
    Some((registry, at, place, size))
}

/// The client's prefix proof (`spec/30 §11` rule 9): a latest checkpoint of the lock's point's
/// own size that names the last root while the point names an older one is a second checkpoint
/// of that size, refused (U7010), never a proof, and recorded as evidence against the registry,
/// the two versions and the places read, as two copies of a kept checkpoint are.
#[test]
fn a_latest_checkpoint_of_the_points_size_is_no_prefix_proof() {
    let (registry, at, place, size) = same_size_latest().unwrap();
    match Registry::open_with("acme", &place, &registry.pin(), &at, &Floors::default()) {
        Err(OpenError::Evidence(misbehavior, why)) => {
            assert!(why.contains("two checkpoints of size"), "{why}");
            assert_eq!(
                misbehavior,
                Misbehavior::Equivocation {
                    size,
                    versions: vec![1, 2],
                    places: vec!["memory".to_owned()],
                }
            );
        }
        other => panic!("{:?}", other.err()),
    }
}

/// Evidence against a registry needs both copies signed by the log: a point of the latest
/// checkpoint's size whose log signature does not verify is refused (U7010) as before, and
/// records no equivocation, since a place alone could have made it.
#[test]
fn a_same_size_point_the_log_did_not_sign_is_no_evidence() {
    let (registry, at, mut place, size) = same_size_latest().unwrap();
    let path = format!("checkpoint/{size}");
    let text = text_of(&place.files[&path]);
    // The point's text, signed by another key under the log's name.
    let forger = keypair(77, &registry.keys.log.1.name);
    let mut forged = format!(
        "{text}\n{}",
        registry::sign_line(&forger.0, &registry.keys.log.1, &text)
    );
    for (pair, key) in &registry.keys.witnesses {
        forged.push_str(&registry::cosign_line(pair, key, &text, NOW - 10 * DAY));
    }
    place.files.insert(path, forged.into_bytes());
    match Registry::open_with("acme", &place, &registry.pin(), &at, &Floors::default()) {
        Err(OpenError::Refused(why)) => assert!(why.contains("two checkpoints of size"), "{why}"),
        other => panic!("{:?}", other.err()),
    }
}
