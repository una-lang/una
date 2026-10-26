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

use registry::{NOW, TestRegistry, hash_hex, key_text, keypair};
use una_project::client::{
    Failure, Floors, LoggedRelease, Misbehavior, Omission, OpenError, Point, Registry, owner_of,
    shown,
};
use una_project::provenance::{Expected, signed_name, verify};
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
    assert_eq!(release.signer().unwrap().public, key.1.public);
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
    assert!(
        open(&memory, &registry)
            .unwrap()
            .releases("acme/ledger")
            .is_err()
    );

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
    assert!(
        open(&memory, &registry)
            .unwrap()
            .releases("acme/ledger")
            .is_err()
    );

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
    assert!(
        open(&memory, &after)
            .unwrap()
            .releases("acme/ledger")
            .is_err()
    );
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
    // The earliest position no place can move: the start is the k-th earliest cosignature on
    // the start checkpoint, so the back-dater does not make it earlier.
    assert_eq!(client.earliest_effect(1).unwrap(), Some(4));
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
    assert!(matches!(
        history.pending[0].change,
        owner::Change::Dispute(_)
    ));
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
    assert_eq!(
        open(&memory, &registry)
            .unwrap()
            .releases("acme/ledger")
            .unwrap()
            .len(),
        1
    );
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
    itaoca.store_root_version(&id, 3).unwrap();
    itaoca.store_root_version(&id, 2).unwrap();
    assert_eq!(itaoca.load_root_version(&id).unwrap(), Some(3));
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
    assert_eq!(
        history.compromised.get(&x.1.public),
        Some(&2),
        "clamped to the registration"
    );
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
        registry.owner_event(
            "acme",
            &format!("add {}", key_text(&pair.1)),
            1,
            &[&a, &pair],
        );
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
            assert!(
                history.is_ok(),
                "{claim_first} {since:?}: {:?}",
                history.err()
            );
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
        let designation =
            registry.owner_event("acme", &format!("cosigners 1 {}", key_text(&z.1)), 1, &[&a]);
        registry.checkpoint(NOW - 29 * DAY);
        let cancelled = cancel
            .then(|| registry.owner_event("acme", &format!("cancel {designation}"), 1, &[&b]));
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
        let designation =
            registry.owner_event("acme", &format!("cosigners 1 {}", key_text(&z.1)), 1, &[&a]);
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
    let (last, _) = registry.release("acme/ledger", "0.3.3", "none", "none", &[&n]);
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
    let c = keypair(62, "acme.c");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b], 1, &[], 0);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 40 * DAY);
    registry.recover("acme", "dispute", &m.1, &[]);
    let second = registry.recover("acme", "dispute", &n.1, &[]);
    registry.checkpoint(NOW - 39 * DAY);
    let third = registry.recover("acme", "dispute", &p.1, &[]);
    registry.owner_event("acme", &format!("add {}", key_text(&c.1)), 1, &[&a, &c]);
    registry.owner_event("acme", "revoke acme.a since 1", 1, &[&b]);
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
    assert_eq!(keys, &vec![a.1.clone(), b.1.clone()]);
    assert_eq!(history.compromised.get(&a.1.public), Some(&1));
    let releases = client.releases("acme/ledger").unwrap();
    assert_eq!(releases.len(), 1);
    assert!(!releases[0].compromised);
}

/// A revocation that the key it names signs is that key's own word, which a thief holding it can
/// give: it states nothing of the past, so what the key signed before it still counts. The same
/// claim signed by another key reaches back.
#[test]
fn a_claim_signed_by_the_key_it_names_states_nothing_of_the_past() {
    let a = keypair(60, "acme.a");
    let b = keypair(61, "acme.b");
    let build = |signer: &(ed25519_dalek::SigningKey, una_project::tlog::note::Key)| {
        let mut registry = TestRegistry::new("acme");
        registry.register_keys("acme", &[&a, &b], 1, &[], 0);
        registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
        registry.checkpoint(NOW - DAY);
        let claim = registry.owner_event("acme", "revoke acme.a since 0", 1, &[signer]);
        registry.release("acme/ledger", "0.3.2", "none", "none", &[&b]);
        registry.checkpoint(NOW);
        (registry, claim)
    };
    let (registry, claim) = build(&a);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.compromised.get(&a.1.public), Some(&claim));
    let releases = client.releases("acme/ledger").unwrap();
    assert!(!releases[0].compromised && !releases[1].compromised);
    let (registry, _) = build(&b);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.compromised.get(&a.1.public), Some(&0));
    let releases = client.releases("acme/ledger").unwrap();
    assert!(releases[0].compromised && !releases[1].compromised);
}

