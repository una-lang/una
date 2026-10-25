//! The registry client's side of publishing, yanks, advisories, audit, watch, mirrors and
//! bundles (ADR 0122 rules 33, 36.2 and 41 to 43, `spec/30 §15`).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "support/registry.rs"]
mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use support::{NOW, TestRegistry, hash_hex, keypair};
use una_project::client::{Point, Registry};
use una_project::content::File;
use una_project::itaoca::Itaoca;
use una_project::mirrors::{Attempt, first_verified};
use una_project::provenance::{Expected, signed_name};
use una_project::registry::audit::{self, Facts, Kind, Locked};
use una_project::registry::bundle::{self, Memory, Recording};
use una_project::registry::entries::{check_advisory, check_yank, cooled, releases, yank_text};
use una_project::registry::keyfile::{SigningKey, owner_keys, sign_note};
use una_project::registry::submit::{self, NameState, Promise, broken, check_promise};
use una_project::registry::watch::{Position, Seen, watch};
use una_project::tlog::note::parse_note;
use una_project::tlog::release::parse_statement;
use una_project::tlog::state::{Advisory, ReleaseState};
use una_project::transport::{Directory, Transport};
use una_project::vendor::Problem;
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

// --- Writing through a transport, and mirrors tried in order (rules 40 and 43) --------------

#[test]
fn the_directory_transport_writes_once_and_reads_back() {
    let dir = scratch("transport");
    let transport = Directory::new(&dir);
    transport.put("submit/abc/entry", b"one").unwrap();
    assert_eq!(
        transport.get("submit/abc/entry", 10).unwrap(),
        Some(b"one".to_vec())
    );
    // The same bytes again are accepted; other bytes are refused.
    transport.put("submit/abc/entry", b"one").unwrap();
    assert!(transport.put("submit/abc/entry", b"two").is_err());
    assert_eq!(transport.get("submit/none", 10), Ok(None));
    for path in ["../x", "a/../b", "/abs", "a//b", "checkpoint"] {
        assert!(transport.put(path, b"x").is_err(), "{path}");
    }
    assert!(transport.get("submit/abc/entry", 2).is_err(), "over the limit");
    // An https location has no transport yet: it never writes.
    let https = una_project::transport::Unavailable {
        url: "https://ita.acme.example".to_owned(),
    };
    assert!(
        https
            .put("submit/abc/entry", b"one")
            .unwrap_err()
            .contains("no https transport yet")
    );
}

