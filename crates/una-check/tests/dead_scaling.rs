//! Scaling of the judgment of dead failure handling (ADR 0100): with family errors in the package,
//! each U3025 site is judged plain or not against the U3007 sites, and that judgment stays linear
//! in the package (#602). Counted first (issue #1197): the sites, family sites and resolutions the
//! judgment examines, and the callers its search for poisoned functions follows, which the
//! process-wide counter `pass_steps` counts, must grow linearly; timed as a fallback, by the shared
//! helper of `una-cli/tests/common/timing.rs`, over a span of 8 times the size, which separates
//! linear (about 8 to 12 times) from quadratic (about 64 times) on a shared machine. This file
//! holds one test, so no other test of its binary moves the counter while it reads it.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::{Duration, Instant};

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, SourceDb, lex, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n";

/// A module of `n` functions, each with an unhandled call (U3007) and a dead clause (U3025).
fn many(n: usize) -> String {
    let mut text = String::from(
        "module m\n\nkind E = | Bad\n\nrisky(n: Int) -> Int\n  fails E\n=\n  if n < 0 then fail Bad else n\n",
    );
    for i in 0..n {
        text.push_str(&format!(
            "\npub f{i}(n: Int) -> Int\n=\n  let a = risky(n)\n  let b = n + {i}\n    if error use 0\n  a + b\n"
        ));
    }
    text
}

/// The codes `check_types` reports for [`many`]`(n)`, the pass steps it took, and its time.
fn checked(n: usize) -> (Vec<String>, usize, Duration) {
    let source = many(n);
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
    let settings = parse_manifest(MANIFEST).0;
    let id = db.add("src/m.una", source.as_bytes());
    let lexed = lex(db.file(id), id, &mut interner);
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
    let steps = una_core::label_counters::pass_steps();
    let started = Instant::now();
    let diagnostics = check_types(&input).diagnostics;
    let took = started.elapsed();
    let steps = una_core::label_counters::pass_steps() - steps;
    (
        diagnostics
            .iter()
            .map(|d| d.code.as_str().to_owned())
            .collect(),
        steps,
        took,
    )
}

/// A doubling of the package at most doubles the pass steps, with a margin of 10 % for the steps
/// that do not grow, and 8 times the size takes less than 24 times as long; every site is judged:
/// one U3007 and one U3025 per function.
#[test]
fn judging_dead_handling_scales_linearly() {
    let sizes = [500, 1_000, 2_000, 4_000];
    let codes_of = |n: usize, codes: &[String]| {
        let count = |code: &str| codes.iter().filter(|c| *c == code).count();
        assert_eq!(count("U3007"), n);
        assert_eq!(count("U3025"), n);
        assert_eq!(codes.len(), 2 * n, "{codes:?}");
    };
    let counts: Vec<usize> = sizes
        .iter()
        .map(|&n| {
            let (codes, steps, _) = checked(n);
            codes_of(n, &codes);
            steps
        })
        .collect();
    timing::assert_count_growth("judging dead handling", &sizes, &counts, 1.1);
    let span = [sizes[0], sizes[3]];
    timing::assert_growth(
        "judging dead handling",
        &span,
        0.0,
        timing::FLOOR,
        24.0,
        |n| {
            let (codes, steps, took) = checked(n);
            codes_of(n, &codes);
            // The count is deterministic: the same in every run.
            let at = sizes.iter().position(|&size| size == n).unwrap();
            assert_eq!(steps, counts[at], "{n}: the steps of a second run");
            took.as_secs_f64()
        },
    );
}
