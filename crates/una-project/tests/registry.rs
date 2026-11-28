//! The registry client's side of publishing, yanks, advisories, audit, watch, mirrors and
//! bundles (ADR 0122 rules 33, 36.2 and 41 to 43, `spec/30 §15`).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "support/registry.rs"]
mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use support::{NOW, TestRegistry, hash_hex, keypair, walked, walked_root};
use una_project::client::{Misbehavior, Omission, Point, Registry};
use una_project::content::File;
use una_project::itaoca::{Itaoca, registry_id};
use una_project::mirrors::{Attempt, first_verified};
use una_project::provenance::{Expected, signed_name};
use una_project::registry::audit::{self, Delay, Facts, Kind, Locked};
use una_project::registry::bundle::{self, Failure, Memory, Recording};
use una_project::registry::keyfile::{SigningKey, owner_keys, sign_note};
use una_project::registry::submit::{self, NameState, PROMISE_WINDOW, Promise, check_promise};
use una_project::registry::watch::{
    MAX_JUDGED, MAX_SETTLED, Position, Seen, Stored, Submitted, Verdict, assess,
    encode_stored_root, judge, parse_stored_root, to_judge, watch,
};
use una_project::tlog::note::parse_note;
use una_project::tlog::owner::{RULE, Unacceptable};
use una_project::tlog::release::parse_statement;
use una_project::tlog::state::Advisory;
use una_project::tlog::state::{Signers, parse_advisory};
use una_project::transport::{Directory, Fallback, Transport};
use una_project::version::{Line, Version};
use una_syntax::ita::{Text, parse_manifest};

const D: &str = "sha256:3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b";

/// A first promise stored under the root whose log key is `log`'s.
fn stored_under(bytes: Vec<u8>, log: &SigningKey) -> Stored {
    Stored {
        bytes,
        log: Ok(log.key().clone()),
    }
}

/// No place behind a bundle's copy: the bundle alone serves.
fn no_place(_: &str) -> Vec<Box<dyn Transport>> {
    Vec::new()
}

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
    for path in [
        "../x",
        "a/../b",
        "/abs",
        "a//b",
        "checkpoint",
        "checkpoint/latest",
        "checkpoint/7",
    ] {
        assert!(transport.put(path, b"x").is_err(), "{path}");
    }
    assert!(
        transport.get("submit/abc/entry", 2).is_err(),
        "over the limit"
    );
    // No staged copy is left behind, whether the link succeeded or found the file there.
    let staged = std::fs::read_dir(dir.join("submit/abc"))
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with('.')
        })
        .count();
    assert_eq!(staged, 0);
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
        let (manifest, _) = parse_manifest(Text::Written(text));
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
    assert!(
        refusal(
            &published("0.3.1", "\nneeds\n  util from path \"../util\"\n"),
            &first,
            &[&publisher]
        )
        .contains("path dependency")
    );
    assert!(
        refusal(
            &published("0.3.1", "\nneeds\n  json 2.1\n"),
            &first,
            &[&publisher]
        )
        .contains("needs only packages of `acme`")
    );
    assert!(refusal(&published("0.3", ""), &first, &[&publisher]).contains("full version"));
    assert!(refusal(&published("0.3.1", ""), &first, &[]).contains("no signing key"));
    // A state of another name is not used.
    let other = NameState {
        name: "acme/other".to_owned(),
        ..first_state()
    };
    assert!(
        refusal(&published("0.3.1", ""), &other, &[&publisher]).contains("not of `acme/ledger`")
    );
    // An earlier release of the line that needed less: authority grows only in a major.
    let earlier = format!(
        "ita-release/1\nsource acme\nname acme/ledger\nversion 0.3.0\nprevious none\ndigest {D}\nmanifest {D}\nedition 0.4\nauthority Http[get]\ntrust none\n"
    );
    let mut state = first_state();
    state.head = Some((7, D.to_owned()));
    state
        .statements
        .insert(v("0.3.0"), parse_statement(&earlier).unwrap());
    assert!(refusal(&published("0.3.1", ""), &state, &[&publisher]).contains("Http[post]"));
    assert!(refusal(&published("0.3.0", ""), &state, &[&publisher]).contains("already released"));
    // Rule 26 compares in log order, against the highest release of the line, even above the new
    // version: 0.3.0 needed `Http[post]`, the later 0.3.5 did not, so 0.3.2 may not need it.
    let mut ordered = first_state();
    let statement_of = |version: &str, authority: &str| {
        parse_statement(&format!(
            "ita-release/1\nsource acme\nname acme/ledger\nversion {version}\nprevious none\ndigest {D}\nmanifest {D}\nedition 0.4\nauthority {authority}\ntrust none\n"
        ))
        .unwrap()
    };
    ordered
        .statements
        .insert(v("0.3.0"), statement_of("0.3.0", "Http[get, post]"));
    ordered
        .statements
        .insert(v("0.3.5"), statement_of("0.3.5", "Http[get]"));
    assert_eq!(
        submit::highest_of_line(&ordered, &v("0.3.2"))
            .unwrap()
            .version,
        v("0.3.5")
    );
    assert!(refusal(&published("0.3.2", ""), &ordered, &[&publisher]).contains("Http[post]"));
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
    assert!(
        submit::release(
            &manifest,
            with_c,
            &BTreeMap::new(),
            &none,
            &state,
            &[&publisher]
        )
        .is_err()
    );
}

#[test]
fn a_promise_is_verified() {
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
}

/// Kept means logged by `by`; a promise past the window, a late entry, and no promise past the
/// window are each broken; and a settled submission is recorded and never judged again.
#[test]
fn promises_are_judged_once_by_the_time_their_entry_was_logged() {
    let submitted = Submitted {
        leaf: "ab".repeat(32),
        url: "file:///srv/ita".to_owned(),
        name: "acme/other".to_owned(),
        time: 1000,
        sent: 1000,
    };
    assert_eq!(
        Submitted::parse(&submitted.leaf, &submitted.encode()).unwrap(),
        submitted
    );
    let promise = |by: u64| Promise {
        source: "acme".to_owned(),
        leaf: format!("sha256:{}", submitted.leaf),
        by,
        document: String::new(),
    };
    let window = 1000 + PROMISE_WINDOW + una_project::client::SKEW;
    // Logged by `by`: kept, whatever the checkpoint's time now.
    assert_eq!(
        judge(&submitted, Some(&promise(2000)), Some(1990), 9_000_000),
        Verdict::Kept
    );
    // Logged after `by`: broken.
    assert!(matches!(
        judge(&submitted, Some(&promise(2000)), Some(2001), 9_000_000),
        Verdict::Broken(_)
    ));
    // Not logged: pending until `by`, broken after.
    assert_eq!(
        judge(&submitted, Some(&promise(2000)), None, 2000),
        Verdict::Pending
    );
    assert!(matches!(
        judge(&submitted, Some(&promise(2000)), None, 2001),
        Verdict::Broken(_)
    ));
    // A `by` past the window is itself a broken promise.
    assert!(matches!(
        judge(&submitted, Some(&promise(window + 1)), Some(1100), 1100),
        Verdict::Broken(_)
    ));
    // No promise: pending within the window, broken after it, kept if logged within it.
    assert_eq!(judge(&submitted, None, None, window), Verdict::Pending);
    assert!(matches!(
        judge(&submitted, None, None, window + 1),
        Verdict::Broken(_)
    ));
    assert_eq!(
        judge(&submitted, None, Some(window), window + 10),
        Verdict::Kept
    );
    // A settled submission is recorded in the position, for a name not followed too, and a
    // second run, which reads the position back, sees it settled.
    let mut first_run = Position::start("acme", "acme");
    first_run.settle(&submitted.leaf, &Verdict::Kept).unwrap();
    let second_run = Position::parse(&first_run.encode(), "acme", "acme").unwrap();
    assert!(!second_run.names.contains_key("acme/other"));
    assert!(second_run.settled.contains(&submitted.leaf));
    // The chains a second run follows keep the settled submissions.
    let mut chain = |_: &str| -> Result<Vec<Seen>, String> { Ok(Vec::new()) };
    let report = watch(&mut chain, &second_run, &[]).unwrap();
    assert_eq!(report.position.settled, second_run.settled);
}

// --- Advisories' thresholds and form (rules 35 and 41) --------------------------------------

/// Holding a release back needs one security key; lifting the cooldown needs the root's security
/// threshold, never fewer than two, or one security key and the owner's threshold.
#[test]
fn lifting_the_cooldown_needs_the_roots_security_threshold() {
    let signers = |security: usize, owner_threshold: bool| Signers {
        security,
        owner_threshold,
    };
    assert!(!signers(0, true).valid());
    assert!(signers(1, false).valid());
    assert!(!signers(1, false).lifts_cooldown(1), "never fewer than two");
    assert!(signers(2, false).lifts_cooldown(1));
    assert!(
        !signers(2, false).lifts_cooldown(3),
        "the root asks for three"
    );
    assert!(signers(3, false).lifts_cooldown(3));
    assert!(
        signers(1, true).lifts_cooldown(3),
        "one and the owner's threshold"
    );
    assert!(!signers(0, true).lifts_cooldown(1));
}

#[test]
fn an_advisory_names_each_version_once_and_its_fix_past_them() {
    let advisory = |affected: &str, fixed: &str| {
        format!(
            "ita-advisory/1\nsource acme\nname acme/ledger\nprevious none\nid ITA-1\nline 0.3\naffected {affected}\nfixed {fixed}\nseverity high\nsummary An overflow\n"
        )
    };
    assert!(parse_advisory(&advisory("0.3.0, 0.3.2", &format!("0.3.3 {D}"))).is_ok());
    assert!(
        parse_advisory(&advisory("0.3.0, 0.3.0", "none")).is_err(),
        "twice"
    );
    assert!(
        parse_advisory(&advisory("0.3.0, 0.3.2", &format!("0.3.1 {D}"))).is_err(),
        "a fix within the affected range"
    );
}

// --- Audit and watch (rules 36, 36.2, 38, 41; U7014) ----------------------------------------

