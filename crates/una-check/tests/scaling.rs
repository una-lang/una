//! Scaling of the type checker by deterministic counters (ADR 0077, #335): the label sets and
//! label names it makes. This file holds one test, so no other test of its binary moves the
//! process-wide counters while it reads them.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, SourceDb, lex, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n";

/// How a value nests: lists and options in turn, as #331 wrote it, lists only, or options only
/// (Una-1's review of #391: each nesting reaches the checker's types by its own path).
#[derive(Clone, Copy, Debug)]
enum Shape {
    Alternating,
    Lists,
    Options,
}

/// Checks the #331 program of ten bindings `depth` deep, and returns the label sets, label names
/// and type nodes made while checking it, and the nodes of the editor index's hover types. With `generic`, the bindings are in a generic body and hold a value
/// of `T` at their leaves, so every position carries the symbolic label `ℓ_T`.
fn made_checking(depth: usize, generic: bool, shape: Shape) -> (usize, usize, usize, usize) {
    let mut value = String::from(if generic { "p" } else { "1" });
    for level in 0..depth {
        let list = match shape {
            Shape::Alternating => level % 2 == 0,
            Shape::Lists => true,
            Shape::Options => false,
        };
        value = if list {
            format!("[{value}]")
        } else {
            format!("Some({value})")
        };
    }
    let header = if generic {
        "pub f[T](p: T) -> Int"
    } else {
        "pub f() -> Int"
    };
    let mut source = format!("module deep\n\n{header}\n=\n");
    for k in 0..10 {
        source.push_str(&format!(
            "  let x{k} = {value}\n  let y{k} = [x{k}, {value}]\n"
        ));
    }
    source.push_str("  1\n");
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
    let settings = parse_manifest(MANIFEST).0;
    let id = db.add("src/deep.una", source.as_bytes());
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
    };
    let before = una_core::label_counters::made();
    let nodes_before = una_core::label_counters::nodes_made();
    assert!(check_names(&input).is_empty());
    let checked = check_types(&input);
    let after = una_core::label_counters::made();
    let nodes = una_core::label_counters::nodes_made() - nodes_before;
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    (
        after.0 - before.0,
        after.1 - before.1,
        nodes,
        checked.index.hover_stats.nodes,
    )
}

#[test]
fn labels_and_type_nodes_made_grow_at_most_as_their_bounds() {
    // ADR 0077 stage 1: a label shares its sets, so copying a type copies no label. For 4x the
    // depth, the sets and names made grow by at most 6x (linear is 4x, quadratic 16x).
    // The passes recurse over the syntax tree: a large stack, as `una-cli` gives them.
    let on_large_stack = |depth: usize, shape: Shape| {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(move || {
                (
                    made_checking(depth, false, shape),
                    made_checking(depth, true, shape),
                )
            })
            .unwrap()
            .join()
            .unwrap()
    };
    // `Some(…)` takes two levels of the parser's nesting limit (`spec/02 §9`), a list one, so
    // options alone nest 30 and 120 deep.
    for (shape, small_depth, large_depth) in [
        (Shape::Alternating, 40, 160),
        (Shape::Lists, 40, 160),
        (Shape::Options, 30, 120),
    ] {
        let (small, small_generic) = on_large_stack(small_depth, shape);
        let (large, large_generic) = on_large_stack(large_depth, shape);
        for (small, large) in [(small, large), (small_generic, large_generic)] {
            assert!(
                small.0 > 0 && small.1 > 0,
                "{shape:?}: the counters count: {small:?}"
            );
            assert!(
                large.0 <= 6 * small.0,
                "{shape:?}: sets: {small:?} then {large:?}"
            );
            assert!(
                large.1 <= 6 * small.1,
                "{shape:?}: names: {small:?} then {large:?}"
            );
            // ADR 0077 stage 2b: an operation that changes nothing below a node keeps it;
            // stage 3a: a join that gives either side back keeps that side's nodes, a node with
            // a label below it is erased once per declaration, and each solved variable is
            // resolved once at its end. Checking makes nodes in proportion to the program's
            // size, in every shape, with a labeled leaf or not: about 4x for 4x the depth
            // (quadratic is 16x).
            assert!(small.2 > 0, "{shape:?}: the node counter counts: {small:?}");
            assert!(
                large.2 <= 6 * small.2,
                "{shape:?}: nodes: {small:?} then {large:?}"
            );
            // ADR 0080: the editor index copies each shared node of the checker's types once,
            // and prints none while checking, so its nodes grow as the checker's do.
            assert!(
                small.3 > 0,
                "{shape:?}: the hover counter counts: {small:?}"
            );
            assert!(
                large.3 <= 6 * small.3,
                "{shape:?}: hover nodes: {small:?} then {large:?}"
            );
        }
    }
}
