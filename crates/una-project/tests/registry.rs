//! The registry client's side of publishing, yanks, advisories, audit, watch, mirrors and
//! bundles (ADR 0122 rules 33, 36.2 and 40 to 43, `spec/30 §14`).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use una_project::content::File;
use una_project::itaoca::Itaoca;
use una_project::provenance::Expected;
use una_project::registry::audit::{self, Kind, Locked};
use una_project::registry::bundle;
use una_project::registry::entries::{check_advisory, check_yank, cooled, releases, yank_text};
use una_project::registry::keyfile::{SigningKey, sign_note};
use una_project::registry::mirrors::{self, first_verified};
use una_project::registry::submit::{self, Promise, broken, check_promise};
use una_project::registry::transport::{Directory, Failure as TransportFailure, Location, Transport, location};
use una_project::registry::view::{ChainEntry, Failure, NameState, OwnerState, Registry};
use una_project::registry::watch::{Position, watch};
use una_project::tlog::note::parse_note;
use una_project::tlog::release::parse_statement;
use una_project::tlog::state::{Advisory, ReleaseState};
use una_project::version::{Line, Version};
use una_syntax::ita::parse_manifest;

const D: &str = "sha256:3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b";

fn v(text: &str) -> Version {
    Version::parse_full(text).unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("registry")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::canonicalize(&dir).unwrap()
}

fn key(seed: u8, name: &str) -> SigningKey {
    SigningKey::from_secret(name, &[seed; 32]).unwrap()
}

// --- URLs, transports and mirrors (rule 40) -------------------------------------------------

#[test]
fn only_https_and_file_urls_are_read() {
    assert_eq!(
        location("https://ita.acme.example/base").unwrap(),
        Location::Https("https://ita.acme.example/base".to_owned())
    );
    assert_eq!(
        location("file:///srv/mirror").unwrap(),
        Location::Directory(PathBuf::from("/srv/mirror"))
    );
    for (url, part) in [
        ("http://ita.acme.example", "plain HTTP"),
        ("HTTP://ita.acme.example", "plain HTTP"),
        ("ftp://ita.acme.example", "scheme `ftp`"),
        ("HTTPS://ita.acme.example", "scheme `HTTPS`"),
        ("ita.acme.example", "no scheme"),
        ("https:///path", "no host"),
        ("file://host/srv", "names a host"),
        ("file:///srv/../etc", "`..`"),
        ("https://ita.acme.example/a b", "space"),
    ] {
        let refusal = location(url).unwrap_err();
        assert!(refusal.contains(part), "{url}: {refusal}");
    }
    assert!(Directory::of("https://ita.acme.example").unwrap_err().contains("no network transport"));
}

#[test]
fn the_directory_transport_writes_once_and_reads_back() {
    let dir = scratch("transport");
    let transport = Directory::new(&dir, "file:///test");
    transport.put("submit/abc/entry", b"one").unwrap();
    assert_eq!(transport.get("submit/abc/entry", 10).unwrap(), b"one");
    // The same bytes again are accepted; other bytes are refused.
    transport.put("submit/abc/entry", b"one").unwrap();
    assert!(transport.put("submit/abc/entry", b"two").is_err());
    assert_eq!(transport.get("submit/none", 10), Err(TransportFailure::Absent));
    for path in ["../x", "a/../b", ".hidden", "/abs", "a//b"] {
        assert!(transport.get(path, 10).is_err(), "{path}");
        assert!(transport.put(path, b"x").is_err(), "{path}");
    }
    assert!(transport.get("submit/abc/entry", 2).is_err(), "over the limit");
}

#[test]
fn mirrors_una_reads_and_refuses_plain_http() {
    let (mirrors, errors) = mirrors::parse(
        "mirrors\n  itaty\n    \"https://ita-mirror.example.net\"\n    \"file:///srv/ita\"\n  acme\n    \"https://acme-mirror.example\"\n",
    );
    assert_eq!(errors, []);
    assert_eq!(mirrors.of("itaty").len(), 2);
    assert_eq!(mirrors.of("acme")[0].url, "https://acme-mirror.example");
    assert!(mirrors.of("other").is_empty());
    let (_, errors) = mirrors::parse("mirrors\n  itaty\n    \"http://ita-mirror.example.net\"\n");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, una_diag::Code::U7008);
    assert!(errors[0].message.contains("plain HTTP"), "{}", errors[0].message);
    for text in [
        "mirror\n  itaty\n    \"https://a.example\"\n",
        "mirrors\n  itaty\n    \"https://a.example\"\n    \"https://a.example\"\n",
        "mirrors\n  itaty\n    \"https://a.example\"\n  itaty\n    \"https://b.example\"\n",
        "mirrors\n  Itaty\n    \"https://a.example\"\n",
        "mirrors\n  itaty\n    https\n",
    ] {
        let (_, errors) = mirrors::parse(text);
        assert!(!errors.is_empty(), "{text:?}");
        assert!(errors.iter().all(|e| e.code == una_diag::Code::U7008));
    }
}

