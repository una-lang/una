//! The memory a package of module constants costs (ADR 0126, #2316): a declaration that the
//! substitution budget rejects (U2041) is left out of Core IR, so a rejected package never
//! builds what the budget refuses, and the editor's hover of a constant is one text shared by
//! all its references, not a copy of its value at each one.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::sync::Arc;

use una_check::{ModuleInput, PackageInput, TypeCheck, check_names, check_types_with_rejected};
use una_project::parse_manifest;
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

const MANIFEST: &str = "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"0.5\"\n";

/// `check_types_with_rejected` on `source`, a module of edition 0.5, with the number of
/// rejected declarations whose Core IR it built.
fn checked(source: &str) -> (TypeCheck, usize) {
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", MANIFEST.as_bytes());
    let settings = parse_manifest(MANIFEST).0;
    let id = db.add("src/m.una", source.as_bytes());
    let lexed = lex_in(db.file(id), id, &mut interner, KeywordSet::V0_5);
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
    let (checked, rejected) = check_types_with_rejected(&input);
    (checked, rejected.len())
}

/// A text constant of 900,000 characters, named `n` times in one `expect` line, and a
/// function that names it once.
fn texts(n: usize) -> String {
    format!(
        "module m\n\npub const big: Text = \"{}\"\n\npub id(x: Int) -> Int = x\n\npub size() -> Int = big.len\n\ntest \"t\"\n  do id(1)\n  expect [{}].len == {n}\n",
        "a".repeat(900_000),
        vec!["big"; n].join(", ")
    )
}

#[test]
fn a_test_past_the_substitution_budget_is_left_out_of_core_ir() {
    // Under the budget: the test is elaborated.
    let (under, _) = checked(&texts(2));
    assert!(under.diagnostics.is_empty(), "{:?}", under.diagnostics);
    assert_eq!(under.program.modules[0].examples.len(), 1);
    // 12 copies of 900,000 characters pass the cap of 10,000,000: U2041, and the test, which
    // would hold those copies, is not elaborated, neither in the program nor among the
    // rejected declarations; the function, under the cap, still is.
    let (over, rejected) = checked(&texts(12));
    let codes: Vec<String> = over
        .diagnostics
        .iter()
        .map(|d| d.code.to_string())
        .collect();
    assert_eq!(codes, ["U2041"], "{:?}", over.diagnostics);
    let module = &over.program.modules[0];
    assert!(module.examples.is_empty(), "{:?}", module.examples.len());
    assert_eq!(rejected, 0);
    assert!(
        module
            .decls
            .iter()
            .any(|decl| matches!(decl, una_core::Decl::Fun(fun) if fun.name == "size")),
        "the function under the cap is elaborated"
    );
}

#[test]
fn every_reference_to_a_constant_shares_one_hover_text() {
    let digits = format!("1{}", "0".repeat(4_299));
    let source = format!(
        "module m\n\npub const n: Int = {digits}\n\npub a() -> Int = n\n\npub b() -> Int = n\n\npub c() -> Int = n\n"
    );
    let (checked, _) = checked(&source);
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let texts = &checked.index.constants;
    assert_eq!(texts.len(), 3, "{texts:?}");
    assert!(texts[0].1.contains(&digits), "{}", texts[0].1);
    assert!(
        texts.iter().all(|(_, text)| Arc::ptr_eq(text, &texts[0].1)),
        "each reference holds its own copy of the hover text"
    );
}
