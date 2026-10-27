//! ADR 0103's soundness guard for refined elements of collections. Unification ignores the
//! refinements at nested positions, as it ignores labels, so a value that moves between two
//! types without its `refinement` obligation would let a `List[Int]` into a `List[Pos]`
//! unseen. Each way a value moves has a conformance case,
//! `tests/conformance/verification/refined_elements_move_<kind>`, whose refuted movement fails
//! if its site is missed. This test fails when a kind of movement has no case, or a case names
//! a kind this list does not have, so the list and the cases stay one.

// Tests may unwrap freely (compiler/conventions.md §1).
#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use una_cli::Tier;

/// Every way a value moves from one type into another (ADR 0103 rules 3 and 4).
const MOVEMENTS: &[&str] = &[
    "annotated_let",
    "apply_result",
    "argument",
    "assignment",
    "call_result",
    "captured",
    "checked",
    "declassify",
    "edit_back",
    "element",
    "field",
    "field_read",
    "generic",
    "generic_field",
    "handler_result",
    "hash_items",
    "hash_with",
    "join",
    "map_entries",
    "map_get",
    "map_keys",
    "map_values",
    "map_with",
    "map_without",
    "operation_argument",
    "operation_result",
    "return",
    "seq_items",
    "seq_peek",
    "seq_update",
    "set_items",
    "set_with",
    "together",
    "with_argument",
];

/// No nested position: a right-hand side whose value is a scalar.
const SCALAR: &str = "";

// rhs-cases:begin
/// The movement case of each form of a Core right-hand side bound to a refined binder (ADR
/// 0103 rule 4), the forms the verifier's `rhs_type` types. The match has no catch-all: a new
/// form of `Rhs`, `Prim`, `MapOp`, `SeqOp`, `SetOp`, `HashMapOp` or `HashSetOp` does not compile
/// until it names its case here, and
/// `every_right_hand_side_has_a_case` then requires the case.
#[allow(dead_code)]
fn rhs_case(rhs: &una_core::Rhs) -> &'static str {
    use una_core::{HashMapOp, HashSetOp, MapOp, Prim, Rhs, SeqOp, SetOp};
    match rhs {
        Rhs::Atom(_) => "annotated_let",
        // An `edit` call's results, rebound to the places.
        Rhs::Tuple(_) => "edit_back",
        Rhs::Prim { prim, .. } => match prim {
            Prim::Map(op) => match op {
                MapOp::Has => SCALAR,
                MapOp::Get => "map_get",
                MapOp::With => "map_with",
                MapOp::Without => "map_without",
                MapOp::Keys => "map_keys",
                MapOp::Values => "map_values",
                MapOp::Entries => "map_entries",
            },
            Prim::Seq(op) => match op {
                SeqOp::PushFront | SeqOp::PushBack => "seq_update",
                SeqOp::PopFront | SeqOp::PopBack => "seq_update",
                SeqOp::PeekFront | SeqOp::PeekBack => "seq_peek",
                SeqOp::Items => "seq_items",
            },
            Prim::Set(op) => match op {
                SetOp::Has | SetOp::IsSubset => SCALAR,
                SetOp::First | SetOp::Last | SetOp::Between | SetOp::Items => "set_items",
                SetOp::With
                | SetOp::Without
                | SetOp::Union
                | SetOp::Intersection
                | SetOp::Difference => "set_with",
            },
            Prim::HashMap(op) => match op {
                HashMapOp::Has | HashMapOp::SameEntries => SCALAR,
                HashMapOp::Get | HashMapOp::Keys | HashMapOp::Values | HashMapOp::Entries => {
                    "hash_items"
                }
                HashMapOp::With | HashMapOp::Without => "hash_with",
            },
            Prim::HashSet(op) => match op {
                HashSetOp::Has | HashSetOp::IsSubset | HashSetOp::SameItems => SCALAR,
                HashSetOp::Items => "hash_items",
                HashSetOp::With
                | HashSetOp::Without
                | HashSetOp::Union
                | HashSetOp::Intersection
                | HashSetOp::Difference => "hash_with",
            },
            Prim::Add
            | Prim::Sub
            | Prim::Mul
            | Prim::Div
            | Prim::Mod
            | Prim::Neg
            | Prim::Eq
            | Prim::Ne
            | Prim::Lt
            | Prim::Le
            | Prim::Gt
            | Prim::Ge
            | Prim::Not
            | Prim::Len
            | Prim::Concat
            | Prim::ToText
            | Prim::Convert(_)
            | Prim::Fixed(..)
            | Prim::Float(_)
            | Prim::Shape(_)
            // A decimal's `Option` results take the binder's type without its refinements.
            | Prim::Decimal(_) => SCALAR,
        },
        Rhs::Call { .. } => "call_result",
        Rhs::Apply { .. } => "apply_result",
        Rhs::Op { .. } => "operation_result",
        Rhs::Construct { .. } => "generic_field",
        Rhs::Field { .. } => "field_read",
        Rhs::List(_) => "element",
        Rhs::Lambda { .. } => "captured",
        Rhs::Checked { .. } => "checked",
        Rhs::Declassify { .. } => "declassify",
    }
}
// rhs-cases:end