/// A mirror that serves a document that does not verify is skipped and named; the next one's
/// verified copy is taken. Verified bytes decide, wherever they come from.
#[test]
fn a_mirror_serving_a_tampered_document_is_skipped() {
    let bad = scratch("mirror-bad");
    let good = scratch("mirror-good");
    let empty = scratch("mirror-empty");
    let archive = package_archive("0.3.1");
    let digest = una_project::tree::VerifiedTree::from_archive(&archive)
        .unwrap()
        .digest()
        .to_owned();
    let mut tampered = archive.clone();
    let last = tampered.len() - 2;
    tampered[last] ^= 1;
    let path = bundle::package_path(&digest).unwrap().replace("packages", "blobs/sha256");
    let bad = Directory::new(&bad, "file:///bad");
    let empty = Directory::new(&empty, "file:///empty");
    let good = Directory::new(&good, "file:///good");
    bad.put(&path, &tampered).unwrap();
    good.put(&path, &archive).unwrap();
    let verify = |bytes: &[u8]| -> Result<String, String> {
        let tree = una_project::tree::VerifiedTree::from_archive(bytes)?;
        if tree.digest() == digest {
            Ok(tree.digest().to_owned())
        } else {
            Err(format!("the digest is {}", tree.digest()))
        }
    };
    let transports: [&dyn Transport; 3] = [&bad, &empty, &good];
    let fetched = first_verified(&transports, &path, 1 << 20, &verify);
    assert_eq!(fetched.value, Some((digest.clone(), "file:///good".to_owned())));
    assert_eq!(fetched.refused.len(), 1);
    assert_eq!(fetched.refused[0].from, "file:///bad");
    assert_eq!(fetched.unavailable[0].from, "file:///empty");
    // With only the bad one, nothing is taken.
    let only_bad: [&dyn Transport; 1] = [&bad];
    assert_eq!(first_verified(&only_bad, &path, 1 << 20, &verify).value, None);
}

/// Trying m mirrors costs O(m): one fetch and one verification each, never more.
#[test]
fn trying_mirrors_scales_linearly() {
    let path = "blobs/sha256/x";
    for m in [10usize, 100, 400] {
        let parent = scratch(&format!("scale-{m}"));
        let dirs: Vec<Directory> = (0..m)
            .map(|i| {
                let transport = Directory::new(&parent.join(format!("m{i}")), &format!("file:///m{i}"));
                let bytes: &[u8] = if i + 1 == m { b"good" } else { b"bad!" };
                transport.put(path, bytes).unwrap();
                transport
            })
            .collect();
        let transports: Vec<&dyn Transport> = dirs.iter().map(|d| d as &dyn Transport).collect();
        let verify = |bytes: &[u8]| -> Result<(), String> {
            if bytes == b"good" {
                Ok(())
            } else {
                Err("bad".to_owned())
            }
        };
        let fetched = first_verified(&transports, path, 16, &verify);
        assert!(fetched.value.is_some());
        assert_eq!(fetched.tried, m);
        assert_eq!(fetched.refused.len(), m - 1);
        let _ = std::fs::remove_dir_all(&parent);
    }
}

// --- Keys, publishing and promises (rules 36.2 and 43) ---------------------------------------

#[test]
fn a_key_file_reads_back_and_signs_notes() {
    let publisher = key(50, "acme.publisher");
    let read = SigningKey::parse(&publisher.encode()).unwrap();
    assert_eq!(read.key(), publisher.key());
    assert!(format!("{read:?}").contains("acme.publisher"));
    assert!(!format!("{read:?}").contains("secret"));
    let note = sign_note("hello\n", &[&publisher]).unwrap();
    assert!(parse_note(&note).unwrap().signed_by(publisher.key()));
    assert!(sign_note("hello\n", &[]).is_err());
    assert!(sign_note("a\n\nb\n", &[&publisher]).is_err());
    assert!(
        SigningKey::parse("ita-key-encrypted/1\n...\n")
            .unwrap_err()
            .contains("passphrase")
    );
    assert!(SigningKey::parse("ita-key/1\nname a+b\nsecret AAAA\n").is_err());
    // A key name with a space cannot name a signature line.
    assert!(SigningKey::from_secret("bad name", &[1; 32]).is_err());
}