/// A chain that does not settle, which a registry must never log, fed directly: `A` adds `C`,
/// then `A` and `B` claim `A` since 0, and `A`, `B` and `C` claim `B` since 0. Assuming both
/// claims voids the addition of `C`; both revocations are then void, stand as revocations that
/// cancel each other, and are each their key's own word, so each claim starts at its own index.
/// From there both apply with another sound signer, so each reaches back to the registration
/// again: the claims alternate. The replay stops at its pass cap, the same on every run,
/// refuses the owner's names, and records the registry's misbehavior.
#[test]
fn a_chain_that_does_not_settle_is_refused_as_registry_misbehavior() {
    let a = keypair(60, "acme.a");
    let b = keypair(61, "acme.b");
    let c = keypair(62, "acme.c");
    let d = keypair(64, "acme.d");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b, &d], 1, &[], 0);
    registry.owner_event("acme", &format!("add {}", key_text(&c.1)), 1, &[&a, &c]);
    let claim = registry.owner_event("acme", "revoke acme.a since 0", 1, &[&a, &b]);
    registry.owner_event("acme", "revoke acme.b since 0", 1, &[&a, &b, &c]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&d]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let refusal = |client: &mut Registry<'_>| client.history("acme").unwrap_err();
    let mut client = open(&memory, &registry).unwrap();
    let first = refusal(&mut client);
    assert!(
        first.contains("logged an owner chain that does not settle")
            && first.contains("registry misbehavior")
            && first.contains("`acme`")
            && first.contains(&format!("first at the log index {claim}")),
        "{first}"
    );
    // The same stop on every run, and on a client of its own.
    assert_eq!(refusal(&mut client), first);
    let mut again = open(&memory, &registry).unwrap();
    assert_eq!(refusal(&mut again), first);
    assert!(client.releases("acme/ledger").is_err());
    assert_eq!(
        client.misbehavior(),
        &[Misbehavior::Unsettled(owner::Unsettled {
            owner: "acme".to_owned(),
            index: claim,
            passes: 6,
        })]
    );
    assert!(client.state("acme/ledger", Line::Zero(3)).is_err());
}

/// The last-moment cancellation: a thief holding the owner's only key cancels its recovery just
/// before the seven days end, and the effect checkpoint follows before the cosigners can
/// overrule it. The overruling still counts, after the seven days, and the recovery takes effect
/// from the entry after it.
#[test]
fn an_overruling_after_the_seven_days_still_counts() {
    let a = keypair(60, "acme.a");
    let x = keypair(70, "trustee.x");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a], 1, &[&x], 1);
    registry.checkpoint(NOW - 30 * DAY);
    let recovery = registry.recover("acme", "recover", &n.1, &[&x]);
    registry.checkpoint(NOW - 29 * DAY);
    let cancel = registry.owner_event("acme", &format!("cancel {recovery}"), 1, &[&a]);
    registry.checkpoint(NOW - 22 * DAY - 12 * 3600);
    registry.append("other", "filler");
    let effect = registry.checkpoint(NOW - 21 * DAY);
    let overrule = registry.owner_event("acme", &format!("overrule {cancel}"), 1, &[&x]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&n]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    assert_eq!(client.recovery_effect(recovery).unwrap(), Some(effect));
    let history = client.history("acme").unwrap();
    assert_eq!(history.took_effect.get(&recovery), Some(&(overrule + 1)));
    let releases = client.releases("acme/ledger").unwrap();
    assert!(!releases[0].compromised);
}

/// A stated threshold is checked as it was known at the entry's logging (ADR 0122 Amendment 9).
/// The owner of `{A, B}` at threshold two, below it once `A` is stolen, asks a recovery its
/// co-signer `X` signs; the thief cancels it with `A` six days in; after the seven days `X`
/// overrules the cancellation, stating the threshold two of its time; `R`, the recovered key,
/// then claims `A` since 0. The claim voids the cancellation, so the recovery takes effect at its
/// deadline, the overruling's own position, with threshold one: the overruling's stated two does
/// not refuse the chain, and `A` signs nothing that counts.
#[test]
fn a_later_claim_never_refuses_an_earlier_stated_threshold() {
    let a = keypair(60, "acme.a");
    let b = keypair(61, "acme.b");
    let r = keypair(63, "acme.r");
    let x = keypair(70, "trustee.x");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b], 2, &[&x], 1);
    registry.checkpoint(NOW - 31 * DAY);
    let recovery = registry.recover("acme", "recover", &r.1, &[&x]);
    registry.checkpoint(NOW - 30 * DAY);
    let cancel = registry.owner_event("acme", &format!("cancel {recovery}"), 2, &[&a]);
    registry.checkpoint(NOW - 24 * DAY);
    registry.append("other", "filler");
    let deadline = registry.checkpoint(NOW - 22 * DAY);
    let overrule = registry.owner_event("acme", &format!("overrule {cancel}"), 2, &[&x]);
    assert_eq!(overrule, deadline);
    registry.checkpoint(NOW - 21 * DAY);
    let claim = registry.owner_event("acme", "revoke acme.a since 0", 1, &[&r]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&r]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert!(history.voided.contains(&cancel), "{:?}", history.voided);
    assert_eq!(history.took_effect.get(&recovery), Some(&deadline));
    assert!(history.compromised.contains_key(&a.1.public));
    assert!(!history.sound(&a.1, cancel));
    assert!(!history.sound(&a.1, claim + 1));
    assert_eq!(history.snapshots.last().unwrap().keys, vec![r.1.clone()]);
    let releases = client.releases("acme/ledger").unwrap();
    assert_eq!(releases.len(), 1);
    assert!(!releases[0].compromised);
}