fn locked(name: &str, version: &str) -> Locked {
    Locked {
        source: "acme".to_owned(),
        name: name.to_owned(),
        version: v(version),
        logged: Some(45),
        digest: None,
        resolution: 100,
        accepted: BTreeSet::new(),
        effects: BTreeSet::new(),
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
    let packages = [
        locked("acme/ledger", "0.3.1"),
        locked("acme/ledger", "0.3.2"),
    ];
    // Withheld cosignatures delay the eligibility of the advisory's fix, a finding, and of
    // another release, a note.
    let fixed_late = Delay {
        version: v("0.3.2"),
        served: 2_000,
        earliest: 1_000,
        fix: true,
    };
    let other_late = Delay {
        version: v("0.4.0"),
        served: 3_000,
        earliest: 2_500,
        fix: false,
    };
    let mut facts_of = |package: &Locked| -> Result<Facts, String> {
        Ok(if package.version == v("0.3.1") {
            Facts {
                yanked: true,
                advisories: vec![advisory("ITA-1")],
                compromised: true,
                compromised_since: Some(40),
                pending: vec![130],
                key_changes: vec![140],
                void_yanks: vec![150],
                accepted_effects: Vec::new(),
                // The fix observed twice is one finding.
                eligibility_delays: vec![
                    fixed_late.clone(),
                    other_late.clone(),
                    Delay {
                        served: 2_100,
                        ..fixed_late.clone()
                    },
                ],
            }
        } else {
            Facts {
                pending: vec![130],
                ..Facts::default()
            }
        })
    };
    let result = audit::audit(&packages, &mut facts_of);
    assert!(result.unverified.is_empty());
    let findings = result.findings;
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
            Kind::Compromised { since: Some(40) },
            Kind::PendingRecovery(130),
            Kind::KeyChange(140),
            Kind::VoidYank(150),
            Kind::EligibilityDelay(fixed_late),
            Kind::EligibilityDelay(other_late),
        ]
    );
    assert_eq!(of(1), [Kind::PendingRecovery(130)]);
    assert_eq!(findings[0].text(&packages[0]), "0.3.1 is yanked");
    let delays: Vec<&audit::Finding> = findings
        .iter()
        .filter(|f| matches!(f.kind, Kind::EligibilityDelay(_)))
        .collect();
    assert!(!delays[0].is_note() && delays[1].is_note());
    let text = delays[0].text(&packages[0]);
    assert!(
        text.contains("0.3.2, the fix of an advisory that names 0.3.1,")
            && text.contains("served as logged at 2000")
            && text.contains("allow 1000"),
        "{text}"
    );
    assert!(findings.iter().filter(|f| !f.is_note()).count() < findings.len());
    // A package that cannot be verified is reported, and the audit goes on with the next:
    // nothing is called clean unverified, and no finding is hidden.
    let mut first_down = |package: &Locked| -> Result<Facts, String> {
        if package.version == v("0.3.1") {
            Err("a bad map proof".to_owned())
        } else {
            Ok(Facts {
                yanked: true,
                ..Facts::default()
            })
        }
    };
    let result = audit::audit(&packages, &mut first_down);
    assert_eq!(result.unverified, [(0, "a bad map proof".to_owned())]);
    assert_eq!(result.findings.len(), 1);
    assert_eq!(result.findings[0].package, 1);
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
        let findings = audit::audit(&packages, &mut facts_of).findings;
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
fn watch_reports_new_entries_and_checks_its_position() {
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
    assert_eq!(
        Position::parse(&position.encode(), "acme", "acme").unwrap(),
        position
    );
    // A position is read only for the source and owner it names, with names of packages.
    assert!(Position::parse(&position.encode(), "other", "acme").is_err());
    assert!(Position::parse(&position.encode(), "acme", "other").is_err());
    assert!(Position::parse("ita-watch/1\nsource acme\nowner acme 01\n", "acme", "acme").is_err());
    assert!(
        Position::parse(
            "ita-watch/1\nsource acme\nowner acme 0\nname Acme/X 1\n",
            "acme",
            "acme"
        )
        .is_err()
    );
    let report = watch(&mut chain, &position, &[]).unwrap();
    let new: Vec<(String, u64)> = report
        .entries
        .iter()
        .map(|(name, e)| (name.clone(), e.index))
        .collect();
    assert_eq!(new, [("acme".to_owned(), 4), ("acme/ledger".to_owned(), 3)]);
    assert_eq!(report.position.owner_count, 2);
    assert_eq!(report.position.names["acme/ledger"], 3);
    // A chain that shrank is refused.
    position.names.insert("acme/ledger".to_owned(), 9);
    assert!(watch(&mut chain, &position, &[]).is_err());
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
    assert!(
        marks.threshold_fix.contains(&v("0.3.1")),
        "two signers lift the cooldown"
    );
    let state = client.state("acme/ledger", Line::Zero(3)).unwrap();
    let old = state.iter().find(|r| r.version == v("0.3.0")).unwrap();
    assert!(old.yanked && old.advised);
    // The state a publisher reads names the chain's head and the yank.
    let name = NameState::read(&mut client, "acme/ledger").unwrap();
    assert!(name.yanked.contains(&v("0.3.0")));
    assert_eq!(name.head.as_ref().unwrap().0, 4, "the advisory is the head");
    // A yank signed by a key that is not the owner's: one the owner's keys as logged do not
    // sign, skipped and recorded against the registry, never a refusal of the name.
    let mut forged = TestRegistry::new("acme");
    forged.register("acme", &owner);
    forged.release("acme/ledger", "0.3.0", "none", "none", &[&owner]);
    let stranger = keypair(61, "stranger");
    let yank = format!(
        "ita-yank/1\nsource acme\nname acme/ledger\nversion 0.3.0\n{}",
        forged.previous("acme/ledger")
    );
    let document = support::signed(&yank, &[&stranger]);
    let unsigned = forged.append("acme/ledger", &document);
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
    let state = client.state("acme/ledger", Line::Zero(3)).unwrap();
    assert!(!state[0].yanked, "{state:?}");
    assert_eq!(
        audit::evidence(&client),
        [audit::Evidence::Unsigned {
            name: "acme/ledger".to_owned(),
            index: unsigned,
        }]
    );
}

/// A yank that counts only with a key compromised before it is void, as such a release is: it
/// is ignored, never applied and never a refusal of the name.
#[test]
fn a_yank_signed_after_its_keys_compromise_is_void() {
    let mut test = TestRegistry::new("acme");
    let first = keypair(60, "acme.laptop");
    let second = keypair(62, "acme.desktop");
    let [c1, c2] = support::companions(&first);
    test.register("acme", &first);
    // The signers are explicit: `first` with its first companion, the owner's threshold of 2.
    test.owner_event(
        "acme",
        &format!("add {}", support::key_text(&second.1)),
        2,
        &[&first, &c1, &second],
    );
    let (good, _) = test.release("acme/ledger", "0.3.0", "none", "none", &[&first, &c1]);
    let yank = format!(
        "ita-yank/1\nsource acme\nname acme/ledger\nversion 0.3.0\n{}",
        test.previous("acme/ledger")
    );
    // Signed after `first` is compromised (since `good`): `c1` alone counts, below 2.
    let document = support::signed(&yank, &[&first, &c1]);
    test.append("acme/ledger", &document);
    test.owner_event(
        "acme",
        &format!("revoke {} since {good}", first.1.name),
        2,
        &[&second, &c2],
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
    assert_eq!(marks.voided.len(), 1, "listed for the audit");
    assert_eq!(marks.voided[0].version, v("0.3.0"));
    let state = client.state("acme/ledger", Line::Zero(3)).unwrap();
    assert!(!state[0].yanked);
    // The audit reports it, and does not refuse the name.
    let facts = audit::facts(
        &mut client,
        &Locked {
            source: "acme".to_owned(),
            name: "acme/ledger".to_owned(),
            version: v("0.3.0"),
            logged: Some(good),
            digest: None,
            resolution: 0,
            accepted: BTreeSet::new(),
            effects: BTreeSet::new(),
        },
    )
    .unwrap();
    assert_eq!(facts.void_yanks, [marks.voided[0].index]);
    // A lock that names another digest or log index for the release is not audited as clean.
    let mismatch = Locked {
        source: "acme".to_owned(),
        name: "acme/ledger".to_owned(),
        version: v("0.3.0"),
        logged: Some(good + 1),
        digest: None,
        resolution: 0,
        accepted: BTreeSet::new(),
        effects: BTreeSet::new(),
    };
    assert!(audit::facts(&mut client, &mismatch).is_err());
}

/// A second yank of a version already yanked is void and ignored, with a note.
#[test]
fn a_second_yank_is_void() {
    let mut test = TestRegistry::new("acme");
    let owner = keypair(60, "acme.laptop");
    test.register("acme", &owner);
    test.release("acme/ledger", "0.3.0", "none", "none", &[&owner]);
    for _ in 0..2 {
        let yank = format!(
            "ita-yank/1\nsource acme\nname acme/ledger\nversion 0.3.0\n{}",
            test.previous("acme/ledger")
        );
        let document = support::signed(&yank, &[&owner]);
        test.append("acme/ledger", &document);
    }
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
    assert_eq!(marks.yanked.len(), 1);
    assert_eq!(marks.voided.len(), 1);
    assert!(marks.voided[0].why.contains("yanked already"));
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
    let mut files = files(&format!(
        "ita acme/ledger\n  version {version}\n  edition 0.4\n"
    ));
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
    bundled_with(Rotation::None)
}

/// How the registry of a bundle's parts rotates its witnesses.
#[derive(Clone, Copy)]
enum Rotation {
    /// Not at all.
    None,
    /// One witness, by root version 2, before the lock's point: the point names version 2.
    BeforePoint,
    /// `count` witnesses, by root version 2, after the lock's point, which names version 1 and
    /// is cosigned by witnesses retired since; a later checkpoint names version 2.
    AfterPoint(usize),
}

/// A bundle's parts, the registry's witnesses rotated as `rotation` says.
fn bundled_with(rotation: Rotation) -> Bundled {
    let mut test = TestRegistry::new("acme");
    let owner = keypair(60, "acme.laptop");
    test.register("acme", &owner);
    let (index, digest) = test.release("acme/ledger", "0.3.1", "none", "none", &[&owner]);
    let size = match rotation {
        Rotation::None => test.checkpoint(NOW),
        Rotation::BeforePoint => {
            test.rotate_witness();
            test.checkpoint(NOW)
        }
        Rotation::AfterPoint(count) => {
            let size = test.checkpoint(NOW - 10 * 86_400);
            test.rotate_witnesses(count, false);
            test.append("other", "after the rotation");
            test.checkpoint(NOW);
            size
        }
    };
    let served = Memory {
        source: "acme".to_owned(),
        files: test.files.clone(),
    };
    let recording = Recording::new(&served);
    let checkpoint =
        parse_note(std::str::from_utf8(&test.files[&format!("checkpoint/{size}")]).unwrap())
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
    let (last_root, last_root_digest) = client
        .last_root()
        .map(|(version, digest)| (version, digest.to_owned()))
        .unwrap();
    let archive =
        served.files[&format!("blobs/sha256/{}", digest.strip_prefix("sha256:").unwrap())].clone();
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
        last_root,
        last_root_digest,
    };
    Bundled {
        archive,
        provenance,
        expected,
        files,
        test,
    }
}