fn published(version: &str, extra: &str) -> String {
    format!(
        "ita acme/ledger\n  version {version}\n  edition 0.4\n  about \"Ledgers\"\n  license \"Apache-2.0\"\n  publish to acme\n{extra}"
    )
}

fn files(manifest: &str) -> Vec<File> {
    vec![
        File {
            path: "ita.una".to_owned(),
            bytes: manifest.as_bytes().to_vec(),
        },
        File {
            path: "src/ledger.una".to_owned(),
            bytes: b"module ledger\n\npub total() -> Int = 3\n".to_vec(),
        },
    ]
}

fn first_state() -> NameState {
    NameState {
        source: "acme".to_owned(),
        name: "acme/ledger".to_owned(),
        ..NameState::default()
    }
}

fn authority(caps: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    caps.iter()
        .map(|(cap, ops)| {
            (
                (*cap).to_owned(),
                ops.iter().map(|op| (*op).to_owned()).collect(),
            )
        })
        .collect()
}

#[test]
fn a_release_is_built_signed_and_sent() {
    let publisher = key(50, "acme.publisher");
    let text = published("0.3.1", "");
    let (manifest, errors) = parse_manifest(&text);
    assert!(errors.is_empty(), "{errors:?}");
    let none = BTreeSet::new();
    let submission = submit::release(
        &manifest,
        files(&text),
        &authority(&[("Http", &["get"])]),
        &none,
        &first_state(),
        &[&publisher],
    )
    .unwrap();
    let note = parse_note(&submission.entry).unwrap();
    assert!(note.signed_by(publisher.key()));
    let statement = parse_statement(&note.text).unwrap();
    assert_eq!(statement.previous, None);
    assert_eq!(statement.digest, submission.digest.clone().unwrap());
    assert!(statement.authority["Http"].contains("get"));
    let archive = submission.archive.clone().unwrap();
    let tree = una_project::tree::VerifiedTree::from_archive(&archive).unwrap();
    assert_eq!(tree.digest(), statement.digest);
    // Sent through the directory transport: the archive, then the entry.
    let dir = scratch("send");
    let transport = Directory::new(&dir, "file:///registry");
    let written = submit::send(&transport, &submission).unwrap();
    assert_eq!(written.len(), 2);
    assert!(written[0].ends_with("/archive") && written[1].ends_with("/entry"));
    assert_eq!(
        transport.get(&written[1], 1 << 16).unwrap(),
        submission.entry.as_bytes()
    );
    // Sending it again is the same submission.
    submit::send(&transport, &submission).unwrap();
}

#[test]
fn publishing_refuses_what_rules_26_and_43_name() {
    let publisher = key(50, "acme.publisher");
    let none = BTreeSet::new();
    let refusal = |text: &str, state: &NameState, keys: &[&SigningKey]| {
        let (manifest, _) = parse_manifest(text);
        submit::release(
            &manifest,
            files(text),
            &authority(&[("Http", &["get", "post"])]),
            &none,
            state,
            keys,
        )
        .unwrap_err()
        .join("; ")
    };
    let first = first_state();
    assert!(refusal(&published("0.3.1", "\nneeds\n  util from path \"../util\"\n"), &first, &[&publisher]).contains("path dependency"));
    assert!(refusal(&published("0.3.1", "\nneeds\n  json 2.1\n"), &first, &[&publisher]).contains("needs only packages of `acme`"));
    assert!(refusal(&published("0.3", ""), &first, &[&publisher]).contains("full version"));
    assert!(refusal(&published("0.3.1", ""), &first, &[]).contains("no signing key"));
    // A state of another name is not used.
    let other = NameState {
        name: "acme/other".to_owned(),
        ..first_state()
    };
    assert!(refusal(&published("0.3.1", ""), &other, &[&publisher]).contains("not of `acme/ledger`"));
    // An earlier release of the line that needed less: authority grows only in a major.
    let earlier = format!(
        "ita-release/1\nsource acme\nname acme/ledger\nversion 0.3.0\nprevious none\ndigest {D}\nmanifest {D}\nedition 0.4\nauthority Http[get]\ntrust none\n"
    );
    let mut state = first_state();
    state.head = Some((7, D.to_owned()));
    state.statements.insert(v("0.3.0"), parse_statement(&earlier).unwrap());
    state.releases.insert(
        v("0.3.0"),
        ReleaseState {
            digest: D.to_owned(),
            logged_at: 1,
            yanked: false,
            advised: Vec::new(),
            threshold_fix: false,
        },
    );
    assert!(refusal(&published("0.3.1", ""), &state, &[&publisher]).contains("Http[post]"));
    assert!(refusal(&published("0.3.0", ""), &state, &[&publisher]).contains("already released"));
    // A new major line may grow, and names the chain's head as its previous entry.
    let text = published("1.0.0", "");
    let (manifest, _) = parse_manifest(&text);
    let submission = submit::release(
        &manifest,
        files(&text),
        &authority(&[("Http", &["get", "post"])]),
        &none,
        &state,
        &[&publisher],
    )
    .unwrap();
    let statement = parse_statement(&parse_note(&submission.entry).unwrap().text).unwrap();
    assert_eq!(statement.previous.unwrap().index, 7);
    // A file a package may not hold is refused (rule 27).
    let (manifest, _) = parse_manifest(&text);
    let mut with_c = files(&text);
    with_c.push(File {
        path: "src/native.c".to_owned(),
        bytes: b"int main;".to_vec(),
    });
    assert!(submit::release(&manifest, with_c, &BTreeMap::new(), &none, &state, &[&publisher]).is_err());
}