/// Trying m places costs O(m): one verification each, never more, and the first that verifies
/// is taken after every bad one is skipped and named.
#[test]
fn trying_mirrors_scales_linearly() {
    for m in [10usize, 100, 1000] {
        let places: Vec<(String, Result<&dyn Transport, String>)> = (0..m)
            .map(|i| (format!("file:///m{i}"), Err("unused".to_owned())))
            .collect();
        let mut seen = 0usize;
        let fetched = first_verified(&places, &mut |_| -> Result<(), Attempt> { Ok(()) });
        assert!(fetched.value.is_none(), "no place has a transport");
        assert_eq!(fetched.unavailable.len(), m);
        let memory = Memory::default();
        let places: Vec<(String, Result<&dyn Transport, String>)> = (0..m)
            .map(|i| (format!("file:///m{i}"), Ok(&memory as &dyn Transport)))
            .collect();
        let fetched = first_verified(&places, &mut |_| {
            seen += 1;
            if seen == m {
                Ok(seen)
            } else {
                Err(Attempt::Refused("a copy that does not verify".to_owned()))
            }
        });
        assert_eq!(fetched.value, Some((m, format!("file:///m{}", m - 1))));
        assert_eq!(fetched.refused.len(), m - 1);
        assert_eq!(seen, m);
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
        SigningKey::parse("ita-keyfile-encrypted/1\n...\n")
            .unwrap_err()
            .contains("passphrase")
    );
    assert!(SigningKey::parse("ita-keyfile/1\nname a+b\nsecret AAAA\n").is_err());
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
    let transport = Directory::new(&dir);
    let written = submit::send(&transport, &submission).unwrap();
    assert_eq!(written.len(), 2);
    assert!(written[0].ends_with("/archive") && written[1].ends_with("/entry"));
    assert_eq!(
        transport.get(&written[1], 1 << 16).unwrap(),
        Some(submission.entry.as_bytes().to_vec())
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

fn locked(name: &str, version: &str) -> Locked {
    Locked {
        source: "acme".to_owned(),
        name: name.to_owned(),
        version: v(version),
        logged: Some(45),
        resolution: 100,
    }
}

fn advisory(id: &str) -> Advisory {
    Advisory {
        source: "acme".to_owned(),
        name: "acme/ledger".to_owned(),
        previous: None,
        id: id.to_owned(),
        line: Line::Zero(3),
        affected: vec![v("0.3.1")],
        fixed: None,
        severity: "high".to_owned(),
        summary: "An overflow".to_owned(),
    }
}

#[test]
fn audit_reports_yanks_advisories_compromises_and_owner_events() {
    let packages = [locked("acme/ledger", "0.3.1"), locked("acme/ledger", "0.3.2")];
    let mut facts_of = |package: &Locked| -> Result<Facts, String> {
        Ok(if package.version == v("0.3.1") {
            Facts {
                yanked: true,
                advisories: vec![advisory("ITA-1")],
                compromised_since: Some(40),
                pending: vec![130],
                key_changes: vec![140],
            }
        } else {
            Facts {
                pending: vec![130],
                ..Facts::default()
            }
        })
    };
    let findings = audit::audit(&packages, &mut facts_of).unwrap();
    let of = |package: usize| -> Vec<Kind> {
        findings
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
            Kind::Compromised { since: 40 },
            Kind::PendingRecovery(130),
            Kind::KeyChange(140),
        ]
    );
    assert_eq!(of(1), [Kind::PendingRecovery(130)]);
    assert_eq!(findings[0].text(&packages[0]), "0.3.1 is yanked");
    // Facts that cannot be had verified fail the audit: nothing is called clean unverified.
    let mut down = |_: &Locked| -> Result<Facts, String> { Err("a bad map proof".to_owned()) };
    assert_eq!(
        audit::audit(&packages, &mut down).unwrap_err(),
        (0, "a bad map proof".to_owned())
    );
}

/// The audit asks once per package and its findings are linear in its facts.
#[test]
fn audit_scales_linearly() {
    for count in [100usize, 1000, 10_000] {
        let packages: Vec<Locked> = (0..count)
            .map(|i| locked(&format!("acme/p{i}"), "1.0.0"))
            .collect();
        let mut asked = 0usize;
        let mut facts_of = |_: &Locked| -> Result<Facts, String> {
            asked += 1;
            Ok(Facts {
                yanked: true,
                advisories: vec![advisory("A"), advisory("B")],
                ..Facts::default()
            })
        };
        let findings = audit::audit(&packages, &mut facts_of).unwrap();
        assert_eq!(asked, count);
        assert_eq!(findings.len(), 3 * count);
    }
}

fn seen(index: u64, format: &str) -> Seen {
    Seen {
        index,
        leaf: format!("sha256:{index:064x}"),
        format: format.to_owned(),
    }
}

#[test]
fn watch_reports_new_entries_and_broken_promises() {
    let chains: BTreeMap<String, Vec<Seen>> = [
        (
            "owner:acme".to_owned(),
            vec![seen(0, "ita-owner/1"), seen(4, "ita-key/1")],
        ),
        (
            "acme/ledger".to_owned(),
            vec![
                seen(1, "ita-release/1"),
                seen(2, "ita-release/1"),
                seen(3, "ita-yank/1"),
            ],
        ),
    ]
    .into_iter()
    .collect();
    let mut chain = |name: &str| -> Result<Vec<Seen>, String> {
        Ok(chains.get(name).cloned().unwrap_or_default())
    };
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
    let kept = promise(seen(3, "").leaf, 10);
    let late = promise(format!("sha256:{}", "cd".repeat(32)), 4000);
    let waiting = promise(format!("sha256:{}", "ef".repeat(32)), 9000);
    let report = watch(
        &mut chain,
        5000,
        &position,
        &[],
        &[kept.clone(), late.clone(), waiting],
    )
    .unwrap();
    let new: Vec<(String, u64)> = report
        .entries
        .iter()
        .map(|(name, e)| (name.clone(), e.index))
        .collect();
    assert_eq!(new, [("acme".to_owned(), 4), ("acme/ledger".to_owned(), 3)]);
    assert_eq!(report.kept, [kept]);
    assert_eq!(report.broken, [late]);
    assert_eq!(report.position.owner_count, 2);
    assert_eq!(report.position.names["acme/ledger"], 3);
    // A chain that shrank is refused.
    position.names.insert("acme/ledger".to_owned(), 9);
    assert!(watch(&mut chain, 5000, &position, &[], &[]).is_err());
}

// --- Yanks and advisories read by the client (rule 41) --------------------------------------

/// A yank signed by the owner and an advisory signed by a security key, logged after the
/// releases they name, mark them; resolution reads them through the client's state.
#[test]
fn the_client_reads_yanks_and_advisories() {
    let mut test = TestRegistry::new("acme");
    let owner = keypair(60, "acme.laptop");
    test.register("acme", &owner);
    test.release("acme/ledger", "0.3.0", "none", "none", &[&owner]);
    let (_, fix_digest) = test.release("acme/ledger", "0.3.1", "none", "none", &[&owner]);
    let yank = format!(
        "ita-yank/1\nsource acme\nname acme/ledger\nversion 0.3.0\n{}",
        test.previous("acme/ledger")
    );
    let document = support::signed(&yank, &[&owner]);
    test.append("acme/ledger", &document);
    let advisory = format!(
        "ita-advisory/1\nsource acme\nname acme/ledger\n{}\nid ITA-1\nline 0.3\naffected 0.3.0\nfixed 0.3.1 {fix_digest}\nseverity high\nsummary An overflow",
        test.previous("acme/ledger")
    );
    let security = (
        ed25519_dalek::SigningKey::from_bytes(&test.keys.security.0.to_bytes()),
        test.keys.security.1.clone(),
    );
    let document = support::signed(&advisory, &[&security, &owner]);
    test.append("acme/ledger", &document);
    test.checkpoint(NOW);
    let memory = Memory {
        source: "acme".to_owned(),
        files: test.files.clone(),
    };
    let mut client = Registry::open(
        "acme",
        &memory,
        &test.pin(),
        &Point::Latest { now: NOW + 60 },
        None,
    )
    .unwrap();
    let marks = client.marks("acme/ledger").unwrap();
    assert!(marks.yanked.contains_key(&v("0.3.0")));
    assert_eq!(marks.advised[&v("0.3.0")], ["ITA-1"]);
    assert!(marks.threshold_fix.contains(&v("0.3.1")), "two signers lift the cooldown");
    let state = client.state("acme/ledger", Line::Zero(3)).unwrap();
    let old = state.iter().find(|r| r.version == v("0.3.0")).unwrap();
    assert!(old.yanked && old.advised);
    // The state a publisher reads names the chain's head and the yank.
    let name = NameState::read(&mut client, "acme/ledger").unwrap();
    assert!(name.yanked.contains(&v("0.3.0")));
    assert_eq!(name.head.as_ref().unwrap().0, 4, "the advisory is the head");
    // A yank signed by a key that is not the owner's refuses the name.
    let mut forged = TestRegistry::new("acme");
    forged.register("acme", &owner);
    forged.release("acme/ledger", "0.3.0", "none", "none", &[&owner]);
    let stranger = keypair(61, "stranger");
    let yank = format!(
        "ita-yank/1\nsource acme\nname acme/ledger\nversion 0.3.0\n{}",
        forged.previous("acme/ledger")
    );
    let document = support::signed(&yank, &[&stranger]);
    forged.append("acme/ledger", &document);
    forged.checkpoint(NOW);
    let memory = Memory {
        source: "acme".to_owned(),
        files: forged.files.clone(),
    };
    let mut client = Registry::open(
        "acme",
        &memory,
        &forged.pin(),
        &Point::Latest { now: NOW + 60 },
        None,
    )
    .unwrap();
    assert!(client.state("acme/ledger", Line::Zero(3)).is_err());
}

/// A yank that counts only with a key compromised before it is void, as such a release is: it
/// is ignored, never applied and never a refusal of the name.
#[test]
fn a_yank_signed_after_its_keys_compromise_is_void() {
    let mut test = TestRegistry::new("acme");
    let first = keypair(60, "acme.laptop");
    let second = keypair(62, "acme.desktop");
    test.register("acme", &first);
    test.owner_event(
        "acme",
        &format!("add {}", support::key_text(&second.1)),
        1,
        &[&first, &second],
    );
    let (good, _) = test.release("acme/ledger", "0.3.0", "none", "none", &[&first]);
    let yank = format!(
        "ita-yank/1\nsource acme\nname acme/ledger\nversion 0.3.0\n{}",
        test.previous("acme/ledger")
    );
    let document = support::signed(&yank, &[&first]);
    test.append("acme/ledger", &document);
    test.owner_event(
        "acme",
        &format!("revoke {} since {good}", first.1.name),
        1,
        &[&second],
    );
    test.checkpoint(NOW);
    let memory = Memory {
        source: "acme".to_owned(),
        files: test.files.clone(),
    };
    let mut client = Registry::open(
        "acme",
        &memory,
        &test.pin(),
        &Point::Latest { now: NOW + 60 },
        None,
    )
    .unwrap();
    let marks = client.marks("acme/ledger").unwrap();
    assert!(marks.yanked.is_empty(), "{marks:?}");
    let state = client.state("acme/ledger", Line::Zero(3)).unwrap();
    assert!(!state[0].yanked);
}

#[test]
fn header_advice_names_public_headers_removed_or_changed() {
    let earlier = files("ita acme/ledger\n  version 0.3.0\n  edition 0.4\n");
    let mut new = files("ita acme/ledger\n  version 0.3.1\n  edition 0.4\n");
    assert!(submit::header_advice(&earlier, &new).is_empty());
    new[1].bytes = b"module ledger\n\npub total(n: Int) -> Int = n\n".to_vec();
    let advice = submit::header_advice(&earlier, &new);
    assert_eq!(advice.len(), 1);
    assert!(advice[0].contains("pub total() -> Int"), "{advice:?}");
}

// --- Bundles (rule 33) -----------------------------------------------------------------------

/// The archive of `acme/ledger` at `version`.
fn package_archive(version: &str) -> Vec<u8> {
    let mut files = files(&format!("ita acme/ledger\n  version {version}\n  edition 0.4\n"));
    una_project::content::archive(&mut files).unwrap()
}

/// A test registry `acme` with one release of `acme/ledger`, and what a bundle of it holds:
/// the package, its provenance, what a lock says of it, and the registry's files the client
/// read at the lock's resolution point.
struct Bundled {
    archive: Vec<u8>,
    provenance: Vec<u8>,
    expected: Expected,
    files: BTreeMap<String, Vec<u8>>,
    test: TestRegistry,
}

fn bundled() -> Bundled {
    let mut test = TestRegistry::new("acme");
    let owner = keypair(60, "acme.laptop");
    test.register("acme", &owner);
    let (index, digest) = test.release("acme/ledger", "0.3.1", "none", "none", &[&owner]);
    let size = test.checkpoint(NOW);
    let served = Memory {
        source: "acme".to_owned(),
        files: test.files.clone(),
    };
    let recording = Recording::new(&served);
    let checkpoint = parse_note(std::str::from_utf8(&test.files[&format!("checkpoint/{size}")]).unwrap())
        .unwrap();
    let root = una_project::tlog::note::parse_checkpoint(&checkpoint.text)
        .unwrap()
        .root;
    let point = Point::At {
        size,
        hash: hash_hex(&root),
    };
    let mut client = Registry::open("acme", &recording, &test.pin(), &point, None).unwrap();
    let release = client.release("acme/ledger", &v("0.3.1")).unwrap();
    client.marks("acme/ledger").unwrap();
    let provenance = client.provenance(&release, &owner.1).unwrap().encode();
    let archive = served.files[&format!("blobs/sha256/{}", digest.strip_prefix("sha256:").unwrap())].clone();
    drop(client);
    let files = recording.seen.into_inner();
    let expected = Expected {
        source: "acme".to_owned(),
        name: "acme/ledger".to_owned(),
        version: "0.3.1".to_owned(),
        digest,
        signed: signed_name("acme/ledger", &owner.1.public),
        logged: index,
        root: test.pin(),
        checkpoint_size: size,
        checkpoint_hash: hash_hex(&root),
    };
    Bundled {
        archive,
        provenance,
        expected,
        files,
        test,
    }
}

fn bundle_of(b: &Bundled, archive: &[u8], provenance: &[u8], files: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    bundle::write(
        bundle::entries(
            &[(
                b.expected.clone(),
                bundle::Packed {
                    archive: archive.to_vec(),
                    provenance: provenance.to_vec(),
                },
            )],
            &[("acme".to_owned(), files.clone())],
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn a_bundle_unbundles_verified_into_itaoca() {
    let b = bundled();
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &b.files);
    let read = bundle::read(&bytes).unwrap();
    assert!(read.entries.keys().any(|path| path.starts_with("registry/acme/root/")));
    let store = Itaoca {
        root: scratch("unbundle"),
    };
    let done = bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected)).unwrap();
    assert_eq!(done.stored, ["acme/ledger 0.3.1"]);
    assert!(done.unused.is_empty(), "{:?}", done.unused);
    // What was stored loads verified, as every load does, and the checkpoint is the floor.
    assert!(una_project::vendor::stored(&store, &b.expected).unwrap().is_some());
    assert!(store.load_checkpoint("acme").unwrap().is_some());
    assert_eq!(store.load_root_version("acme").unwrap(), Some(1));
}

/// A bundle is trusted for nothing: a tampered archive, provenance or registry file, or a
/// changed byte of the bundle itself, is refused, and nothing is written.
#[test]
fn a_tampered_bundle_is_refused_and_writes_nothing() {
    let b = bundled();
    let store = Itaoca {
        root: scratch("unbundle-tampered"),
    };
    let refused = |bytes: &[u8]| -> (Option<usize>, String) {
        match bundle::unbundle(bytes, &store, std::slice::from_ref(&b.expected)).unwrap_err() {
            (at, Problem::Refused(why)) => (at, why),
            (at, Problem::Absent(why)) => (at, format!("absent: {why}")),
        }
    };
    // Another package's archive under the digest's path: its digest is not the lock's.
    let other = package_archive("0.3.2");
    let (at, why) = refused(&bundle_of(&b, &other, &b.provenance, &b.files));
    assert_eq!(at, Some(0));
    assert!(why.contains("not the lock's"), "{why}");
    // A provenance whose statement was edited: its signature no longer verifies.
    let edited = String::from_utf8(b.provenance.clone())
        .unwrap()
        .replace("authority none", "authority nope");
    let (_, why) = refused(&bundle_of(&b, &b.archive, edited.as_bytes(), &b.files));
    assert!(why.contains("provenance"), "{why}");
    // A registry file changed in the bundle's copy: the client refuses it.
    let mut files = b.files.clone();
    let tile = files
        .keys()
        .find(|path| path.starts_with("tile/entries/"))
        .unwrap()
        .clone();
    let entry = files.get_mut(&tile).unwrap();
    let middle = entry.len() / 2;
    entry[middle] ^= 1;
    let (at, why) = refused(&bundle_of(&b, &b.archive, &b.provenance, &files));
    assert_eq!(at, Some(0), "{why}");
    // A bundle without the registry's files: its state cannot be verified.
    let (_, why) = refused(&bundle_of(&b, &b.archive, &b.provenance, &BTreeMap::new()));
    assert!(why.contains("absent") && why.contains("no copy of the registry"), "{why}");
    // A byte of the bundle changed: its listing's hash refuses it.
    let mut bytes = bundle_of(&b, &b.archive, &b.provenance, &b.files);
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let (at, _) = refused(&bytes);
    assert_eq!(at, None);
    // Nothing was written by any of them.
    assert!(!store.contains(&b.expected.digest).unwrap());
    assert!(store.load_checkpoint("acme").unwrap().is_none());
    // A path the bundle does not name is refused when writing.
    assert!(bundle::write(vec![("../x".to_owned(), Vec::new())]).is_err());
    assert!(bundle::write(vec![("packages/zz".to_owned(), Vec::new())]).is_err());
}

/// Unbundling respects the root version stored as a floor (rule 39): a bundle whose root chain
/// stops below it is refused, and nothing is written.
#[test]
fn unbundling_respects_the_root_version_floor() {
    let b = bundled();
    let store = Itaoca {
        root: scratch("unbundle-root-floor"),
    };
    store.store_root_version("acme", 2).unwrap();
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &b.files);
    let (_, problem) = bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected))
        .unwrap_err();
    assert!(matches!(problem, Problem::Refused(_)), "{problem:?}");
    assert!(!store.contains(&b.expected.digest).unwrap());
    assert_eq!(store.load_root_version("acme").unwrap(), Some(2));
    assert!(store.load_checkpoint("acme").unwrap().is_none());
}