/// Each entry is checked as known at its own logging, not as the group of entries logged with
/// the same claims: the overruling of [`a_later_claim_never_refuses_an_earlier_stated_threshold`]
/// is checked at its logging though, after it and before the owner's claim, the registry logged
/// the thief's addition signed by `A`, no longer a key there, which the claim later voids. The
/// owner's chain stands.
#[test]
fn an_entry_refused_later_in_its_group_leaves_the_earlier_ones_checked_at_logging() {
    let a = keypair(60, "acme.a");
    let b = keypair(61, "acme.b");
    let r = keypair(63, "acme.r");
    let t = keypair(66, "acme.t");
    let x = keypair(70, "trustee.x");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b], 2, &[&x], 1);
    registry.checkpoint(NOW - 31 * DAY);
    let recovery = registry.recover("acme", "recover", &r.1, &[&x]);
    registry.checkpoint(NOW - 30 * DAY);
    let cancel = registry.owner_event("acme", &format!("cancel {recovery}"), 2, &[&a]);
    registry.checkpoint(NOW - 24 * DAY);
    registry.append("other", "filler");
    let deadline = registry.checkpoint(NOW - 22 * DAY);
    let overrule = registry.owner_event("acme", &format!("overrule {cancel}"), 2, &[&x]);
    assert_eq!(overrule, deadline);
    registry.checkpoint(NOW - 21 * DAY);
    let added = registry.owner_event("acme", &format!("add {}", key_text(&t.1)), 1, &[&a, &t]);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&r]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&r]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert!(history.voided.contains(&cancel) && history.voided.contains(&added));
    assert_eq!(history.took_effect.get(&recovery), Some(&deadline));
    assert_eq!(history.snapshots.last().unwrap().keys, vec![r.1.clone()]);
    let releases = client.releases("acme/ledger").unwrap();
    assert!(!releases[0].compromised);
}

/// The cap of claims (ADR 0122 Amendment 10): `A` and `D` registered, `D` revoked by the owner,
/// then the thief, holding `A`, logs `claims` claims on the former key `D` after a release of its
/// own. Returns the registry, the last honest index and the thief's release.
fn claims_spent(
    claims: usize,
    cosigners: &[&(ed25519_dalek::SigningKey, una_project::tlog::note::Key)],
) -> (TestRegistry, u64) {
    let a = keypair(60, "acme.a");
    let d = keypair(61, "acme.d");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &d], 1, cosigners, 1);
    let honest = registry.owner_event("acme", "revoke acme.d", 1, &[&a]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    for _ in 0..claims {
        registry.owner_event("acme", "revoke acme.d since 0", 1, &[&a]);
    }
    (registry, honest)
}

/// Past the cap of claims between two reset points, the owner's chain is not replayed: its
/// names are refused and no release counts, the thief's included, and it is not the registry's
/// misbehavior.
#[test]
fn past_the_cap_of_claims_no_release_of_the_owner_counts() {
    let (mut registry, _) = claims_spent(MAX_CLAIMS + 1, &[]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    assert!(client.history("acme").is_err());
    assert!(client.releases("acme/ledger").is_err());
    assert!(client.misbehavior().is_empty());
}

/// A dispute that takes effect restarts the count: the thief spends the cap of claims, the
/// owner's dispute names `R`, and `R`'s claim on `A`, the first after the reset, is replayed with
/// all of them. The owner's names resolve, and the thief's release is skipped.
#[test]
fn a_dispute_that_takes_effect_restarts_the_count_of_claims() {
    let r = keypair(63, "acme.r");
    let (mut registry, honest) = claims_spent(MAX_CLAIMS, &[]);
    registry.checkpoint(NOW - 30 * DAY);
    registry.recover("acme", "dispute", &r.1, &[]);
    registry.checkpoint(NOW - 29 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 20 * DAY);
    registry.owner_event("acme", &format!("revoke acme.a since {honest}"), 1, &[&r]);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&r]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    client.history("acme").unwrap();
    let releases = client.releases("acme/ledger").unwrap();
    assert_eq!(releases.len(), 2);
    assert!(releases[0].compromised && !releases[1].compromised);
    assert!(client.misbehavior().is_empty());
}

