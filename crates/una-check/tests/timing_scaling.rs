//! Scaling of the timing label of the sites (ADR 0116 amendment of 2026-10-25, E and I; slice 7
//! of #1087): `τ⁰` of each site comes from per-node summaries closed once per package
//! (`timing::Summaries`), not from a search of the call graph per site, which made `n` sites that
//! reach `n` functions O(n²). Two shapes, each at four sizes:
//! - a chain of `n` functions, each with a `together` whose branch calls the next under a
//!   `within`, so every site reaches the rest of the chain;
//! - one function with `n` `within`s beside one construct.
//!
//! Counted first (ADR 0077): the timing and pass steps, which the process-wide counters count,
//! must at most double with each doubling, with a margin of 10 %. Timed as a fallback, over a
//! span of 8 times the size, by the shared helper of `una-cli/tests/common/timing.rs` (#1197):
//! less than 24 times as long. The nested shape of section I comes with slice 8. This file holds
//! one test, so no other test of its binary moves the counters while it reads them.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.4\"\n";

const HEAD: &str = "module app\n\nimport std.time (millisecond)\n\nstep(n: Int) -> Int = n + 1\n";

/// A chain of `n` functions, each with a `together` whose first branch calls the next under a
/// `within`: that `within` reaches the rest of the chain.
fn chain(n: usize) -> String {
    let mut text = String::from(HEAD);
    for i in 0..n {
        let call = if i + 1 == n {
            "step(n)".to_owned()
        } else {
            format!("f{}(n)", i + 1)
        };
        text.push_str(&format!(
            "\npub f{i}(n: Int) -> Int\n=\n  together\n    let a = {call} within 10 * millisecond\n      if error use 0\n    let b = step(n)\n  a + b\n"
        ));
    }
    text
}

/// One function with `n` `within`s after one construct whose branches call `g`, which `run`
/// reaches from a part.
fn many(n: usize) -> String {
    let mut text = String::from(HEAD);
    text.push_str("\npub run(n: Int) -> Int\n=\n");
    for i in 0..n {
        text.push_str(&format!(
            "  let w{i} = step(n) within 10 * millisecond\n    if error use 0\n"
        ));
    }
    text.push_str("  n\n\npub outer(n: Int) -> Int\n=\n  together\n    let a = run(n)\n    let b = step(n)\n  a + b\n");
    text
}

/// The codes of the package `text`, and the timing and pass steps its type check took.
fn checked(text: &str) -> (Vec<String>, usize) {
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
    // Edition 0.4 is described by `ita.una`; the settings `una.toml` gives are the same, and
    // the project error that names `ita.una` does not matter here, as in `tests/dense.rs`.
    let settings = parse_manifest(MANIFEST).0;
    let id = db.add("src/app.una", text.as_bytes());
    let lexed = lex_in(db.file(id), id, &mut interner, KeywordSet::V0_4);
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
    let steps =
        || una_core::label_counters::timing_steps() + una_core::label_counters::pass_steps();
    let before = steps();
    let check = check_types(&input);
    let steps = steps() - before;
    (
        check
            .diagnostics
            .iter()
            .map(|d| format!("{} {}", d.code.as_str(), d.message))
            .collect(),
        steps,
    )
}

#[test]
fn the_timing_label_of_the_sites_scales_linearly() {
    let sizes = [250, 500, 1_000, 2_000];
    for (shape, make) in [
        ("a chain", chain as fn(usize) -> String),
        ("many sites", many),
    ] {
        let counts: Vec<usize> = sizes
            .iter()
            .map(|&n| {
                let (codes, steps) = checked(&make(n));
                assert_eq!(codes, Vec::<String>::new(), "{shape} of {n}");
                steps
            })
            .collect();
        timing::assert_count_growth(shape, &sizes, &counts, 1.1);
        let span = [sizes[0], sizes[3]];
        timing::assert_growth(shape, &span, 0.0, timing::FLOOR, 24.0, |n| {
            let (text, at) = (make(n), sizes.iter().position(|&size| size == n).unwrap());
            let ((codes, steps), took) = timing::timed(|| checked(&text));
            assert_eq!(codes, Vec::<String>::new(), "{shape} of {n}");
            // The count is deterministic: the same in every run.
            assert_eq!(
                steps, counts[at],
                "{shape} of {n}: the steps of a second run"
            );
            took
        });
    }
}