#[test]
fn a_promise_is_verified_and_broken_only_past_its_time() {
    let log = key(20, "acme.example/log");
    let other = key(21, "acme.example/other");
    let leaf = format!("sha256:{}", "ab".repeat(32));
    let text = format!("ita-promise/1\nsource acme\nleaf {leaf}\nby 1000\n");
    let document = sign_note(&text, &[&log]).unwrap();
    let promise = check_promise(&document, log.key(), "acme", &leaf).unwrap();
    assert_eq!(promise.by, 1000);
    assert!(check_promise(&document, other.key(), "acme", &leaf).is_err());
    assert!(check_promise(&document, log.key(), "itaty", &leaf).is_err());
    assert!(check_promise(&document, log.key(), "acme", D).is_err());
    assert!(!broken(&promise, 1000, false));
    assert!(broken(&promise, 1001, false));
    assert!(!broken(&promise, 5000, true));
}

// --- Yanks, advisories and the cooldown (rules 24 and 41) -----------------------------------

#[test]
fn yanks_need_the_owner_threshold_and_advisories_a_security_key() {
    let owner_a = key(60, "acme.a");
    let owner_b = key(61, "acme.b");
    let security_a = key(70, "security.a");
    let security_b = key(71, "security.b");
    let owners = [owner_a.key().clone(), owner_b.key().clone()];
    let security = [security_a.key().clone(), security_b.key().clone()];
    let text = yank_text("acme", "acme/ledger", &v("0.3.1"), Some((9, D)));
    let one = sign_note(&text, &[&owner_a]).unwrap();
    let both = sign_note(&text, &[&owner_a, &owner_b]).unwrap();
    assert!(check_yank(&one, &owners, 2).unwrap_err().contains("fewer than its threshold of 2"));
    assert_eq!(check_yank(&both, &owners, 2).unwrap().version, v("0.3.1"));
    assert!(check_yank(&one, &owners, 1).is_ok());
    let advisory = format!(
        "ita-advisory/1\nsource acme\nname acme/ledger\nprevious 9 {D}\nid ITA-1\nline 0.3\naffected 0.3.1\nfixed 0.3.2 {D}\nseverity high\nsummary An overflow\n"
    );
    let unsigned = sign_note(&advisory, &[&owner_a]).unwrap();
    assert!(check_advisory(&unsigned, &security, &owners, 2).is_err());
    let single = sign_note(&advisory, &[&security_a]).unwrap();
    let (_, signers) = check_advisory(&single, &security, &owners, 2).unwrap();
    assert!(signers.valid() && !signers.lifts_cooldown());
    let two = sign_note(&advisory, &[&security_a, &security_b]).unwrap();
    assert!(check_advisory(&two, &security, &owners, 2).unwrap().1.lifts_cooldown());
    let with_owner = sign_note(&advisory, &[&security_a, &owner_a, &owner_b]).unwrap();
    assert!(check_advisory(&with_owner, &security, &owners, 2).unwrap().1.lifts_cooldown());
}

