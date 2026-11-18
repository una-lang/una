//! Scaling of the reach of `via` functions for the timing rule of `spec/07 §5` rule 7 (#1710):
//! the search without `Λ` and the look at each node it reaches for a function value from outside
//! ([`foreign_values`] in `types/timing.rs`) are linear, with a table of what each operation
//! answers built once. Two shapes, each at four sizes: a `via` that performs `n` operations of
//! one capability, and a `via` that reaches a chain of `n` functions ending in a call of a
//! function value it names. Counted (ADR 0077): the timing and pass steps, which the
//! process-wide counters count, must at most double with each doubling, with a margin of 10 %.
//! A lookup that scanned the operations for each reached one counted n(n+1)/2. This file holds
//! one test, so no other test of its binary moves the counters while it reads them.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.4\"\n";

const HEAD: &str = "module app\n\nimport std.time (millisecond)\n\npub policy key_of\n  from internal Int\n  to Int\n  via reveal_it\n  purpose \"a test\"\n  owner privacy\n";

/// A `via` that performs `n` operations of one capability, and a `within` that it runs.
fn operations(n: usize) -> String {
    let mut text = String::from(HEAD);
    text.push_str("\npub capability Tool\n");
    for i in 0..n {
        text.push_str(&format!("  op o{i}(x: internal Int) -> internal Int\n"));
    }
    text.push_str("\npub handler plain() for Tool\n");
    for i in 0..n {
        text.push_str(&format!("  op o{i}(x) = x\n"));
    }
    let names: Vec<String> = (0..n).map(|i| format!("o{i}")).collect();
    text.push_str(&format!(
        "\nreveal_it(t: internal Int) -> Int\n  uses @tool: Tool[{}]\n=\n",
        names.join(", ")
    ));
    for i in 0..n {
        text.push_str(&format!("  let v{i} = @tool.o{i}(t)\n"));
    }
    text.push_str("  let w: restricted Int = 1 within 1 * millisecond\n    if error use 0\n  t\n");
    text.push_str("\npub once(n: internal Int) -> Int\n=\n  key_of(n) with @tool = plain()\n");
    text
}

/// A `via` that reaches a chain of `n` functions, the last of which calls a function value it
/// names, and a `within` at the end of the chain.
fn chain(n: usize) -> String {
    let mut text = String::from(HEAD);
    text.push_str("\nreveal_it(t: internal Int) -> Int = f0(t)\n");
    for i in 0..n {
        let body = if i + 1 == n {
            "twice(f: bump, x: t)".to_owned()
        } else {
            format!("f{}(t)", i + 1)
        };
        text.push_str(&format!(
            "\nf{i}(t: internal Int) -> internal Int = {body}\n"
        ));
    }
    text.push_str(
        "\ntwice(f: fn(internal Int) -> internal Int, x: internal Int) -> internal Int = f(f(x))\n",
    );
    text.push_str("\nbump(x: internal Int) -> internal Int\n=\n  let w: restricted Int = 1 within 1 * millisecond\n    if error use 0\n  x + 1\n");
    text.push_str("\npub once(n: internal Int) -> Int = key_of(n)\n");
    text
}

/// The codes of the package `text`, and the timing and pass steps its type check took.
fn checked(text: &str) -> (Vec<String>, usize) {
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
    let steps =
        || una_core::label_counters::timing_steps() + una_core::label_counters::pass_steps();
    let before = steps();
    let check = check_types(&input);
    let steps = steps() - before;
    (
        check
            .diagnostics
            .iter()
            .map(|d| d.code.as_str().to_owned())
            .collect(),
        steps,
    )
}

#[test]
fn the_reach_of_via_functions_scales_linearly() {
    let sizes = [200, 400, 800, 1_600];
    for (shape, make) in [
        ("operations", operations as fn(usize) -> String),
        ("a chain", chain),
    ] {
        let counts: Vec<(Vec<String>, usize)> = sizes.iter().map(|&n| checked(&make(n))).collect();
        let first = &counts[0].0;
        for (codes, _) in &counts {
            assert_eq!(
                codes, first,
                "{shape}: the diagnostics change with the size"
            );
        }
        for pair in counts.windows(2) {
            let (small, large) = (pair[0].1, pair[1].1);
            assert!(small > 0, "{shape}: no steps counted");
            assert!(
                large * 10 <= small * 22,
                "{shape}: {small} -> {large} steps for a doubling: {:?}",
                counts.iter().map(|c| c.1).collect::<Vec<_>>()
            );
        }
    }
}
