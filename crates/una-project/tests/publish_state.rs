//! Publishing's client side and the per-name state (ADR 0122 rules 24, 26, 37, 41 and 43).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};

use pretty_assertions::assert_eq;
use una_project::tlog::publish::{Release, refusals, statement};
use una_project::tlog::release::parse_statement;
use una_project::tlog::state::{Entry, Signers, audit, fold, parse_advisory, parse_yank};
use una_project::version::Version;
use una_syntax::ita::parse_manifest;

const D: &str = "sha256:3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b";
const E: &str = "sha256:4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e4e";

fn published(extra: &str) -> String {
    format!(
        "ita acme/ledger\n  version 0.3.1\n  edition 0.4\n  about \"Ledgers\"\n  license \"Apache-2.0\"\n  publish to itaty\n{extra}"
    )
}

#[test]
fn publishing_refuses_what_rule_43_names() {
    let (ok, _) = parse_manifest(&published("\nneeds\n  json 2.1\n"));
    assert_eq!(refusals(&ok), Vec::<String>::new());
    let (none, _) = parse_manifest("ita x\n  version 0.1.0\n  edition 0.4\n");
    assert!(refusals(&none)[0].contains("publish to"));
    for (extra, part) in [
        ("\nneeds\n  util from path \"../util\"\n", "path dependency"),
        ("\nneeds\n  store from workspace\n", "workspace member"),
        (
            "\nsources\n  acme \"https://ita.acme.example\"\n    root \"sha256:00\"\n\nneeds\n  acme/x 1.0 from acme\n",
            "needs only packages of `itaty`",
        ),
        ("\ntrust\n  json modules json.keys\n", "no `trust` section"),
    ] {
        let (manifest, _) = parse_manifest(&published(extra));
        let found = refusals(&manifest);
        assert!(found.iter().any(|r| r.contains(part)), "{extra:?}: {found:?}");
    }
    let (bare, _) = parse_manifest("ita x\n  version 0.1.0\n  edition 0.4\n  publish to itaty\n");
    assert_eq!(refusals(&bare).len(), 2, "about and license");
}

#[test]
fn the_statement_written_reads_back() {
    let (manifest, _) = parse_manifest(&published("\nneeds\n  text_utf8 1.0.3\n  json 2.1\n"));
    let authority: BTreeMap<String, BTreeSet<String>> = [(
        "Http".to_owned(),
        ["get".to_owned()].into_iter().collect(),
    )]
    .into_iter()
    .collect();
    let trust: BTreeSet<String> = ["ledger.keys".to_owned()].into_iter().collect();
    let text = statement(
        &manifest,
        &Release {
            previous: Some((47755, D)),
            digest: D,
            manifest_digest: E,
            authority: &authority,
            trust: &trust,
        },
    )
    .unwrap();
    let read = parse_statement(&text).unwrap();
    assert_eq!(read.name, "acme/ledger");
    assert_eq!(read.needs[0].1, "json");
    assert_eq!(read.needs[0].2.to_string(), "2.1.0");
    assert!(read.authority["Http"].contains("get"));
    assert!(read.trust.contains("ledger.keys"));
}

fn release(version: &str, digest: &str, logged_at: u64) -> Entry {
    let text = format!(
        "ita-release/1\nsource itaty\nname acme/ledger\nversion {version}\nprevious none\ndigest {digest}\nmanifest {E}\nedition 0.4\nauthority none\ntrust none\n"
    );
    Entry::Release(parse_statement(&text).unwrap(), logged_at)
}

fn advisory(fixed: &str) -> String {
    format!(
        "ita-advisory/1\nsource itaty\nname acme/ledger\nprevious 3 {D}\nid ITA-2026-0007\nline 0.3\naffected 0.3.0, 0.3.1\nfixed {fixed}\nseverity high\nsummary An overflow in balances\n"
    )
}

#[test]
fn the_state_folds_releases_yanks_and_advisories() {
    let yank = parse_yank(&format!(
        "ita-yank/1\nsource itaty\nname acme/ledger\nversion 0.3.0\nprevious 2 {D}\n"
    ))
    .unwrap();
    let one_key = Signers {
        security: 1,
        owner_threshold: false,
    };
    let two_keys = Signers {
        security: 2,
        owner_threshold: false,
    };
    let fix = parse_advisory(&advisory(&format!("0.3.2 {E}"))).unwrap();
    let entries = vec![
        release("0.3.0", D, 100),
        release("0.3.1", D, 200),
        Entry::Yank(yank),
        release("0.3.2", E, 300),
        Entry::Advisory(fix.clone(), one_key),
    ];
    let state = fold(&entries).unwrap();
    let v = |t: &str| Version::parse_full(t).unwrap();
    assert!(state[&v("0.3.0")].yanked);
    assert_eq!(state[&v("0.3.1")].advised, ["ITA-2026-0007"]);
    // One security key holds a release back, never hurries one in.
    assert!(!state[&v("0.3.2")].threshold_fix);
    let mut lifted = entries.clone();
    lifted[4] = Entry::Advisory(fix.clone(), two_keys);
    assert!(fold(&lifted).unwrap()[&v("0.3.2")].threshold_fix);
    let owner = Signers {
        security: 1,
        owner_threshold: true,
    };
    lifted[4] = Entry::Advisory(fix.clone(), owner);
    assert!(fold(&lifted).unwrap()[&v("0.3.2")].threshold_fix);
    // An advisory signed by no security key, a second statement for a version, a yank of
    // nothing: each is refused.
    lifted[4] = Entry::Advisory(fix, Signers { security: 0, owner_threshold: true });
    assert!(fold(&lifted).is_err());
    assert!(fold(&[release("0.3.0", D, 1), release("0.3.0", E, 2)]).is_err());
    assert_eq!(
        audit(&state, &v("0.3.1")),
        ["0.3.1 is named by the advisory ITA-2026-0007"]
    );
    // A yanked release that an advisory also names reports both (rule 41): neither hides
    // the other.
    assert_eq!(
        audit(&state, &v("0.3.0")),
        [
            "0.3.0 is yanked",
            "0.3.0 is named by the advisory ITA-2026-0007"
        ]
    );
    assert!(audit(&state, &v("0.3.2")).is_empty());
}

#[test]
fn advisories_read_strictly() {
    assert!(parse_advisory(&advisory("none")).unwrap().fixed.is_none());
    assert!(parse_advisory(&advisory("0.3.1 sha256:00")).is_err(), "a short digest");
    assert!(parse_advisory(&advisory(&format!("0.3.1 {E}"))).is_err(), "a fix that is affected");
    assert!(parse_advisory(&advisory(&format!("1.0.0 {E}"))).is_err(), "a fix on another line");
    assert!(parse_advisory(&advisory("none").replace("high", "urgent")).is_err());
}
