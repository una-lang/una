//! Scaling of the closure that marks calls `quiet` (S-39, S-42, #1515, `spec/26 §6` rule 4,
//! review L4 of #1551): `close_quiet` closes the functions that can append an event of their
//! own over the reversed call edges with one worklist, each function entered once, so it is
//! linear in the call edges, and each call under a raised program counter is then judged by one
//! set lookup. A chain of `n` functions down to one operation, each of them called under a
//! `secret` branch, would take quadratic time with a search of the callee's calls per quiet
//! call. Timed, since the closure has no counter, by the shared helper of
//! `una-cli/tests/common/timing.rs` (#1197), over a span of 8 times the size against a bound of
//! 24 times, which separates linear (about 8 times) from quadratic (about 64 times) on a shared
//! machine. The marks are counted too: every one of the `n` calls under a branch, and no other
//! call, is `quiet` in Core, so the test also fails on a closure that stops short of the chain.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::{Duration, Instant};

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, SourceDb, lex, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n";

/// A module of `n` functions `f<i>`, each calling `f<i+1>` at a `public` program counter, the
/// last one performing an operation of a capability that accepts `secret`, and `n` functions
/// `g<i>`, each calling `f<i>` under a `secret` branch.
fn app(n: usize) -> String {
    let mut text = String::from(
        "module app\n\ncapability Sink\n  op put(x: Int) -> Unit\n    accepts secret\n",
    );
    for i in 0..n {
        let body = if i + 1 == n {
            "@sink.put(k)".to_owned()
        } else {
            format!("f{}(k)", i + 1)
        };
        text.push_str(&format!(
            "\nf{i}(k: Int) -> Unit\n  uses @sink: Sink[put]\n=\n  {body}\n"
        ));
    }
    for i in 0..n {
        text.push_str(&format!(
            "\npub g{i}(s: secret Int) -> Unit\n  uses @sink: Sink[put]\n=\n  if s > 0\n    f{i}(1)\n  else\n    unit\n"
        ));
    }
    text
}

/// The codes of the package of size `n`, the number of `quiet` marks in its Core, and the time
/// its type check took.
fn checked(n: usize) -> (Vec<String>, usize, Duration) {
    let text = app(n);
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
    let settings = parse_manifest(MANIFEST).0;
    let id = db.add("src/app.una", text.as_bytes());
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
    let check = check_types(&input);
    let took = started.elapsed();
    let marks = check
        .program
        .modules
        .iter()
        .map(|module| {
            una_core::print_module(module)
                .lines()
                .filter(|line| line.trim() == "quiet")
                .count()
        })
        .sum();
    (
        check
            .diagnostics
            .iter()
            .map(|d| format!("{} {}", d.code.as_str(), d.message))
            .collect(),
        marks,
        took,
    )
}

/// The package is accepted and marks exactly the `n` calls under a branch `quiet` at every
/// size; 8 times the size takes less than 24 times as long, by the shared helper of
/// `una-cli/tests/common/timing.rs`.
#[test]
fn marking_calls_quiet_through_a_chain_scales_linearly() {
    let sizes = [250, 2_000];
    for &n in &[3, sizes[0]] {
        let (codes, marks, _) = checked(n);
        assert_eq!(codes, Vec::<String>::new(), "a chain of {n} functions");
        assert_eq!(marks, n, "a chain of {n} functions: the quiet calls");
    }
    timing::assert_growth(
        "marking calls quiet",
        &sizes,
        0.0,
        timing::FLOOR,
        24.0,
        |n| {
            let (codes, marks, took) = checked(n);
            assert_eq!(codes, Vec::<String>::new(), "a chain of {n} functions");
            assert_eq!(marks, n, "a chain of {n} functions: the quiet calls");
            took.as_secs_f64()
        },
    );
}