#[test]
fn the_cooldown_is_a_wait_in_witness_time() {
    let hours = 3600;
    assert!(!cooled(100, 100 + 71 * hours, 72 * hours, false));
    assert!(cooled(100, 100 + 72 * hours, 72 * hours, false));
    assert!(cooled(100, 100, 72 * hours, true), "a threshold fix lifts it");
    // A checkpoint whose witness time is before the release waited nothing.
    assert!(!cooled(500, 100, 1, false));
    let mut state = BTreeMap::new();
    for (version, yanked, advised) in [("0.3.0", false, false), ("0.3.1", true, false), ("0.3.2", false, true)] {
        state.insert(
            v(version),
            ReleaseState {
                digest: D.to_owned(),
                logged_at: 5,
                yanked,
                advised: if advised { vec!["ITA-1".to_owned()] } else { Vec::new() },
                threshold_fix: false,
            },
        );
    }
    let read = releases(&state, &|version| *version == v("0.3.0"));
    assert!(read[0].yanked, "signed after a stated compromise");
    assert!(read[1].yanked && !read[1].advised);
    assert!(!read[2].yanked && read[2].advised);
}

// --- Audit and watch (rules 36, 36.2, 38, 41; U7014) ----------------------------------------

/// A registry of verified states, as the client gives them.
#[derive(Default)]
struct Fake {
    names: BTreeMap<String, NameState>,
    owners: BTreeMap<String, OwnerState>,
    asked: std::cell::Cell<usize>,
}

impl Registry for Fake {
    fn name(&self, _source: &str, name: &str, held: u64) -> Result<NameState, Failure> {
        self.asked.set(self.asked.get() + 1);
        let mut state = self.names.get(name).cloned().unwrap_or_else(|| NameState {
            name: name.to_owned(),
            ..NameState::default()
        });
        state.entries = state.entries.split_off((held as usize).min(state.entries.len()));
        Ok(state)
    }

    fn owner(&self, _source: &str, owner: &str, held: u64) -> Result<OwnerState, Failure> {
        self.asked.set(self.asked.get() + 1);
        let mut state = self.owners.get(owner).cloned().unwrap_or_else(|| OwnerState {
            owner: owner.to_owned(),
            ..OwnerState::default()
        });
        state.entries = state.entries.split_off((held as usize).min(state.entries.len()));
        Ok(state)
    }
}

fn entry(index: u64, format: &str) -> ChainEntry {
    ChainEntry {
        index,
        leaf: format!("sha256:{index:064x}"),
        format: format.to_owned(),
        text: format!("{format}\n"),
    }
}

fn release_state(yanked: bool, advised: &[&str]) -> ReleaseState {
    ReleaseState {
        digest: D.to_owned(),
        logged_at: 1,
        yanked,
        advised: advised.iter().map(|a| (*a).to_owned()).collect(),
        threshold_fix: false,
    }
}

fn locked(name: &str, version: &str, logged: u64) -> Locked {
    Locked {
        source: "acme".to_owned(),
        name: name.to_owned(),
        version: v(version),
        signed: Some("ed25519:acme:aa".to_owned()),
        logged: Some(logged),
        resolution: 100,
    }
}

