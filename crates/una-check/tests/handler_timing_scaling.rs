//! Scaling of the timing label through handlers (ADR 0116 IF5 item 3, issue #1436, review R1 of
//! #1440): a site takes each handler it reaches once, with what its operation bodies run, and
//! looks up each function's installs by a range of a set ordered by caller, never by a scan of
//! every install per handler. A chain of `n` handlers, each of whose operation calls a function
//! that installs the next one, reached from one `within`, would take quadratic time with a scan
//! per handler. Counted first (ADR 0077): the steps of the timing label, which the process-wide
//! counter `timing_steps` counts, must grow linearly; timed as a fallback, over a span of 8
//! times the size, best of three, which separates linear (about 8 to 12 times) from quadratic
//! (about 64 times) on a shared machine. This file holds one test, so no other test of its
//! binary moves the counter while it reads it.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::{Duration, Instant};

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.4\"\n";

/// A module of `n` capabilities `C<i>`, a handler `h<i>` of each whose operation calls `g<i>`,
/// and `g<i>`, which installs `h<i+1>` and performs its operation, under one `within` that
/// installs `h0`.
fn app(n: usize) -> String {
    let mut text =
        String::from("module app\n\nimport std.time (millisecond)\n\npub kind E = | TimedOut\n");
    for i in 0..n {
        let body = if i + 1 == n {
            "k + 1".to_owned()
        } else {
            format!("@c.get(k) with @c = h{}()", i + 1)
        };
        text.push_str(&format!(
            "\ncapability C{i}\n  op get(k: Int) -> Int\n\nhandler h{i}() for C{i}\n  op get(k) = g{i}(k)\n\ng{i}(k: Int) -> Int\n=\n  {body}\n"
        ));
    }
    text.push_str(
        "\npub top(n: Int) -> Int\n  fails E\n=\n  let a = @c.get(n) with @c = h0() within 10 * millisecond\n    if error pass\n  a\n",
    );
    text
}

/// The codes of the package of size `n`, the timing steps its type check took, and its time.
fn checked(n: usize) -> (Vec<String>, usize, Duration) {
    let text = app(n);
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
    let steps = una_core::label_counters::timing_steps();
    let started = Instant::now();
    let check = check_types(&input);
    let took = started.elapsed();
    let steps = una_core::label_counters::timing_steps() - steps;
    (
        check
            .diagnostics
            .iter()
            .map(|d| format!("{} {}", d.code.as_str(), d.message))
            .collect(),
        steps,
        took,
    )
}

/// A doubling of the chain at most doubles the timing steps, with a margin of 10 % for the
/// steps that do not grow; 8 times the size takes at most 24 times as long, best of three; and
/// the chain, on `public` data with no function value, keeps `if error pass` accepted.
#[test]
fn the_timing_label_through_a_chain_of_handlers_scales_linearly() {
    let sizes = [250, 500, 1_000, 2_000];
    let mut steps = Vec::new();
    let mut times = Vec::new();
    for &n in &sizes {
        let runs: Vec<(usize, Duration)> = (0..3)
            .map(|_| {
                let (codes, steps, took) = checked(n);
                assert_eq!(codes, Vec::<String>::new(), "a chain of {n} handlers");
                (steps, took)
            })
            .collect();
        // The count is deterministic: the same in every run.
        assert!(runs.iter().all(|run| run.0 == runs[0].0), "{n}: {runs:?}");
        steps.push(runs[0].0);
        times.push(runs.iter().map(|run| run.1).min().unwrap());
    }
    assert!(steps[0] >= sizes[0], "the counter counts: {steps:?}");
    for (pair, sizes_pair) in steps.windows(2).zip(sizes.windows(2)) {
        assert!(
            pair[1] as f64 <= 2.2 * pair[0] as f64,
            "{} handlers took {} timing steps, {} took {}",
            sizes_pair[1],
            pair[1],
            sizes_pair[0],
            pair[0]
        );
    }
    let (small, large) = (times[0], times[3]);
    assert!(
        large.as_secs_f64() <= 24.0 * small.as_secs_f64() + 0.05,
        "{} handlers took {large:?}, {} took {small:?}",
        sizes[3],
        sizes[0]
    );
}
