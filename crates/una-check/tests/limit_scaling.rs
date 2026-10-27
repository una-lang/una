//! Scaling of `L_lim` (ADR 0117 IF4, ADR 0116 P8, slice T1): the label of each request covers
//! the bodies of every ancestor of its function in the call graph, computed by two passes of
//! strongly connected components, never by a search per request. A chain of `n` functions, each
//! of which makes a request and calls the next, has `n` requests with up to `n` ancestors each,
//! so a search per request would take quadratic time. Timed, so this file holds one test.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::{Duration, Instant};

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, SourceDb, lex, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n";

/// A module of `n` functions, each making one request and calling the next, under a `top` that
/// works on a `secret` before it calls the first: an ancestor of every request.
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
        "\npub top(url: Text, key: secret Int) -> Unit\n  uses @http: Http[get]\n=\n  let doubled = key * 2\n  f0(url)\n",
    );
    text
}

/// Whether `L_lim` of the package of size `n` is above `internal`, its codes, and the time it
/// took.
fn checked(n: usize) -> (bool, Vec<String>, Duration) {
    let text = app(n);
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
    let (settings, errors) = parse_manifest(MANIFEST);
    assert!(errors.is_empty(), "{errors:?}");
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
        took,
    )
}

/// A doubling of the chain may not take more than 2.5 times as long, and the `secret` work of
/// `top`, an ancestor of every request, raises `L_lim` above `internal`.
#[test]
fn the_history_of_a_limit_scales_linearly() {
    let sizes = [250, 500, 1_000, 2_000];
    let times: Vec<Duration> = sizes
        .iter()
        .map(|&n| {
            (0..3)
                .map(|_| {
                    let (raised, codes, took) = checked(n);
                    assert!(raised, "`L_lim` of {n} functions stays at most `internal`");
                    assert_eq!(codes, Vec::<String>::new());
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