fn bundle_of(
    b: &Bundled,
    archive: &[u8],
    provenance: &[u8],
    files: &BTreeMap<String, Vec<u8>>,
) -> Vec<u8> {
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
    assert!(
        read.entries
            .keys()
            .any(|path| path.starts_with("registry/acme/root/"))
    );
    let store = Itaoca {
        root: scratch("unbundle"),
    };
    let done =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap();
    assert_eq!(done.stored, ["acme/ledger 0.3.1"]);
    assert!(done.unused.is_empty(), "{:?}", done.unused);
    // What was stored loads verified, as every load does, and the checkpoint is the floor.
    assert!(
        una_project::vendor::stored(&store, &b.expected)
            .unwrap()
            .is_some()
    );
    let id = registry_id(&b.expected.root);
    assert!(store.load_checkpoint(&id).unwrap().is_some());
    // The root-version floor binds the digest of the text of that root metadata (ADR 0122,
    // amendments of 2026-11-09, L3, and of 2026-11-19).
    assert_eq!(
        store.load_root_version(&id).unwrap(),
        Some(una_project::itaoca::RootFloor {
            version: 1,
            digest: Some(una_project::itaoca::FloorDigest::Text(
                una_project::tlog::root::text_digest(&b.test.roots[0]).unwrap()
            )),
        })
    );
    assert!(done.notes.is_empty(), "{:?}", done.notes);
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
        match bundle::unbundle(bytes, &store, std::slice::from_ref(&b.expected), &no_place)
            .unwrap_err()
        {
            (at, Failure::Refused(why)) => (at, why),
            (at, Failure::Absent(why)) => (at, format!("absent: {why}")),
            (at, Failure::Unavailable(why) | Failure::Omitted(_, why)) => {
                (at, format!("unavailable: {why}"))
            }
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
    // A bundle without the registry's files, and no place behind it: the state is
    // unavailable, an omission, not a refusal.
    let (_, why) = refused(&bundle_of(&b, &b.archive, &b.provenance, &BTreeMap::new()));
    assert!(why.starts_with("unavailable: "), "{why}");
    // A byte of the bundle changed: its listing's hash refuses it.
    let mut bytes = bundle_of(&b, &b.archive, &b.provenance, &b.files);
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let (at, _) = refused(&bytes);
    assert_eq!(at, None);
    // Nothing was written by any of them.
    assert!(!store.contains(&b.expected.digest).unwrap());
    assert!(
        store
            .load_checkpoint(&registry_id(&b.expected.root))
            .unwrap()
            .is_none()
    );
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
    let id = registry_id(&b.expected.root);
    let other = format!("sha256:{}", "0".repeat(64));
    store.store_root_chain(&id, &[walked(2, &other)]).unwrap();
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &b.files);
    let (_, problem) =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap_err();
    assert!(matches!(problem, Failure::Refused(_)), "{problem:?}");
    assert!(!store.contains(&b.expected.digest).unwrap());
    assert_eq!(
        store
            .load_root_version(&id)
            .unwrap()
            .map(|floor| floor.version),
        Some(2)
    );
    assert!(store.load_checkpoint(&id).unwrap().is_none());
}

/// A run at the same time stores the root-version floor after the unbundling read it and before
/// the unbundling stores its own (#2099, `spec/30 §14`, Floors). The places behind the bundle
/// are asked for only after the floors are read, so storing from there is that other run,
/// deterministically. With another digest at the version the bundle's chain holds, the
/// unbundling holds another digest for the same root version: refused (U7010), at the
/// source's first package, with the other run's floor kept and no package, provenance or
/// checkpoint written. With the same digest, it unbundles, with no note.
#[test]
fn unbundling_fails_when_a_run_at_the_same_time_stored_a_forked_root() {
    let b = bundled();
    let id = registry_id(&b.expected.root);
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &b.files);
    let first = una_project::tlog::root::text_digest(&b.test.roots[0]).unwrap();
    let other = format!("sha256:{}", "0".repeat(64));
    // The floor the unbundling reads: of the earliest form, or none at all.
    for (name, read) in [("old-form", Some("1\n")), ("none", None)] {
        for (case, meanwhile) in [("fork", &other), ("same", &first)] {
            let store = Itaoca {
                root: scratch(&format!("unbundle-root-race-{name}-{case}")),
            };
            let dir = store.root.join("registry").join(&id);
            if let Some(text) = read {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join("root-version"), text).unwrap();
            }
            let asked = std::cell::Cell::new(false);
            let behind = |_: &str| -> Vec<Box<dyn Transport>> {
                if !asked.replace(true) {
                    store
                        .store_root_chain(&id, &[walked(1, meanwhile)])
                        .unwrap();
                }
                Vec::new()
            };
            let result =
                bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &behind);
            assert!(asked.get());
            let floor = std::fs::read_to_string(dir.join("root-version")).unwrap();
            assert_eq!(floor, format!("1 text {meanwhile}\n"), "{name} {case}");
            if case == "same" {
                let done = result.unwrap();
                assert!(done.notes.is_empty(), "{name}: {:?}", done.notes);
                assert!(store.contains(&b.expected.digest).unwrap());
                assert!(store.load_checkpoint(&id).unwrap().is_some());
                continue;
            }
            let (at, problem) = result.unwrap_err();
            assert_eq!(at, Some(0), "{name}");
            assert!(
                matches!(
                    &problem,
                    Failure::Refused(why)
                        if why.contains("another digest is held for the same root version")
                            && why.contains("a run at the same time")
                            && why.contains(&format!("registry/{id}/root-version"))
                            && why.contains(&first)
                            && why.contains(&other)
                ),
                "{name}: {problem:?}"
            );
            assert!(!store.contains(&b.expected.digest).unwrap());
            assert!(store.load_checkpoint(&id).unwrap().is_none());
            assert!(
                store
                    .load_provenance(
                        &b.expected.source,
                        &b.expected.name,
                        &b.expected.version,
                        b.expected.checkpoint_size,
                    )
                    .unwrap()
                    .is_none()
            );
            // The next unbundling reads the floor with its digest, and refuses in the walk.
            let (_, again) =
                bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place)
                    .unwrap_err();
            assert!(
                matches!(&again, Failure::Refused(why) if why.contains("forked")),
                "{again:?}"
            );
        }
    }
}

/// #2132, fixed by ADR 0122, amendment of 2026-11-19, for a bundle: a bundle made by someone
/// who holds no key cannot poison the root-version floor. The lock was made under root 1
/// (`last root 1`); the registry has since rotated to root 2 with the same witnesses. The
/// bundle's copy of the registry holds the honest root 2 with one junk signature line more,
/// and no latest checkpoint. It unbundles with no note, as before, and the floor stored holds
/// the digest of the text of root 2, which is the honest note's too: the honest bundle of the
/// same lock unbundles afterwards, and the honest registry is accepted as a place. Before the
/// fix the floor held the digest of the altered note, and both were refused as forked (U7010).
///
/// A machine whose floor is of the form of 2026-11-09, stored from the honest note: the
/// altered bundle is refused, as it was, and the honest one upgrades the floor to the digest
/// of the text of that same note.
#[test]
fn a_bundle_with_no_key_cannot_poison_the_root_floor() {
    use una_project::itaoca::{FloorDigest, RootFloor};
    use una_project::tlog::root::{digest, text_digest};

    let mut b = bundled_with(Rotation::AfterPoint(0));
    // The lock was written under root 1, before the rotation.
    b.expected.last_root = 1;
    b.expected.last_root_digest = b.test.pin();
    let id = registry_id(&b.expected.root);
    let altered = format!(
        "{}\u{2014} nobody {}\n",
        b.test.roots[1],
        una_project::tlog::note::encode_base64(&[7u8; 68])
    );
    assert_ne!(digest(&altered), digest(&b.test.roots[1]));
    let mut files = b.files.clone();
    assert!(files.remove("checkpoint").is_some());
    files.insert("root/2".to_owned(), altered.clone().into_bytes());
    let hostile = bundle_of(&b, &b.archive, &b.provenance, &files);
    let honest = bundle_of(&b, &b.archive, &b.provenance, &b.files);
    let store = Itaoca {
        root: scratch("unbundle-root-poison"),
    };
    // What the machine stored when it locked, under root 1.
    store
        .store_root_chain(&id, &[walked_root(1, &b.test.roots[0])])
        .unwrap();

    let done = bundle::unbundle(
        &hostile,
        &store,
        std::slice::from_ref(&b.expected),
        &no_place,
    )
    .unwrap();
    assert_eq!(done.stored, ["acme/ledger 0.3.1"]);
    assert!(done.notes.is_empty(), "{:?}", done.notes);
    let floor = RootFloor {
        version: 2,
        digest: Some(FloorDigest::Text(text_digest(&b.test.roots[1]).unwrap())),
    };
    assert_eq!(store.load_root_version(&id).unwrap(), Some(floor.clone()));

    // The honest bundle of the same lock unbundles from then on.
    let done = bundle::unbundle(
        &honest,
        &store,
        std::slice::from_ref(&b.expected),
        &no_place,
    )
    .unwrap();
    assert!(done.notes.is_empty(), "{:?}", done.notes);
    // And the honest registry is accepted, read as a place at its latest checkpoint.
    let served = Memory {
        source: "acme".to_owned(),
        files: b.test.files.clone(),
    };
    let floors = una_project::client::Floors {
        checkpoint: None,
        root_version: Some(floor.version),
        root_digest: floor.digest.as_ref(),
    };
    let opened = Registry::open_with(
        "acme",
        &served,
        &b.test.pin(),
        &Point::Latest { now: NOW + 60 },
        &floors,
    );
    assert!(
        opened.is_ok(),
        "{:?}",
        opened.err().map(una_project::client::OpenError::message)
    );
    assert_eq!(store.load_root_version(&id).unwrap(), Some(floor));

    // A floor of the form of 2026-11-09, stored from the honest note of root 2.
    let store = Itaoca {
        root: scratch("unbundle-root-poison-earlier-form"),
    };
    let dir = store.root.join("registry").join(&id);
    std::fs::create_dir_all(&dir).unwrap();
    let earlier = format!("2 {}\n", digest(&b.test.roots[1]));
    std::fs::write(dir.join("root-version"), &earlier).unwrap();
    let (_, problem) = bundle::unbundle(
        &hostile,
        &store,
        std::slice::from_ref(&b.expected),
        &no_place,
    )
    .unwrap_err();
    assert!(
        matches!(&problem, Failure::Refused(why) if why.contains("another form or another text") && why.contains("earlier form") && !why.contains("forked")),
        "{problem:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("root-version")).unwrap(),
        earlier
    );
    assert!(!store.contains(&b.expected.digest).unwrap());
    bundle::unbundle(
        &honest,
        &store,
        std::slice::from_ref(&b.expected),
        &no_place,
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("root-version")).unwrap(),
        format!("2 text {}\n", text_digest(&b.test.roots[1]).unwrap())
    );
    // Upgraded, the floor accepts the other form of the same text.
    bundle::unbundle(
        &hostile,
        &store,
        std::slice::from_ref(&b.expected),
        &no_place,
    )
    .unwrap();
}

