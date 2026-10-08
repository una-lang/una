//! Scaling of the type checker by deterministic counters (ADR 0077, #335): the label sets and
//! label names it makes. This file holds one test, so no other test of its binary moves the
//! process-wide counters while it reads them.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use una_check::{ModuleInput, PackageInput, check_names, check_types};
use una_project::parse_manifest;
use una_syntax::{Interner, SourceDb, lex, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.1\"\n";

/// Checks the #331 program of ten bindings `depth` deep, and returns the label sets, label names
/// and type nodes made while checking it. With `generic`, the bindings are in a generic body and hold a value
/// of `T` at their leaves, so every position carries the symbolic label `ℓ_T`.
fn made_checking(depth: usize, generic: bool) -> (usize, usize, usize) {
    let mut value = String::from(if generic { "p" } else { "1" });
    for level in 0..depth {
        value = if level % 2 == 0 {
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
    (after.0 - before.0, after.1 - before.1, nodes)
}

#[test]
fn labels_and_type_nodes_made_grow_at_most_as_their_bounds() {
    // ADR 0077 stage 1: a label shares its sets, so copying a type copies no label. For 4x the
    // depth, the sets and names made grow by at most 6x (linear is 4x, quadratic 16x).
    // The passes recurse over the syntax tree: a large stack, as `una-cli` gives them.
    let on_large_stack = |depth: usize| {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(move || (made_checking(depth, false), made_checking(depth, true)))
            .unwrap()
            .join()
            .unwrap()
    };
    let (small, small_generic) = on_large_stack(40);
    let (large, large_generic) = on_large_stack(160);
    for (small, large) in [(small, large), (small_generic, large_generic)] {
        assert!(small.0 > 0 && small.1 > 0, "the counters count: {small:?}");
        assert!(large.0 <= 6 * small.0, "sets: {small:?} then {large:?}");
        assert!(large.1 <= 6 * small.1, "names: {small:?} then {large:?}");
        // ADR 0077 stage 2a: the type nodes made still grow with the copies each expression
        // keeps of its type, about 15x for 4x the depth (quadratic is 16x). Stage 2b, whose
        // operations keep unchanged nodes shared, lowers this bound to 6x.
        assert!(small.2 > 0, "the node counter counts: {small:?}");
        assert!(large.2 <= 16 * small.2, "nodes: {small:?} then {large:?}");
    }
}
