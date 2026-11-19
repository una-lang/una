//! End to end (ADR 0122 rules 34 to 46): the registry client's code publishes to a registry
//! served over HTTP in this process, on a file-backed store; witnesses replay the log and
//! cosign; and the client verifies what it reads with its own code: the root metadata, the
//! cosigned checkpoint, inclusion and consistency proofs from tiles, the name map's proof and
//! the name's chain, and the promises.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used
)]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use una_itaty::entry::owner_chain;
use una_itaty::http::{ServerConfig, Service, Work, serve, serve_with};
use una_itaty::namemap::prove_from_tiles;
use una_itaty::registry::{Clock, Config, Registry, map_root_of};
use una_itaty::sign::{Signer, note};
use una_itaty::store::{FileStore, Store};
use una_itaty::tiles::{TileReader, WIDTH, entry_bundle_path, read_bundle};
use una_itaty::tree::{consistency, inclusion};
use una_itaty::witness::Witness;
use una_itaty::{MAX_ENTRY, digest_string};
use una_project::content::{File, archive, digest_of};
use una_project::tlog::map::{Head, Link, verify as verify_map, walk};
use una_project::tlog::merkle::{leaf_hash, verify_consistency, verify_inclusion};
use una_project::tlog::note::{Key, encode_base64, parse_checkpoint, parse_note};
use una_project::tlog::publish::{Release, statement};
use una_project::tlog::release::parse_statement;
use una_project::tlog::root::{digest as root_digest, named, trust_first};
use una_project::tlog::witness::{Cosigned, checkpoint_time, logged_at};
use una_syntax::ita::parse_manifest;

const TOKEN: &str = "operator-token-for-tests";

struct TestClock(AtomicU64);

impl Clock for TestClock {
    fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn signer(name: &str, seed: u8) -> Signer {
    Signer::from_seed(name, &[seed; 32])
}

fn key_line(signer: &Signer) -> String {
    format!(
        "key {} {}",
        signer.key().name,
        encode_base64(&signer.key().public)
    )
}

struct Keys {
    root: Vec<Signer>,
    log: Signer,
    witnesses: Vec<Signer>,
    security: Vec<Signer>,
    itauna: Signer,
}

fn keys() -> Keys {
    Keys {
        root: (0..3)
            .map(|i| signer(&format!("root.{i}"), 10 + i))
            .collect(),
        log: signer("itaty.test/log", 20),
        witnesses: (0..3)
            .map(|i| signer(&format!("witness.{i}"), 30 + i))
            .collect(),
        security: (0..2)
            .map(|i| signer(&format!("security.{i}"), 40 + i))
            .collect(),
        itauna: signer("itauna.0", 50),
    }
}

fn root_document(keys: &Keys) -> String {
    let mut text =
        String::from("ita-root/1\nsource itaty\nversion 1\nexpires 4000000000\nroot 2\n");
    for k in &keys.root {
        text.push_str(&format!("{}\n", key_line(k)));
    }
    text.push_str(&format!(
        "log {} {}\n",
        keys.log.key().name,
        encode_base64(&keys.log.key().public)
    ));
    text.push_str("witnesses 2\n");
    for k in &keys.witnesses {
        text.push_str(&format!("{}\n", key_line(k)));
    }
    text.push_str("security 2\n");
    for k in &keys.security {
        text.push_str(&format!("{}\n", key_line(k)));
    }
    text.push_str(&format!("itauna 1\n{}\n", key_line(&keys.itauna)));
    let signers: Vec<&Signer> = keys.root.iter().collect();
    note(&text, &signers)
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("una-itaty-{name}-{}-{nanos}", std::process::id()))
}

/// One HTTP/1.1 request over a fresh connection: the status and the body.
fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect(addr).unwrap();
    let mut head = format!("{method} /{path} HTTP/1.1\r\nHost: itaty.test\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if method == "POST" {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let end = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let status_line = String::from_utf8_lossy(&response[..end]).to_string();
    let status: u16 = status_line.split(' ').nth(1).unwrap().parse().unwrap();
    (status, response[end + 4..].to_vec())
}

fn get(addr: SocketAddr, path: &str) -> Option<Vec<u8>> {
    match request(addr, "GET", path, &[], b"") {
        (200, body) => Some(body),
        (404, _) => None,
        (status, body) => panic!("GET {path}: {status} {}", String::from_utf8_lossy(&body)),
    }
}

fn post(addr: SocketAddr, path: &str, body: &[u8]) -> (u16, String) {
    let (status, body) = request(addr, "POST", path, &[], body);
    (status, String::from_utf8(body).unwrap())
}