/// A recovery that a cancellation entry names does not restart the count, even once its
/// co-signers overruled the cancellation and it took effect: the owner's claim after it is past
/// the cap, and no release of the owner counts.
#[test]
fn a_cancelled_recovery_does_not_restart_the_count_of_claims() {
    let r = keypair(63, "acme.r");
    let x = keypair(70, "trustee.x");
    let a = keypair(60, "acme.a");
    let (mut registry, honest) = claims_spent(MAX_CLAIMS, &[&x]);
    registry.checkpoint(NOW - 30 * DAY);
    let recovery = registry.recover("acme", "recover", &r.1, &[&x]);
    registry.checkpoint(NOW - 29 * DAY);
    let cancel = registry.owner_event("acme", &format!("cancel {recovery}"), 1, &[&a]);
    registry.checkpoint(NOW - 28 * DAY);
    registry.owner_event("acme", &format!("overrule {cancel}"), 1, &[&x]);
    registry.checkpoint(NOW - 27 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 20 * DAY);
    registry.owner_event("acme", &format!("revoke acme.a since {honest}"), 1, &[&r]);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&r]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    assert!(client.history("acme").is_err());
    assert!(client.releases("acme/ledger").is_err());
}

/// Once the cosigners overruled a cancellation of a recovery, a later cancellation by an owner
/// key does not count, and the recovery takes effect at its seven days.
#[test]
fn a_cancellation_after_an_overruling_does_not_count() {
    let a = keypair(60, "acme.a");
    let x = keypair(70, "trustee.x");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a], 1, &[&x], 1);
    registry.checkpoint(NOW - 30 * DAY);
    let recovery = registry.recover("acme", "recover", &n.1, &[&x]);
    registry.checkpoint(NOW - 29 * DAY);
    let cancel = registry.owner_event("acme", &format!("cancel {recovery}"), 1, &[&a]);
    registry.owner_event("acme", &format!("overrule {cancel}"), 1, &[&x]);
    let again = registry.owner_event("acme", &format!("cancel {recovery}"), 1, &[&a]);
    // The cancellations and the overruling inside the seven days: their sizes are cosigned
    // well before the deadline.
    registry.checkpoint(NOW - 28 * DAY);
    registry.append("other", "filler");
    let effect = registry.checkpoint(NOW - 21 * DAY);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&n]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.took_effect.get(&recovery), Some(&effect));
    assert!(
        history
            .cancellations
            .get(&recovery)
            .is_some_and(|c| !c.contains(&again) && c.is_empty()),
        "{:?}",
        history.cancellations
    );
    assert!(!client.releases("acme/ledger").unwrap()[0].compromised);
}

/// The cosigners' threshold claims the owner's last key compromised, as a claim only: the key
/// stays, and the release it signed after the claimed position is skipped.
#[test]
fn the_cosigners_claim_the_last_key_and_void_the_thiefs_releases() {
    let a = keypair(60, "acme.a");
    let x = keypair(70, "trustee.x");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a], 1, &[&x], 1);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - DAY);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&x]);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert_eq!(history.compromised.get(&a.1.public), Some(&0));
    assert_eq!(history.snapshots.last().unwrap().keys, vec![a.1.clone()]);
    assert!(client.releases("acme/ledger").unwrap()[0].compromised);
}

/// A cancelled recovery ends when its guarding cosigners are replaced: an overruling by them
/// afterwards revives nothing.
#[test]
fn a_cancelled_recovery_ends_when_its_cosigners_are_replaced() {
    let a = keypair(60, "acme.a");
    let b = keypair(61, "acme.b");
    let x = keypair(70, "trustee.x");
    let y = keypair(71, "trustee.y");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a, &b], 1, &[&x], 1);
    registry.checkpoint(NOW - 30 * DAY);
    let recovery = registry.recover("acme", "recover", &n.1, &[&x]);
    let designation =
        registry.owner_event("acme", &format!("cosigners 1 {}", key_text(&y.1)), 1, &[&b]);
    registry.checkpoint(NOW - 29 * DAY);
    let cancel = registry.owner_event("acme", &format!("cancel {recovery}"), 1, &[&a]);
    registry.checkpoint(NOW - 23 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 21 * DAY);
    registry.owner_event("acme", &format!("overrule {cancel}"), 1, &[&x]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&b]);
    registry.checkpoint(NOW);
    let history = history_of(&registry).unwrap();
    assert!(history.took_effect.contains_key(&designation));
    assert!(!history.took_effect.contains_key(&recovery));
    assert_eq!(history.cosigners, Some((1, vec![y.1.clone()])));
    assert_eq!(
        history.snapshots.last().unwrap().keys,
        vec![a.1.clone(), b.1.clone()]
    );
}