#[test]
fn audit_reports_yanks_advisories_compromises_and_owner_events() {
    let mut fake = Fake::default();
    let mut ledger = NameState {
        source: "acme".to_owned(),
        name: "acme/ledger".to_owned(),
        ..NameState::default()
    };
    ledger.releases.insert(v("0.3.1"), release_state(true, &["ITA-1"]));
    ledger.releases.insert(v("0.3.2"), release_state(false, &[]));
    ledger.advisories.insert(
        "ITA-1".to_owned(),
        Advisory {
            name: "acme/ledger".to_owned(),
            id: "ITA-1".to_owned(),
            line: Line::Zero(3),
            affected: vec![v("0.3.1")],
            fixed: None,
            severity: "high".to_owned(),
            summary: "An overflow".to_owned(),
        },
    );
    ledger.entries = vec![entry(50, "ita-release/1"), entry(120, "ita-transfer/1")];
    fake.names.insert("acme/ledger".to_owned(), ledger);
    fake.owners.insert(
        "acme".to_owned(),
        OwnerState {
            owner: "acme".to_owned(),
            compromised: vec![("ed25519:acme:aa".to_owned(), 40)],
            pending_recoveries: vec![130],
            key_events: vec![60, 140],
            transfers: vec![(120, "acme/ledger".to_owned())],
            ..OwnerState::default()
        },
    );
    let packages = [locked("acme/ledger", "0.3.1", 45), locked("acme/ledger", "0.3.2", 30)];
    let result = audit::audit(&packages, &fake).unwrap();
    let of = |package: usize| -> Vec<Kind> {
        result
            .findings
            .iter()
            .filter(|f| f.package == package)
            .map(|f| f.kind.clone())
            .collect()
    };
    assert_eq!(
        of(0),
        [
            Kind::Yanked,
            Kind::Advisory {
                id: "ITA-1".to_owned(),
                severity: "high".to_owned(),
                summary: "An overflow".to_owned()
            },
            Kind::Transfer(120),
            Kind::Compromised { since: 40 },
            Kind::PendingRecovery(130),
            Kind::KeyChange(140),
        ]
    );
    // Logged before the compromise: not flagged for it; the owner's events still are.
    assert_eq!(
        of(1),
        [Kind::Transfer(120), Kind::PendingRecovery(130), Kind::KeyChange(140)]
    );
    // One state per name and per owner.
    assert_eq!(result.asked, 2);
    let finding = &result.findings[0];
    assert_eq!(finding.text(&packages[0]), "0.3.1 is yanked");
    // A state that cannot be verified fails the audit: nothing is called clean unverified.
    struct Down;
    impl Registry for Down {
        fn name(&self, _: &str, _: &str, _: u64) -> Result<NameState, Failure> {
            Err(Failure::Refused("a bad map proof".to_owned()))
        }
        fn owner(&self, _: &str, _: &str, _: u64) -> Result<OwnerState, Failure> {
            Err(Failure::Unavailable("no checkpoint".to_owned()))
        }
    }
    assert_eq!(
        audit::audit(&packages, &Down).unwrap_err(),
        (0, Failure::Refused("a bad map proof".to_owned()))
    );
}

/// The audit asks one state per name and owner and examines each entry once: its work per
/// package stays constant as the lock grows.
#[test]
fn audit_scales_linearly() {
    let mut per_package = Vec::new();
    for count in [100usize, 1000, 10_000] {
        let mut fake = Fake::default();
        let mut packages = Vec::new();
        for i in 0..count {
            let name = format!("acme/p{i}");
            let mut state = NameState {
                name: name.clone(),
                ..NameState::default()
            };
            state.releases.insert(v("1.0.0"), release_state(i % 2 == 0, &["A", "B"]));
            state.entries = vec![entry(1, "ita-release/1")];
            fake.names.insert(name.clone(), state);
            packages.push(locked(&name, "1.0.0", 1));
        }
        let result = audit::audit(&packages, &fake).unwrap();
        assert_eq!(result.asked, count + 1);
        assert_eq!(fake.asked.get(), count + 1);
        per_package.push(result.work as f64 / count as f64);
    }
    assert!(per_package.windows(2).all(|w| (w[1] - w[0]).abs() < 0.01), "{per_package:?}");
}

#[test]
fn watch_reports_new_entries_and_broken_promises() {
    let mut fake = Fake::default();
    let mut ledger = NameState {
        name: "acme/ledger".to_owned(),
        count: 3,
        ..NameState::default()
    };
    ledger.entries = vec![entry(1, "ita-release/1"), entry(2, "ita-release/1"), entry(3, "ita-yank/1")];
    fake.names.insert("acme/ledger".to_owned(), ledger);
    fake.owners.insert(
        "acme".to_owned(),
        OwnerState {
            owner: "acme".to_owned(),
            count: 2,
            checkpoint_time: 5000,
            packages: vec!["acme/ledger".to_owned()],
            entries: vec![entry(0, "ita-key/1"), entry(4, "ita-key/1")],
            ..OwnerState::default()
        },
    );
    let mut position = Position::start("acme", "acme");
    position.owner_count = 1;
    position.names.insert("acme/ledger".to_owned(), 2);
    assert_eq!(Position::parse(&position.encode()).unwrap(), position);
    assert!(Position::parse("ita-watch/1\nsource acme\nowner acme 01\n").is_err());
    let promise = |leaf: String, by: u64| Promise {
        source: "acme".to_owned(),
        leaf,
        by,
        document: String::new(),
    };
    let kept = promise(entry(3, "").leaf, 10);
    let late = promise(format!("sha256:{}", "cd".repeat(32)), 4000);
    let waiting = promise(format!("sha256:{}", "ef".repeat(32)), 9000);
    let report = watch(&fake, &position, &[], &[kept.clone(), late.clone(), waiting]).unwrap();
    let seen: Vec<(String, u64)> = report
        .entries
        .iter()
        .map(|(name, e)| (name.clone(), e.index))
        .collect();
    assert_eq!(seen, [("acme".to_owned(), 4), ("acme/ledger".to_owned(), 3)]);
    assert_eq!(report.kept, [kept]);
    assert_eq!(report.broken, [late]);
    assert_eq!(report.position.owner_count, 2);
    assert_eq!(report.position.names["acme/ledger"], 3);
    // A chain that shrank is refused.
    position.names.insert("acme/ledger".to_owned(), 9);
    assert!(matches!(watch(&fake, &position, &[], &[]), Err(Failure::Refused(_))));
}