/// The log's entries from `from` to `to`, read from the entry bundles.
fn entries(addr: SocketAddr, from: u64, to: u64) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut index = from;
    while index < to {
        let n = index / WIDTH;
        let width = (to - n * WIDTH).min(WIDTH);
        let bytes = get(addr, &entry_bundle_path(n, width))
            .or_else(|| get(addr, &entry_bundle_path(n, WIDTH)))
            .unwrap();
        let all = read_bundle(&bytes, width.max(1), MAX_ENTRY)
            .or_else(|_| read_bundle(&bytes, WIDTH, MAX_ENTRY))
            .unwrap();
        for (i, entry) in all.into_iter().enumerate() {
            let at = n * WIDTH + i as u64;
            if at >= index && at < to {
                out.push(entry);
            }
        }
        index = (n + 1) * WIDTH;
    }
    out
}

/// Every witness replays the log since its last cosignature and cosigns the newest checkpoint.
fn witness_round(addr: SocketAddr, witnesses: &mut [Witness], now: u64) {
    let next = String::from_utf8(get(addr, "witness/next").unwrap()).unwrap();
    let size = parse_checkpoint(&parse_note(&next).unwrap().text)
        .unwrap()
        .size;
    for witness in witnesses.iter_mut() {
        let new = entries(addr, witness.size(), size);
        let line = witness.cosign(&next, &new, now).unwrap();
        let (status, body) = post(addr, "cosign", format!("{size}\n{line}").as_bytes());
        assert_eq!(status, 200, "{body}");
    }
}

struct Package {
    archive: Vec<u8>,
    digest: String,
    manifest: una_syntax::ita::Manifest,
    manifest_digest: String,
}

fn package(version: &str, needs: &str, body: &str) -> Package {
    let ita = format!(
        "ita acme/ledger\n  version {version}\n  edition 0.4\n  about \"Ledgers\"\n  license \"Apache-2.0\"\n  publish to itaty\n{needs}"
    );
    let source = format!("module ledger.book\n\n{body}");
    let mut files = vec![
        File {
            path: "ita.una".to_owned(),
            bytes: ita.clone().into_bytes(),
        },
        File {
            path: "src/book.una".to_owned(),
            bytes: source.into_bytes(),
        },
    ];
    let digest = digest_of(&mut files).unwrap();
    let archive = archive(&mut files).unwrap();
    let (manifest, errors) = parse_manifest(&ita);
    assert!(errors.is_empty(), "{errors:?}");
    Package {
        archive,
        digest,
        manifest,
        manifest_digest: digest_string(&Sha256::digest(ita.as_bytes()).into()),
    }
}

fn authority(entries: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    entries
        .iter()
        .map(|(cap, ops)| {
            (
                (*cap).to_owned(),
                ops.iter().map(|o| (*o).to_owned()).collect(),
            )
        })
        .collect()
}

/// The client's side of a publication: the statement from the manifest, signed, framed with
/// the archive.
fn publication(
    package: &Package,
    previous: Option<(u64, &str)>,
    authority: &BTreeMap<String, BTreeSet<String>>,
    signers: &[&Signer],
) -> (String, Vec<u8>) {
    let trust = BTreeSet::new();
    let text = statement(
        &package.manifest,
        &Release {
            previous,
            digest: &package.digest,
            manifest_digest: &package.manifest_digest,
            authority,
            trust: &trust,
        },
    )
    .unwrap();
    let document = note(&text, signers);
    let mut body = format!("ita-publish/1\n{}\n", document.len()).into_bytes();
    body.extend_from_slice(document.as_bytes());
    body.extend_from_slice(&package.archive);
    (document, body)
}

/// The cosignatures of a checkpoint document by the root's witnesses.
fn cosigned(document: &str, witnesses: &[Key]) -> Vec<Cosigned> {
    let read = parse_note(document).unwrap();
    let size = parse_checkpoint(&read.text).unwrap().size;
    witnesses
        .iter()
        .filter_map(|w| {
            read.cosignature(w).map(|time| Cosigned {
                witness: w.name.clone(),
                size,
                time,
            })
        })
        .collect()
}