/// An overruling counts only within seven days of witness time after the cancellation it
/// overrules: later, the cancelled recovery stays cancelled.
#[test]
fn an_overruling_counts_only_within_seven_days_of_its_cancellation() {
    let a = keypair(60, "acme.a");
    let x = keypair(70, "trustee.x");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::new("acme");
    registry.register_keys("acme", &[&a], 1, &[&x], 1);
    registry.checkpoint(NOW - 40 * DAY);
    let recovery = registry.recover("acme", "recover", &n.1, &[&x]);
    registry.checkpoint(NOW - 39 * DAY);
    let cancel = registry.owner_event("acme", &format!("cancel {recovery}"), 1, &[&a]);
    registry.checkpoint(NOW - 38 * DAY);
    registry.append("other", "filler");
    registry.checkpoint(NOW - 30 * DAY);
    registry.owner_event("acme", &format!("overrule {cancel}"), 1, &[&x]);
    registry.checkpoint(NOW);
    let history = history_of(&registry).unwrap();
    assert!(!history.took_effect.contains_key(&recovery));
    assert!(
        history
            .cancellations
            .get(&recovery)
            .is_some_and(|c| c.contains(&cancel))
    );
    assert_eq!(history.snapshots.last().unwrap().keys, vec![a.1.clone()]);
}

/// A kept checkpoint that the place omits is read from the next place; with no place serving
/// it, the result is unavailable, never an effect that did not happen (`spec/30 §14` rule 1).
#[test]
fn an_omitted_kept_checkpoint_is_unavailable_never_a_later_effect() {
    let a = keypair(60, "acme.a");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.checkpoint(NOW - 30 * DAY);
    let dispute = registry.recover("acme", "dispute", &n.1, &[]);
    registry.checkpoint(NOW - 29 * DAY);
    registry.append("other", "filler");
    let effect = registry.checkpoint(NOW - 20 * DAY);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&n]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW);
    // The full view: the dispute takes effect, the claim stands, the thief's release is
    // skipped.
    let full = history_of(&registry).unwrap();
    assert!(full.took_effect.contains_key(&dispute));
    // A place that omits the checkpoint of the effect, alone: unavailable.
    let mut omitting = Memory {
        files: registry.files.clone(),
    };
    omitting.files.remove(&format!("checkpoint/{effect}"));
    let mut client = open(&omitting, &registry).unwrap();
    assert!(client.history("acme").is_err());
    assert_eq!(
        client.take_omitted(),
        vec![Omission::Document(format!("checkpoint/{effect}"))]
    );
    // Behind it, a place that serves it: the full view.
    let full_place = Memory {
        files: registry.files.clone(),
    };
    let both =
        una_project::transport::Fallback::new(Box::new(omitting), vec![Box::new(full_place)]);
    let mut client = open(&both, &registry).unwrap();
    assert_eq!(client.history("acme").unwrap(), full);
    assert!(client.take_omitted().is_empty());
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

/// A place withholds the cosignatures of the checkpoint where a dispute takes effect, but one:
/// with an honest place behind it, the cosignatures are merged and the result is the full one;
/// alone, the owner's verdict depends on what it withholds, so the name is unavailable, an
/// omission of cosignatures, never refused.
#[test]
fn withheld_cosignatures_are_merged_or_unavailable_never_refused() {
    let a = keypair(60, "acme.a");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.checkpoint(NOW - 30 * DAY);
    let dispute = registry.recover("acme", "dispute", &n.1, &[]);
    registry.checkpoint(NOW - 29 * DAY);
    registry.append("other", "filler");
    let effect = registry.checkpoint(NOW - 20 * DAY);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&n]);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&n]);
    registry.checkpoint(NOW);
    let full = history_of(&registry).unwrap();
    assert_eq!(full.took_effect.get(&dispute), Some(&effect));
    let path = format!("checkpoint/{effect}");
    let mut stripping = Memory {
        files: registry.files.clone(),
    };
    let log = registry.keys.log.1.name.clone();
    let witness = registry.keys.witnesses[0].1.name.clone();
    let stripped = withhold(&registry.files[&path], &log, &witness);
    stripping.files.insert(path.clone(), stripped);
    // With an honest place behind it: merged, the full result.
    let honest = Memory {
        files: registry.files.clone(),
    };
    let stripping_files = stripping.files.clone();
    let both = una_project::transport::Fallback::new(Box::new(stripping), vec![Box::new(honest)]);
    let mut client = open(&both, &registry).unwrap();
    assert_eq!(client.history("acme").unwrap(), full);
    assert!(!client.releases("acme/ledger").unwrap()[0].compromised);
    assert!(client.misbehavior().is_empty());
    // Alone: unavailable, by an omission of cosignatures.
    let alone = Memory {
        files: stripping_files,
    };
    let mut client = open(&alone, &registry).unwrap();
    assert!(client.history("acme").is_err());
    assert_eq!(
        client.take_omitted(),
        vec![Omission::Cosignatures("acme".to_owned())]
    );
    // Its only place read, the request has no deadline checkpoint: evidence from an absence,
    // with the request's index, its target witness time and the place read, never the
    // registry's fault, since a place may withhold.
    let target = client.logged_at(dispute).unwrap() + owner::RECOVERY_DELAY;
    assert_eq!(
        client.misbehavior(),
        &[Misbehavior::MissingDeadline {
            owner: "acme".to_owned(),
            index: dispute,
            target,
            places: vec!["memory".to_owned()],
            places_agreeing: 1,
        }]
    );
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

