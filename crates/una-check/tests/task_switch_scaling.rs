//! Scaling of the rule on check points that labeled data decide (`spec/27 §8` rule 5, issue
//! #1684): each check point is recorded in O(1) while its body is checked, and one pass over the
//! recorded sites and generic calls, with one worklist over the calls that pass a type
//! parameter's label on, reports them. A package of `n` functions, each with a call under a
//! `secret` branch and a call of a generic function at the head of a chain of `n` generic
//! functions, would take quadratic time with a walk of the chain per call. Timed over a span of
//! 8 times the size, by the shared helper of `una-cli/tests/common/timing.rs` (#1197), which
//! separates linear (about 8 to 12 times) from quadratic (about 64 times) on a shared machine.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::Instant;

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.4\"\n";

/// A module of a chain of `n` generic functions `g<i>`, the first of which calls a function
/// under a branch on values of its type parameter, of `n` functions `f<i>` that each call a
/// function under a `secret` branch and then `g<i>` with their `secret` parameter, and of one
/// `tasks` statement that starts each `f<i>`.
fn app(n: usize) -> String {
    let mut text = String::from(
        "module app\n\nnoop() -> Unit = unit\n\ng0[T](x: T, y: T) -> Unit\n=\n  if x == y\n    noop()\n",
    );
    for i in 1..=n {
        let before = i - 1;
        text.push_str(&format!(
            "\ng{i}[T](x: T, y: T) -> Unit = g{before}(x, y: y)\n\nf{i}(s: secret Int) -> Unit\n=\n  if s > 0\n    noop()\n  g{i}(s, y: 0)\n"
        ));
    }
    text.push_str("\npub main() -> Unit\n=\n  tasks\n");
    for i in 1..=n {
        text.push_str(&format!("    start f{i}(1)\n"));
    }
    text
}

/// The codes of the package of size `n`, and the seconds its type check took.
fn checked(n: usize) -> (Vec<String>, f64) {
    let text = app(n);
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
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
    let took = started.elapsed().as_secs_f64();
    (
        check
            .diagnostics
            .iter()
            .map(|d| format!("{} {}", d.code.as_str(), d.message))
            .collect(),
        took,
    )
}

/// 8 times the functions take less than 24 times as long, and each `f<i>` has its two U4002:
/// the call under its `secret` branch, and the call of `g<i>` whose chain ends at a check
/// point that its type argument, `secret` here, decides.
#[test]
fn the_rule_on_check_points_scales_linearly() {
    let span = [250, 2_000];
    timing::assert_growth(
        "check points that labeled data decide",
        &span,
        0.0,
        timing::FLOOR,
        24.0,
        |n| {
            let (codes, took) = checked(n);
            assert_eq!(codes.len(), 2 * n, "{n} functions: {:?}", codes.first());
            assert!(
                codes.iter().all(|code| code.starts_with("U4002 ")),
                "{:?}",
                codes.first()
            );
            took
        },
    );
}
