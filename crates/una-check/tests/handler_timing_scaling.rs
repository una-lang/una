//! Scaling of the timing label through handlers (ADR 0116 IF5 item 3, issue #1436, review R1 of
//! #1440): a site takes each handler it reaches once, with what its operation bodies run, and
//! looks up each function's installs by a range of a set ordered by caller, never by a scan of
//! every install per handler. A chain of `n` handlers, each of whose operation calls a function
//! that installs the next one, reached from one `within`, would take quadratic time with a scan
//! per handler. Timed, so this file holds one test.

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

/// The codes of the package of size `n`, and the time its type check took.
fn checked(n: usize) -> (Vec<String>, Duration) {
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
    let started = Instant::now();
    let check = check_types(&input);
    let took = started.elapsed();
    (
        check
            .diagnostics
            .iter()
            .map(|d| format!("{} {}", d.code.as_str(), d.message))
            .collect(),
        took,
    )
}

/// A doubling of the chain may not take more than 2.5 times as long, and the chain, on
/// `public` data with no function value, keeps `if error pass` accepted.
#[test]
fn the_timing_label_through_a_chain_of_handlers_scales_linearly() {
    let sizes = [250, 500, 1_000, 2_000];
    let times: Vec<Duration> = sizes
        .iter()
        .map(|&n| {
            (0..3)
                .map(|_| {
                    let (codes, took) = checked(n);
                    assert_eq!(codes, Vec::<String>::new(), "a chain of {n} handlers");
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
            "{} handlers took {large:?}, {} took {small:?}",
            sizes_pair[1],
            sizes_pair[0]
        );
    }
}