#[test]
fn a_client_publishes_and_verifies_every_proof() {
    let keys = keys();
    let root_doc = root_document(&keys);
    let pin = root_digest(&root_doc);
    let dir = temp_dir("e2e");
    let store: Arc<dyn Store> = Arc::new(FileStore::open(&dir).unwrap());
    store.put("root/1", root_doc.as_bytes()).unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(1_900_000_000)));
    let config = Config {
        pin: pin.clone(),
        log_seed: [20; 32],
    };
    let registry = Registry::open(Arc::clone(&store), &config, clock.clone()).unwrap();
    let service = Arc::new(
        Service::new(
            Arc::clone(&store),
            registry,
            ServerConfig {
                register_token: Some(TOKEN.to_owned()),
                ..ServerConfig::default()
            },
        )
        .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    {
        let service = Arc::clone(&service);
        std::thread::spawn(move || serve(&listener, &service));
    }

    // The client trusts the root by its pin, fetched like any file.
    let fetched_root = String::from_utf8(get(addr, "root/1").unwrap()).unwrap();
    let root = trust_first(&fetched_root, &pin).unwrap();
    let mut witnesses: Vec<Witness> = keys
        .witnesses
        .iter()
        .enumerate()
        .map(|(i, _)| {
            Witness::new(
                signer(&format!("witness.{i}"), 30 + i as u8),
                root.log.clone(),
            )
        })
        .collect();

    // An owner registers: only with the operator's token, every key signing.
    // At the key floor: three keys at a threshold of 2; `acme` and `acme_b` sign its entries.
    let acme = signer("acme.a", 60);
    let acme_b = signer("acme.b", 62);
    let acme_c = signer("acme.c", 63);
    let registration = format!(
        "ita-owner/1\nsource itaty\nowner acme\nprevious none\nthreshold 2\n{}\n{}\n{}\n",
        key_line(&acme),
        key_line(&acme_b),
        key_line(&acme_c)
    );
    let registration = note(&registration, &[&acme, &acme_b, &acme_c]);
    let (status, _) = request(addr, "POST", "register", &[], registration.as_bytes());
    assert_eq!(status, 401);
    let auth = format!("Bearer {TOKEN}");
    let (status, promise) = request(
        addr,
        "POST",
        "register",
        &[("Authorization", &auth)],
        registration.as_bytes(),
    );
    let promise = String::from_utf8(promise).unwrap();
    assert_eq!(status, 200, "{promise}");
    let promise = parse_note(&promise).unwrap();
    assert!(promise.signed_by(&root.log));
    assert!(promise.text.contains("index 0\n"));
    // A name one edit from it is held for review.
    let other = signer("acne.a", 61);
    let held = note(
        &format!(
            "ita-owner/1\nsource itaty\nowner acne\nprevious none\nthreshold 1\n{}\n",
            key_line(&other)
        ),
        &[&other],
    );
    let (status, _) = request(
        addr,
        "POST",
        "register",
        &[("Authorization", &auth)],
        held.as_bytes(),
    );
    assert_eq!(status, 409);

    // The first release: Console[print], from a public function's `uses`.
    let body_1 = "pub say(line: Text) -> Unit\n  uses @c: Console[print]\n=\n  @c.print(line)\n";
    let v1 = package("1.0.0", "", body_1);
    let console = authority(&[("Console", &["print"])]);
    // A statement that understates the authority is refused.
    let (_, understated) = publication(&v1, None, &authority(&[]), &[&acme, &acme_b]);
    let (status, why) = post(addr, "publish", &understated);
    assert_eq!(status, 400, "{why}");
    assert!(why.contains("authority"), "{why}");
    // A statement signed by a key that is not the owner's is refused.
    let (_, unsigned) = publication(&v1, None, &console, &[&other]);
    let (status, why) = post(addr, "publish", &unsigned);
    assert_eq!(status, 400, "{why}");
    let (document_1, body) = publication(&v1, None, &console, &[&acme, &acme_b]);
    let (status, why) = post(addr, "publish", &body);
    assert_eq!(status, 200, "{why}");
    // Immutable: the same version again is refused, whatever its `previous`.
    let (status, _) = post(addr, "publish", &body);
    assert_eq!(status, 400);

    clock.0.store(1_900_000_100, Ordering::SeqCst);
    witness_round(addr, &mut witnesses[..2], 1_900_000_100);

    // The client reads the cosigned checkpoint and checks its signatures and witness time.
    let checkpoint_doc = String::from_utf8(get(addr, "checkpoint").unwrap()).unwrap();
    let checkpoint_note = parse_note(&checkpoint_doc).unwrap();
    assert!(checkpoint_note.signed_by(&root.log));
    let cosigns = cosigned(&checkpoint_doc, &root.witnesses.keys);
    assert_eq!(
        checkpoint_time(&cosigns, root.witnesses.threshold),
        Ok(1_900_000_100)
    );
    let checkpoint = parse_checkpoint(&checkpoint_note.text).unwrap();
    assert_eq!(checkpoint.size, 2);
    // It names the root version it was signed under (spec/30 §11 rule 9).
    assert_eq!(named(&checkpoint.extensions), Ok((1, pin.clone())));

    // The release statement is the log's entry 1: its inclusion proof, from tiles.
    let logged = entries(addr, 0, checkpoint.size);
    assert_eq!(logged[1], document_1.as_bytes());
    let mut fetch = |path: &str| Ok(get(addr, path));
    let mut reader = TileReader::new(checkpoint.size, &mut fetch);
    let proof = inclusion(&mut reader, 1, checkpoint.size).unwrap();
    let leaf_1 = leaf_hash(&logged[1]);
    assert!(verify_inclusion(
        1,
        checkpoint.size,
        &leaf_1,
        &proof,
        &checkpoint.root
    ));
    assert!(!verify_inclusion(
        1,
        checkpoint.size,
        &leaf_hash(b"other"),
        &proof,
        &checkpoint.root
    ));

    // The name map: the head of acme/ledger and its proof, then its chain.
    let map_root = map_root_of(&checkpoint).unwrap();
    let mut fetch = |path: &str| Ok(get(addr, path));
    let (head, map_proof) = prove_from_tiles(&map_root, "acme/ledger", &mut fetch).unwrap();
    let head = head.unwrap();
    assert_eq!(
        head,
        Head {
            index: 1,
            leaf: leaf_1,
            count: 1
        }
    );
    assert!(verify_map(
        &map_root,
        "acme/ledger",
        Some(&head),
        &map_proof
    ));
    let mut fetch = |path: &str| Ok(get(addr, path));
    let (owner_head, owner_proof) =
        prove_from_tiles(&map_root, &owner_chain("acme"), &mut fetch).unwrap();
    assert!(verify_map(
        &map_root,
        &owner_chain("acme"),
        owner_head.as_ref(),
        &owner_proof
    ));
    let mut fetch = |path: &str| Ok(get(addr, path));
    let (absent, absent_proof) = prove_from_tiles(&map_root, "acme/nothing", &mut fetch).unwrap();
    assert_eq!(absent, None);
    assert!(verify_map(&map_root, "acme/nothing", None, &absent_proof));

    // The archive, by its digest, matches the statement.
    let statement_1 = parse_statement(&parse_note(&document_1).unwrap().text).unwrap();
    let blob = get(addr, &format!("blobs/sha256/{}", &statement_1.digest[7..])).unwrap();
    assert_eq!(blob, v1.archive);

    // Rule 26: a minor release that needs Http[get] is refused.
    let leaf_1_text = digest_string(&leaf_1);
    let body_wider = "pub say(line: Text) -> Unit\n  uses @c: Console[print]\n=\n  @c.print(line)\n\npub fetch(url: Text) -> Text\n  uses @h: Http[get]\n=\n  url\n";
    let wider = package("1.1.0", "", body_wider);
    let (_, body) = publication(
        &wider,
        Some((1, &leaf_1_text)),
        &authority(&[("Console", &["print"]), ("Http", &["get"])]),
        &[&acme, &acme_b],
    );
    let (status, why) = post(addr, "publish", &body);
    assert_eq!(status, 400, "{why}");
    assert!(why.contains("next major version"), "{why}");
    // A `previous` that is not the name's head is refused.
    let v11 = package("1.1.0", "", body_1);
    let (_, stale) = publication(&v11, None, &console, &[&acme, &acme_b]);
    let (status, why) = post(addr, "publish", &stale);
    assert_eq!(status, 400, "{why}");
    // As the next major version, the wider release is logged.
    let major = package("2.0.0", "", body_wider);
    let (document_2, body) = publication(
        &major,
        Some((1, &leaf_1_text)),
        &authority(&[("Console", &["print"]), ("Http", &["get"])]),
        &[&acme, &acme_b],
    );
    let (status, why) = post(addr, "publish", &body);
    assert_eq!(status, 200, "{why}");
    let leaf_2 = leaf_hash(document_2.as_bytes());

    // A yank by the owner, and an advisory by a security key; a yank by another is refused.
    let yank = format!(
        "ita-yank/1\nsource itaty\nname acme/ledger\nversion 1.0.0\nprevious 2 {}\n",
        digest_string(&leaf_2)
    );
    let (status, _) = post(addr, "submit", note(&yank, &[&other]).as_bytes());
    assert_eq!(status, 400);
    let yank_doc = note(&yank, &[&acme, &acme_b]);
    let (status, why) = post(addr, "submit", yank_doc.as_bytes());
    assert_eq!(status, 200, "{why}");
    let advisory = format!(
        "ita-advisory/1\nsource itaty\nname acme/ledger\nprevious 3 {}\nid ITA-2026-0001\nline 1\naffected 1.0.0\nfixed none\nseverity high\nsummary a test advisory\n",
        digest_string(&leaf_hash(yank_doc.as_bytes()))
    );
    let (status, _) = post(addr, "submit", note(&advisory, &[&acme]).as_bytes());
    assert_eq!(status, 400, "an owner key is not a security key");
    let (status, why) = post(
        addr,
        "submit",
        note(&advisory, &[&keys.security[0]]).as_bytes(),
    );
    assert_eq!(status, 200, "{why}");

    clock.0.store(1_900_000_200, Ordering::SeqCst);
    witness_round(addr, &mut witnesses[1..], 1_900_000_200);
    witness_round(addr, &mut witnesses[..1], 1_900_000_150);

    // The newer checkpoint is consistent with the one the client holds.
    let newer_doc = String::from_utf8(get(addr, "checkpoint").unwrap()).unwrap();
    let newer = parse_checkpoint(&parse_note(&newer_doc).unwrap().text).unwrap();
    assert_eq!(newer.size, 5);
    let mut fetch = |path: &str| Ok(get(addr, path));
    let mut reader = TileReader::new(newer.size, &mut fetch);
    let proof = consistency(&mut reader, checkpoint.size, newer.size).unwrap();
    assert!(verify_consistency(
        checkpoint.size,
        newer.size,
        &checkpoint.root,
        &newer.root,
        &proof
    ));
    // The release was logged at the k-th earliest of the witnesses' first cosignatures.
    let mut all = cosigns.clone();
    all.extend(cosigned(&newer_doc, &root.witnesses.keys));
    assert_eq!(
        logged_at(1, &all, root.witnesses.threshold),
        Ok(1_900_000_100)
    );

    // The client walks the name's chain from the map's head back to the entry it holds.
    let map_root = map_root_of(&newer).unwrap();
    let mut fetch = |path: &str| Ok(get(addr, path));
    let (new_head, proof) = prove_from_tiles(&map_root, "acme/ledger", &mut fetch).unwrap();
    let new_head = new_head.unwrap();
    assert!(verify_map(
        &map_root,
        "acme/ledger",
        Some(&new_head),
        &proof
    ));
    assert_eq!(new_head.count, 4);
    let log = entries(addr, 0, newer.size);
    let mut link = |index: u64| -> Result<Link, String> {
        let entry = una_itaty::entry::parse_entry(&log[index as usize])?;
        Ok(Link {
            index,
            leaf: entry.leaf(),
            previous: entry.previous,
        })
    };
    let walked = walk(&new_head, Some(&head), &mut link).unwrap();
    assert_eq!(
        walked.iter().map(|l| l.index).collect::<Vec<_>>(),
        vec![4, 3, 2]
    );

    // Opened again on the same store, the registry replays the same log.
    let reopened = Registry::open(Arc::clone(&store), &config, clock.clone()).unwrap();
    assert_eq!(reopened.size(), 5);
    assert_eq!(reopened.map_root(), map_root);

    // A path outside rule 34 is never served.
    assert_eq!(request(addr, "GET", "state/size", &[], b"").0, 404);
    assert_eq!(request(addr, "GET", "tile/../state/size", &[], b"").0, 404);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A publication takes its publishing slot before its body is read, so a third one, while
/// two are being read, is refused before any byte of its body arrives.
#[test]
fn publications_take_their_slot_before_their_body_is_read() {
    let keys = keys();
    let root_doc = root_document(&keys);
    let dir = temp_dir("slots");
    let store: Arc<dyn Store> = Arc::new(FileStore::open(&dir).unwrap());
    store.put("root/1", root_doc.as_bytes()).unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(1_900_000_000)));
    let config = Config {
        pin: root_digest(&root_doc),
        log_seed: [20; 32],
    };
    let registry = Registry::open(Arc::clone(&store), &config, clock).unwrap();
    let service =
        Arc::new(Service::new(Arc::clone(&store), registry, ServerConfig::default()).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    {
        let service = Arc::clone(&service);
        std::thread::spawn(move || serve(&listener, &service));
    }
    let head = format!(
        "POST /publish HTTP/1.1\r\nContent-Length: {}\r\n\r\nita-publish/1\n",
        una_itaty::http::MAX_PUBLISH_BODY
    );
    // Two publications declare the largest body and send only its first bytes.
    let stalled: Vec<TcpStream> = (0..2)
        .map(|_| {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream.write_all(head.as_bytes()).unwrap();
            stream
        })
        .collect();
    // Both hold a slot while their bodies are read; the wait is bounded, not timed.
    let mut polls = 0;
    while service.publications_in_progress() < 2 {
        polls += 1;
        assert!(polls < 2000, "the stalled publications took no slot");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // A third gets 503 from its head alone.
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.write_all(head.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 503"), "{response}");
    drop(stalled);
    // The slots are given back once the stalled connections end.
    let mut polls = 0;
    while service.publications_in_progress() > 0 {
        polls += 1;
        assert!(polls < 2000, "the slots were not given back");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bodies_past_their_bound_are_refused_before_they_are_read() {
    let keys = keys();
    let root_doc = root_document(&keys);
    let dir = temp_dir("bounds");
    let store: Arc<dyn Store> = Arc::new(FileStore::open(&dir).unwrap());
    store.put("root/1", root_doc.as_bytes()).unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(1_900_000_000)));
    let config = Config {
        pin: root_digest(&root_doc),
        log_seed: [20; 32],
    };
    let registry = Registry::open(Arc::clone(&store), &config, clock).unwrap();
    let service =
        Arc::new(Service::new(Arc::clone(&store), registry, ServerConfig::default()).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    {
        let service = Arc::clone(&service);
        std::thread::spawn(move || serve(&listener, &service));
    }
    // A Content-Length past the bound is refused with no body sent.
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .write_all(b"POST /submit HTTP/1.1\r\nContent-Length: 999999999\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 413"), "{response}");
    // Chunked bodies and missing lengths are refused.
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .write_all(b"POST /submit HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 501"), "{response}");
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.write_all(b"POST /submit HTTP/1.1\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 411"), "{response}");
    // Registration is closed without an operator token.
    assert_eq!(request(addr, "POST", "register", &[], b"x").0, 403);
    // A root metadata pin that does not match refuses to open.
    let wrong = Config {
        pin: digest_string(&[0u8; 32]),
        log_seed: [20; 32],
    };
    let clock = Arc::new(TestClock(AtomicU64::new(1_900_000_000)));
    assert!(Registry::open(Arc::clone(&store), &wrong, clock.clone()).is_err());
    // A log key that is not the root's refuses to open.
    let wrong = Config {
        pin: root_digest(&root_doc),
        log_seed: [21; 32],
    };
    assert!(Registry::open(store, &wrong, clock).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A registry served over HTTP in this process with `server`, on a fresh file-backed store.
fn served(name: &str, server: ServerConfig) -> (Arc<Service>, SocketAddr, std::path::PathBuf) {
    let keys = keys();
    let root_doc = root_document(&keys);
    let dir = temp_dir(name);
    let store: Arc<dyn Store> = Arc::new(FileStore::open(&dir).unwrap());
    store.put("root/1", root_doc.as_bytes()).unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(1_900_000_000)));
    let config = Config {
        pin: root_digest(&root_doc),
        log_seed: [20; 32],
    };
    let registry = Registry::open(Arc::clone(&store), &config, clock).unwrap();
    let service = Arc::new(Service::new(Arc::clone(&store), registry, server).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    {
        let service = Arc::clone(&service);
        std::thread::spawn(move || serve(&listener, &service));
    }
    (service, addr, dir)
}

/// Two publications that declare the largest body and send a byte now and then lose their
/// slots once their first idle timeout is past, and a third publication is then served.
#[test]
fn trickling_publications_lose_their_slots_after_the_idle_timeout() {
    let idle = std::time::Duration::from_secs(1);
    let (service, addr, dir) = served(
        "trickle",
        ServerConfig {
            idle_timeout: idle,
            ..ServerConfig::default()
        },
    );
    let head = format!(
        "POST /publish HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        una_itaty::http::MAX_PUBLISH_BODY
    );
    let mut streams: Vec<TcpStream> = (0..2)
        .map(|_| {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream.write_all(head.as_bytes()).unwrap();
            stream
        })
        .collect();
    let mut polls = 0;
    while service.publications_in_progress() < 2 {
        polls += 1;
        assert!(polls < 2000, "the trickling publications took no slot");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // A byte every 0.3 seconds never trips the idle timeout; the rate does.
    let started = std::time::Instant::now();
    while service.publications_in_progress() > 0 {
        assert!(
            started.elapsed() < 4 * idle,
            "the slots were held for {:?}",
            started.elapsed()
        );
        for stream in &mut streams {
            let _ = stream.write_all(b"x");
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    // Each was answered 408; a byte written after the close may turn the answer into a
    // reset, so only an answer that arrived is checked.
    for stream in &mut streams {
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response);
        let response = String::from_utf8_lossy(&response);
        assert!(
            response.is_empty() || response.starts_with("HTTP/1.1 408"),
            "{response}"
        );
    }
    // The slot is free again: a third publication is read, and refused for its content.
    let (status, _) = request(addr, "POST", "publish", &[], b"not a publication");
    assert_eq!(status, 400);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A publication refused for the lack of a slot, which sent a small body with its head,
/// reads the 503 rather than a reset.
#[test]
fn a_publication_refused_before_its_body_reads_its_refusal() {
    let (service, addr, dir) = served("refused", ServerConfig::default());
    let head = format!(
        "POST /publish HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        una_itaty::http::MAX_PUBLISH_BODY
    );
    let _stalled: Vec<TcpStream> = (0..2)
        .map(|_| {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream.write_all(head.as_bytes()).unwrap();
            stream
        })
        .collect();
    let mut polls = 0;
    while service.publications_in_progress() < 2 {
        polls += 1;
        assert!(polls < 2000, "the stalled publications took no slot");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    for _ in 0..3 {
        let len = 32 << 10;
        let mut stream = TcpStream::connect(addr).unwrap();
        let mut sent =
            format!("POST /publish HTTP/1.1\r\nContent-Length: {len}\r\n\r\n").into_bytes();
        sent.resize(sent.len() + len, b'a');
        stream.write_all(&sent).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 503"), "{response}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The size of the blob the tests of slow readers serve: well past the buffers of a loopback
/// connection (8 MiB at most on macOS, 10 MiB on Linux), so a peer that never reads stalls
/// the service's writes.
const BIG: usize = 32 << 20;

/// A registry served with `server` through `serve_with` and `spawn`, holding a blob of `BIG`
/// bytes; its path.
fn served_blob(
    name: &str,
    server: ServerConfig,
    spawn: impl Fn(Work) -> std::io::Result<()> + Send + 'static,
) -> (Arc<Service>, SocketAddr, std::path::PathBuf, String) {
    let keys = keys();
    let root_doc = root_document(&keys);
    let dir = temp_dir(name);
    let store: Arc<dyn Store> = Arc::new(FileStore::open(&dir).unwrap());
    store.put("root/1", root_doc.as_bytes()).unwrap();
    let blob: Vec<u8> = (0..BIG).map(|i| (i % 251) as u8).collect();
    let path = format!("blobs/sha256/{}", "ab".repeat(32));
    store.put(&path, &blob).unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(1_900_000_000)));
    let config = Config {
        pin: root_digest(&root_doc),
        log_seed: [20; 32],
    };
    let registry = Registry::open(Arc::clone(&store), &config, clock).unwrap();
    let service = Arc::new(Service::new(Arc::clone(&store), registry, server).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    {
        let service = Arc::clone(&service);
        std::thread::spawn(move || serve_with(&listener, &service, spawn));
    }
    (service, addr, dir, path)
}

fn spawn_thread(work: Work) -> std::io::Result<()> {
    std::thread::Builder::new().spawn(work).map(drop)
}

/// Waits until `held` is true, at most `within`.
fn wait_until(within: Duration, what: &str, held: impl Fn() -> bool) {
    let started = Instant::now();
    while !held() {
        assert!(started.elapsed() < within, "{what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Sends `GET /path` and reads nothing.
fn unread_get(addr: SocketAddr, path: &str) -> TcpStream {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .write_all(format!("GET /{path} HTTP/1.1\r\n\r\n").as_bytes())
        .unwrap();
    stream
}

/// Everything a connection gives until it closes or resets.
fn read_all(stream: &mut TcpStream) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 64 << 10];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return out,
            Ok(n) => out.extend_from_slice(&chunk[..n]),
        }
    }
}

/// A client that never reads a large response holds its connection slot while the response
/// is written, and loses it, with its connection, once a write waits the write timeout; an
/// honest reader of the same file gets it whole.
#[test]
fn a_client_that_never_reads_is_dropped_within_the_write_bound() {
    let write_timeout = Duration::from_secs(1);
    let (service, addr, dir, path) = served_blob(
        "unread",
        ServerConfig {
            write_timeout,
            ..ServerConfig::default()
        },
        spawn_thread,
    );
    let started = Instant::now();
    let mut stalled = unread_get(addr, &path);
    wait_until(Duration::from_secs(10), "the GET took no slot", || {
        service.connections_in_progress() == 1
    });
    wait_until(
        Duration::from_secs(30),
        "the slot was never given back",
        || service.connections_in_progress() == 0,
    );
    let held = started.elapsed();
    // The slot is given back only after a write waited the whole write timeout.
    assert!(
        held >= write_timeout,
        "the slot was given back after {held:?}"
    );
    // The connection was closed short of its `Content-Length`.
    let received = read_all(&mut stalled);
    assert!(received.starts_with(b"HTTP/1.1 200"));
    assert!(received.len() < BIG, "{} bytes received", received.len());
    // An honest reader gets the whole file, sent in parts.
    let (status, body) = request(addr, "GET", &path, &[], b"");
    assert_eq!((status, body.len()), (200, BIG));
    assert!(body.iter().enumerate().all(|(i, b)| *b == (i % 251) as u8));
    let _ = std::fs::remove_dir_all(&dir);
}

/// While `max_connections` slots are held by clients that do not read, a further connection is
/// answered 503 and closed from the accept loop, and starts no thread; once the slots are given
/// back, connections are served again.
#[test]
fn connections_past_the_bound_are_refused_and_start_no_thread() {
    let spawned = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&spawned);
    let (service, addr, dir, path) = served_blob(
        "bound",
        ServerConfig {
            max_connections: 2,
            write_timeout: Duration::from_secs(5),
            ..ServerConfig::default()
        },
        move |work| {
            counted.fetch_add(1, Ordering::SeqCst);
            spawn_thread(work)
        },
    );
    let stalled: Vec<TcpStream> = (0..2).map(|_| unread_get(addr, &path)).collect();
    wait_until(Duration::from_secs(10), "the GETs took no slot", || {
        service.connections_in_progress() == 2
    });
    for _ in 0..5 {
        // The refusal is written as soon as the connection is accepted; a client that sent
        // nothing reads it before the close.
        let mut stream = TcpStream::connect(addr).unwrap();
        let response = String::from_utf8_lossy(&read_all(&mut stream)).to_string();
        assert!(response.starts_with("HTTP/1.1 503"), "{response}");
        assert!(response.contains("too many connections"), "{response}");
    }
    assert_eq!(spawned.load(Ordering::SeqCst), 2);
    assert_eq!(service.connections_in_progress(), 2);
    drop(stalled);
    wait_until(
        Duration::from_secs(30),
        "the slots were not given back",
        || service.connections_in_progress() == 0,
    );
    assert!(get(addr, "root/1").is_some());
    assert_eq!(spawned.load(Ordering::SeqCst), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A connection whose thread cannot start is closed and gives its slot back, and the accept
/// loop goes on serving.
#[test]
fn a_thread_that_cannot_start_does_not_stop_the_accept_loop() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let (service, addr, dir, _) = served_blob(
        "spawn",
        ServerConfig {
            max_connections: 1,
            ..ServerConfig::default()
        },
        move |work| {
            if counted.fetch_add(1, Ordering::SeqCst) < 2 {
                drop(work);
                return Err(std::io::Error::other("no thread"));
            }
            spawn_thread(work)
        },
    );
    for _ in 0..2 {
        let mut stream = TcpStream::connect(addr).unwrap();
        assert!(read_all(&mut stream).is_empty());
        wait_until(
            Duration::from_secs(10),
            "the slot was not given back",
            || service.connections_in_progress() == 0,
        );
    }
    // The one slot is free and the loop still accepts.
    assert!(get(addr, "root/1").is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Connections that send nothing hold their slots for the first-byte timeout, not for the
/// head's time: they are answered 408 and closed, and the slots serve other clients.
#[test]
fn connections_that_send_nothing_lose_their_slots_at_the_first_byte_timeout() {
    let first_byte_timeout = Duration::from_millis(300);
    let config = ServerConfig {
        max_connections: 2,
        first_byte_timeout,
        ..ServerConfig::default()
    };
    // Far less than the head's time, which held the slots before.
    let bound = Duration::from_secs(10);
    assert!(bound < config.head_timeout);
    let (service, addr, dir) = served("idle", config);
    let started = Instant::now();
    let mut idle: Vec<TcpStream> = (0..2).map(|_| TcpStream::connect(addr).unwrap()).collect();
    wait_until(bound, "the idle connections took no slot", || {
        service.connections_in_progress() == 2
    });
    wait_until(bound, "the idle connections kept their slots", || {
        service.connections_in_progress() == 0
    });
    assert!(started.elapsed() >= first_byte_timeout);
    for stream in &mut idle {
        let response = String::from_utf8_lossy(&read_all(stream)).to_string();
        assert!(response.starts_with("HTTP/1.1 408"), "{response}");
        assert!(response.contains("did not start in time"), "{response}");
    }
    assert!(get(addr, "root/1").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// One address holds at most `max_connections_per_peer` slots: its next connection is
/// answered 503 while the service has slots left, and is served once it gives one back.
#[test]
fn one_address_holds_at_most_its_bound_of_connections() {
    let (service, addr, dir) = served(
        "peer",
        ServerConfig {
            max_connections: 8,
            max_connections_per_peer: 2,
            // The idle connections below hold their slots until they are dropped.
            first_byte_timeout: Duration::from_secs(25),
            ..ServerConfig::default()
        },
    );
    let idle: Vec<TcpStream> = (0..2).map(|_| TcpStream::connect(addr).unwrap()).collect();
    wait_until(
        Duration::from_secs(10),
        "the connections took no slot",
        || service.connections_in_progress() == 2,
    );
    for _ in 0..3 {
        let mut stream = TcpStream::connect(addr).unwrap();
        let response = String::from_utf8_lossy(&read_all(&mut stream)).to_string();
        assert!(response.starts_with("HTTP/1.1 503"), "{response}");
        assert!(
            response.contains("too many connections from this address"),
            "{response}"
        );
    }
    // The refusals took no slot of the six the service has left.
    assert_eq!(service.connections_in_progress(), 2);
    drop(idle);
    wait_until(
        Duration::from_secs(10),
        "the slots were not given back",
        || service.connections_in_progress() == 0,
    );
    assert!(get(addr, "root/1").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A registry on a fresh file-backed store, not yet served.
fn opened(name: &str) -> (Arc<dyn Store>, Registry, std::path::PathBuf) {
    let keys = keys();
    let root_doc = root_document(&keys);
    let dir = temp_dir(name);
    let store: Arc<dyn Store> = Arc::new(FileStore::open(&dir).unwrap());
    store.put("root/1", root_doc.as_bytes()).unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(1_900_000_000)));
    let config = Config {
        pin: root_digest(&root_doc),
        log_seed: [20; 32],
    };
    let registry = Registry::open(Arc::clone(&store), &config, clock).unwrap();
    (store, registry, dir)
}

/// A configuration with a zero timeout, which a socket takes as no timeout, makes no service.
#[test]
fn a_service_is_not_made_with_a_zero_timeout() {
    let (store, registry, dir) = opened("zero");
    let refused = Service::new(
        store,
        registry,
        ServerConfig {
            write_timeout: Duration::ZERO,
            ..ServerConfig::default()
        },
    )
    .unwrap_err();
    assert!(refused.contains("`write_timeout` is zero"), "{refused}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A listener handed over in non-blocking mode is served: `serve` makes it block (that its
/// loop cannot spin on one that does not is a unit test of the loop).
#[test]
fn a_listener_that_does_not_block_is_served() {
    let (store, registry, dir) = opened("nonblocking");
    let service = Arc::new(Service::new(store, registry, ServerConfig::default()).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || serve(&listener, &service));
    for _ in 0..3 {
        assert!(get(addr, "root/1").is_some());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_store_has_one_writer() {
    let dir = temp_dir("lock");
    let first = FileStore::open(&dir).unwrap();
    let second = FileStore::open(&dir).unwrap_err();
    assert!(second.contains("one writer"), "{second}");
    drop(first);
    let reopened = FileStore::open(&dir).unwrap();
    drop(reopened);
    std::fs::remove_dir_all(&dir).unwrap();
}