/// Unbundling never moves the checkpoint stored for a source backwards (rule 39).
#[test]
fn unbundling_keeps_a_larger_floor() {
    let mut b = bundled();
    let store = Itaoca {
        root: scratch("unbundle-floor"),
    };
    // The client has seen a larger checkpoint of the same log.
    let owner = keypair(60, "acme.laptop");
    b.test.release("acme/ledger", "0.3.2", "none", "none", &[&owner]);
    let larger = b.test.checkpoint(NOW + 10);
    let document = String::from_utf8(b.test.files[&format!("checkpoint/{larger}")].clone()).unwrap();
    store.store_checkpoint("acme", &document).unwrap();
    // The bundle's files hold the larger log's tiles too, so its consistency can be proven.
    let mut files = b.files.clone();
    for (path, bytes) in &b.test.files {
        if path.starts_with("tile/") {
            files.insert(path.clone(), bytes.clone());
        }
    }
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &files);
    let result = bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected));
    let stored = store.load_checkpoint("acme").unwrap().unwrap();
    assert_eq!(stored, document, "{result:?}");
}

/// Reading a bundle is one pass: its work grows with its bytes, entry by entry.
#[test]
fn bundles_read_in_linear_time() {
    let mut per_entry = Vec::new();
    for count in [100usize, 1000, 10_000] {
        let entries: Vec<(String, Vec<u8>)> = (0..count)
            .map(|i| (format!("registry/acme/tile/n{i}"), vec![b'x'; 64]))
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

// --- The owner's key files, the source's URL and git (rules 8, 38 and 43) --------------------

/// Writes `key` as the file `name` of `dir`, readable by its owner only.
fn write_key(dir: &std::path::Path, name: &str, text: &str) {
    let file = dir.join(name);
    std::fs::write(&file, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[test]
fn the_owners_registered_keys_sign_at_least_its_threshold() {
    let dir = scratch("keys");
    let a = key(80, "acme.a");
    let b = key(81, "acme.b");
    let unregistered = key(82, "acme.c");
    let other = key(83, "other.a");
    write_key(&dir, "acme-4f9c.key", &a.encode());
    write_key(&dir, "acme-0a.key", &b.encode());
    write_key(&dir, "acme-ff.key", &unregistered.encode());
    write_key(&dir, "other-1.key", &other.encode());
    // A file that is not named as a key file of the owner is not read.
    write_key(&dir, "acme-notes.txt", "not a key");
    let registered = [a.key().clone(), b.key().clone()];
    let chosen = owner_keys(&dir, "acme", &registered, 2).unwrap();
    let names: Vec<&str> = chosen.iter().map(|k| k.key().name.as_str()).collect();
    assert_eq!(names, ["acme.b", "acme.a"], "by file name, registered keys only");
    let refusal = owner_keys(&dir, "acme", &registered, 3).unwrap_err();
    assert!(refusal.contains("fewer than its threshold of 3"), "{refusal}");
    // A key registered under another name does not sign: its line would carry another key hash.
    let renamed = [una_project::tlog::note::Key {
        name: "acme.renamed".to_owned(),
        public: a.key().public,
    }];
    assert!(owner_keys(&dir, "acme", &renamed, 1).is_err());
    // An encrypted key file of the owner refuses the whole choice, never skipped.
    write_key(&dir, "acme-1.key", "ita-keyfile-encrypted/1\nopaque\n");
    let refusal = owner_keys(&dir, "acme", &registered, 1).unwrap_err();
    assert!(refusal.contains("not implemented yet"), "{refusal}");
    std::fs::remove_file(dir.join("acme-1.key")).unwrap();
    // So does one that others can read.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("acme-0a.key"), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        let refusal = owner_keys(&dir, "acme", &registered, 1).unwrap_err();
        assert!(refusal.contains("readable by its owner only"), "{refusal}");
    }
    assert!(owner_keys(&dir.join("none"), "acme", &registered, 1).is_err());
}

#[test]
fn a_git_entry_in_the_directory_or_an_ancestor_is_found() {
    let base = scratch("git");
    let package = base.join("a/b");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(base.join(".git"), "gitdir: elsewhere\n").unwrap();
    assert_eq!(submit::git_entry(&package), Some(base.join(".git")));
    std::fs::create_dir(package.join(".git")).unwrap();
    assert_eq!(submit::git_entry(&package), Some(package.join(".git")));
}

#[test]
fn a_published_source_is_the_one_its_workspace_declares() {
    let workspace = scratch("workspace");
    let package = workspace.join("pkg");
    std::fs::create_dir_all(&package).unwrap();
    assert!(
        submit::declared_source(&package, "acme")
            .unwrap_err()
            .contains("declared by no `itaty.una`")
    );
    std::fs::write(
        workspace.join("itaty.una"),
        format!(
            "itaty\n  members\n    \"pkg\"\n\nsources\n  acme \"https://ita.acme.example\"\n    root \"sha256:{}\"\n    owners acme\n",
            "5e".repeat(32)
        ),
    )
    .unwrap();
    assert_eq!(
        submit::declared_source(&package, "acme").unwrap().url.value,
        "https://ita.acme.example"
    );
    assert!(submit::declared_source(&package, "other").unwrap_err().contains("not a source"));
    assert!(
        submit::declared_source(&package, "itaty")
            .unwrap_err()
            .contains("no https transport yet")
    );
}
