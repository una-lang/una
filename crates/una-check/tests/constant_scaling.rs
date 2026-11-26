//! Scaling of module constants (ADR 0126, Costs): a module of `n` constants, each naming the
//! one before it, checks, evaluates and elaborates in time linear in `n`. The order of the
//! constants and their cycles are found without recursion across constants, and each value is
//! evaluated once, so 16 times the constants take less than 32 times as long, which separates
//! linear (about 16 times) from quadratic (about 256 times) on a shared machine, by the shared
//! helper of `una-cli/tests/common/timing.rs`. A list constant named many times in one function
//! is also checked to cost a time linear in its references.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::Instant;

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.5\"\n";

/// The time `check_types` takes on `source`, a module of edition 0.5, which must check without
/// a diagnostic.
fn checked(source: &str) -> f64 {
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
    let settings = parse_manifest(MANIFEST).0;
    let id = db.add("src/m.una", source.as_bytes());
    let lexed = lex_in(db.file(id), id, &mut interner, KeywordSet::V0_5);
    let parsed = parse(&lexed, id, &interner);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    let input = PackageInput {
        sources: &db,
        interner: &interner,
        single: false,
        modules: vec![ModuleInput {
            id,
            tree: &parsed.file,
            broken: false,
        }],
        manifest: Some(manifest),
        settings: &settings,
        run: None,
        earlier_errors: false,
        locks: &[],
    };
    assert!(check_names(&input).is_empty());
    let started = Instant::now();
    let checked = check_types(&input);
    let took = started.elapsed().as_secs_f64();
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    took
}

/// `n` constants, each the one before it plus one, the last named by a function.
fn chain(n: usize) -> String {
    let mut text = String::from("module m\n\npub const c0: Int = 0\n");
    for i in 1..n {
        text.push_str(&format!("\npub const c{i}: Int = c{} + 1\n", i - 1));
    }
    text.push_str(&format!("\npub last() -> Int = c{}\n", n - 1));
    text
}

/// A list constant of 100 elements named `n` times in one function.
fn references(n: usize) -> String {
    let items: Vec<String> = (0..100).map(|i| i.to_string()).collect();
    let mut text = format!(
        "module m\n\npub const xs: List[Int] = [{}]\n\npub count() -> Int\n=\n  var total = 0\n",
        items.join(", ")
    );
    for _ in 0..n {
        text.push_str("  total = total + xs.len\n");
    }
    text.push_str("  total\n");
    text
}

#[test]
fn a_chain_of_constants_checks_in_linear_time() {
    timing::assert_growth(
        "a chain of module constants",
        &[1_000, 16_000],
        0.0,
        timing::FLOOR,
        32.0,
        |n| checked(&chain(n)),
    );
}

#[test]
fn references_to_a_list_constant_check_in_linear_time() {
    timing::assert_growth(
        "references to a list constant",
        &[100, 1_600],
        0.0,
        timing::FLOOR,
        32.0,
        |n| checked(&references(n)),
    );
}
