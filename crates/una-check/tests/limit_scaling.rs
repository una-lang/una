//! Scaling of `L_lim` (ADR 0117 IF4, ADR 0116 P8, slice T1): the label of each request covers the
//! bodies of every ancestor of its function in the call graph, computed by two passes of strongly
//! connected components, never by a search per request. A chain of `n` functions, each of which
//! makes a request and calls the next, has `n` requests with up to `n` ancestors each, so a search
//! per request would take quadratic time. Counted first (issue #1197): the successors the closures
//! follow, the edges of the search for the nearest raising ancestor and the requests labeled, which
//! the process-wide counter `pass_steps` counts, must grow linearly; timed as a fallback, by the
//! shared helper of `una-cli/tests/common/timing.rs`, over a span of 16 times the size against a
//! bound of 32 times, which separates linear (about 16 times) from quadratic (about 256 times) on a
//! shared machine: with a time a·n + b·n², it tolerates a quadratic share r = b·n0/a at the
//! smallest size n0 of at most (32 - 16)/(256 - 32) = 1/14, as tight as the old bound of 2.5 times
//! per doubling over 8 times the size (1/12). A second shape, counted only, holds `n` requests,
//! each in a task started by its own `tasks` statement: the precise `τ_conc` of each request comes
//! from `Concurrency`, built once, in O(1) per request (#1154), where a search of the constructs
//! per request would take O(n²). This file holds one test, so no other test of its binary moves
//! the counter while it reads it.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::{Duration, Instant};

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n";

/// Edition 0.4, for `tasks`. It is described by `ita.una`; the settings `una.toml` gives are the
/// same, and the project error that names `ita.una` does not matter here, as in
/// `tests/timing_scaling.rs`.
const MANIFEST_0_4: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.4\"\n";

/// A module of `n` functions, each making one request and calling the next, under a `top` that
/// works on a `secret` between two calls of the first: an ancestor of every request, whose code
/// between two requests (ADR 0116 P11) is `secret`.
fn app(n: usize) -> String {
    let mut text = String::from("module app\n");
    for i in 0..n {
        let next = if i + 1 == n {
            "unit".to_owned()
        } else {
            format!("f{}(url)", i + 1)
        };
        text.push_str(&format!(
            "\nf{i}(url: Text) -> Unit\n  uses @http: Http[get]\n=\n  let reply = attempt @http.get(url, headers: [], key_name: None)\n  {next}\n"
        ));
    }
    text.push_str(
        "\npub top(url: Text, key: secret Int) -> Unit\n  uses @http: Http[get]\n=\n  f0(url)\n  let doubled = key * 2\n  f0(url)\n",
    );
    text
}

/// A module of `n` functions, each making one request, and a `run` with `n` `tasks` statements,
/// each starting one of them beside work on a `secret`: `n` requests, each beside its own
/// construct.
fn started(n: usize) -> String {
    let mut text = String::from("module app\n\ncrunch(key: secret Int) -> secret Int = key * 3\n");
    for i in 0..n {
        text.push_str(&format!(
            "\nf{i}(url: Text) -> Unit\n  uses @http: Http[get]\n=\n  let reply = attempt @http.get(url, headers: [], key_name: None)\n  unit\n"
        ));
    }
    text.push_str("\npub run(url: Text, key: secret Int) -> Unit\n  uses @http: Http[get]\n=\n");
    for i in 0..n {
        text.push_str(&format!(
            "  tasks\n    start f{i}(url)\n    let spent{i} = crunch(key)\n    unit\n"
        ));
    }
    text.push_str("  unit\n");
    text
}

/// Whether `L_lim` of the package of size `n` is above `internal`, its codes, the pass steps
/// its type check took, and its time.
fn checked(n: usize) -> (bool, Vec<String>, usize, Duration) {
    checked_text(&app(n), MANIFEST, KeywordSet::V0_1)
}

/// [`checked`] for the package `text` under `manifest`.
fn checked_text(
    text: &str,
    manifest_text: &str,
    keywords: KeywordSet,
) -> (bool, Vec<String>, usize, Duration) {
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", manifest_text.as_bytes());
    let (settings, errors) = parse_manifest(manifest_text);
    // Only the error that names `ita.una`, for edition 0.4 (see [`MANIFEST_0_4`]).
    assert!(
        errors.is_empty() || (manifest_text == MANIFEST_0_4 && errors.len() == 1),
        "{errors:?}"
    );
    let id = db.add("src/app.una", text.as_bytes());
    let lexed = lex_in(db.file(id), id, &mut interner, keywords);
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
    let check = check_types(&input);
    let took = started.elapsed();
    let steps = una_core::label_counters::pass_steps() - steps;
    let internal = una_core::Label {
        level: una_syntax::ast::Level::Internal,
        ..una_core::Label::public()
    };
    (
        !check.limit_label.flows_to(&internal),
        check
            .diagnostics
            .iter()
            .map(|d| d.code.as_str().to_owned())
            .collect(),
        steps,
        took,
    )
}

/// A doubling of the chain at most doubles the pass steps, with a margin of 10 % for the steps that
/// do not grow, and 16 times the size takes less than 32 times as long; the `secret` work of `top`,
/// an ancestor of every request, raises `L_lim` above `internal`.
#[test]
fn the_history_of_a_limit_scales_linearly() {
    let sizes = [250, 500, 1_000, 2_000, 4_000];
    let judged = |n: usize, raised: bool, codes: &[String]| {
        assert!(raised, "`L_lim` of {n} functions stays at most `internal`");
        assert_eq!(codes, Vec::<String>::new());
    };
    let counts: Vec<usize> = sizes
        .iter()
        .map(|&n| {
            let (raised, codes, steps, _) = checked(n);
            judged(n, raised, &codes);
            steps
        })
        .collect();
    timing::assert_count_growth("the history of a limit", &sizes, &counts, 1.1);
    // Requests beside constructs, counted: `τ_conc` of each request is O(1) once `Concurrency`
    // is built, so a doubling at most doubles the pass steps, with the same margin.
    let beside: Vec<usize> = sizes[..4]
        .iter()
        .map(|&n| {
            let (raised, codes, steps, _) =
                checked_text(&started(n), MANIFEST_0_4, KeywordSet::V0_4);
            judged(n, raised, &codes);
            steps
        })
        .collect();
    timing::assert_count_growth("requests beside constructs", &sizes[..4], &beside, 1.1);
    // The fallback spans 16 times the smallest size against a bound of 32 (see the module
    // documentation).
    let span = [sizes[0], sizes[4]];
    timing::assert_growth(
        "the history of a limit",
        &span,
        0.0,
        timing::FLOOR,
        32.0,
        |n| {
            let (raised, codes, steps, took) = checked(n);
            judged(n, raised, &codes);
            // The count is deterministic: the same in every run.
            let expected = counts[sizes.iter().position(|&size| size == n).unwrap()];
            assert_eq!(steps, expected, "{n}: the steps of a second run");
            took.as_secs_f64()
        },
    );
}