// --- Bundles (rule 33) -----------------------------------------------------------------------

/// The archive of `acme/ledger` at `version`.
fn package_archive(version: &str) -> Vec<u8> {
    let mut files = files(&format!("ita acme/ledger\n  version {version}\n  edition 0.4\n"));
    una_project::content::archive(&mut files).unwrap()
}

/// A test registry `acme` with one release of `acme/ledger`, signed by its own root, log,
/// witness and publisher: the archive, the provenance, and what a lock says of it.
fn registry_release() -> (Vec<u8>, Vec<u8>, Expected) {
    use una_project::provenance::{Provenance, sha256_digest, signed_name};
    use una_project::tlog::merkle::{inclusion_proof, leaf_hash, root as tree_root};
    use una_project::tlog::note::encode_base64;

    let archive = package_archive("0.3.1");
    let tree = una_project::tree::VerifiedTree::from_archive(&archive).unwrap();
    let digest = tree.digest().to_owned();
    let root_key = key(10, "root.a");
    let log = key(20, "acme.example/log");
    let witness = key(30, "witness.a");
    let security = key(40, "security.a");
    let itauna = key(41, "itauna.a");
    let line = |k: &SigningKey| format!("key {} {}\n", k.key().name, encode_base64(&k.key().public));
    let mut text = "ita-root/1\nsource acme\nversion 1\nexpires 4000000000\nroot 1\n".to_owned();
    text.push_str(&line(&root_key));
    text.push_str(&format!("log {} {}\n", log.key().name, encode_base64(&log.key().public)));
    text.push_str("witnesses 1\n");
    text.push_str(&line(&witness));
    text.push_str("security 1\n");
    text.push_str(&line(&security));
    text.push_str("itauna 1\n");
    text.push_str(&line(&itauna));
    let root_document = sign_note(&text, &[&root_key]).unwrap();
    let manifest = sha256_digest(tree.file("ita.una").unwrap());
    let statement_text = format!(
        "ita-release/1\nsource acme\nname acme/ledger\nversion 0.3.1\nprevious none\ndigest {digest}\nmanifest {manifest}\nedition 0.4\nauthority none\ntrust none\n"
    );
    let publisher = key(50, "acme.publisher");
    let statement = sign_note(&statement_text, &[&publisher]).unwrap();
    let leaves = [leaf_hash(b"an earlier entry"), leaf_hash(statement.as_bytes())];
    let top = tree_root(&leaves);
    let checkpoint_text = format!("{}\n2\n{}\n", log.key().name, encode_base64(&top));
    let mut checkpoint = sign_note(&checkpoint_text, &[&log]).unwrap();
    checkpoint.push_str(&cosign_line(&witness, &checkpoint_text, 1_900_000_000));
    let provenance = Provenance {
        roots: vec![root_document.clone()],
        checkpoint,
        statement,
        index: 1,
        proof: inclusion_proof(1, &leaves),
        key: publisher.key().clone(),
    };
    let hex: String = top.iter().map(|b| format!("{b:02x}")).collect();
    let expected = Expected {
        source: "acme".to_owned(),
        name: "acme/ledger".to_owned(),
        version: "0.3.1".to_owned(),
        digest,
        signed: signed_name("acme/ledger", &publisher.key().public),
        logged: 1,
        root: una_project::tlog::root::digest(&root_document),
        checkpoint_size: 2,
        checkpoint_hash: format!("sha256:{hex}"),
    };
    (archive, provenance.encode(), expected)
}