/// ADR 0122, amendment of 2026-11-09 (L3): unbundling refuses a bundle whose root chain holds
/// the floor's version with another digest than the one stored, a fork of the root chain
/// (U7010), and writes nothing; with the same digest, or a floor of the earlier form, it
/// unbundles; a floor with a digest is kept as it is, and a floor of the earlier form takes the
/// digest of the root chain the unbundling verified at its version (#1970).
#[test]
fn unbundling_refuses_a_fork_at_the_floor_version() {
    let b = bundled();
    let id = registry_id(&b.expected.root);
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &b.files);
    let first = una_project::tlog::root::text_digest(&b.test.roots[0]).unwrap();
    let store = Itaoca {
        root: scratch("unbundle-root-fork"),
    };
    store
        .store_root_chain(&id, &[walked(1, &format!("sha256:{}", "0".repeat(64)))])
        .unwrap();
    let (_, problem) =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap_err();
    assert!(
        matches!(&problem, Failure::Refused(why) if why.contains("another digest") && why.contains("forked")),
        "{problem:?}"
    );
    assert!(!store.contains(&b.expected.digest).unwrap());
    assert!(store.load_checkpoint(&id).unwrap().is_none());
    // The same digest: unbundled.
    let store = Itaoca {
        root: scratch("unbundle-root-same"),
    };
    store.store_root_chain(&id, &[walked(1, &first)]).unwrap();
    let done =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap();
    assert!(done.notes.is_empty(), "{:?}", done.notes);
    assert!(store.contains(&b.expected.digest).unwrap());
    // The earlier form, as builds before v0.4.0 write it: compared by version, unbundled, and
    // given the digest of the verified chain's version.
    let store = Itaoca {
        root: scratch("unbundle-root-old-form"),
    };
    let dir = store.root.join("registry").join(&id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("root-version"), "1\n").unwrap();
    // A bundle that does not verify (a registry file changed in its copy) records no digest:
    // the floor of the earlier form is kept as it is.
    let mut files = b.files.clone();
    let tile = files
        .keys()
        .find(|path| path.starts_with("tile/entries/"))
        .unwrap()
        .clone();
    let entry = files.get_mut(&tile).unwrap();
    let middle = entry.len() / 2;
    entry[middle] ^= 1;
    let tampered = bundle_of(&b, &b.archive, &b.provenance, &files);
    assert!(
        bundle::unbundle(
            &tampered,
            &store,
            std::slice::from_ref(&b.expected),
            &no_place
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("root-version")).unwrap(),
        "1\n"
    );
    let done =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap();
    assert!(done.notes.is_empty(), "{:?}", done.notes);
    assert_eq!(
        std::fs::read_to_string(dir.join("root-version")).unwrap(),
        format!("1 text {first}\n")
    );
    // A floor line of no form: unavailable (U7013), nothing written.
    let store = Itaoca {
        root: scratch("unbundle-root-malformed"),
    };
    let dir = store.root.join("registry").join(&id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("root-version"), format!("1 {first}x\n")).unwrap();
    let (_, problem) =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap_err();
    assert!(matches!(problem, Failure::Unavailable(_)), "{problem:?}");
    assert!(!store.contains(&b.expected.digest).unwrap());
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
    b.test
        .release("acme/ledger", "0.3.2", "none", "none", &[&owner]);
    let larger = b.test.checkpoint(NOW + 10);
    let document =
        String::from_utf8(b.test.files[&format!("checkpoint/{larger}")].clone()).unwrap();
    let id = registry_id(&b.expected.root);
    store
        .store_checkpoint(&id, &document, &mut |_: &str| Ok(()))
        .unwrap();
    // The bundle's files hold the larger log's tiles too, so its consistency can be proven.
    let mut files = b.files.clone();
    for (path, bytes) in &b.test.files {
        if path.starts_with("tile/") {
            files.insert(path.clone(), bytes.clone());
        }
    }
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &files);
    let result = bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place);
    let stored = store.load_checkpoint(&id).unwrap().unwrap();
    assert_eq!(stored, document, "{result:?}");
}

/// A floor that cannot be stored is a note, never a failure: the packages are stored.
#[cfg(unix)]
#[test]
fn unbundling_notes_a_floor_it_cannot_store() {
    use std::os::unix::fs::PermissionsExt;
    let b = bundled();
    let store = Itaoca {
        root: scratch("unbundle-floor-note"),
    };
    let dir = store
        .root
        .join("registry")
        .join(registry_id(&b.expected.root));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    // A process that ignores file modes, as root does, can write here anyway.
    let probe = dir.join("probe");
    if std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .is_ok()
    {
        let _ = std::fs::remove_file(&probe);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        println!("skipped: running with privileges that ignore file modes");
        return;
    }
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &b.files);
    let done =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(done.stored, ["acme/ledger 0.3.1"]);
    assert!(!done.notes.is_empty(), "the floor not stored is said");
    assert!(done.notes.iter().all(|note| note.starts_with("note: ")));
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
    assert!(
        per_entry[2] < per_entry[0] * 20.0 + 50_000.0,
        "{per_entry:?}"
    );
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
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // A directory others can enter is refused first.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            owner_keys(&dir, "acme", &[], 1)
                .unwrap_err()
                .contains("0700")
        );
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
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
    assert_eq!(
        names,
        ["acme.b", "acme.a"],
        "by file name, registered keys only"
    );
    let refusal = owner_keys(&dir, "acme", &registered, 3).unwrap_err();
    assert!(
        refusal.contains("fewer than its threshold of 3"),
        "{refusal}"
    );
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
        std::fs::set_permissions(
            dir.join("acme-0a.key"),
            std::fs::Permissions::from_mode(0o644),
        )
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
    assert_eq!(
        submit::git_entry(&package).unwrap(),
        Some(base.join(".git"))
    );
    std::fs::create_dir(package.join(".git")).unwrap();
    assert_eq!(
        submit::git_entry(&package).unwrap(),
        Some(package.join(".git"))
    );
    // It fails closed: a directory that cannot be resolved refuses.
    assert!(submit::git_entry(&base.join("missing")).is_err());
}

