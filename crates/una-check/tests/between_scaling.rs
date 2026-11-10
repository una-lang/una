//! Scaling of the summaries of the code between requests (ADR 0116 P11, issue #1155): one walk
//! per body to collect its calls, the components of the call graph, one walk per body to build
//! its graph of expressions and four searches over it, O(V + E) for the whole program. Two
//! shapes, counted by the process-wide counter `pass_steps`: a chain of `n` functions, each
//! making a request, working, and calling the next, whose summaries a search per caller would
//! take quadratic time to join; and one body of `n` statements, requests and loops alternating,
//! whose graph a search per request would take quadratic time to cover. This file holds one test,
//! so no other test of its binary moves the counter while it reads it.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use una_check::{ModuleInput, PackageInput, between_summaries, check_names, check_types};
use una_core::{Label, Level};
use una_project::parse_manifest;
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.4\"\n";

/// A request, its outcome bound to `reply{i}`.
fn request(i: usize) -> String {
    format!("let reply{i} = attempt @http.get(url, headers: [], key_name: None)")
}

/// `n` functions, each making a request, working on a `public` and calling the next.
fn chain(n: usize) -> String {
    let mut text = String::from("module app\n\nwork(n: Int) -> Int = n * 3\n");
    for i in 0..n {
        let next = if i + 1 == n {
            "unit".to_owned()
        } else {
            format!("f{}(url, n)", i + 1)
        };
        text.push_str(&format!(
            "\nf{i}(url: Text, n: Int) -> Unit\n  uses @http: Http[get]\n=\n  {}\n  let spent = work(n)\n  {next}\n",
            request(i)
        ));
    }
    text
}

/// One body of `n` statements: a request, then a loop that makes one, in turn.
fn long(n: usize) -> String {
    let mut text = String::from(
        "module app\n\nwork(n: Int) -> Int = n * 3\n\npub run(url: Text, n: Int) -> Unit\n  uses @http: Http[get]\n=\n",
    );
    for i in 0..n {
        if i % 2 == 0 {
            text.push_str(&format!("  {}\n  let spent{i} = work(n)\n", request(i)));
        } else {
            text.push_str(&format!(
                "  for j{i} in 0..n\n    {}\n    let more{i} = work(j{i})\n    unit\n",
                request(i)
            ));
        }
    }
    text.push_str("  unit\n");
    text
}

/// The pass steps the summaries of `text` took.
fn steps(text: &str) -> usize {
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
    let (settings, _) = parse_manifest(MANIFEST);
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
    let names = check_names(&input);
    assert!(names.is_empty(), "{:?}", names.first());
    let check = check_types(&input);
    let top = Label {
        level: Level::Restricted,
        ..Label::public()
    };
    let before = una_core::label_counters::pass_steps();
    let units = between_summaries(&check.program, &top);
    let steps = una_core::label_counters::pass_steps() - before;
    // Every function is in Core IR and reaches a request at most `internal`.
    for (name, b) in &units {
        if name.starts_with("app.f") || name == "app.run" {
            assert!(b.reaches, "{name}");
            assert_eq!(b.gap.to_string(), "internal", "{name}");
        }
    }
    steps
}

/// Each doubling of either shape at most doubles the pass steps, with a margin of 10 % for the
/// steps that do not grow (the prelude's bodies).
#[test]
fn the_summaries_between_requests_scale_linearly() {
    let sizes = [250, 500, 1_000, 2_000];
    for shape in [chain as fn(usize) -> String, long] {
        let counts: Vec<usize> = sizes.iter().map(|&n| steps(&shape(n))).collect();
        for pair in counts.windows(2) {
            assert!(
                pair[1] * 10 <= pair[0] * 22,
                "the steps grew from {} to {} for a doubling: {counts:?}",
                pair[0],
                pair[1]
            );
        }
    }
}
