//! Scaling of the timing label through handlers (ADR 0116 IF5 item 3, issue #1436, review R1 of
//! #1440): a site takes each handler it reaches once, with what its operation bodies run, and looks
//! up each function's installs by a range of a set ordered by caller, never by a scan of every
//! install per handler. A chain of `n` handlers, each of whose operation calls a function that
//! installs the next one, reached from one `within`, would take quadratic time with a scan per
//! handler. Counted first (ADR 0077): the steps of the timing label, which the process-wide counter
//! `timing_steps` counts, must grow linearly; timed as a fallback, over a span of 8 times the size,
//! by the shared helper of `una-cli/tests/common/timing.rs` (#1197), which separates linear (about
//! 8 to 12 times) from quadratic (about 64 times) on a shared machine. This file holds one test, so
//! no other test of its binary moves the counter while it reads it.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::{Duration, Instant};

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

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
/// steps that do not grow; 8 times the size takes less than 24 times as long, by the shared
/// helper of `una-cli/tests/common/timing.rs`; and the chain, on `public` data with no function
/// value, keeps `if error pass` accepted.
#[test]
fn the_timing_label_through_a_chain_of_handlers_scales_linearly() {
    let sizes = [250, 500, 1_000, 2_000];
    let counts: Vec<usize> = sizes
        .iter()
        .map(|&n| {
            let (codes, steps, _) = checked(n);
            assert_eq!(codes, Vec::<String>::new(), "a chain of {n} handlers");
            steps
        })
        .collect();
    assert!(counts[0] >= sizes[0], "the counter counts: {counts:?}");
    timing::assert_count_growth("the timing label", &sizes, &counts, 1.1);
    let span = [sizes[0], sizes[3]];
    timing::assert_growth("the timing label", &span, 0.0, timing::FLOOR, 24.0, |n| {
        let (codes, steps, took) = checked(n);
        assert_eq!(codes, Vec::<String>::new(), "a chain of {n} handlers");
        // The count is deterministic: the same in every run.
        let at = sizes.iter().position(|&size| size == n).unwrap();
        assert_eq!(steps, counts[at], "{n}: the steps of a second run");
        took.as_secs_f64()
    });
}
