//! Scaling of the labels of concurrent work (ADR 0116 P2 to P4, slice 2 of #1087): `κ` of each
//! construct from its seed, `ctx` down each body's construct tree, and `conc` of every node in one
//! pass over the strongly connected components of the reversed graph, O(V + E + F) joins in all.
//! Six shapes, each at four sizes: a chain of `n` functions, each holding a `together` whose
//! branch calls the next; the same chain whose last branch reaches one `within`, so that every
//! round of the probe of pass 2 computes the labels (slice 3); one function holding `n`
//! constructs one after another; `n` `tasks` statements whose tasks make requests, each of which
//! `L_res` counts with its `τ_conc` (slice 4); `n` `for ever within` loops over a chain of
//! calls (slices 5 and 7); and one
//! function holding `together for`s nested `d` deep, one call at each level, where a `ctx` that
//! walked the parents of each construct would take O(d²). Counted first (ADR 0077): the package
//! pass steps, which the process-wide counter `pass_steps` counts, must at most double with each
//! doubling, with a margin of 10 %. The first five shapes are also timed, as a fallback, over a
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

/// [`chain`] whose last branch calls one function with a `within`, which a part of every
/// construct reaches: each round of the probe of pass 2 computes `κ`, `ctx` and `conc` of the
/// whole package for it (slice 3 of #1087), O(V + E + F) per round, and the one site's `τ` costs
/// O(V + E).
fn chain_within(n: usize) -> String {
    let text = chain(n).replacen(
        "module app\n\n",
        "module app\n\nimport std.time (millisecond)\n\n",
        1,
    );
    let mut text = text.replace(
        "    let a = step(n)\n    let b = step(n)\n",
        "    let a = timed(n)\n    let b = step(n)\n",
    );
    text.push_str(
        "\ntimed(n: Int) -> Int\n=\n  let w = step(n) within 10 * millisecond\n    if error use 0\n  w\n",
    );
    text
}

/// `n` functions, each with a `tasks` statement whose started task makes a request, and one
/// `within`: `L_res` counts each statement with the precise `τ_conc` of its site (slice 4 of
/// #1087), O(1) joins each once the labels are computed, so the probe's rounds stay linear.
fn requests(n: usize) -> String {
    let mut text = String::from(
        "module app\n\nimport std.time (millisecond)\n\nstep(n: Int) -> Int = n + 1\n\nfetch(url: Text) -> Unit\n  uses @http: Http[get]\n=\n  let outcome = attempt @http.get(url, headers: [], key_name: None)\n  unit\n",
    );
    for i in 0..n {
        text.push_str(&format!(
            "\npub s{i}(url: Text) -> Unit\n  uses @http: Http[get]\n=\n  tasks\n    start fetch(url)\n  unit\n"
        ));
    }
    text.push_str(
        "\npub timed(n: Int) -> internal Int\n=\n  let w = step(n) within 10 * millisecond\n    if error use 0\n  w\n",
    );
    text
}

/// `n` functions that say `diverges`, each with a `for ever within` whose body calls `h<i>`,
/// where `h<i>` calls `h<i+1>`: loop `i` reaches the rest of the chain, so a search per loop
/// takes O(n²) in all, and the summaries of slice 7, with `τ_conc` taken at the loop once per
/// package (slice 5), O(n).
fn loops(n: usize) -> String {
    let mut text = String::from("module app\n\nimport std.time (millisecond)\n");
    for i in 0..n {
        let next = if i + 1 == n {
            "n".to_owned()
        } else {
            format!("h{}(n) + 1", i + 1)
        };
        text.push_str(&format!(
            "\nh{i}(n: Int) -> Int = {next}\n\npub g{i}(n: Int) -> Int\n  diverges\n=\n  let edit k = n\n  for ever within 100 * millisecond\n    k = h{i}(k)\n    stop\n  k\n"
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
        ("a chain with a `within`", chain_within),
        ("`tasks` statements that make requests", requests),
        ("`for ever within` loops", loops),
    ] {
        let counts: Vec<usize> = sizes
            .iter()
            .map(|&n| {
                let (codes, steps, constructs) = checked(&make(n));
                assert_eq!(codes, Vec::<String>::new(), "{shape} of {n}");
                // The loops are sites, not constructs.
                let expected = if shape.contains("loops") { 0 } else { n };
                assert_eq!(constructs, expected, "{shape} of {n}");
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