/// A cosignature/v1 line by `witness` of a checkpoint's text at `time`.
fn cosign_line(witness: &SigningKey, text: &str, time: u64) -> String {
    use ed25519_dalek::Signer as _;
    use una_project::tlog::note::encode_base64;
    // The test signs with the same seed the key was made from.
    let secret = ed25519_dalek::SigningKey::from_bytes(&[30; 32]);
    assert_eq!(secret.verifying_key().to_bytes(), witness.key().public);
    let message = format!("cosignature/v1\ntime {time}\n{text}");
    let mut bytes = witness.key().cosignature_id().to_vec();
    bytes.extend_from_slice(&time.to_be_bytes());
    bytes.extend_from_slice(&secret.sign(message.as_bytes()).to_bytes());
    format!("\u{2014} {} {}\n", witness.key().name, encode_base64(&bytes))
}

#[test]
fn a_bundle_unbundles_verified_into_itaoca() {
    let (archive, provenance, expected) = registry_release();
    let packed = bundle::Packed {
        archive: archive.clone(),
        provenance: provenance.clone(),
    };
    let entries = bundle::entries(
        &[(expected.clone(), packed)],
        vec![("state/acme/ledger-chain".to_owned(), b"opaque".to_vec())],
    )
    .unwrap();
    let bytes = bundle::write(entries).unwrap();
    assert_eq!(bundle::read(&bytes).unwrap().entries.len(), 3);
    let store = Itaoca {
        root: scratch("unbundle"),
    };
    let done = bundle::unbundle(&bytes, &store, std::slice::from_ref(&expected)).unwrap();
    assert_eq!(done.stored, ["acme/ledger 0.3.1"]);
    assert_eq!(done.unused, ["state/acme/ledger-chain"]);
    // What was stored loads verified, as every load does.
    assert!(una_project::vendor::stored(&store, &expected).unwrap().is_some());
}

/// A bundle is trusted for nothing: a tampered archive, a tampered provenance, or a changed
/// byte of the bundle itself, is refused, and nothing is written.
#[test]
fn a_tampered_bundle_is_refused_and_writes_nothing() {
    let (archive, provenance, expected) = registry_release();
    let write = |archive: &[u8], provenance: &[u8]| {
        bundle::write(
            bundle::entries(
                &[(
                    expected.clone(),
                    bundle::Packed {
                        archive: archive.to_vec(),
                        provenance: provenance.to_vec(),
                    },
                )],
                Vec::new(),
            )
            .unwrap(),
        )
        .unwrap()
    };
    // Another package's archive under the digest's path: its digest is not the lock's.
    let other = package_archive("0.3.2");
    let store = Itaoca {
        root: scratch("unbundle-tampered"),
    };
    let (at, why) = bundle::unbundle(&write(&other, &provenance), &store, std::slice::from_ref(&expected))
        .unwrap_err();
    assert_eq!(at, Some(0));
    assert!(why.contains("not the lock's"), "{why}");
    // A provenance whose statement was edited: its signature no longer verifies.
    let edited = String::from_utf8(provenance.clone())
        .unwrap()
        .replace("authority none", "authority nope");
    let (_, why) = bundle::unbundle(&write(&archive, edited.as_bytes()), &store, std::slice::from_ref(&expected))
        .unwrap_err();
    assert!(why.contains("provenance"), "{why}");
    // A byte of the bundle changed: its listing's hash refuses it.
    let mut bytes = write(&archive, &provenance);
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let (at, _) = bundle::unbundle(&bytes, &store, std::slice::from_ref(&expected)).unwrap_err();
    assert_eq!(at, None);
    // Nothing was written by any of them.
    assert!(!store.contains(&expected.digest).unwrap());
    // A path the bundle does not name is refused when writing and reading.
    assert!(bundle::write(vec![("../x".to_owned(), Vec::new())]).is_err());
    assert!(bundle::write(vec![("packages/zz".to_owned(), Vec::new())]).is_err());
}

/// Reading a bundle is one pass: its work grows with its bytes, entry by entry.
#[test]
fn bundles_read_in_linear_time() {
    let mut per_entry = Vec::new();
    for count in [100usize, 1000, 10_000] {
        let entries: Vec<(String, Vec<u8>)> = (0..count)
            .map(|i| (format!("state/acme/n{i}"), vec![b'x'; 64]))
            .collect();
        let bytes = bundle::write(entries).unwrap();
        let started = std::time::Instant::now();
        let read = bundle::read(&bytes).unwrap();
        let elapsed = started.elapsed().as_nanos().max(1) as f64;
        assert_eq!(read.entries.len(), count);
        per_entry.push(elapsed / count as f64);
    }
    // Time per entry may vary with caches, never by the factor quadratic growth would give.
    assert!(per_entry[2] < per_entry[0] * 20.0 + 50_000.0, "{per_entry:?}");
}