#[test]
fn every_right_hand_side_has_a_case() {
    let source = include_str!("refined_elements.rs");
    let block = source
        .split("// rhs-cases:begin")
        .nth(1)
        .and_then(|rest| rest.split("// rhs-cases:end").next())
        .unwrap();
    let named: BTreeSet<&str> = block
        .split("=> \"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    assert!(named.len() >= 15, "{named:?}");
    let listed: BTreeSet<&str> = MOVEMENTS.iter().copied().collect();
    let missing: Vec<&&str> = named.difference(&listed).collect();
    assert!(
        missing.is_empty(),
        "right-hand side forms whose case is not a movement: {missing:?}"
    );
}

#[test]
fn every_movement_has_a_refined_elements_case() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance/verification");
    let found: BTreeSet<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| {
            let name = entry.unwrap().file_name().into_string().unwrap();
            name.strip_prefix("refined_elements_move_")
                .map(str::to_owned)
        })
        .collect();
    let listed: BTreeSet<String> = MOVEMENTS.iter().map(|kind| (*kind).to_owned()).collect();
    let missing: Vec<&String> = listed.difference(&found).collect();
    let unlisted: Vec<&String> = found.difference(&listed).collect();
    assert!(
        missing.is_empty(),
        "movements without a refined_elements_move_ case: {missing:?}"
    );
    assert!(
        unlisted.is_empty(),
        "refined_elements_move_ cases for movements this list does not have: {unlisted:?}"
    );
    for kind in MOVEMENTS {
        let src = dir.join(format!("refined_elements_move_{kind}/src/move_{kind}.una"));
        let text = std::fs::read_to_string(&src).unwrap();
        assert!(
            text.lines()
                .any(|line| line.starts_with("-- expect: error U5001 at ")),
            "{kind}: the case must expect the refuted movement"
        );
    }
}

/// A package of edition 0.3 under the test's temporary directory, with the repository's lock,
/// holding `source` as its one module.
fn package(name: &str, source: &str) -> PathBuf {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("refined_elements")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("una.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"0.3\"\n"),
    )
    .unwrap();
    std::fs::copy(
        repository.join("verification.lock"),
        root.join("verification.lock"),
    )
    .unwrap();
    std::fs::write(root.join("src/m.una"), source).unwrap();
    root
}

/// The goals of the `refinement` obligations that tier 1 would generate for `source`, not
/// solved, on a stack as large as tier 1 gives them.
fn refinement_goals(name: &str, source: &str) -> Vec<String> {
    let root = package(name, source);
    let checked = una_cli::check(&root, Tier::Zero).unwrap();
    assert!(
        checked.diagnostics.is_empty(),
        "{name}: {:?}",
        checked.diagnostics
    );
    let obligations = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn_scoped(scope, || {
                una_verify::obligations(
                    &checked.program,
                    &checked.sources,
                    una_verify::SourceLayout::Joined,
                )
            })
            .unwrap()
            .join()
            .unwrap()
    })
    .unwrap();
    obligations
        .into_iter()
        .filter(|o| o.kind == una_diag::ObligationKind::Refinement)
        .map(|o| o.goal)
        .collect()
}

const POS: &str = "module m\n\npub kind Pos = Int where it > 0\n\n";

#[test]
fn a_list_literal_gives_one_obligation_per_element() {
    // ADR 0103 Complexity: a literal of k elements gives k obligations, linear in k.
    let counts: Vec<usize> = [10, 100, 1000]
        .iter()
        .map(|&k| {
            let items = vec!["p"; k].join(", ");
            let source = format!("{POS}pub many(p: Pos) -> List[Pos] = [{items}]\n");
            refinement_goals(&format!("literal_{k}"), &source).len()
        })
        .collect();
    assert_eq!(counts, vec![10, 100, 1000]);
}

#[test]
fn appends_in_a_body_give_one_obligation_each() {
    // ADR 0103 Complexity: each `append` into a `List[Pos]` binding is one argument flow.
    let counts: Vec<usize> = [10, 100, 1000]
        .iter()
        .map(|&k| {
            let mut source =
                format!("{POS}pub grown(p: Pos) -> List[Pos]\n=\n  let edit out: List[Pos] = []\n");
            for _ in 0..k {
                writeln!(source, "  out = out.append(item: p)").unwrap();
            }
            source.push_str("  out\n");
            refinement_goals(&format!("appends_{k}"), &source).len()
        })
        .collect();
    // At least one obligation per append, and the growth is linear: 10x the appends gives
    // about 10x the obligations.
    assert!(counts[0] >= 10, "{counts:?}");
    assert!(
        counts[2] <= counts[1] * 11 && counts[1] <= counts[0] * 11,
        "{counts:?}"
    );
}

#[test]
fn a_nested_flow_gives_one_obligation_per_refined_position() {
    // ADR 0103 Complexity: a flow compares a type of depth d in O(d), with one obligation per
    // refined position, here the innermost one, whatever the depth.
    for depth in 1..=8 {
        let wrap = |inner: &str| (0..depth).fold(inner.to_owned(), |ty, _| format!("List[{ty}]"));
        let source = format!(
            "{POS}pub moved(xs: {}) -> {} = xs\n",
            wrap("Int"),
            wrap("Pos")
        );
        let goals = refinement_goals(&format!("depth_{depth}"), &source);
        assert_eq!(goals.len(), 1, "depth {depth}: {goals:?}");
    }
}