#[test]
fn a_published_source_is_the_one_its_workspace_declares() {
    let workspace = scratch("workspace");
    let package = workspace.join("pkg");
    std::fs::create_dir_all(&package).unwrap();
    assert_eq!(
        submit::declared_source(&package, "acme").unwrap_err(),
        submit::Undeclared::NoWorkspace {
            source: "acme".to_owned()
        }
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
    // A source the workspace does not declare is refused, by its kind.
    assert_eq!(
        submit::declared_source(&package, "other").unwrap_err(),
        submit::Undeclared::NotInWorkspace {
            source: "other".to_owned(),
            workspace: std::fs::canonicalize(&workspace).unwrap().join("itaty.una"),
        }
    );
    assert_eq!(
        submit::declared_source(&package, "itaty").unwrap_err(),
        submit::Undeclared::Itaty
    );
}

// --- Recheck: void owner events, the promise window, disjoint signers, bad promises ----------

/// The window runs from the later of the witness time and the local clock less the skew: an
/// honest submission read at a checkpoint 20 hours old is not broken. Kept is final; broken is
/// provisional, and a broken verdict from incomplete kept checkpoints becomes kept once they are
/// served complete.
#[test]
fn the_promise_window_is_anchored_and_only_kept_is_settled() {
    let hour = 3600;
    let sent = 2_000_000_000;
    let honest = Submitted {
        leaf: "cd".repeat(32),
        url: "file:///srv/ita".to_owned(),
        name: "acme/ledger".to_owned(),
        time: sent - 20 * hour,
        sent,
    };
    assert_eq!(honest.anchor(), sent);
    let promise = Promise {
        source: "acme".to_owned(),
        leaf: format!("sha256:{}", honest.leaf),
        by: sent + hour / 2,
        document: String::new(),
    };
    assert_eq!(
        judge(&honest, Some(&promise), Some(sent + hour / 4), sent + hour),
        Verdict::Kept
    );
    // A mirror served too few kept checkpoints: the entry looks logged late.
    let provisional = judge(&honest, Some(&promise), Some(sent + hour), sent + 2 * hour);
    assert!(matches!(provisional, Verdict::Broken(_)));
    let mut position = Position::start("acme", "acme");
    position.settle(&honest.leaf, &provisional).unwrap();
    assert!(position.settled.is_empty(), "broken is never settled");
    // The complete checkpoints give its true logged-at time: kept, and settled.
    let complete = judge(&honest, Some(&promise), Some(sent), sent + 3 * hour);
    assert_eq!(complete, Verdict::Kept);
    position.settle(&honest.leaf, &complete).unwrap();
    assert!(position.settled.contains(&honest.leaf));
}

/// Settled lines whose record is gone are pruned; past the limit, settling is refused with a
/// message that says what to remove.
#[test]
fn settled_lines_are_pruned_and_bounded() {
    let mut position = Position::start("acme", "acme");
    for i in 0..3u32 {
        position
            .settle(&format!("{i:064x}"), &Verdict::Kept)
            .unwrap();
    }
    let existing: BTreeSet<String> = [format!("{:064x}", 1)].into_iter().collect();
    position.prune(&existing);
    assert_eq!(position.settled, existing);
    let mut full = Position::start("acme", "acme");
    full.settled = (0..MAX_SETTLED).map(|i| format!("{i:064x}")).collect();
    let refusal = full
        .settle(&format!("{:064x}", MAX_SETTLED), &Verdict::Kept)
        .unwrap_err();
    assert!(refusal.contains("remove old records"), "{refusal}");
}

/// One key that is both a security key of the root and the owner's key counts once, as a
/// security key: it holds a release back, and never lifts the cooldown alone.
#[test]
fn a_key_both_security_and_owner_does_not_lift_the_cooldown_alone() {
    let mut test = TestRegistry::new("acme");
    let both = (
        ed25519_dalek::SigningKey::from_bytes(&test.keys.security.0.to_bytes()),
        test.keys.security.1.clone(),
    );
    test.register("acme", &both);
    test.release("acme/ledger", "0.3.0", "none", "none", &[&both]);
    let (_, fix) = test.release("acme/ledger", "0.3.1", "none", "none", &[&both]);
    let advisory = format!(
        "ita-advisory/1\nsource acme\nname acme/ledger\n{}\nid ITA-1\nline 0.3\naffected 0.3.0\nfixed 0.3.1 {fix}\nseverity high\nsummary An overflow",
        test.previous("acme/ledger")
    );
    let document = support::signed(&advisory, &[&both]);
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
    assert_eq!(marks.advised[&v("0.3.0")], ["ITA-1"], "it holds back");
    assert!(marks.threshold_fix.is_empty(), "it does not hurry in");
}

/// One bad promise among good ones: it is reported and counts as none, and the others are
/// judged as usual.
#[test]
fn a_bad_promise_is_reported_and_counts_as_none() {
    let log = key(20, "acme.example/log");
    let stranger = key(21, "acme.example/other");
    // Past the window of a submission at 1000: an hour, and the hour of skew.
    let now = 10_000;
    let record = |leaf: &str| Submitted {
        leaf: leaf.to_owned(),
        url: "file:///srv/ita".to_owned(),
        name: "acme/ledger".to_owned(),
        time: 1000,
        sent: 1000,
    };
    let promise_of = |leaf: &str, signer: &SigningKey, source: &str| {
        sign_note(
            &format!("ita-promise/1\nsource {source}\nleaf sha256:{leaf}\nby 2000\n"),
            &[signer],
        )
        .unwrap()
        .into_bytes()
    };
    let good = "aa".repeat(32);
    let forged = "bb".repeat(32);
    let elsewhere = "cc".repeat(32);
    let ok = assess(
        &record(&good),
        Ok(Some(promise_of(&good, &log, "acme"))),
        None,
        log.key(),
        "acme",
        Some(1500),
        now,
    );
    assert_eq!(ok.verdict, Verdict::Kept);
    assert!(ok.notes.is_empty());
    for (leaf, served) in [
        (&forged, promise_of(&forged, &stranger, "acme")),
        (&elsewhere, promise_of(&elsewhere, &log, "other")),
        (&forged, b"not a promise".to_vec()),
    ] {
        let bad = assess(
            &record(leaf),
            Ok(Some(served)),
            None,
            log.key(),
            "acme",
            None,
            now,
        );
        assert_eq!(bad.notes.len(), 1, "{bad:?}");
        assert!(bad.notes[0].contains("counts as none"));
        // Judged as no promise: past the window, not logged, broken.
        assert!(matches!(bad.verdict, Verdict::Broken(_)));
    }
}

/// Rule 26 compares a backfill release, 0.3.1 published after 0.3.5, against 0.3.5, the highest
/// release of the line in log order.
#[test]
fn a_backfill_release_is_checked_against_the_highest_of_its_line() {
    let publisher = key(50, "acme.publisher");
    let none = BTreeSet::new();
    let mut state = first_state();
    state.statements.insert(
        v("0.3.5"),
        parse_statement(&format!(
            "ita-release/1\nsource acme\nname acme/ledger\nversion 0.3.5\nprevious none\ndigest {D}\nmanifest {D}\nedition 0.4\nauthority Http[get]\ntrust none\n"
        ))
        .unwrap(),
    );
    let text = published("0.3.1", "");
    let (manifest, _) = parse_manifest(&text);
    let grows = submit::release(
        &manifest,
        files(&text),
        &authority(&[("Http", &["get", "post"])]),
        &none,
        &state,
        &[&publisher],
    );
    assert!(grows.unwrap_err().join("; ").contains("Http[post]"));
    let same = submit::release(
        &manifest,
        files(&text),
        &authority(&[("Http", &["get"])]),
        &none,
        &state,
        &[&publisher],
    );
    assert!(same.is_ok());
}

// --- Third recheck: the window's end, replaced promises, the cap, accepted consent ----------

/// The window ends at the later of the witness time and the local clock at sending, plus an
/// hour and the hour of skew: an honest promise two minutes past an hour from sending is kept.
#[test]
fn an_honest_promise_with_latency_is_kept() {
    let sent = 2_000_000_000;
    let record = Submitted {
        leaf: "ee".repeat(32),
        url: "file:///srv/ita".to_owned(),
        name: "acme/ledger".to_owned(),
        time: sent - 600,
        sent,
    };
    let promise = Promise {
        source: "acme".to_owned(),
        leaf: format!("sha256:{}", record.leaf),
        by: sent + PROMISE_WINDOW + 120,
        document: String::new(),
    };
    assert_eq!(
        judge(
            &record,
            Some(&promise),
            Some(sent + PROMISE_WINDOW + 60),
            sent + 3 * 3600
        ),
        Verdict::Kept
    );
}

/// The first verified promise is stored; a later one with another text is a replaced promise,
/// reported as evidence, and the earliest `by` is the one judged, so a replacement cannot turn
/// broken into kept.
#[test]
fn a_replaced_promise_is_evidence_and_the_earliest_by_is_judged() {
    let log = key(20, "acme.example/log");
    let leaf = "dd".repeat(32);
    let record = Submitted {
        leaf: leaf.clone(),
        url: "file:///srv/ita".to_owned(),
        name: "acme/ledger".to_owned(),
        time: 1000,
        sent: 1000,
    };
    let promise = |by: u64| {
        sign_note(
            &format!("ita-promise/1\nsource acme\nleaf sha256:{leaf}\nby {by}\n"),
            &[&log],
        )
        .unwrap()
    };
    let early = promise(2000);
    let late = promise(5000);
    // The first one seen is to be stored.
    let first = assess(
        &record,
        Ok(Some(early.clone().into_bytes())),
        None,
        log.key(),
        "acme",
        None,
        1500,
    );
    assert_eq!(first.first.as_deref(), Some(early.as_str()));
    assert!(!first.replaced);
    // The registry serves another: replaced, and judged by the earliest `by`.
    let replaced = assess(
        &record,
        Ok(Some(late.into_bytes())),
        Some(stored_under(early.clone().into_bytes(), &log)),
        log.key(),
        "acme",
        Some(4000),
        6000,
    );
    assert!(replaced.replaced);
    assert!(replaced.first.is_none(), "the first one stays");
    assert!(
        replaced
            .notes
            .iter()
            .any(|note| note.contains("replaced its promise"))
    );
    assert!(
        matches!(replaced.verdict, Verdict::Broken(_)),
        "logged at 4000, after 2000"
    );
    // The same promise again is no replacement.
    let same = assess(
        &record,
        Ok(Some(early.clone().into_bytes())),
        Some(stored_under(early.into_bytes(), &log)),
        log.key(),
        "acme",
        Some(1500),
        6000,
    );
    assert!(!same.replaced);
    assert_eq!(same.verdict, Verdict::Kept);
}

/// At most `MAX_JUDGED` submissions are judged per run, the oldest first, and the rest are
/// counted.
#[test]
fn a_run_judges_a_bounded_number_of_submissions() {
    let all: Vec<String> = (0..MAX_JUDGED + 5).map(|i| format!("{i:064x}")).collect();
    let (judged, waiting) = to_judge(all.clone());
    assert_eq!(judged.len(), MAX_JUDGED);
    assert_eq!(judged[0], all[0], "the oldest first");
    assert_eq!(waiting, 5);
    let (judged, waiting) = to_judge(all[..3].to_vec());
    assert_eq!((judged.len(), waiting), (3, 0));
}

// --- Owner chains in the audit (`spec/30 §14` rule 5) --------------------------------------

/// The client of `test` at its latest checkpoint.
fn client_of<'t>(memory: &'t Memory, test: &TestRegistry) -> Registry<'t> {
    Registry::open(
        "acme",
        memory,
        &test.pin(),
        &Point::Latest { now: NOW + 60 },
        None,
    )
    .unwrap()
}

