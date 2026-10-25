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
use std::sync::atomic::{AtomicU64, Ordering};

use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use una_itaty::entry::owner_chain;
use una_itaty::http::{ServerConfig, Service, serve};
use una_itaty::namemap::prove_from_tiles;
use una_itaty::registry::{Cause, Clock, Config, Registry, map_root_of};
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
use una_project::tlog::root::{digest as root_digest, trust_first};
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
    let registry = Registry::open(Arc::clone(&store), &config, clock.clone(), recording()).unwrap();
    let outbox = temp_dir("e2e-outbox");
    let service = Arc::new(
        Service::new(
            Arc::clone(&store),
            registry,
            ServerConfig {
                register_token: Some(TOKEN.to_owned()),
                outbox: Some(outbox.clone()),
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
    let acme = signer("acme.a", 60);
    let registration = format!(
        "ita-owner/1\nsource itaty\nowner acme\nprevious none\nthreshold 1\n{}\n",
        key_line(&acme)
    );
    let registration = note(&registration, &[&acme]);
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
    let (_, understated) = publication(&v1, None, &authority(&[]), &[&acme]);
    let (status, why) = post(addr, "publish", &understated);
    assert_eq!(status, 400, "{why}");
    // Its cause, typed, is the interface (rule 37).
    let (understated_note, _) = publication(&v1, None, &authority(&[]), &[&acme]);
    assert_eq!(
        Registry::prepare(understated_note.as_bytes(), v1.archive.clone())
            .unwrap_err()
            .cause(),
        Some(&Cause::Interface)
    );
    // A statement signed by a key that is not the owner's is refused.
    let (_, unsigned) = publication(&v1, None, &console, &[&other]);
    let (status, why) = post(addr, "publish", &unsigned);
    assert_eq!(status, 400, "{why}");
    let (document_1, body) = publication(&v1, None, &console, &[&acme]);
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
        &[&acme],
    );
    let (status, why) = post(addr, "publish", &body);
    assert_eq!(status, 400, "{why}");
    // A `previous` that is not the name's head is refused.
    let v11 = package("1.1.0", "", body_1);
    let (_, stale) = publication(&v11, None, &console, &[&acme]);
    let (status, why) = post(addr, "publish", &stale);
    assert_eq!(status, 400, "{why}");
    // As the next major version, the wider release is logged.
    let major = package("2.0.0", "", body_wider);
    let (document_2, body) = publication(
        &major,
        Some((1, &leaf_1_text)),
        &authority(&[("Console", &["print"]), ("Http", &["get"])]),
        &[&acme],
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
    let yank_doc = note(&yank, &[&acme]);
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
    let reopened = Registry::open(Arc::clone(&store), &config, clock.clone(), recording()).unwrap();
    assert_eq!(reopened.size(), 5);
    assert_eq!(reopened.map_root(), map_root);

    // A path outside rule 34 is never served.
    assert_eq!(request(addr, "GET", "state/size", &[], b"").0, 404);
    assert_eq!(request(addr, "GET", "tile/../state/size", &[], b"").0, 404);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&outbox);
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
    let registry = Registry::open(Arc::clone(&store), &config, clock.clone(), recording()).unwrap();
    // A registry is never served without a notifier: no outbox named, no service.
    let refused = Service::new(Arc::clone(&store), registry, ServerConfig::default()).unwrap_err();
    assert_eq!(refused, una_itaty::http::StartError::NoNotifier);
    let registry = Registry::open(Arc::clone(&store), &config, clock, recording()).unwrap();
    let service = Arc::new(Service::unnotified_for_tests(
        Arc::clone(&store),
        registry,
        ServerConfig::default(),
    ));
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
    assert!(Registry::open(Arc::clone(&store), &wrong, clock.clone(), recording()).is_err());
    // A log key that is not the root's refuses to open.
    let wrong = Config {
        pin: root_digest(&root_doc),
        log_seed: [21; 32],
    };
    assert!(Registry::open(store, &wrong, clock, recording()).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rule_26_is_checked_against_the_highest_release_logged_before_as_the_client_does() {
    let keys = keys();
    let root_doc = root_document(&keys);
    let store: Arc<dyn Store> = Arc::new(una_itaty::store::MemoryStore::new());
    store.put("root/1", root_doc.as_bytes()).unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(1_900_000_000)));
    let config = Config {
        pin: root_digest(&root_doc),
        log_seed: [20; 32],
    };
    let mut registry = Registry::open(Arc::clone(&store), &config, clock, recording()).unwrap();
    let acme = signer("acme.a", 60);
    let registration = format!(
        "ita-owner/1\nsource itaty\nowner acme\nprevious none\nthreshold 1\n{}\n",
        key_line(&acme)
    );
    registry
        .register(note(&registration, &[&acme]).as_bytes(), false)
        .unwrap();
    let with_http = "pub fetch(url: Text) -> Text\n  uses @h: Http[get]\n=\n  url\n";
    let without = "pub answer() -> Int = 42\n";
    let http = authority(&[("Http", &["get"])]);
    let none = authority(&[]);
    let publish = |registry: &mut Registry,
                   version: &str,
                   body: &str,
                   previous: Option<(u64, &str)>,
                   authority: &BTreeMap<String, BTreeSet<String>>| {
        let release = package(version, "", body);
        let (document, _) = publication(&release, previous, authority, &[&acme]);
        let prepared = Registry::prepare(document.as_bytes(), release.archive.clone()).unwrap();
        registry.publish(prepared)
    };
    // 1.0.0 needs Http; 1.2.0 needs nothing.
    let first = publish(&mut registry, "1.0.0", with_http, None, &http).unwrap();
    let first_leaf = digest_string(&first.leaf);
    let second = publish(
        &mut registry,
        "1.2.0",
        without,
        Some((first.index, &first_leaf)),
        &none,
    )
    .unwrap();
    // 1.1.0 needs Http again: within 1.0.0, but not within 1.2.0, the highest release of
    // its line logged before it, which is what every client checks it against.
    let second_leaf = digest_string(&second.leaf);
    let refused = publish(
        &mut registry,
        "1.1.0",
        with_http,
        Some((second.index, &second_leaf)),
        &http,
    )
    .unwrap_err();
    assert_eq!(refused.cause(), Some(&Cause::Rule26), "{refused:?}");
}

#[test]
fn an_advisory_identifier_is_given_once_per_name() {
    let keys = keys();
    let root_doc = root_document(&keys);
    let store: Arc<dyn Store> = Arc::new(una_itaty::store::MemoryStore::new());
    store.put("root/1", root_doc.as_bytes()).unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(1_900_000_000)));
    let config = Config {
        pin: root_digest(&root_doc),
        log_seed: [20; 32],
    };
    let mut registry = Registry::open(Arc::clone(&store), &config, clock, recording()).unwrap();
    let acme = signer("acme.a", 60);
    let registration = format!(
        "ita-owner/1\nsource itaty\nowner acme\nprevious none\nthreshold 1\n{}\n",
        key_line(&acme)
    );
    registry
        .register(note(&registration, &[&acme]).as_bytes(), false)
        .unwrap();
    let release = package("1.0.0", "", "pub answer() -> Int = 42\n");
    let (document, _) = publication(&release, None, &authority(&[]), &[&acme]);
    let prepared = Registry::prepare(document.as_bytes(), release.archive.clone()).unwrap();
    let mut head = registry.publish(prepared).unwrap();
    let advisory = |previous: &una_itaty::registry::Receipt| {
        let text = format!(
            "ita-advisory/1\nsource itaty\nname acme/ledger\nprevious {} {}\nid ITA-2026-0007\nline 1\naffected 1.0.0\nfixed none\nseverity low\nsummary a test advisory\n",
            previous.index,
            digest_string(&previous.leaf)
        );
        note(&text, &[&keys.security[0]])
    };
    head = registry.submit(advisory(&head).as_bytes()).unwrap();
    let repeated = registry.submit(advisory(&head).as_bytes()).unwrap_err();
    assert_eq!(
        repeated.cause(),
        Some(&Cause::AdvisoryIdRepeated),
        "{repeated:?}"
    );
}

