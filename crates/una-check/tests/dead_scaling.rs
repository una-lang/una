//! Scaling of the judgment of dead failure handling (ADR 0100): with family errors in the
//! package, each U3025 site is judged plain or not against the U3007 sites, and that judgment
//! stays linear in the package (#602). Timed, so this file holds one test.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::{Duration, Instant};

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

/// The codes `check_types` reports for [`many`]`(n)`, and the time it took.
fn checked(n: usize) -> (Vec<String>, Duration) {
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
    let started = Instant::now();
    let diagnostics = check_types(&input).diagnostics;
    let took = started.elapsed();
    (
        diagnostics
            .iter()
            .map(|d| d.code.as_str().to_owned())
            .collect(),
        took,
    )
}

/// A doubling of the package may not take more than 2.5 times as long, and every site is
/// judged: one U3007 and one U3025 per function.
#[test]
fn judging_dead_handling_scales_linearly() {
    let sizes = [500, 1_000, 2_000, 4_000];
    let times: Vec<Duration> = sizes
        .iter()
        .map(|&n| {
            (0..3)
                .map(|_| {
                    let (codes, took) = checked(n);
                    let count = |code: &str| codes.iter().filter(|c| *c == code).count();
                    assert_eq!(count("U3007"), n);
                    assert_eq!(count("U3025"), n);
                    assert_eq!(codes.len(), 2 * n, "{codes:?}");
                    took
                })
                .min()
                .unwrap()
        })
        .collect();
    for (pair, sizes_pair) in times.windows(2).zip(sizes.windows(2)) {
        let (small, large) = (pair[0], pair[1]);
        assert!(
            large.as_secs_f64() <= 2.5 * small.as_secs_f64() + 0.02,
            "{} functions took {large:?}, {} took {small:?}",
            sizes_pair[1],
            sizes_pair[0]
        );
    }
}