/// The attack: a thief who stole the owner's keys K and L adds a key T, yanks with T and K and
/// publishes with them; the owner, with its two other keys, claims K compromised since before
/// the theft. The addition is void, so the yank is void and reported, the release is skipped as
/// compromised, and the name is never refused.
#[test]
fn entries_signed_only_through_a_void_owner_event_are_void() {
    let mut test = TestRegistry::new("acme");
    let stolen = keypair(60, "acme.laptop");
    let also = keypair(61, "acme.phone");
    let kept = keypair(62, "acme.desktop");
    let spare = keypair(63, "acme.spare");
    let thief = keypair(66, "acme.thief");
    test.register_keys("acme", &[&stolen, &also, &kept, &spare], 2, &[], 0);
    let (good, _) = test.release("acme/ledger", "0.3.0", "none", "none", &[&stolen, &kept]);
    test.owner_event(
        "acme",
        &format!("add {}", support::key_text(&thief.1)),
        2,
        &[&stolen, &also, &thief],
    );
    let yank = format!(
        "ita-yank/1\nsource acme\nname acme/ledger\nversion 0.3.0\n{}",
        test.previous("acme/ledger")
    );
    let document = support::signed(&yank, &[&thief, &stolen]);
    test.append("acme/ledger", &document);
    test.release("acme/ledger", "0.3.1", "none", "none", &[&thief, &stolen]);
    test.owner_event(
        "acme",
        &format!("revoke {} since {good}", stolen.1.name),
        2,
        &[&kept, &spare],
    );
    test.checkpoint(NOW);
    let memory = Memory {
        source: "acme".to_owned(),
        files: test.files.clone(),
    };
    let mut client = client_of(&memory, &test);
    let marks = client.marks("acme/ledger").unwrap();
    assert!(marks.yanked.is_empty(), "{marks:?}");
    assert_eq!(marks.voided.len(), 1);
    let releases = client.releases("acme/ledger").unwrap();
    let late = releases
        .iter()
        .find(|r| r.statement.version == v("0.3.1"))
        .unwrap();
    assert!(late.compromised);
    let state = client.state("acme/ledger", Line::Zero(3)).unwrap();
    assert!(state.iter().any(|r| r.version == v("0.3.1") && r.yanked));
    assert!(audit::evidence(&client).is_empty());
}

/// The audit honors an `accepted` or an `effect` index only as `una ita lock` does: for a
/// recovery or a dispute of the owner's chain, which this release never replays. Either line
/// naming a key event after the resolution point counts for nothing: the key change is
/// reported.
#[test]
fn the_audit_honors_accepted_indices_as_the_lock_does() {
    let mut test = TestRegistry::new("acme");
    let owner = keypair(60, "acme.laptop");
    test.register("acme", &owner);
    let (index, digest) = test.release("acme/ledger", "0.3.1", "none", "none", &[&owner]);
    test.checkpoint(NOW - 100);
    let resolution = test.entries.len() as u64;
    let added = keypair(64, "acme.new");
    let change = test.owner_event(
        "acme",
        &format!("add {}", support::key_text(&added.1)),
        2,
        &[&owner, &added],
    );
    test.checkpoint(NOW);
    let memory = Memory {
        source: "acme".to_owned(),
        files: test.files.clone(),
    };
    let mut client = client_of(&memory, &test);
    let mut locked = Locked {
        source: "acme".to_owned(),
        name: "acme/ledger".to_owned(),
        version: v("0.3.1"),
        logged: Some(index),
        digest: Some(digest),
        resolution,
        accepted: BTreeSet::new(),
        effects: BTreeSet::new(),
    };
    for (accepted, effects) in [(false, false), (true, false), (false, true)] {
        locked.accepted = accepted.then_some(change).into_iter().collect();
        locked.effects = effects.then_some(change).into_iter().collect();
        let facts = audit::facts(&mut client, &locked).unwrap();
        assert_eq!(facts.key_changes, [change], "{facts:?}");
        assert!(facts.pending.is_empty() && facts.accepted_effects.is_empty());
        let kinds: Vec<Kind> = audit::findings(0, &facts)
            .into_iter()
            .map(|f| f.kind)
            .collect();
        assert_eq!(kinds, [Kind::KeyChange(change)]);
    }
}

/// An owner chain that holds a kind of entry this release does not replay, a co-signer change
/// here: the audit cannot verify the package, unavailable (U7013), never refused, and nothing is
/// evidence against the registry.
#[test]
fn the_audit_reports_an_unreplayed_owner_entry_as_unavailable() {
    let mut test = TestRegistry::new("acme");
    let owner = keypair(60, "acme.laptop");
    test.register("acme", &owner);
    let (index, digest) = test.release("acme/ledger", "0.3.1", "none", "none", &[&owner]);
    test.checkpoint(NOW - 100);
    let resolution = test.entries.len() as u64;
    let trustee = keypair(67, "trustee.x");
    let change = test.owner_event(
        "acme",
        &format!("cosigners 1 {}", support::key_text(&trustee.1)),
        2,
        &[&owner],
    );
    test.checkpoint(NOW);
    let memory = Memory {
        source: "acme".to_owned(),
        files: test.files.clone(),
    };
    let mut client = client_of(&memory, &test);
    let locked = Locked {
        source: "acme".to_owned(),
        name: "acme/ledger".to_owned(),
        version: v("0.3.1"),
        logged: Some(index),
        digest: Some(digest),
        resolution,
        accepted: BTreeSet::new(),
        effects: BTreeSet::new(),
    };
    let (code, why) = audit::facts(&mut client, &locked).unwrap_err();
    assert_eq!(code, una_diag::Code::U7013, "{why}");
    assert!(
        why.contains(&format!("a co-signer change, at log entry {change},")),
        "{why}"
    );
    assert!(audit::evidence(&client).is_empty());
}

/// An owner chain entry the registry must not log, a claim signed by the key it names: the
/// audit refuses the package (U7010) and reports the registry's misbehavior as evidence, from
/// the client's typed record, once however often it is read.
#[test]
fn the_audit_reports_an_unacceptable_entry_as_evidence() {
    let mut test = TestRegistry::new("acme");
    let a = keypair(60, "acme.a");
    let b = keypair(61, "acme.b");
    let c = keypair(62, "acme.c");
    let d = keypair(64, "acme.d");
    test.register_keys("acme", &[&a, &b, &c, &d], 2, &[], 0);
    let (index, digest) = test.release("acme/ledger", "0.3.1", "none", "none", &[&c, &d]);
    test.checkpoint(NOW - 100);
    let resolution = test.entries.len() as u64;
    let claim = test.owner_event("acme", "revoke acme.a since 0", 2, &[&a, &b]);
    test.checkpoint(NOW);
    let memory = Memory {
        source: "acme".to_owned(),
        files: test.files.clone(),
    };
    let mut client = client_of(&memory, &test);
    assert!(audit::evidence(&client).is_empty());
    let locked = Locked {
        source: "acme".to_owned(),
        name: "acme/ledger".to_owned(),
        version: v("0.3.1"),
        logged: Some(index),
        digest: Some(digest),
        resolution,
        accepted: BTreeSet::new(),
        effects: BTreeSet::new(),
    };
    let (code, _) = audit::facts(&mut client, &locked).unwrap_err();
    assert_eq!(code, una_diag::Code::U7010);
    let evidence = audit::evidence(&client);
    let bad = match evidence.as_slice() {
        [audit::Evidence::Unacceptable(bad)] => bad.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        bad,
        Unacceptable {
            owner: "acme".to_owned(),
            index: claim,
            why: bad.why.clone(),
            rule: RULE,
        }
    );
    assert!(bad.why.contains("signed by `acme.a` itself"), "{}", bad.why);
    assert!(evidence[0].attributed());
    // Read again, it is recorded once.
    assert!(audit::facts(&mut client, &locked).is_err());
    assert_eq!(audit::evidence(&client), evidence);
    let text = evidence[0].text("acme");
    assert!(
        text.contains("evidence against the registry `acme`")
            && text.contains(&format!("the entry {claim} of the owner `acme`"))
            && text.contains("the owner's packages are refused"),
        "{text}"
    );
}

/// What a place failed to give never makes a verdict kept: a promise that does not verify, or
/// cannot be read, leaves a verdict that would be kept unavailable; a place that serves no
/// promise is judged by the window, as ruled; a broken verdict stays broken.
#[test]
fn a_promise_missed_never_makes_a_verdict_kept() {
    let log = key(20, "acme.example/log");
    let stranger = key(21, "acme.example/other");
    let leaf = "aa".repeat(32);
    let record = Submitted {
        leaf: leaf.clone(),
        url: "file:///srv/ita".to_owned(),
        name: "acme/ledger".to_owned(),
        time: 1000,
        sent: 1000,
    };
    let forged = sign_note(
        &format!("ita-promise/1\nsource acme\nleaf sha256:{leaf}\nby 2000\n"),
        &[&stranger],
    )
    .unwrap()
    .into_bytes();
    let now = 10_000;
    // Logged within the window: kept with no promise served, unavailable with one missed.
    let none = assess(&record, Ok(None), None, log.key(), "acme", Some(1500), now);
    assert_eq!(none.verdict, Verdict::Kept);
    let bad = assess(
        &record,
        Ok(Some(forged.clone())),
        None,
        log.key(),
        "acme",
        Some(1500),
        now,
    );
    assert!(matches!(bad.verdict, Verdict::Unavailable(_)), "{bad:?}");
    let unread = assess(
        &record,
        Err("no https transport yet".to_owned()),
        None,
        log.key(),
        "acme",
        Some(1500),
        now,
    );
    assert!(
        matches!(unread.verdict, Verdict::Unavailable(_)),
        "{unread:?}"
    );
    let stored = assess(
        &record,
        Ok(None),
        Some(stored_under(forged, &log)),
        log.key(),
        "acme",
        Some(1500),
        now,
    );
    assert!(
        matches!(stored.verdict, Verdict::Unavailable(_)),
        "{stored:?}"
    );
    // An unavailable verdict is not settled.
    let mut position = Position::start("acme", "acme");
    position.settle(&leaf, &unread.verdict).unwrap();
    assert!(position.settled.is_empty());
    // Not logged past the window: broken, whatever promise was missed.
    let broken = assess(
        &record,
        Err("no https transport yet".to_owned()),
        None,
        log.key(),
        "acme",
        None,
        now,
    );
    assert!(matches!(broken.verdict, Verdict::Broken(_)), "{broken:?}");
}