/// A registry that keeps the rules and no more: witnesses cosign each request's start
/// checkpoint, its deadline checkpoint and the one before it, and one checkpoint per 64 sizes,
/// at times of their own. An honest owner stays available, both timings agree, and nothing is
/// recorded.
#[test]
fn a_sparsely_cosigned_honest_registry_keeps_its_owner_available() {
    let a = keypair(60, "acme.a");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::new("acme");
    registry.sparse = true;
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 40 * DAY);
    let dispute = registry.recover("acme", "dispute", &n.1, &[]);
    for at in 0..150 {
        registry.append("other", &format!("filler {at}"));
    }
    registry.checkpoint(NOW - 20 * DAY);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&n]);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&n]);
    for at in 0..70 {
        registry.append("other", &format!("later filler {at}"));
    }
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    let history = client.history("acme").unwrap();
    assert!(history.took_effect.contains_key(&dispute));
    assert_eq!(
        client.earliest_effect(dispute).unwrap(),
        client.recovery_effect(dispute).unwrap()
    );
    let releases = client.releases("acme/ledger").unwrap();
    assert!(releases[0].compromised && !releases[1].compromised);
    assert!(client.take_omitted().is_empty());
    assert!(client.misbehavior().is_empty());
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

/// With three witnesses all needed, honest cosignatures straddling a dispute's seven-day
/// target: the earliest and the served effect read a checkpoint's witness time by the same
/// order statistic, so they agree, the owner stays available, and nothing is recorded.
#[test]
fn straddling_cosignatures_with_every_witness_needed_keep_the_owner_available() {
    let a = keypair(60, "acme.a");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::with_witness_threshold("acme", 3);
    registry.sparse = true;
    registry.spread = 2 * 3600;
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 40 * DAY);
    let dispute = registry.recover("acme", "dispute", &n.1, &[]);
    for at in 0..150 {
        registry.append("other", &format!("filler {at}"));
    }
    registry.checkpoint(NOW - 20 * DAY);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&n]);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&n]);
    for at in 0..70 {
        registry.append("other", &format!("later filler {at}"));
    }
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    assert!(
        client
            .history("acme")
            .unwrap()
            .took_effect
            .contains_key(&dispute)
    );
    assert_eq!(
        client.earliest_effect(dispute).unwrap(),
        client.recovery_effect(dispute).unwrap()
    );
    assert!(!client.releases("acme/ledger").unwrap()[1].compromised);
    assert!(client.take_omitted().is_empty());
    assert!(client.misbehavior().is_empty());
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

/// Sparse cosignatures spread three days apart with one witness needed: the checkpoint before
/// a dispute's deadline is cosigned by a witness later than another witness cosigned the
/// deadline checkpoint. The earliest and the served effect take the same scan of the running
/// maximum of witness time, so they agree and the owner stays available.
#[test]
fn a_pre_deadline_checkpoint_cosigned_late_keeps_the_owner_available() {
    let a = keypair(60, "acme.a");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::with_witness_threshold("acme", 1);
    registry.sparse = true;
    registry.spread = 3 * DAY;
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 40 * DAY);
    let dispute = registry.recover("acme", "dispute", &n.1, &[]);
    for at in 0..150 {
        registry.append("other", &format!("filler {at}"));
    }
    registry.checkpoint(NOW - 20 * DAY);
    registry.owner_event("acme", "revoke acme.a since 0", 1, &[&n]);
    registry.release("acme/ledger", "0.3.2", "none", "none", &[&n]);
    for at in 0..70 {
        registry.append("other", &format!("later filler {at}"));
    }
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    assert!(
        client
            .history("acme")
            .unwrap()
            .took_effect
            .contains_key(&dispute)
    );
    assert_eq!(
        client.earliest_effect(dispute).unwrap(),
        client.recovery_effect(dispute).unwrap()
    );
    assert!(client.take_omitted().is_empty());
    assert!(client.misbehavior().is_empty());
}