/// A notifier that records notices in memory, which every registry needs.
fn recording() -> Arc<una_itaty::notify::Recording> {
    Arc::new(una_itaty::notify::Recording::new())
}

#[test]
fn releases_and_yanks_are_judged_by_the_clients_replay() {
    let keys = keys();
    let root_doc = root_document(&keys);
    let store: Arc<dyn Store> = Arc::new(una_itaty::store::MemoryStore::new());
    store.put("root/1", root_doc.as_bytes()).unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(1_900_000_000)));
    let config = Config {
        pin: root_digest(&root_doc),
        log_seed: [20; 32],
    };
    let mut registry = Registry::open(Arc::clone(&store), &config, clock, recording()).unwrap();
    let acme = signer("acme.a", 60);
    let trustee = signer("trustee.c", 61);
    let other = signer("other.o", 62);
    let registration = format!(
        "ita-owner/1\nsource itaty\nowner acme\nprevious none\nthreshold 1\n{}\ncosigner {} {}\ncosigners 1\n",
        key_line(&acme),
        trustee.key().name,
        encode_base64(&trustee.key().public)
    );
    let registered = registry
        .register(note(&registration, &[&acme]).as_bytes(), false)
        .unwrap();
    let body = "pub answer() -> Int = 42\n";
    let none = authority(&[]);
    let release = package("1.0.0", "", body);
    let (document, _) = publication(&release, None, &none, &[&acme]);
    let first = registry
        .publish(Registry::prepare(document.as_bytes(), release.archive.clone()).unwrap())
        .unwrap();
    let first_leaf = digest_string(&first.leaf);
    // A release signed by no key of its owner is refused, judged as the client judges it.
    let next = package("1.1.0", "", body);
    let (unsigned, _) = publication(&next, Some((first.index, &first_leaf)), &none, &[&other]);
    assert_eq!(
        registry
            .publish(Registry::prepare(unsigned.as_bytes(), next.archive.clone()).unwrap())
            .unwrap_err()
            .cause(),
        Some(&Cause::NotSigned)
    );
    // The cosigners claim the owner's last key compromised since 1.
    let claim = format!(
        "ita-key/1\nsource itaty\nowner acme\nprevious 0 {}\nrevoke acme.a since 1\nthreshold 1\n",
        digest_string(&registered.leaf)
    );
    registry
        .submit(note(&claim, &[&trustee]).as_bytes())
        .unwrap();
    // What that key signs from then on would be void: a yank and a release are refused.
    let yank = format!(
        "ita-yank/1\nsource itaty\nname acme/ledger\nversion 1.0.0\nprevious {} {first_leaf}\n",
        first.index
    );
    assert_eq!(
        registry
            .submit(note(&yank, &[&acme]).as_bytes())
            .unwrap_err()
            .cause(),
        Some(&Cause::Void)
    );
    let (compromised, _) = publication(&next, Some((first.index, &first_leaf)), &none, &[&acme]);
    assert_eq!(
        registry
            .publish(Registry::prepare(compromised.as_bytes(), next.archive.clone()).unwrap())
            .unwrap_err()
            .cause(),
        Some(&Cause::Void)
    );
}
