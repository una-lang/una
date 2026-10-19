//! Dense values of edition 0.4 (`spec/29 §17`, ADR 0120): the prelude's contracts are all
//! elaborated, the methods resolve by their receiver, and the checker's scaling stays linear.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

use std::time::Instant;

use una_check::{ModuleInput, PackageInput, TypeCheck, check_names, check_types, standard_source};
use una_diag::Diagnostic;
use una_project::{Edition, parse_manifest};
use una_syntax::{Interner, KeywordSet, SourceDb, lex_in, parse};

/// The name and type checks of one module `m` in a package of `edition`.
fn run(edition: &str, source: &str) -> (Vec<Diagnostic>, TypeCheck) {
    let manifest_text =
        format!("[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"{edition}\"\n");
    let mut db = SourceDb::new();
    let mut interner = Interner::new();
    let manifest = db.add("una.toml", manifest_text.as_bytes());
    let settings = parse_manifest(&manifest_text).0;
    let keywords = match settings.edition {
        Edition::V0_1 => KeywordSet::V0_1,
        Edition::V0_2 => KeywordSet::V0_2,
        Edition::V0_3 => KeywordSet::V0_3,
        Edition::V0_4 => KeywordSet::V0_4,
    };
    let id = db.add("src/m.una", source.as_bytes());
    let lexed = lex_in(db.file(id), id, &mut interner, keywords);
    let parsed = parse(&lexed, id, &interner);
    assert!(
        lexed.errors.is_empty() && parsed.errors.is_empty(),
        "syntax errors: {:?} {:?}",
        lexed.errors,
        parsed.errors
    );
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
    let types = check_types(&input);
    (names, types)
}

/// The names of the public functions a prelude source declares, its predicates aside.
fn declared_functions(prelude: &str) -> Vec<String> {
    prelude
        .lines()
        .filter_map(|line| line.strip_prefix("pub "))
        .filter(|rest| rest.starts_with(|c: char| c.is_ascii_lowercase()))
        .filter(|rest| {
            !rest.starts_with("kind ")
                && !rest.starts_with("capability ")
                && !rest.starts_with("predicate ")
        })
        .filter_map(|rest| {
            let end = rest.find(['(', '['])?;
            Some(rest[..end].to_owned())
        })
        .collect()
}

#[test]
fn every_function_of_the_prelude_is_elaborated_with_its_contract() {
    // A prelude function whose clauses do not elaborate would leave Core without its
    // contract, and calls of it without their preconditions: in every edition, the library
    // holds every function the prelude declares.
    for (edition, name) in [
        (Edition::V0_1, "0.1"),
        (Edition::V0_2, "0.2"),
        (Edition::V0_3, "0.3"),
        (Edition::V0_4, "0.4"),
    ] {
        let (names, types) = run(name, "module m\n\npub f() -> Int = 1\n");
        assert!(names.is_empty(), "{names:?}");
        assert!(types.diagnostics.is_empty(), "{:?}", types.diagnostics);
        let library: Vec<&str> = types
            .program
            .library
            .iter()
            .filter(|s| s.path.module == "prelude")
            .map(|s| s.path.name.as_str())
            .collect();
        let declared = declared_functions(&standard_source(edition).prelude);
        assert!(!declared.is_empty());
        for function in &declared {
            assert!(
                library.contains(&function.as_str()),
                "edition {name}: `{function}` of the prelude is not in Core's library"
            );
        }
    }
}

#[test]
fn every_dense_contract_keeps_each_of_its_clauses() {
    // Each function the prelude of edition 0.4 adds keeps, in Core, as many `requires` and
    // `ensures` clauses as its source writes (review of #1048, L1): counted from the source of
    // its declarations, one line per clause.
    let (_, types) = run("0.4", "module m\n\npub f() -> Int = 1\n");
    let source = standard_source(Edition::V0_4).prelude;
    let start = source.find("pub array[").expect("the dense prelude");
    let mut expected: Vec<(String, usize, usize)> = Vec::new();
    for line in source[start..].lines() {
        if let Some(rest) = line.strip_prefix("pub ") {
            let end = rest.find(['(', '[']).unwrap();
            expected.push((rest[..end].to_owned(), 0, 0));
        } else if let Some(last) = expected.last_mut() {
            if line.starts_with("  requires ") {
                last.1 += 1;
            } else if line.starts_with("  ensures ") {
                last.2 += 1;
            }
        }
    }
    assert_eq!(expected.len(), 132, "{expected:?}");
    for (name, requires, ensures) in expected {
        let signature = types
            .program
            .library
            .iter()
            .find(|s| s.path.module == "prelude" && s.path.name == name)
            .unwrap_or_else(|| panic!("`{name}` is in the library"));
        assert_eq!(signature.requires.len(), requires, "{name}");
        assert_eq!(signature.ensures.len(), ensures, "{name}");
    }
}

