//! Scaling of the labels of concurrent work (ADR 0116 P2 to P4, slice 2 of #1087): `κ` of each
//! construct from its seed, `ctx` down each body's construct tree, and `conc` of every node in one
//! pass over the strongly connected components of the reversed graph, O(V + E + F) joins in all.
//! Three shapes, each at four sizes: a chain of `n` functions, each holding a `together` whose
//! branch calls the next; one function holding `n` constructs one after another; and one
//! function holding `together for`s nested `d` deep, one call at each level, where a `ctx` that
//! walked the parents of each construct would take O(d²). Counted first (ADR 0077): the package
//! pass steps, which the process-wide counter `pass_steps` counts, must at most double with each
//! doubling, with a margin of 10 %. The first two shapes are also timed, as a fallback, over a
//! span of 8 times the size, by the shared helper of `una-cli/tests/common/timing.rs` (#1197).
//! The nested shape is counted only: checking nested bodies is superlinear in time already,
//! before this analysis (the copying of items into every open frame, amendment E, which slice 8
//! of the plan on #1087 removes). This file holds one test, so no other test of its binary moves
//! the counter while it reads it.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

#[path = "../../una-cli/tests/common/timing.rs"]
mod timing;

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.4\"\n";

/// A chain of `n` functions, each with a `together` whose first branch calls the next.
fn chain(n: usize) -> String {
    let mut text = String::from("module app\n\nstep(n: Int) -> Int = n + 1\n");
    for i in 0..n {
        let call = if i + 1 == n {
            "step(n)".to_owned()
        } else {
            format!("f{}(n)", i + 1)
        };
        text.push_str(&format!(
            "\npub f{i}(n: Int) -> Int\n=\n  together\n    let a = {call}\n    let b = step(n)\n  a + b\n"
        ));
    }
    text
}

/// One function with `n` constructs one after another, each calling its own function.
fn many(n: usize) -> String {
    let mut text = String::from("module app\n\nstep(n: Int) -> Int = n + 1\n");
    for i in 0..n {
        text.push_str(&format!("\ng{i}(n: Int) -> Int = n + {i}\n"));
    }
    text.push_str("\npub run(n: Int) -> Int\n=\n");
    for i in 0..n {
        text.push_str(&format!(
            "  together\n    let a{i} = g{i}(n)\n    let b{i} = step(n)\n"
        ));
    }
    text.push_str("  n\n");
    text
}

/// `together for`s nested `d` deep in one function, each level calling `step` once.
fn nested(d: usize) -> String {
    let mut text =
        String::from("module app\n\nstep(n: Int) -> Int = n + 1\n\npub run(xs: List[Int]) -> ");
    text.push_str(&"List[".repeat(d));
    text.push_str("Int");
    text.push_str(&"]".repeat(d));
    text.push_str("\n=\n");
    for level in 0..d {
        let pad = "  ".repeat(level + 1);
        text.push_str(&format!("{pad}together for x{level} in xs\n"));
        text.push_str(&format!("{pad}  let c{level} = step(x{level})\n"));
    }
    text.push_str(&format!("{}step(x{})\n", "  ".repeat(d + 1), d - 1));
    text
}

/// The codes of the package `text`, the pass steps its type check took, and the number of
/// constructs it recorded.
fn checked(text: &str) -> (Vec<String>, usize, usize) {
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
    let steps = una_core::label_counters::pass_steps();
    let check = check_types(&input);
    let steps = una_core::label_counters::pass_steps() - steps;
    (
        check
            .diagnostics
            .iter()
            .map(|d| format!("{} {}", d.code.as_str(), d.message))
            .collect(),
        steps,
        check.timing_graph.constructs.len(),
    )
}

#[test]
fn the_labels_of_concurrent_work_scale_linearly() {
    let sizes = [250, 500, 1_000, 2_000];
    for (shape, make) in [
        ("a chain", chain as fn(usize) -> String),
        ("many constructs", many),
    ] {
        let counts: Vec<usize> = sizes
            .iter()
            .map(|&n| {
                let (codes, steps, constructs) = checked(&make(n));
                assert_eq!(codes, Vec::<String>::new(), "{shape} of {n}");
                assert_eq!(constructs, n, "{shape} of {n}");
                steps
            })
            .collect();
        timing::assert_count_growth(shape, &sizes, &counts, 1.1);
        let span = [sizes[0], sizes[3]];
        timing::assert_growth(shape, &span, 0.0, timing::FLOOR, 24.0, |n| {
            let (text, at) = (make(n), sizes.iter().position(|&size| size == n).unwrap());
            let ((codes, steps, _), took) = timing::timed(|| checked(&text));
            assert_eq!(codes, Vec::<String>::new(), "{shape} of {n}");
            // The count is deterministic: the same in every run.
            assert_eq!(
                steps, counts[at],
                "{shape} of {n}: the steps of a second run"
            );
            took
        });
    }
    // Nested constructs, counted: `ctx` comes from the root down, once per construct.
    let depths = [25, 50, 100, 200];
    let counts: Vec<usize> = depths
        .iter()
        .map(|&d| {
            // Deep nesting recurses deeply in the parser and the checker: a large stack, as
            // `una` itself runs them on.
            let text = nested(d);
            let (codes, steps, constructs) = std::thread::Builder::new()
                .stack_size(256 << 20)
                .spawn(move || checked(&text))
                .unwrap()
                .join()
                .unwrap();
            assert_eq!(codes, Vec::<String>::new(), "nested {d} deep");
            assert_eq!(constructs, d, "nested {d} deep");
            steps
        })
        .collect();
    timing::assert_count_growth("nested constructs", &depths, &counts, 1.1);
}
