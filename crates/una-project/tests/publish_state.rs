//! Publishing's client side and the per-name state (ADR 0122 rules 24, 26, 37, 41 and 43).

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};

use pretty_assertions::assert_eq;
use una_project::tlog::publish::{Release, refusals, statement};
use una_project::tlog::release::parse_statement;
use una_project::tlog::state::{parse_advisory, parse_yank};
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
        assert!(
            found.iter().any(|r| r.contains(part)),
            "{extra:?}: {found:?}"
        );
    }
    let (bare, _) = parse_manifest("ita x\n  version 0.1.0\n  edition 0.4\n  publish to itaty\n");
    assert_eq!(refusals(&bare).len(), 2, "about and license");
}

#[test]
fn the_statement_written_reads_back() {
    let (manifest, _) = parse_manifest(&published("\nneeds\n  text_utf8 1.0.3\n  json 2.1\n"));
    let authority: BTreeMap<String, BTreeSet<String>> =
        [("Http".to_owned(), ["get".to_owned()].into_iter().collect())]
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

fn advisory(fixed: &str) -> String {
    format!(
        "ita-advisory/1\nsource itaty\nname acme/ledger\nprevious 3 {D}\nid ITA-2026-0007\nline 0.3\naffected 0.3.0, 0.3.1\nfixed {fixed}\nseverity high\nsummary An overflow in balances\n"
    )
}

/// A name's yanks and advisories are folded by the registry client (`Registry::marks`), whose
/// tests are in `registry.rs`; here, their form.
#[test]
fn yanks_read_strictly() {
    let yank = parse_yank(&format!(
        "ita-yank/1\nsource itaty\nname acme/ledger\nversion 0.3.0\nprevious 2 {D}\n"
    ))
    .unwrap();
    assert_eq!(yank.version, Version::parse_full("0.3.0").unwrap());
    assert_eq!(yank.previous.unwrap().index, 2);
    assert!(
        parse_yank("ita-yank/1\nsource itaty\nname acme/ledger\nversion 0.3\nprevious none\n")
            .is_err()
    );
}

#[test]
fn advisories_read_strictly() {
    assert!(parse_advisory(&advisory("none")).unwrap().fixed.is_none());
    assert!(
        parse_advisory(&advisory("0.3.1 sha256:00")).is_err(),
        "a short digest"
    );
    assert!(
        parse_advisory(&advisory(&format!("0.3.1 {E}"))).is_err(),
        "a fix that is affected"
    );
    assert!(
        parse_advisory(&advisory(&format!("1.0.0 {E}"))).is_err(),
        "a fix on another line"
    );
    assert!(parse_advisory(&advisory("none").replace("high", "urgent")).is_err());
}