#[test]
fn a_function_that_computes_elements_keeps_no_refinement_in_its_result() {
    // Review of #1048, C1: `matrix_diagonal` places `+0` off the diagonal, so over a refined
    // element type its result is over the base; a view keeps the refinement.
    let source = "module m\n\npub f(a: Array[I32 in 1..10]) -> I32\n  requires a.len >= 2\n=\n  let d = matrix_diagonal(values: a)\n  let x = d.get(row: 0, column: 1)\n  100 / x\n";
    let (names, types) = run("0.4", source);
    assert!(names.is_empty() && types.diagnostics.is_empty());
    // Core's own text, which writes a refined type as `{base | refinements}` (review of #1048,
    // N6): the matrix is over the base, and no matrix of the function is over a refined type.
    let core = una_core::print_module(&types.program.modules[0]);
    assert!(core.contains("Matrix[I32]"), "{core}");
    assert!(!core.contains("Matrix[{"), "{core}");
}

#[test]
fn tensor_methods_take_the_function_of_their_rank() {
    // A list literal of r elements selects the prelude function of rank r, and its elements
    // are the arguments (`spec/29 §17`): the call in Core is to that function, with as many
    // index arguments as the literal has.
    let source = "module m\n\npub f(t: Tensor[F64]) -> F64\n  requires t.rank == 3 and t.dim(axis: 0) > 1 and t.dim(axis: 1) > 2 and t.dim(axis: 2) > 3\n=\n  t.get(at: [1, 2, 3])\n\npub g() -> Tensor[I8]\n=\n  let zero: I8 = 0\n  tensor_filled(shape: [2, 2], value: zero)\n";
    let (names, types) = run("0.4", source);
    assert!(names.is_empty(), "{names:?}");
    assert!(types.diagnostics.is_empty(), "{:?}", types.diagnostics);
    let core = una_core::print_module(&types.program.modules[0]);
    assert!(core.contains("dense_tensor_get_3"), "{core}");
    assert!(core.contains("dense_tensor_filled_2"), "{core}");
    let (_, types) = run(
        "0.4",
        "module m\n\npub f(s: List[Int]) -> Tensor[F64] = tensor_filled(shape: s, value: 0.0)\n",
    );
    assert_eq!(
        types
            .diagnostics
            .iter()
            .map(|d| d.code.as_str())
            .collect::<Vec<_>>(),
        vec!["U2038"]
    );
}

#[test]
fn dense_values_check_in_edition_0_4_only() {
    let source = "module m\n\npub f(a: Array[F64], m: Matrix[F64]) -> Int = a.len + m.rows\n";
    let (names, types) = run("0.4", source);
    assert!(names.is_empty(), "{names:?}");
    assert!(types.diagnostics.is_empty(), "{:?}", types.diagnostics);
    assert_eq!(types.program.modules[0].decls.len(), 1);
    let (names, _) = run("0.3", source);
    assert!(!names.is_empty(), "`Matrix` is not a name in edition 0.3");
}

#[test]
fn checking_dense_operators_is_linear_in_the_program() {
    // Each operator elaborates to one call and one lambda: the checker's time per operator
    // stays within a fixed ratio as the program grows tenfold (O(n)).
    let program = |n: usize| {
        let mut source = String::from(
            "module m\n\npub f(a: Array[F64], b: Array[F64]) -> Array[F64]\n  requires a.len == b.len\n=\n  let x0 = a + b\n",
        );
        for i in 1..n {
            source.push_str(&format!("  let x{i} = x{} - b + 2.0 * a\n", i - 1));
        }
        source.push_str(&format!("  x{}\n", n - 1));
        source
    };
    let time = |n: usize| {
        let source = program(n);
        let start = Instant::now();
        let (names, types) = run("0.4", &source);
        assert!(names.is_empty() && types.diagnostics.is_empty());
        start.elapsed().as_secs_f64()
    };
    let _ = time(20);
    let small = time(40).max(1e-4);
    let large = time(400);
    assert!(
        large / small < 40.0,
        "tenfold the operators took {:.1} times as long",
        large / small
    );
}
