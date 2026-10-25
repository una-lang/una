//! Registry packages on disk (ADR 0122 rules 17, 30 and 32): provenance verified against the
//! lock, Itaoca's stored provenance, and vendoring with its re-verification on every load.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::path::PathBuf;

use ed25519_dalek::{Signer, SigningKey};
use pretty_assertions::assert_eq;
use una_project::content::{File, archive, digest_of};
use una_project::itaoca::Itaoca;
use una_project::provenance::{Expected, Provenance, parse, sha256_digest, signed_name, verify};
use una_project::tlog::merkle::{inclusion_proof, leaf_hash, root as tree_root};
use una_project::tlog::note::{Key, encode_base64};
use una_project::vendor::{self, Place, Problem};

const SOURCE: &str = "acme";
const NAME: &str = "acme/ledger";
const VERSION: &str = "0.3.1";
const LOG: &str = "acme.example/log";
const WITNESS_TIME: u64 = 1_900_000_000;
const EXPIRES: u64 = 4_000_000_000;

fn keypair(seed: u8, name: &str) -> (SigningKey, Key) {
    let pair = SigningKey::from_bytes(&[seed; 32]);
    let public: [u8; 32] = pair.verifying_key().to_bytes();
    (
        pair,
        Key {
            name: name.to_owned(),
            public,
        },
    )
}

fn sign_line(pair: &SigningKey, key: &Key, text: &str) -> String {
    let mut bytes = key.id().to_vec();
    bytes.extend_from_slice(&pair.sign(text.as_bytes()).to_bytes());
    format!("\u{2014} {} {}\n", key.name, encode_base64(&bytes))
}

fn cosign_line(pair: &SigningKey, key: &Key, text: &str, time: u64) -> String {
    let message = format!("cosignature/v1\ntime {time}\n{text}");
    let mut bytes = key.cosignature_id().to_vec();
    bytes.extend_from_slice(&time.to_be_bytes());
    bytes.extend_from_slice(&pair.sign(message.as_bytes()).to_bytes());
    format!("\u{2014} {} {}\n", key.name, encode_base64(&bytes))
}

