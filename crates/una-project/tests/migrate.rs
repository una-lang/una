//! `una migrate`'s texts (ADR 0122 rules 63 and 64): every row of the mapping, names with
//! `-`, comments carried and left, and refusals.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use pretty_assertions::assert_eq;
use una_project::Edition;
use una_project::ita::{parse_ita_lock, parse_ita_manifest};
use una_project::migrate::{grant_line, migrate, text};

const LOCK: &str = "encoding_version = \"una-smt-1\"\nsolver = \"z3\"\nsolver_version = \"5.1.0\"\nlogic = \"QF_UFLIA\"\nstep_budget = 7\nrandom_seed = 3\n";

#[test]
fn every_row_of_the_mapping() {
    let toml = r#"# The events package.
[package]
name = "events-example"
version = "0.1.0"
# The edition before migration.
edition = "0.3"

[labels]
categories = ["PII", "Location"]

[trusted]
modules = ["events.keys"]

[escape_budget]
checked_per_module = 1  # one checked escape

[boundaries]
"gallery.thumbnailer" = ["Files[read]"]
"webhooks.template" = []

[contracts]
"pricing.update.update_prices" = ["@catalog: Catalog[price, set_price where not dry_run]"]

[run]
entry = "events.main"
grants = ["console", "files:data/:read,write", "keys:keys.toml", "http:https://api.example.com:8443:get,post", "http:https://other.example"]
"#;
    let migration = migrate(toml, Some(LOCK), None).unwrap();
    assert_eq!(
        migration.manifest,
        r#"-- The events package.
ita events_example
  version 0.1.0
  -- The edition before migration.
  edition 0.4

labels
  PII
  Location

trusted
  events.keys

escapes
  checked per module 1

boundaries
  gallery.thumbnailer
    may Files[read]
  webhooks.template

contracts
  pricing.update.update_prices
    @catalog: Catalog[price, set_price where not dry_run]

run
  entry events.main
  grants
    console
    files "data/" read, write
    keys "keys.toml"
    http "https://api.example.com:8443" get, post
    http "https://other.example"
"#
    );
    assert_eq!(
        migration.name,
        ("events-example".to_owned(), "events_example".to_owned())
    );
    assert_eq!(migration.edition, Edition::V0_3);
    assert_eq!(
        migration.comments_left,
        [(15, "# one checked escape".to_owned())]
    );
    assert!(!migration.default_lock);
    // The texts read back with no error, and the lock keeps the pin.
    let (manifest, errors) = parse_ita_manifest(&migration.manifest);
    assert_eq!(errors, []);
    assert_eq!(manifest.grants.len(), 5);
    let (lock, errors) = parse_ita_lock(&migration.lock);
    assert_eq!(errors, []);
    assert_eq!(lock.solver.step_budget, 7);
    assert_eq!(lock.solver.random_seed, 3);
}

#[test]
fn a_package_without_a_lock_gets_the_default_solver() {
    let toml = "[package]\nname = \"a\"\nversion = \"1.0.0\"\nedition = \"0.1\"\n";
    let migration = migrate(toml, None, None).unwrap();
    assert!(migration.default_lock);
    assert_eq!(
        migration.manifest,
        "ita a\n  version 1.0.0\n  edition 0.4\n"
    );
    assert!(migration.lock.contains("  steps 5000000\n"));
}

#[test]
fn edition_0_4_in_una_toml_migrates() {
    let toml = "[package]\nname = \"a\"\nversion = \"1.0.0\"\nedition = \"0.4\"\n";
    let migration = migrate(toml, Some(LOCK), None).unwrap();
    assert_eq!(migration.edition, Edition::V0_4);
}

#[test]
fn refusals() {
    let refused = |toml: &str, name: Option<&str>| migrate(toml, Some(LOCK), name).unwrap_err();
    let reasons = refused(
        "[package]\nname = \"9lives\"\nversion = \"1.0.0\"\nedition = \"0.1\"\n",
        None,
    );
    assert!(reasons[0].contains("--name"), "{reasons:?}");
    assert!(
        migrate(
            "[package]\nname = \"9lives\"\nversion = \"1.0.0\"\nedition = \"0.1\"\n",
            Some(LOCK),
            Some("lives")
        )
        .is_ok()
    );
    let reasons = refused(
        "[package]\nname = \"a\"\nversion = \"1.0\"\nedition = \"0.1\"\n",
        None,
    );
    assert!(reasons[0].contains("full version"), "{reasons:?}");
    let reasons = refused("[package]\nname = \"a\"\n", None);
    assert!(reasons.iter().any(|r| r.contains("U7002")), "{reasons:?}");
    let reasons = migrate(
        "[package]\nname = \"a\"\nversion = \"1.0.0\"\nedition = \"0.1\"\n",
        Some("solver = \"cvc5\"\n"),
        None,
    )
    .unwrap_err();
    assert!(reasons.iter().any(|r| r.starts_with("verification.lock")), "{reasons:?}");
}

#[test]
fn grants_and_texts() {
    assert_eq!(grant_line("console"), "console");
    assert_eq!(grant_line("files:a:b/c:read"), "files \"a:b/c\" read");
    assert_eq!(grant_line("http:http://h:80"), "http \"http://h:80\"");
    assert_eq!(text("a\"b\\c{d}\u{202e}"), "\"a\\\"b\\\\c\\{d\\}\\u{202E}\"");
}
