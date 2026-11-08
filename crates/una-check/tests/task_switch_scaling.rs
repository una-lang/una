//! Scaling of the rule on check points that labeled data decide (`spec/27 §8` rule 5, issue
//! #1684): each check point is recorded in O(1) while its body is checked, and one pass over the
//! recorded sites and generic calls, with one worklist over the calls that pass a type
//! parameter's label on, reports them. A package of `n` functions, each with a call under a
//! `secret` branch and a call of a generic function at the head of a chain of `n` generic
//! functions, would take quadratic time with a walk of the chain per call. Timed over a span of
//! 8 times the size, by the shared helper of `una-cli/tests/common/timing.rs` (#1197), which
//! separates linear (about 8 to 12 times) from quadratic (about 64 times) on a shared machine.
//! With `internal` in place of `secret`, the same pass returns the check points of the residual
//! of #1708 for the report instead, `task switch` in `information.timing`, at the same cost.

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
/// function under a branch on their parameter, labeled `label`, and then `g<i>` with it, and,
/// with `tasks`, of one `tasks` statement that starts each `f<i>`.
fn app(n: usize, label: &str, tasks: bool) -> String {
    let mut text = String::from(
        "module app\n\nnoop() -> Unit = unit\n\ng0[T](x: T, y: T) -> Unit\n=\n  if x == y\n    noop()\n",
    );
    for i in 1..=n {
        let before = i - 1;
        text.push_str(&format!(
            "\ng{i}[T](x: T, y: T) -> Unit = g{before}(x, y: y)\n\nf{i}(s: {label} Int) -> Unit\n=\n  if s > 0\n    noop()\n  g{i}(s, y: 0)\n"
        ));
    }
    if tasks {
        text.push_str("\npub main() -> Unit\n=\n  tasks\n");
        for i in 1..=n {
            text.push_str(&format!("    start f{i}(1)\n"));
        }
    } else {
        text.push_str("\npub main() -> Unit\n=\n");
        for i in 1..=n {
            text.push_str(&format!("  f{i}(1)\n"));
        }
    }
    text
}

/// The codes of the package of size `n` whose data are labeled `label`, the constructs and
/// labels of its report's `timing` list, and the seconds its type check took.
fn checked(n: usize, label: &str, tasks: bool) -> (Vec<String>, Vec<String>, f64) {
    let text = app(n, label, tasks);
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
        check
            .timing
            .iter()
            .map(|site| format!("{} {} {}", site.construct, site.function, site.label))
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
            let (codes, timing, took) = checked(n, "secret", true);
            assert_eq!(codes.len(), 2 * n, "{n} functions: {:?}", codes.first());
            assert!(timing.is_empty(), "{:?}", timing.first());
            assert!(
                codes.iter().all(|code| code.starts_with("U4002 ")),
                "{:?}",
                codes.first()
            );
            took
        },
    );
}

/// #1708: with `internal` data, nothing is refused, and each `f<i>` has its two `task switch`
/// entries at `internal` for the report, the call under its branch and the call of `g<i>`,
/// found by the same pass in the same near linear time.
#[test]
fn the_residual_of_internal_data_is_listed_in_linear_time() {
    let span = [250, 2_000];
    timing::assert_growth(
        "check points that internal data decide",
        &span,
        0.0,
        timing::FLOOR,
        24.0,
        |n| {
            let (codes, timing, took) = checked(n, "internal", true);
            assert!(codes.is_empty(), "{n} functions: {:?}", codes.first());
            assert_eq!(timing.len(), 2 * n, "{n} functions: {:?}", timing.first());
            assert!(
                timing
                    .iter()
                    .all(|entry| entry.starts_with("task switch app.f")
                        && entry.ends_with(" internal")),
                "{:?}",
                timing.first()
            );
            took
        },
    );
}

/// #1708: the report lists no `task switch` for `public` data, nor in a program that starts
/// no task, whose schedule no count of check points moves.
#[test]
fn the_residual_needs_labeled_data_and_started_tasks() {
    let (codes, timing, _) = checked(4, "public", true);
    assert!(
        codes.is_empty() && timing.is_empty(),
        "{codes:?} {timing:?}"
    );
    let (codes, timing, _) = checked(4, "internal", false);
    assert!(
        codes.is_empty() && timing.is_empty(),
        "{codes:?} {timing:?}"
    );
    let (codes, timing, _) = checked(4, "secret", false);
    assert!(
        codes.is_empty() && timing.is_empty(),
        "{codes:?} {timing:?}"
    );
}