fn key_line(key: &Key) -> String {
    format!("key {} {}\n", key.name, encode_base64(&key.public))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn files() -> Vec<File> {
    vec![
        File {
            path: "ita.una".to_owned(),
            bytes: b"ita acme/ledger\n  version 0.3.1\n  edition 0.4\n".to_vec(),
        },
        File {
            path: "src/ledger.una".to_owned(),
            bytes: b"module ledger\n\npub total() -> Int = 3\n".to_vec(),
        },
    ]
}

/// A release of a test registry: its archive, its provenance, and what a lock says of it.
struct Release {
    archive: Vec<u8>,
    provenance: Provenance,
    expected: Expected,
}

/// The release of [`files`] at `acme`, whose root expires at `expires`, cosigned by the witnesses
/// of `witnesses` (seeds 30 and 31, threshold 2).
fn make_release(expires: u64, witnesses: &[u8]) -> Release {
    let mut files = files();
    let archive = archive(&mut files).unwrap();
    let digest = digest_of(&mut files).unwrap();
    let (r1, k1) = keypair(10, "root.a");
    let (r2, k2) = keypair(11, "root.b");
    let (log_pair, log) = keypair(20, LOG);
    let (_, wa) = keypair(30, "witness.a");
    let (_, wb) = keypair(31, "witness.b");
    let (_, security) = keypair(40, "security.a");
    let (_, itauna) = keypair(41, "itauna.a");
    let mut text = format!("ita-root/1\nsource {SOURCE}\nversion 1\nexpires {expires}\nroot 2\n");
    text.push_str(&key_line(&k1));
    text.push_str(&key_line(&k2));
    text.push_str(&format!(
        "log {} {}\n",
        log.name,
        encode_base64(&log.public)
    ));
    text.push_str("witnesses 2\n");
    text.push_str(&key_line(&wa));
    text.push_str(&key_line(&wb));
    text.push_str("security 1\n");
    text.push_str(&key_line(&security));
    text.push_str("itauna 1\n");
    text.push_str(&key_line(&itauna));
    let root_document = format!(
        "{text}\n{}{}",
        sign_line(&r1, &k1, &text),
        sign_line(&r2, &k2, &text)
    );
    let manifest = sha256_digest(&files.iter().find(|f| f.path == "ita.una").unwrap().bytes);
    let statement_text = format!(
        "ita-release/1\nsource {SOURCE}\nname {NAME}\nversion {VERSION}\nprevious none\ndigest {digest}\nmanifest {manifest}\nedition 0.4\nauthority none\ntrust none\n"
    );
    let (publisher_pair, publisher) = keypair(50, "acme.publisher");
    let statement = format!(
        "{statement_text}\n{}",
        sign_line(&publisher_pair, &publisher, &statement_text)
    );
    let leaves = [
        leaf_hash(b"an earlier entry"),
        leaf_hash(statement.as_bytes()),
    ];
    let top = tree_root(&leaves);
    let checkpoint_text = format!("{LOG}\n2\n{}\n", encode_base64(&top));
    let mut checkpoint = format!(
        "{checkpoint_text}\n{}",
        sign_line(&log_pair, &log, &checkpoint_text)
    );
    for (offset, seed) in witnesses.iter().enumerate() {
        let (pair, key) = keypair(
            *seed,
            if *seed == 30 {
                "witness.a"
            } else {
                "witness.b"
            },
        );
        checkpoint.push_str(&cosign_line(
            &pair,
            &key,
            &checkpoint_text,
            WITNESS_TIME + offset as u64,
        ));
    }
    let root_digest = una_project::tlog::root::digest(&root_document);
    Release {
        archive,
        provenance: Provenance {
            roots: vec![root_document],
            checkpoint,
            statement,
            index: 1,
            proof: inclusion_proof(1, &leaves),
            key: publisher.clone(),
        },
        expected: Expected {
            source: SOURCE.to_owned(),
            name: NAME.to_owned(),
            version: VERSION.to_owned(),
            digest,
            signed: signed_name(NAME, &publisher.public),
            logged: 1,
            root: root_digest,
            checkpoint_size: 2,
            checkpoint_hash: format!("sha256:{}", hex(&top)),
        },
    }
}

fn good() -> Release {
    make_release(EXPIRES, &[30, 31])
}

/// A fresh directory named `name` under the tests' temporary directory.
fn fresh(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("vendor")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_release_verifies_from_its_provenance() {
    let release = good();
    let encoded = release.provenance.encode();
    let read = parse(&encoded).unwrap();
    assert_eq!(read, release.provenance);
    let statement = verify(&read, &release.expected).unwrap();
    assert_eq!(statement.name, NAME);
    assert!(release.expected.signed.starts_with("ed25519:acme:"));
    assert_eq!(
        signed_name("json", &[1; 32]).split(':').nth(1),
        Some("itauna")
    );
}

/// A change of what the lock says, by the name of what it changes.
type Change = (&'static str, Box<dyn Fn(&mut Expected)>);

#[test]
fn each_thing_the_lock_says_is_verified() {
    let release = good();
    let changes: Vec<Change> = vec![
        (
            "root",
            Box::new(|e: &mut Expected| e.root = format!("sha256:{}", "0".repeat(64))),
        ),
        (
            "source",
            Box::new(|e: &mut Expected| e.source = "other".to_owned()),
        ),
        (
            "name",
            Box::new(|e: &mut Expected| e.name = "acme/other".to_owned()),
        ),
        (
            "version",
            Box::new(|e: &mut Expected| e.version = "0.3.2".to_owned()),
        ),
        (
            "digest",
            Box::new(|e: &mut Expected| e.digest = format!("sha256:{}", "1".repeat(64))),
        ),
        (
            "key",
            Box::new(|e: &mut Expected| e.signed = signed_name(NAME, &[7; 32])),
        ),
        ("index", Box::new(|e: &mut Expected| e.logged = 0)),
        ("size", Box::new(|e: &mut Expected| e.checkpoint_size = 3)),
        (
            "hash",
            Box::new(|e: &mut Expected| e.checkpoint_hash = format!("sha256:{}", "2".repeat(64))),
        ),
    ];
    for (what, change) in changes {
        let mut expected = release.expected.clone();
        change(&mut expected);
        assert!(verify(&release.provenance, &expected).is_err(), "{what}");
    }
}

#[test]
fn each_part_of_the_provenance_is_verified() {
    let release = good();
    let mut changes: Vec<(&str, Provenance)> = Vec::new();
    let mut proof = release.provenance.clone();
    proof.proof[0][0] ^= 1;
    changes.push(("a proof hash", proof));
    let mut index = release.provenance.clone();
    index.index = 0;
    changes.push(("the index", index));
    let mut other_key = release.provenance.clone();
    other_key.key = keypair(51, "acme.publisher").1;
    changes.push(("another key", other_key));
    let mut statement = release.provenance.clone();
    statement.statement = statement
        .statement
        .replacen("trust none", "trust modules x", 1);
    changes.push(("an edited statement", statement));
    let mut checkpoint = release.provenance.clone();
    checkpoint.checkpoint = checkpoint.checkpoint.replacen("\n2\n", "\n3\n", 1);
    changes.push(("an edited checkpoint", checkpoint));
    let mut unpinned = release.provenance.clone();
    unpinned.roots[0] = unpinned.roots[0].replacen("version 1", "version 2", 1);
    changes.push(("an edited root", unpinned));
    changes.push(("one witness", make_release(EXPIRES, &[30]).provenance));
    for (what, provenance) in changes {
        assert!(verify(&provenance, &release.expected).is_err(), "{what}");
    }
    // A root that expired before the checkpoint's witness time, against its own pin.
    let expired = make_release(WITNESS_TIME - 1, &[30, 31]);
    assert!(
        verify(&expired.provenance, &expired.expected)
            .unwrap_err()
            .contains("expired")
    );
    let witness = make_release(EXPIRES, &[30]);
    assert!(
        verify(&witness.provenance, &witness.expected)
            .unwrap_err()
            .contains("fewer than its threshold")
    );
}

/// The reader of provenance documents on mutated inputs: deterministic, it never panics, and
/// what it accepts is in its one encoding.
#[test]
fn the_provenance_reader_is_robust_to_mutated_documents() {
    let good = good().provenance.encode();
    assert!(parse(&[good.clone(), b"x".to_vec()].concat()).is_err());
    assert!(parse(&good[..good.len() - 1]).is_err());
    assert!(parse(b"").is_err());
    assert!(parse(&vec![b'a'; (1 << 20) + 1]).is_err());
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..4_000 {
        let mut bytes = good.clone();
        match next() % 4 {
            0 => {
                let at = (next() as usize) % bytes.len();
                bytes[at] ^= 1 << (next() % 8);
            }
            1 => {
                let at = (next() as usize) % bytes.len();
                bytes.truncate(at);
            }
            2 => {
                let at = (next() as usize) % bytes.len();
                bytes.insert(at, (next() % 256) as u8);
            }
            _ => {
                let at = (next() as usize) % bytes.len();
                bytes.remove(at);
            }
        }
        if let Ok(read) = parse(&bytes) {
            assert_eq!(read.encode(), bytes);
        }
    }
}

#[test]
fn itaoca_stores_a_provenance_once() {
    let release = good();
    let itaoca = Itaoca {
        root: fresh("store-provenance"),
    };
    let bytes = release.provenance.encode();
    assert_eq!(
        itaoca.load_provenance(SOURCE, NAME, VERSION, 2).unwrap(),
        None
    );
    itaoca
        .store_provenance(SOURCE, NAME, VERSION, 2, &bytes)
        .unwrap();
    assert_eq!(
        itaoca.load_provenance(SOURCE, NAME, VERSION, 2).unwrap(),
        Some(bytes.clone())
    );
    // Again, the same: fine. Another document for the same point: refused.
    itaoca
        .store_provenance(SOURCE, NAME, VERSION, 2, &bytes)
        .unwrap();
    let other = make_release(EXPIRES, &[30]).provenance.encode();
    assert!(
        itaoca
            .store_provenance(SOURCE, NAME, VERSION, 2, &other)
            .is_err()
    );
    // A document that does not read is never stored, nor a name that is not a release's.
    assert!(
        itaoca
            .store_provenance(SOURCE, NAME, VERSION, 3, b"nothing")
            .is_err()
    );
    assert!(
        itaoca
            .store_provenance(SOURCE, "../x", VERSION, 2, &bytes)
            .is_err()
    );
    assert!(
        itaoca
            .store_provenance("../x", NAME, VERSION, 2, &bytes)
            .is_err()
    );
}

fn stored(name: &str) -> (Release, Itaoca) {
    let release = good();
    let itaoca = Itaoca {
        root: fresh(&format!("{name}-itaoca")),
    };
    assert_eq!(
        itaoca.store(&release.archive).unwrap(),
        release.expected.digest
    );
    itaoca
        .store_provenance(SOURCE, NAME, VERSION, 2, &release.provenance.encode())
        .unwrap();
    (release, itaoca)
}

#[test]
fn a_release_loads_from_itaoca_verified() {
    let (release, itaoca) = stored("from-itaoca");
    let root = fresh("from-itaoca-root");
    let loaded = vendor::load(&root, Some(&itaoca), &release.expected).unwrap();
    assert_eq!(loaded.place, Place::Itaoca);
    assert_eq!(loaded.tree.files().len(), 2);
    // Nothing anywhere: absent.
    let empty = Itaoca {
        root: fresh("from-itaoca-empty"),
    };
    assert!(matches!(
        vendor::load(&root, Some(&empty), &release.expected),
        Err(Problem::Absent(_))
    ));
    assert!(matches!(
        vendor::load(&root, None, &release.expected),
        Err(Problem::Absent(_))
    ));
}

/// Itaoca holding a release's files without its provenance is an inconsistent store, refused
/// (U7010), not a package in neither place (U7011).
#[test]
fn itaoca_files_without_provenance_are_refused() {
    let release = good();
    let itaoca = Itaoca {
        root: fresh("files-only-itaoca"),
    };
    itaoca.store(&release.archive).unwrap();
    let root = fresh("files-only-root");
    let refusal = vendor::load(&root, Some(&itaoca), &release.expected).unwrap_err();
    assert!(
        matches!(&refusal, Problem::Refused(message) if message.contains("inconsistent")),
        "{refusal:?}"
    );
}

/// A stray file in a vendored copy, such as the `.DS_Store` a file browser leaves, refuses the
/// load (U7010), and the refusal names it.
#[test]
fn a_stray_file_in_a_vendored_copy_is_named() {
    let (release, itaoca) = stored("vendor-stray");
    let root = fresh("vendor-stray-root");
    vendor::vendor(&root, Some(&itaoca), &release.expected).unwrap();
    let dir = root.join("vendor/ita/acme/acme+ledger-0.3.1");
    std::fs::write(dir.join("src/.DS_Store"), b"\0\0\0\x01Bud1").unwrap();
    let refusal = vendor::load(&root, None, &release.expected).unwrap_err();
    assert!(
        matches!(&refusal, Problem::Refused(message) if message.contains("`src/.DS_Store`")),
        "{refusal:?}"
    );
}

/// A store of a provenance interrupted after its link, which left a second link in a fresh
/// directory beside it, does not refuse the provenance forever: the stray link is removed when
/// it is next loaded.
#[cfg(unix)]
#[test]
fn an_interrupted_provenance_store_is_repaired_on_load() {
    let release = good();
    let itaoca = Itaoca {
        root: fresh("provenance-stray"),
    };
    let bytes = release.provenance.encode();
    itaoca
        .store_provenance(SOURCE, NAME, VERSION, 2, &bytes)
        .unwrap();
    let release_dir = itaoca
        .root
        .join("registry/acme/releases/acme+ledger-0.3.1");
    let stray = release_dir.join(".una-fresh-9-9-9");
    std::fs::create_dir(&stray).unwrap();
    std::fs::hard_link(
        release_dir.join("checkpoint-2.provenance"),
        stray.join("file"),
    )
    .unwrap();
    assert_eq!(
        itaoca.load_provenance(SOURCE, NAME, VERSION, 2).unwrap(),
        Some(bytes)
    );
    assert!(!stray.exists());
}

/// A link planted at the name of a provenance to store is never written through: the store is
/// refused and the link's target keeps its bytes.
#[cfg(unix)]
#[test]
fn a_provenance_is_never_stored_through_a_link() {
    let release = good();
    let itaoca = Itaoca {
        root: fresh("provenance-link"),
    };
    let release_dir = itaoca
        .root
        .join("registry/acme/releases/acme+ledger-0.3.1");
    std::fs::create_dir_all(&release_dir).unwrap();
    let target = fresh("provenance-link-target").join("kept");
    std::fs::write(&target, b"kept").unwrap();
    std::os::unix::fs::symlink(&target, release_dir.join("checkpoint-2.provenance")).unwrap();
    assert!(
        itaoca
            .store_provenance(SOURCE, NAME, VERSION, 2, &release.provenance.encode())
            .is_err()
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"kept");
}

#[test]
fn vendoring_copies_the_release_and_every_load_verifies_it() {
    let (release, itaoca) = stored("vendoring");
    let root = fresh("vendoring-root");
    let written = vendor::vendor(&root, Some(&itaoca), &release.expected).unwrap();
    assert_eq!(
        written,
        [
            "vendor/ita/acme/acme+ledger-0.3.1",
            "vendor/ita/acme/acme+ledger-0.3.1.provenance"
        ]
    );
    // It loads from the vendor directory, with no Itaoca at all.
    let loaded = vendor::load(&root, None, &release.expected).unwrap();
    assert_eq!(loaded.place, Place::Vendor);
    // Again: nothing to write.
    assert_eq!(
        vendor::vendor(&root, Some(&itaoca), &release.expected).unwrap(),
        Vec::<String>::new()
    );
    let dir = root.join("vendor/ita/acme/acme+ledger-0.3.1");
    // A changed byte of a vendored file: refused, Itaoca is not used in its place, and
    // vendoring again leaves the copy as it is.
    std::fs::write(
        dir.join("src/ledger.una"),
        "module ledger\n\npub total() -> Int = 4\n",
    )
    .unwrap();
    assert!(matches!(
        vendor::load(&root, Some(&itaoca), &release.expected),
        Err(Problem::Refused(_))
    ));
    assert!(matches!(
        vendor::vendor(&root, Some(&itaoca), &release.expected),
        Err(Problem::Refused(_))
    ));
    assert_eq!(
        std::fs::read_to_string(dir.join("src/ledger.una")).unwrap(),
        "module ledger\n\npub total() -> Int = 4\n"
    );
    std::fs::write(
        dir.join("src/ledger.una"),
        "module ledger\n\npub total() -> Int = 3\n",
    )
    .unwrap();
    assert!(vendor::load(&root, None, &release.expected).is_ok());
    // A file added to the copy: refused.
    std::fs::write(dir.join("src/extra.una"), "module extra\n").unwrap();
    assert!(vendor::load(&root, None, &release.expected).is_err());
    std::fs::remove_file(dir.join("src/extra.una")).unwrap();
    // The provenance edited: refused; vendoring again takes Itaoca's, since the files verify.
    let beside = root.join("vendor/ita/acme/acme+ledger-0.3.1.provenance");
    let mut bytes = std::fs::read(&beside).unwrap();
    let at = bytes.len() - 10;
    bytes[at] ^= 1;
    std::fs::write(&beside, &bytes).unwrap();
    assert!(matches!(
        vendor::load(&root, None, &release.expected),
        Err(Problem::Refused(_))
    ));
    assert_eq!(
        vendor::vendor(&root, Some(&itaoca), &release.expected).unwrap(),
        ["vendor/ita/acme/acme+ledger-0.3.1.provenance"]
    );
    assert!(vendor::load(&root, None, &release.expected).is_ok());
    // A provenance missing beside a vendored copy: refused, never taken from Itaoca on load.
    std::fs::remove_file(&beside).unwrap();
    assert!(matches!(
        vendor::load(&root, Some(&itaoca), &release.expected),
        Err(Problem::Refused(_))
    ));
}

#[cfg(unix)]
#[test]
fn vendoring_never_writes_through_a_link() {
    let (release, itaoca) = stored("vendor-link");
    let root = fresh("vendor-link-root");
    let away = fresh("vendor-link-away");
    std::os::unix::fs::symlink(&away, root.join("vendor")).unwrap();
    assert!(matches!(
        vendor::vendor(&root, Some(&itaoca), &release.expected),
        Err(Problem::Refused(_))
    ));
    assert_eq!(std::fs::read_dir(&away).unwrap().count(), 0);
    // Nor loads through one.
    assert!(matches!(
        vendor::load(&root, Some(&itaoca), &release.expected),
        Err(Problem::Refused(_))
    ));
    // A component further down, `vendor/ita/acme`, that is a link: refused the same way.
    let root = fresh("vendor-link-deeper");
    let away = fresh("vendor-link-deeper-away");
    std::fs::create_dir_all(root.join("vendor/ita")).unwrap();
    std::os::unix::fs::symlink(&away, root.join("vendor/ita/acme")).unwrap();
    assert!(matches!(
        vendor::vendor(&root, Some(&itaoca), &release.expected),
        Err(Problem::Refused(_))
    ));
    assert_eq!(std::fs::read_dir(&away).unwrap().count(), 0);
    // A vendored file replaced by a link to a file of the same bytes: refused, not followed.
    let root = fresh("vendor-file-link");
    vendor::vendor(&root, Some(&itaoca), &release.expected).unwrap();
    let file = root.join("vendor/ita/acme/acme+ledger-0.3.1/src/ledger.una");
    let copy = root.join("ledger.una");
    std::fs::copy(&file, &copy).unwrap();
    std::fs::remove_file(&file).unwrap();
    std::os::unix::fs::symlink(&copy, &file).unwrap();
    assert!(matches!(
        vendor::load(&root, Some(&itaoca), &release.expected),
        Err(Problem::Refused(_))
    ));
}

#[test]
fn release_directories_are_built_from_checked_names_only() {
    assert_eq!(
        vendor::release_dir("acme/ledger", "0.3.1").unwrap(),
        "acme+ledger-0.3.1"
    );
    assert_eq!(
        vendor::release_dir("json", "2.1.0-rc.1").unwrap(),
        "json-2.1.0-rc.1"
    );
    for (name, version) in [
        ("../x", "1.0.0"),
        ("a/b/c", "1.0.0"),
        ("Json", "1.0.0"),
        (".json", "1.0.0"),
        ("json", "1.0"),
        ("json", "1.0.0/.."),
        ("json", "1.0.0+build"),
    ] {
        assert!(
            vendor::release_dir(name, version).is_err(),
            "{name} {version}"
        );
    }
    assert!(vendor::check_source("acme").is_ok());
    for source in ["", "..", "Acme", "a/b", "1acme"] {
        assert!(vendor::check_source(source).is_err(), "{source}");
    }
}