/// A transport over files in memory that counts the kept checkpoints read.
struct Counting {
    memory: Memory,
    reads: std::cell::Cell<usize>,
}

impl Transport for Counting {
    fn get(&self, path: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
        if path.starts_with("checkpoint/") {
            self.reads.set(self.reads.get() + 1);
        }
        self.memory.get(path, limit)
    }

    fn describe(&self) -> String {
        self.memory.describe()
    }
}

/// A sparse log of a few thousand sizes over forty days, with a dispute near its start: the
/// registry and the dispute's index.
fn long_log() -> (TestRegistry, u64) {
    let a = keypair(60, "acme.a");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::new("acme");
    registry.sparse = true;
    registry.register("acme", &a);
    registry.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    registry.checkpoint(NOW - 40 * DAY);
    let dispute = registry.recover("acme", "dispute", &n.1, &[]);
    for at in 0..3000 {
        registry.append("other", &format!("filler {at}"));
    }
    registry.checkpoint(NOW);
    (registry, dispute)
}

/// The deadline of a request W sizes into a long log is found by reading the pinned multiples
/// of 64 and then the sizes up to it: O(W/64 + 64) kept checkpoints, never W, and the honest
/// sparse rendering stays available.
#[test]
fn a_deadline_reads_pinned_checkpoints_and_one_bracket() {
    let (registry, dispute) = long_log();
    let counting = Counting {
        memory: Memory {
            files: registry.files.clone(),
        },
        reads: std::cell::Cell::new(0),
    };
    let mut client = open(&counting, &registry).unwrap();
    counting.reads.set(0);
    let effect = client.recovery_effect(dispute).unwrap().unwrap();
    let reads = counting.reads.get() as u64;
    let w = effect - dispute;
    assert!(w > 256, "{w}");
    // The pinned ones, one bracket of at most 64, and the logged time's scan of at most 64.
    assert!(reads <= w / 64 + 2 * 64 + 2, "{reads} reads for {w} sizes");
    assert!(reads < w / 2, "{reads} reads for {w} sizes");
    assert_eq!(client.earliest_effect(dispute).unwrap(), Some(effect));
    assert!(
        client
            .history("acme")
            .unwrap()
            .took_effect
            .contains_key(&dispute)
    );
    assert!(client.take_omitted().is_empty());
    assert!(client.misbehavior().is_empty());
}

/// A deadline search that reaches the cap of sizes past the request without a deadline: an
/// omission of its own kind, unavailable (U7013), never a later or absent effect.
#[test]
fn a_deadline_past_the_scan_cap_is_unavailable() {
    let (registry, dispute) = long_log();
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    client.lower_scan_cap(128);
    assert!(client.recovery_effect(dispute).is_err());
    let omitted = client.take_omitted();
    assert!(
        omitted
            .iter()
            .any(|omission| matches!(omission, Omission::ScanCap { .. })),
        "{omitted:?}"
    );
    assert!(client.misbehavior().is_empty());
}