/// A registry file the bundle lacks is an omission, as any place's is: with no place behind
/// the bundle, the state is unavailable (U7013), never refused; with a place behind it that
/// serves the file, the bundle unbundles.
#[test]
fn a_file_the_bundle_lacks_is_an_omission() {
    let b = bundled();
    let mut files = b.files.clone();
    let tile = files
        .keys()
        .find(|path| path.starts_with("tile/entries/"))
        .unwrap()
        .clone();
    files.remove(&tile);
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &files);
    let store = Itaoca {
        root: scratch("unbundle-omission"),
    };
    let (at, failure) =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap_err();
    assert_eq!(at, Some(0));
    assert!(
        matches!(failure, Failure::Omitted(Omission::Document(_), _)),
        "{failure:?}"
    );
    assert!(!store.contains(&b.expected.digest).unwrap());
    // A place behind the bundle serves the file: verified like the rest, it fills the gap.
    let registry = Memory {
        source: "acme".to_owned(),
        files: b.test.files.clone(),
    };
    let behind = |_: &str| -> Vec<Box<dyn Transport>> { vec![Box::new(registry.clone())] };
    let done =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &behind).unwrap();
    assert_eq!(done.stored.len(), 1, "{done:?}");
}

/// A bundle carries every place's distinct copy of a kept checkpoint, whose cosignatures the
/// client merges: the writer records each, and the bundle's copy serves them all again, so
/// unbundling merges the same cosignatures offline.
#[test]
fn a_bundle_carries_every_places_copy_of_a_kept_checkpoint() {
    let place_of = |bytes: &[u8]| Memory {
        source: "acme".to_owned(),
        files: BTreeMap::from([("checkpoint/3".to_owned(), bytes.to_vec())]),
    };
    let place = Fallback::new(
        Box::new(place_of(b"one")),
        vec![Box::new(place_of(b"two")), Box::new(place_of(b"one"))],
    );
    let recording = Recording::new(&place);
    let copies = recording.get_all("checkpoint/3", 100).unwrap();
    assert_eq!(copies, [b"one".to_vec(), b"two".to_vec(), b"one".to_vec()]);
    let seen = recording.seen.into_inner();
    assert_eq!(seen["checkpoint/3"], b"one");
    assert_eq!(seen[&bundle::copy_path(1, "checkpoint/3")], b"two");
    assert_eq!(seen.len(), 2, "{seen:?}");
    let copy = Memory {
        source: "acme".to_owned(),
        files: seen,
    };
    assert_eq!(
        copy.get_all("checkpoint/3", 100).unwrap(),
        [b"one".to_vec(), b"two".to_vec()]
    );
    assert!(copy.get_all("checkpoint/4", 100).unwrap().is_empty());
}

/// A cadence gap the client met is evidence the audit reports, with its range of sizes, as a
/// note: evidence from an absence.
#[test]
fn the_audit_reports_cadence_evidence() {
    let a = keypair(60, "acme.a");
    let mut test = TestRegistry::new("acme");
    // A registry that leaves its checkpoints below a cosigned one uncosigned.
    test.bare = true;
    test.register("acme", &a);
    test.checkpoint(NOW - 30 * 86_400);
    let (release, _) = test.release("acme/ledger", "0.3.1", "none", "none", &[&a]);
    for at in 0..70 {
        test.append("other", &format!("filler {at}"));
    }
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
    // Never the point's time: the logged time is unavailable, an omission of the cadence, and
    // the gap is recorded as evidence.
    assert!(client.logged_at(release).is_err());
    // One place read: evidence from an absence, a note.
    let place = memory.describe();
    let gap = audit::Evidence::CadenceGap {
        from: release + 1,
        to: release + 64,
        places: vec![place.clone()],
        places_agreeing: 1,
    };
    assert_eq!(audit::evidence(&client), std::slice::from_ref(&gap));
    assert!(!gap.attributed());
    let text = gap.text("acme");
    assert!(
        text.starts_with("unattributed: a place or the registry `acme`")
            && text.contains(&format!("sizes {} to {}", release + 1, release + 64))
            && text.contains(&format!("places read: {place}; 1 of them")),
        "{text}"
    );
    // Two places with the same view: both agree, and it is still a note, since two places
    // can be one party.
    let both = Fallback::new(Box::new(memory.clone()), vec![Box::new(memory.clone())]);
    let mut client = Registry::open(
        "acme",
        &both,
        &test.pin(),
        &Point::Latest { now: NOW + 60 },
        None,
    )
    .unwrap();
    assert!(client.logged_at(release).is_err());
    let evidence = audit::evidence(&client);
    assert_eq!(evidence.len(), 1, "{evidence:?}");
    assert!(!evidence[0].attributed(), "{evidence:?}");
    assert!(
        evidence[0]
            .text("acme")
            .contains("2 of them served the same copy")
    );
}

/// A bundle carries the last root version its checkpoints name: the writer records the whole
/// root chain the client walks. One the bundle lacks is an omission, unavailable (U7013), and
/// never an older root trusted in its place.
#[test]
fn a_bundle_without_its_last_root_version_is_unavailable() {
    let b = bundled_with(Rotation::BeforePoint);
    assert!(b.files.contains_key("root/2"), "{:?}", b.files.keys());
    let store = Itaoca {
        root: scratch("unbundle-last-root"),
    };
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &b.files);
    let done =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap();
    assert_eq!(done.stored.len(), 1);
    let mut files = b.files.clone();
    files.remove("root/2");
    let store = Itaoca {
        root: scratch("unbundle-last-root-missing"),
    };
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &files);
    let (_, failure) =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap_err();
    assert!(
        matches!(failure, Failure::Unavailable(_) | Failure::Omitted(..)),
        "{failure:?}"
    );
    assert!(!store.contains(&b.expected.digest).unwrap());
}

/// A lock's point under an older root, cosigned by witnesses a later root retired, leaving
/// fewer than the threshold of old witnesses: the bundle carries the latest checkpoint under
/// the current root and the tiles of the prefix proof the client read, and every root version,
/// so unbundling verifies the point through the client's own rule, offline. Without the latest
/// checkpoint, the retired witnesses count for nothing and the point is unavailable.
#[test]
fn a_point_cosigned_by_retired_witnesses_unbundles() {
    for count in [1, 2] {
        let b = bundled_with(Rotation::AfterPoint(count));
        assert!(b.files.contains_key("checkpoint"), "{:?}", b.files.keys());
        assert!(b.files.contains_key("root/2"), "{:?}", b.files.keys());
        let store = Itaoca {
            root: scratch(&format!("unbundle-retired-{count}")),
        };
        let bytes = bundle_of(&b, &b.archive, &b.provenance, &b.files);
        let done =
            bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap();
        assert_eq!(done.stored.len(), 1, "{count}");
    }
    // Two of three witnesses retired, below the threshold of two: the point needs the proof,
    // and without the latest checkpoint it is an omission of the prefix proof, unavailable
    // (U7013), never a refusal.
    let b = bundled_with(Rotation::AfterPoint(2));
    let mut files = b.files.clone();
    files.remove("checkpoint");
    let store = Itaoca {
        root: scratch("unbundle-retired-no-latest"),
    };
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &files);
    let (_, failure) =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap_err();
    assert!(
        matches!(
            failure,
            Failure::Omitted(Omission::PrefixProof(size), _) if size == b.expected.checkpoint_size
        ),
        "{failure:?}"
    );
    assert!(!store.contains(&b.expected.digest).unwrap());
}

/// `spec/30 §11` rule 9 at unbundling: bundled copies whose latest checkpoint, signed by the log
/// under root 2, is of the lock's point's own size, while the point names root 1, are two
/// checkpoints of one size. Refused (U7010), said as evidence against the registry with both
/// versions, as `una ita lock --check` and `una ita bundle` say it, and nothing is stored.
#[test]
fn a_same_size_latest_in_a_bundle_is_evidence() {
    let b = bundled_with(Rotation::AfterPoint(2));
    let size = b.expected.checkpoint_size;
    let point = String::from_utf8(b.files[&format!("checkpoint/{size}")].clone()).unwrap();
    let text = point.split_once("\n\n").unwrap().0.to_owned() + "\n";
    let first_line = format!(
        "ita-root 1 {}",
        una_project::tlog::root::digest(&b.test.roots[0])
    );
    assert!(text.contains(&first_line), "{text}");
    let text = text.replacen(&first_line, &b.test.root_line(), 1);
    let mut latest = format!(
        "{text}\n{}",
        support::sign_line(&b.test.keys.log.0, &b.test.keys.log.1, &text)
    );
    for (pair, key) in &b.test.keys.witnesses {
        latest.push_str(&support::cosign_line(pair, key, &text, NOW));
    }
    let mut files = b.files.clone();
    files.insert("checkpoint".to_owned(), latest.into_bytes());
    let store = Itaoca {
        root: scratch("unbundle-same-size"),
    };
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &files);
    let (_, failure) =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap_err();
    assert!(
        matches!(
            &failure,
            Failure::Refused(why) if why.contains(&format!("two checkpoints of size {size}"))
                && why.contains("evidence against the registry `acme`")
                && why.contains("naming the root versions 1 and 2")
        ),
        "{failure:?}"
    );
    assert!(!store.contains(&b.expected.digest).unwrap());
}

