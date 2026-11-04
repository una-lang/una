//! Scaling of the checks of calls into a boundary (ADR 0093): each call's grants are checked once,
//! and the releases reachable from the calls are found by one reverse search of the call graph, so
//! a package with a chain of `n` boundary functions and `n` calls into them is checked in linear
//! time (#565). Counted first (issue #1197): the steps of the reverse search and of the lookups at
//! each call, which the process-wide counter `pass_steps` counts, must grow linearly; timed as a
//! fallback, by the shared helper of `una-cli/tests/common/timing.rs`, over a span of 16 times the
//! size against a bound of 32 times, which separates linear (about 16 times) from quadratic (about
//! 256 times) on a shared machine: with a time a·n + b·n², it tolerates a quadratic share r =
//! b·n0/a at the smallest size n0 of at most (32 - 16)/(256 - 32) = 1/14, as tight as the old bound
//! of 2.5 times per doubling over 8 times the size (1/12). This file holds one test, so no other
//! test of its binary moves the counter while it reads it.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::{Duration, Instant};

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, SourceDb, lex, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n\n[boundaries]\n\"vendor\" = []\n\n[trusted]\nmodules = [\"release\"]\n";

const RELEASE: &str = "trusted module release\n\npub policy reveal\n  from secret Int\n  to public Int\n  via open\n  purpose \"scale\"\n  owner ops\n\nopen(n: secret Int) -> public Int = n\n";

/// A boundary module of `n` functions, each calling the next, the last one releasing.
fn vendor(n: usize) -> String {
    let mut text = String::from("module vendor\n\nimport release (reveal)\n");
    for i in 0..n {
        let body = if i + 1 == n {
            "reveal(n)".to_owned()
        } else {
            format!("f{}(n)", i + 1)
        };
        text.push_str(&format!("\npub f{i}(n: secret Int) -> Int = {body}\n"));
    }
    text
}

/// A module of `n` functions, each calling into the boundary.
fn app(n: usize) -> String {
    let names: Vec<String> = (0..n).map(|i| format!("f{i}")).collect();
    let mut text = format!("module app\n\nimport vendor ({})\n", names.join(", "));
    for i in 0..n {
        text.push_str(&format!("\npub g{i}(n: secret Int) -> Int = f{i}(n)\n"));
    }
    text
}

/// The codes `check_types` reports for the package of size `n`, the pass steps it took, and its
/// time.
fn checked(n: usize) -> (Vec<String>, usize, Duration) {
    let texts = [
        ("src/release.una", RELEASE.to_owned()),
        ("src/vendor.una", vendor(n)),
        ("src/app.una", app(n)),
    ];
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
    let (settings, errors) = parse_manifest(MANIFEST);
    assert!(errors.is_empty(), "{errors:?}");
    let ids: Vec<_> = texts
        .iter()
        .map(|(path, text)| db.add(*path, text.as_bytes()))
        .collect();
    let parsed: Vec<_> = ids
        .iter()
        .map(|&id| {
            let lexed = lex(db.file(id), id, &mut interner);
            let parsed = parse(&lexed, id, &interner);
            assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
            parsed
        })
        .collect();
    let input = PackageInput {
        sources: &db,
        interner: &interner,
        single: false,
        modules: ids
            .iter()
            .zip(&parsed)
            .map(|(&id, parsed)| ModuleInput {
                id,
                tree: &parsed.file,
                broken: false,
            })
            .collect(),
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
/// that do not grow, and 16 times the size takes less than 32 times as long; every call into the
/// boundary reaches the release at the end of the chain: one U3024 each.
#[test]
fn boundary_checks_scale_linearly() {
    let sizes = [250, 500, 1_000, 2_000, 4_000];
    let codes_of = |n: usize, codes: &[String]| {
        assert_eq!(codes, vec!["U3024".to_owned(); n]);
    };
    let counts: Vec<usize> = sizes
        .iter()
        .map(|&n| {
            let (codes, steps, _) = checked(n);
            codes_of(n, &codes);
            steps
        })
        .collect();
    timing::assert_count_growth("boundary checks", &sizes, &counts, 1.1);
    // The fallback spans 16 times the smallest size against a bound of 32 (see the module
    // documentation).
    let span = [sizes[0], sizes[4]];
    timing::assert_growth("boundary checks", &span, 0.0, timing::FLOOR, 32.0, |n| {
        let (codes, steps, took) = checked(n);
        codes_of(n, &codes);
        // The count is deterministic: the same in every run.
        let expected = counts[sizes.iter().position(|&size| size == n).unwrap()];
        assert_eq!(steps, expected, "{n}: the steps of a second run");
        took.as_secs_f64()
    });
}