/// The dip: a pinned checkpoint whose witness time reaches a dispute's target, then a later one,
/// cosigned by another witness, whose witness time is below it. The first crossing is found,
/// by both timings alike, reading the pinned checkpoints up to it and one bracket.
#[test]
fn a_dip_after_the_first_crossing_keeps_the_first() {
    let a = keypair(60, "acme.a");
    let n = keypair(63, "acme.n");
    let t0 = NOW - 40 * DAY;
    let mut registry = TestRegistry::with_witness_threshold("acme", 1);
    registry.register("acme", &a);
    registry.checkpoint(t0);
    let dispute = registry.recover("acme", "dispute", &n.1, &[]);
    let fill = |registry: &mut TestRegistry, to: u64| {
        while (registry.entries.len() as u64) < to {
            let at = registry.entries.len();
            registry.append("other", &format!("filler {at}"));
        }
    };
    fill(&mut registry, 128);
    registry.checkpoint(t0 + DAY);
    fill(&mut registry, 192);
    registry.checkpoint_with(&[Some(t0 + 9 * DAY), None, None]);
    fill(&mut registry, 256);
    registry.checkpoint_with(&[None, Some(t0 + 2 * DAY), None]);
    fill(&mut registry, 300);
    registry.checkpoint(NOW);
    let counting = Counting {
        memory: Memory {
            files: registry.files.clone(),
        },
        reads: std::cell::Cell::new(0),
    };
    let mut client = open(&counting, &registry).unwrap();
    counting.reads.set(0);
    assert_eq!(client.recovery_effect(dispute).unwrap(), Some(129));
    assert!(counting.reads.get() <= 8, "{} reads", counting.reads.get());
    assert_eq!(client.earliest_effect(dispute).unwrap(), Some(129));
    assert!(client.take_omitted().is_empty());
    assert!(client.misbehavior().is_empty());
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

/// An honest registry where a checkpoint between two multiples of 64, cosigned late by one
/// witness, reaches a dispute's target, and the next multiple, cosigned by another, dips below
/// it: the deadline is found from the first multiple that reaches the target, as the registry
/// finds it, so both timings agree and the owner stays available.
#[test]
fn a_late_cosigned_checkpoint_between_multiples_is_not_the_deadline() {
    let a = keypair(60, "acme.a");
    let n = keypair(63, "acme.n");
    let t0 = NOW - 40 * DAY;
    let mut registry = TestRegistry::with_witness_threshold("acme", 1);
    registry.register("acme", &a);
    registry.checkpoint(t0);
    let dispute = registry.recover("acme", "dispute", &n.1, &[]);
    let fill = |registry: &mut TestRegistry, to: u64| {
        while (registry.entries.len() as u64) < to {
            let at = registry.entries.len();
            registry.append("other", &format!("filler {at}"));
        }
    };
    fill(&mut registry, 128);
    registry.checkpoint(t0 + DAY);
    // N: sizes 129 to 150, one witness late, past the target of t0 + 8 days.
    fill(&mut registry, 150);
    registry.checkpoint_with(&[Some(t0 + 9 * DAY), None, None]);
    // The multiple 192, another witness, below it.
    fill(&mut registry, 192);
    registry.checkpoint_with(&[None, Some(t0 + 2 * DAY), None]);
    // The multiple 256, every witness, past it.
    fill(&mut registry, 256);
    registry.checkpoint(t0 + 10 * DAY);
    fill(&mut registry, 300);
    registry.checkpoint(NOW);
    let memory = Memory {
        files: registry.files.clone(),
    };
    let mut client = open(&memory, &registry).unwrap();
    assert_eq!(client.recovery_effect(dispute).unwrap(), Some(193));
    assert_eq!(client.earliest_effect(dispute).unwrap(), Some(193));
    let history = client.history("acme").unwrap();
    assert_eq!(history.took_effect.get(&dispute), Some(&193));
    assert!(client.take_omitted().is_empty());
    assert!(client.misbehavior().is_empty());
}

/// The deadline checkpoint signed at or after the target, every witness cosigning it then: a
/// place that strips one cosignature leaves k, so the effect does not move; one that strips
/// all but one leaves fewer than k, so the two timings differ and the name is unavailable,
/// never a later effect taken unseen.
#[test]
fn stripping_the_deadline_cannot_move_the_effect_unseen() {
    let a = keypair(60, "acme.a");
    let n = keypair(63, "acme.n");
    let mut registry = TestRegistry::new("acme");
    registry.register("acme", &a);
    registry.checkpoint(NOW - 30 * DAY);
    let dispute = registry.recover("acme", "dispute", &n.1, &[]);
    registry.checkpoint(NOW - 29 * DAY);
    registry.append("other", "filler");
    let effect = registry.checkpoint(NOW - 20 * DAY);
    registry.append("other", "another filler");
    registry.checkpoint(NOW);
    let full = history_of(&registry).unwrap();
    assert_eq!(full.took_effect.get(&dispute), Some(&effect));
    let path = format!("checkpoint/{effect}");
    let log = registry.keys.log.1.name.clone();
    let last = registry.keys.witnesses[2].1.name.clone();
    // One line stripped: k remain, all at or after the target.
    let document = String::from_utf8(registry.files[&path].clone()).unwrap();
    let stripped: Vec<&str> = document
        .lines()
        .filter(|line| !line.starts_with(&format!("\u{2014} {last} ")))
        .collect();
    let mut one = Memory {
        files: registry.files.clone(),
    };
    let stripped = format!("{}\n", stripped.join("\n")).into_bytes();
    one.files.insert(path.clone(), stripped);
    let mut client = open(&one, &registry).unwrap();
    assert_eq!(client.history("acme").unwrap(), full);
    assert!(client.take_omitted().is_empty());
    // All but one stripped: fewer than k, the timings differ, the name is unavailable.
    let kept = registry.keys.witnesses[0].1.name.clone();
    let mut most = Memory {
        files: registry.files.clone(),
    };
    let withheld = withhold(&registry.files[&path], &log, &kept);
    most.files.insert(path, withheld);
    let mut client = open(&most, &registry).unwrap();
    assert!(client.history("acme").is_err());
    assert!(!client.take_omitted().is_empty());
    assert_ne!(
        client.earliest_effect(dispute).unwrap(),
        client.recovery_effect(dispute).unwrap()
    );
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