/// ADR 0122, amendment of 2026-11-03: unbundling verifies each provenance as every load does.
/// A provenance whose point a retired witness cosigns, without the prefix proof it carries in
/// its newer format, is unavailable (U7013) even when the bundle holds the latest checkpoint
/// and its tiles; a chain below the root-version floor is refused (U7010); and a floor that
/// exists and cannot be read stops the unbundling (U7013), never a note.
#[test]
fn unbundling_verifies_the_provenance_as_every_load_does() {
    let b = bundled_with(Rotation::AfterPoint(2));
    let mut document = una_project::provenance::parse(&b.provenance).unwrap();
    assert!(document.latest.is_some());
    document.latest = None;
    document.format = una_project::provenance::Format::V1;
    let first = document.encode();
    let store = Itaoca {
        root: scratch("unbundle-format-1"),
    };
    let bytes = bundle_of(&b, &b.archive, &first, &b.files);
    let (_, failure) =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap_err();
    assert!(
        matches!(&failure, Failure::Unavailable(why) if why.contains("retired")),
        "{failure:?}"
    );
    assert!(!store.contains(&b.expected.digest).unwrap());
    // Below the floor: refused.
    let store = Itaoca {
        root: scratch("unbundle-below-floor"),
    };
    let id = registry_id(&b.expected.root);
    store
        .store_root_chain(&id, &[walked(3, &format!("sha256:{}", "0".repeat(64)))])
        .unwrap();
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &b.files);
    let (_, failure) =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place).unwrap_err();
    assert!(
        matches!(&failure, Failure::Refused(why) if why.contains("older than root version 3")),
        "{failure:?}"
    );
    assert!(!store.contains(&b.expected.digest).unwrap());
    // A floor that does not read, of either kind.
    for floor in ["root-version", "checkpoint"] {
        let store = Itaoca {
            root: scratch(&format!("unbundle-unreadable-{floor}")),
        };
        let dir = store.root.join("registry").join(&id);
        std::fs::create_dir_all(dir.join(floor)).unwrap();
        let (_, failure) =
            bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &no_place)
                .unwrap_err();
        assert!(
            matches!(&failure, Failure::Unavailable(why) if why.contains("cannot be read")),
            "{floor}: {failure:?}"
        );
        assert!(!store.contains(&b.expected.digest).unwrap());
    }
}

/// An equivocation, two checkpoints of one size the log signed naming different root
/// versions, is the registry's misbehavior: two signed copies back it.
#[test]
fn the_audit_reports_an_equivocation_against_the_registry() {
    let evidence = audit::Evidence::from(Misbehavior::Equivocation {
        size: 12,
        versions: vec![1, 2],
        places: vec!["a".to_owned(), "b".to_owned()],
    });
    assert!(evidence.attributed());
    let text = evidence.text("acme");
    assert!(
        text.starts_with("evidence against the registry `acme`")
            && text.contains("two checkpoints of size 12")
            && text.contains("root versions 1 and 2")
            && text.contains("places read: a, b"),
        "{text}"
    );
}

/// A first promise is verified against the log key of the root it was stored under, which
/// Itaoca records with it: a later rotation of the log's key leaves it verifiable, and the
/// submission is judged, not left unavailable.
#[test]
fn a_stored_promise_survives_a_rotation_of_the_log_key() {
    let old = key(20, "acme.example/log");
    let new = key(22, "acme.example/log2");
    let leaf = "aa".repeat(32);
    let record = Submitted {
        leaf: leaf.clone(),
        url: "file:///srv/ita".to_owned(),
        name: "acme/ledger".to_owned(),
        time: 1000,
        sent: 1000,
    };
    let promise = sign_note(
        &format!("ita-promise/1\nsource acme\nleaf sha256:{leaf}\nby 2000\n"),
        &[&old],
    )
    .unwrap()
    .into_bytes();
    // Stored under the old root, verified against its log key: kept, though the last root's
    // log key is the new one and the place serves no promise any more.
    let kept = assess(
        &record,
        Ok(None),
        Some(stored_under(promise.clone(), &old)),
        new.key(),
        "acme",
        Some(1500),
        10_000,
    );
    assert_eq!(kept.verdict, Verdict::Kept, "{kept:?}");
    assert!(kept.notes.is_empty(), "{kept:?}");
    // Verified against the new key instead, it would not verify: never kept.
    let wrong = assess(
        &record,
        Ok(None),
        Some(stored_under(promise.clone(), &new)),
        new.key(),
        "acme",
        Some(1500),
        10_000,
    );
    assert!(
        matches!(wrong.verdict, Verdict::Unavailable(_)),
        "{wrong:?}"
    );
    // A root the chain read does not hold: the promise does not verify, never kept.
    let lost = assess(
        &record,
        Ok(None),
        Some(Stored {
            bytes: promise,
            log: Err("not in the chain".to_owned()),
        }),
        new.key(),
        "acme",
        Some(1500),
        10_000,
    );
    assert!(matches!(lost.verdict, Verdict::Unavailable(_)), "{lost:?}");
    // The record of the root reads back, and nothing else does.
    let digest = format!("sha256:{}", "ab".repeat(32));
    let text = encode_stored_root(3, &digest);
    assert_eq!(parse_stored_root(&text).unwrap(), (3, digest.clone()));
    for bad in [
        String::new(),
        text.replace("root 3", "root 03"),
        text.replace("root 3", "root 0"),
        text.replace(&digest, "sha256:xyz"),
        format!("{text}extra\n"),
    ] {
        assert!(parse_stored_root(&bad).is_err(), "{bad:?}");
    }
}

/// The security keys of an advisory are judged first: the owner's signers, and so any doubt
/// about the timing of the owner's requests, matter only for a fix the security keys alone do
/// not lift. So an advisory with no fix, or one whose security keys alone lift its fix, never
/// waits on the owner's timing.
#[test]
fn an_advisory_judges_its_security_keys_before_the_owner() {
    let with = |security: usize| Signers {
        security,
        owner_threshold: false,
    };
    // Two security keys meet the lifting threshold of two alone: the owner is not needed.
    assert!(!with(2).needs_owner(true, 2));
    assert!(with(2).lifts_cooldown(2));
    // One security key and a fix: only the owner's threshold can lift it.
    assert!(with(1).needs_owner(true, 2));
    // No fix: nothing to lift, the owner is not needed.
    assert!(!with(1).needs_owner(false, 2));
    // No security key: the advisory does not stand, whatever the owner signs.
    assert!(!with(0).needs_owner(true, 2));
    // A threshold above two: two security keys are not enough alone.
    assert!(with(2).needs_owner(true, 3));
    // The owner's threshold given does not change whether it must be judged.
    let owner = Signers {
        security: 1,
        owner_threshold: true,
    };
    assert!(owner.needs_owner(true, 2));
}

/// A tile the bundle lacks, with only an https place behind it, which this release cannot
/// read: the place cannot serve, so it is skipped, and the tile is an omission of a document,
/// unavailable (U7013), never a refusal.
#[test]
fn a_tile_the_bundle_lacks_with_an_https_place_behind_is_an_omission() {
    let b = bundled();
    let mut files = b.files.clone();
    let tile = files
        .keys()
        .find(|path| path.starts_with("tile/entries/"))
        .unwrap()
        .clone();
    files.remove(&tile);
    let bytes = bundle_of(&b, &b.archive, &b.provenance, &files);
    let store = Itaoca {
        root: scratch("unbundle-https-behind"),
    };
    let https = |_: &str| -> Vec<Box<dyn Transport>> {
        vec![Box::new(una_project::transport::Unavailable {
            url: "https://ita.acme.example".to_owned(),
        })]
    };
    let (at, failure) =
        bundle::unbundle(&bytes, &store, std::slice::from_ref(&b.expected), &https).unwrap_err();
    assert_eq!(at, Some(0));
    assert!(
        matches!(&failure, Failure::Omitted(Omission::Document(path), _) if path.starts_with("tile/")),
        "{failure:?}"
    );
    assert!(!store.contains(&b.expected.digest).unwrap());
}

/// Each failure is coded by its own cause, never by an earlier one's: a package whose state
/// needs a kept checkpoint no place serves is U7013, and a package whose lock names another
/// release than the registry's is U7010, in either order on one client.
#[test]
fn audit_codes_each_failure_by_its_own_cause() {
    let mut test = TestRegistry::new("acme");
    let owner = keypair(60, "acme.laptop");
    test.register("acme", &owner);
    let (ledger, ledger_digest) = test.release("acme/ledger", "0.3.1", "none", "none", &[&owner]);
    test.append("other", "filler one");
    test.append("other", "filler two");
    let (tools, tools_digest) = test.release("acme/tools", "0.3.1", "none", "none", &[&owner]);
    test.checkpoint(NOW);
    // The kept checkpoint that times the ledger's release is served by no place.
    let mut files = test.files.clone();
    files.remove(&format!("checkpoint/{}", ledger + 1));
    let memory = Memory {
        source: "acme".to_owned(),
        files,
    };
    let locked_of = |name: &str, index: u64, digest: &str| Locked {
        source: "acme".to_owned(),
        name: name.to_owned(),
        version: v("0.3.1"),
        logged: Some(index),
        digest: Some(digest.to_owned()),
        resolution: 1,
        accepted: BTreeSet::new(),
        effects: BTreeSet::new(),
    };
    let unserved = locked_of("acme/ledger", ledger, &ledger_digest);
    // The lock names another log index than the registry's release.
    let mismatch = locked_of("acme/tools", tools + 1, &tools_digest);
    for packages in [
        [unserved.clone(), mismatch.clone()],
        [mismatch.clone(), unserved.clone()],
    ] {
        let mut client = Registry::open(
            "acme",
            &memory,
            &test.pin(),
            &Point::Latest { now: NOW + 60 },
            None,
        )
        .unwrap();
        let mut facts_of = |locked: &Locked| audit::facts(&mut client, locked);
        let result = audit::audit(&packages, &mut facts_of);
        let codes: Vec<(String, una_diag::Code)> = result
            .unverified
            .iter()
            .map(|(at, (code, _))| (packages[*at].name.clone(), *code))
            .collect();
        let expected = |name: &str| {
            if name == "acme/ledger" {
                una_diag::Code::U7013
            } else {
                una_diag::Code::U7010
            }
        };
        assert_eq!(codes.len(), 2, "{:?}", result.unverified);
        for (name, code) in &codes {
            assert_eq!(*code, expected(name), "{name}: {:?}", result.unverified);
        }
    }
}
