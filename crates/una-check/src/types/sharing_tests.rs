//! ADR 0077 stage 2b: the type operations that return unchanged nodes as they are give the
//! same values as the operations they replace, which rebuild every node. The reference
//! operations below are those of main before stage 2b; values are compared by their `Debug`
//! dump, which shows every label and every refinement (`==` ignores refinements).

use una_core::Level;
use una_syntax::{FileId, SourceDb, Span};

use super::label::Label;
use super::ty::{
    Flagged, Flags, FnRefinements, FnTy, FnUse, KindLabels, Refinement, Shared, Ty, starred,
};
use super::unify::{Origin, Vars, combine, union_uses};

fn span() -> Span {
    let mut db = SourceDb::new();
    let file: FileId = db.add("a.una", b"x");
    Span::at(file, 0)
}

fn dump(ty: &Ty) -> String {
    format!("{ty:?}")
}

// ----- the operations before stage 2b, rebuilding every node -----

fn old_function(function: &FnTy, mut map: impl FnMut(&Ty) -> Ty, fails: bool) -> Ty {
    Ty::Fn(Shared::new(FnTy {
        params: function.params.iter().map(&mut map).collect(),
        result: map(&function.result),
        uses: function.uses.clone(),
        fails: if fails {
            function.fails.as_ref().map(&mut map)
        } else {
            function.fails.clone()
        },
        refinements: function.refinements.clone(),
    }))
}

fn old_erase(ty: &Ty) -> Ty {
    match ty {
        Ty::Labeled(_, inner) => old_erase(inner),
        Ty::List(element) => Ty::List(Shared::new(old_erase(element))),
        Ty::Map(key, value) => Ty::Map(Shared::new(old_erase(key)), Shared::new(old_erase(value))),
        Ty::Kind(id, args) => Ty::Kind(*id, args.iter().map(old_erase).collect()),
        Ty::Fn(function) => old_function(function, old_erase, true),
        other => other.clone(),
    }
}

fn old_substitute(ty: &Ty, args: &[Ty]) -> Ty {
    match ty {
        Ty::Labeled(label, inner) => old_substitute(inner, args).with_label(label),
        Ty::Param(index) => args.get(*index).cloned().unwrap_or(Ty::Error),
        Ty::List(element) => Ty::List(Shared::new(old_substitute(element, args))),
        Ty::Map(key, value) => Ty::Map(
            Shared::new(old_substitute(key, args)),
            Shared::new(old_substitute(value, args)),
        ),
        Ty::Kind(id, kind_args) => Ty::Kind(
            *id,
            kind_args.iter().map(|a| old_substitute(a, args)).collect(),
        ),
        Ty::Fn(function) => old_function(function, |t| old_substitute(t, args), true),
        other => other.clone(),
    }
}

fn old_raise_deep(ty: &Ty, label: &Label) -> Ty {
    let inner = match ty.unlabeled() {
        Ty::List(element) => Ty::List(Shared::new(old_raise_deep(element, label))),
        Ty::Map(key, value) => Ty::Map(
            Shared::new(old_raise_deep(key, label)),
            Shared::new(old_raise_deep(value, label)),
        ),
        Ty::Kind(id, args) => {
            Ty::Kind(*id, args.iter().map(|a| old_raise_deep(a, label)).collect())
        }
        other => other.clone(),
    };
    inner.with_label(&ty.top_label().join(label))
}

fn old_map_labels(ty: &Ty, f: &dyn Fn(&Label) -> Label) -> Ty {
    match ty {
        Ty::Labeled(label, inner) => old_map_labels(inner, f).with_label(&f(label)),
        Ty::List(element) => Ty::List(Shared::new(old_map_labels(element, f))),
        Ty::Map(key, value) => Ty::Map(
            Shared::new(old_map_labels(key, f)),
            Shared::new(old_map_labels(value, f)),
        ),
        Ty::Kind(id, args) => Ty::Kind(*id, args.iter().map(|a| old_map_labels(a, f)).collect()),
        Ty::Fn(function) => old_function(function, |t| old_map_labels(t, f), false),
        other => other.clone(),
    }
}

/// `Vars::resolve_shape` before stage 2b, for a type whose variables are all unsolved: every
/// node rebuilt, its failure kind included (#216), and each label put in normal form.
fn old_resolve_shape(ty: &Ty) -> Ty {
    match ty {
        Ty::Labeled(label, inner) => old_resolve_shape(inner).with_label(label),
        Ty::List(element) => Ty::List(Shared::new(old_resolve_shape(element))),
        Ty::Map(key, value) => Ty::Map(
            Shared::new(old_resolve_shape(key)),
            Shared::new(old_resolve_shape(value)),
        ),
        Ty::Kind(id, args) => Ty::Kind(*id, args.iter().map(old_resolve_shape).collect()),
        Ty::Fn(function) => old_function(function, old_resolve_shape, true),
        other => other.clone(),
    }
}

fn old_symbolic(ty: &Ty, names: &[String]) -> Ty {
    match ty {
        Ty::Labeled(label, inner) => old_symbolic(inner, names).with_label(label),
        Ty::Param(index) => match names.get(*index) {
            Some(name) => Ty::Param(*index).with_label(&Label::symbol(name.clone())),
            None => Ty::Param(*index),
        },
        Ty::List(element) => Ty::List(Shared::new(old_symbolic(element, names))),
        Ty::Map(key, value) => Ty::Map(
            Shared::new(old_map_labels(&old_symbolic(key, names), &starred)),
            Shared::new(old_symbolic(value, names)),
        ),
        Ty::Kind(id, args) => Ty::Kind(*id, args.iter().map(|a| old_symbolic(a, names)).collect()),
        Ty::Fn(function) => old_function(function, |t| old_symbolic(t, names), false),
        other => other.clone(),
    }
}

fn old_op_parameter(ty: &Ty, positive: bool, top: &Label, kinds: &KindLabels) -> Ty {
    let inner = match ty.unlabeled() {
        Ty::List(element) => Ty::List(Shared::new(old_op_parameter(element, positive, top, kinds))),
        Ty::Map(key, value) => Ty::Map(
            Shared::new(old_op_parameter(key, positive, top, kinds)),
            Shared::new(old_op_parameter(value, positive, top, kinds)),
        ),
        Ty::Kind(id, args) => Ty::Kind(
            *id,
            args.iter()
                .enumerate()
                .map(|(index, arg)| {
                    if kinds.is_invariant(*id, index) {
                        arg.clone()
                    } else {
                        old_op_parameter(arg, positive, top, kinds)
                    }
                })
                .collect(),
        ),
        Ty::Fn(function) => Ty::Fn(Shared::new(FnTy {
            params: function
                .params
                .iter()
                .map(|p| old_op_parameter(p, !positive, top, kinds))
                .collect(),
            result: old_op_parameter(&function.result, positive, top, kinds),
            uses: function.uses.clone(),
            fails: function.fails.clone(),
            refinements: function.refinements.clone(),
        })),
        other => other.clone(),
    };
    if positive {
        inner.with_label(top)
    } else {
        inner.with_label(&ty.top_label())
    }
}

fn old_combine(a: &Ty, b: &Ty, join: bool, kinds: &KindLabels) -> Ty {
    let (la, lb) = (a.top_label(), b.top_label());
    let label = if join { la.join(&lb) } else { la.meet(&lb) };
    let base = match (a.unlabeled(), b.unlabeled()) {
        (Ty::Never, other) | (other, Ty::Never) => return other.clone().with_label(&label),
        (Ty::Error, _) | (_, Ty::Error) => return Ty::Error,
        (Ty::List(x), Ty::List(y)) => Ty::List(Shared::new(old_combine(x, y, join, kinds))),
        (Ty::Map(xk, xv), Ty::Map(yk, yv)) => Ty::Map(
            Shared::new(old_combine(xk, yk, join, kinds)),
            Shared::new(old_combine(xv, yv, join, kinds)),
        ),
        (Ty::Kind(i, xs), Ty::Kind(j, ys)) if i == j && xs.len() == ys.len() => Ty::Kind(
            *i,
            xs.iter()
                .zip(ys)
                .enumerate()
                .map(|(index, (x, y))| {
                    if kinds.is_invariant(*i, index) {
                        x.clone()
                    } else {
                        old_combine(x, y, join, kinds)
                    }
                })
                .collect(),
        ),
        (Ty::Fn(f), Ty::Fn(g)) if f.params.len() == g.params.len() => Ty::Fn(Shared::new(FnTy {
            params: f
                .params
                .iter()
                .zip(&g.params)
                .map(|(x, y)| old_combine(x, y, !join, kinds))
                .collect(),
            result: old_combine(&f.result, &g.result, join, kinds),
            uses: union_uses(&f.uses, &g.uses),
            fails: f.fails.clone().or_else(|| g.fails.clone()),
            refinements: f.refinements.combine(&g.refinements, join),
        })),
        (Ty::Var(_), other) if !matches!(other, Ty::Var(_)) => other.clone(),
        (same, _) => same.clone(),
    };
    base.with_label(&label)
}

// ----- random types, deterministic -----

/// A linear congruential generator: the same sequence on every run.
struct Random(u64);

impl Random {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
}

fn some_label(random: &mut Random) -> Label {
    let mut label = match random.below(4) {
        0 => Label::public(),
        1 => Label::symbol("T"),
        2 => Label::symbol("T*"),
        _ if random.below(2) == 0 => Label::symbol("?1"),
        _ => Label::symbol("?3"),
    };
    if random.below(3) == 0 {
        label.level = Level::Secret;
    }
    if random.below(4) == 0 {
        label.categories.insert("PII");
    }
    label
}

fn some_refinements(random: &mut Random, params: usize) -> FnRefinements {
    let range = |random: &mut Random| -> Vec<Refinement> {
        (0..random.below(3))
            .map(|i| Refinement::Range(format!("in 0..{}", 5 + i), span()))
            .collect()
    };
    FnRefinements {
        params: (0..params).map(|_| range(random)).collect(),
        result: range(random),
    }
}

/// A random type of at most `depth` levels. `pool` holds nodes already made, which later
/// positions share: types with shared children, as unification makes them.
fn some_type(random: &mut Random, depth: u32, pool: &mut Vec<Shared<Ty>>) -> Ty {
    if depth == 0 || random.below(5) == 0 {
        return match random.below(9) {
            0 => Ty::Int,
            1 => Ty::Text,
            2 => Ty::Param(0),
            3 => Ty::Param(1),
            4 => Ty::Var(random.below(3) as usize),
            5 => Ty::Never,
            6 => Ty::Error,
            7 => Ty::Bool,
            _ => Ty::Unit,
        };
    }
    let child = |random: &mut Random, pool: &mut Vec<Shared<Ty>>| -> Shared<Ty> {
        if !pool.is_empty() && random.below(3) == 0 {
            return pool[random.below(pool.len() as u64) as usize].clone();
        }
        let node = Shared::new(some_type(random, depth - 1, pool));
        pool.push(node.clone());
        node
    };
    match random.below(7) {
        0 => Ty::List(child(random, pool)),
        1 => Ty::Map(child(random, pool), child(random, pool)),
        2 => {
            let count = random.below(3) as usize;
            let args: Vec<Ty> = (0..count).map(|_| (*child(random, pool)).clone()).collect();
            Ty::Kind(random.below(2) as usize, args.into())
        }
        3 => {
            let count = random.below(3) as usize;
            let params: Vec<Ty> = (0..count).map(|_| (*child(random, pool)).clone()).collect();
            Ty::Fn(Shared::new(FnTy {
                params,
                result: (*child(random, pool)).clone(),
                // Sometimes two of them name the same capability, which no checked program has
                // (U1002, U3006) and the fast paths must not rely on.
                uses: (0..random.below(4))
                    .map(|_| FnUse {
                        name: if random.below(2) == 0 { "log" } else { "out" }.to_owned(),
                        capability: (random.below(3) == 0).then_some(0),
                        ops: (0..1 + random.below(2))
                            .map(|_| {
                                if random.below(2) == 0 {
                                    "write"
                                } else {
                                    "read"
                                }
                                .to_owned()
                            })
                            .collect(),
                    })
                    .collect(),
                fails: (random.below(4) == 0).then(|| (*child(random, pool)).clone()),
                refinements: some_refinements(random, count),
            }))
        }
        4 => (*child(random, pool))
            .clone()
            .with_label(&some_label(random)),
        // Not in the form `with_label` builds: a label under a label, or a `public` one.
        5 => Ty::Labeled(some_label(random), child(random, pool)),
        _ => Ty::Labeled(Label::public(), child(random, pool)),
    }
}

/// Kinds 0 and 1, of one and two parameters; the first parameter of kind 1 is invariant.
fn kinds() -> KindLabels {
    let invariant_field = Ty::Fn(Shared::new(FnTy {
        params: vec![Ty::Param(0)],
        result: Ty::Unit,
        uses: Vec::new(),
        fails: None,
        refinements: FnRefinements::default(),
    }));
    let plain_field = Ty::Param(0);
    KindLabels::new(&[vec![&plain_field], vec![&invariant_field]], &[1, 2])
}

#[test]
fn every_fast_path_gives_the_value_the_rebuilding_operation_gave() {
    let kinds = kinds();
    let names = ["T".to_owned(), "U".to_owned()];
    let args = [
        Ty::Int.with_label(&Label::symbol("A")),
        Ty::List(Shared::new(Ty::Text)),
    ];
    let mut random = Random(0x5eed);
    let vars = Vars::default();
    for case in 0..4000 {
        let mut pool = Vec::new();
        let ty = some_type(&mut random, 5, &mut pool);
        let label = some_label(&mut random);
        let f = |l: &Label| {
            if l.is_public() {
                l.clone()
            } else {
                l.join(&Label::symbol("X"))
            }
        };
        let at = |what: &str| format!("case {case}, {what}: {ty:?}");
        assert_eq!(dump(&ty.erase()), dump(&old_erase(&ty)), "{}", at("erase"));
        assert_eq!(
            dump(&ty.substitute(&args)),
            dump(&old_substitute(&ty, &args)),
            "{}",
            at("substitute")
        );
        assert_eq!(
            dump(&ty.raise_deep(&label)),
            dump(&old_raise_deep(&ty, &label)),
            "{}",
            at("raise_deep")
        );
        assert_eq!(
            dump(&ty.map_labels(&f)),
            dump(&old_map_labels(&ty, &f)),
            "{}",
            at("map_labels")
        );
        assert_eq!(
            dump(&ty.map_labels(&|l: &Label| l.clone())),
            dump(&old_map_labels(&ty, &|l: &Label| l.clone())),
            "{}",
            at("map_labels by identity")
        );
        assert_eq!(
            dump(&ty.symbolic(&names)),
            dump(&old_symbolic(&ty, &names)),
            "{}",
            at("symbolic")
        );
        for positive in [true, false] {
            assert_eq!(
                dump(&ty.op_parameter(positive, &label, &kinds)),
                dump(&old_op_parameter(&ty, positive, &label, &kinds)),
                "{}",
                at("op_parameter")
            );
        }
        // No variable is solved: resolving rebuilds the type with its labels in normal form.
        assert_eq!(
            dump(&vars.resolve_shape(&ty)),
            dump(&old_resolve_shape(&ty)),
            "{}",
            at("resolve_shape")
        );
        let other = some_type(&mut random, 5, &mut pool);
        for join in [true, false] {
            for (a, b) in [(&ty, &ty), (&ty, &other), (&other, &ty)] {
                assert_eq!(
                    dump(&combine(a, b, join, &kinds)),
                    dump(&old_combine(a, b, join, &kinds)),
                    "case {case}, combine {a:?} with {b:?}"
                );
            }
        }
    }
}

#[test]
fn function_types_that_differ_only_in_refinements_are_never_taken_for_each_other() {
    // `==` ignores refinements (D-150), so only the same node may stand for a function type:
    // the join of two function nodes equal but for their refinements keeps the refinements
    // both share in its result and those of either in its parameters.
    let kinds = KindLabels::default();
    let refined = |result: &[&str], param: &[&str]| {
        let refinements = |names: &[&str]| -> Vec<Refinement> {
            names
                .iter()
                .map(|n| Refinement::Range((*n).to_owned(), span()))
                .collect()
        };
        Ty::Fn(Shared::new(FnTy {
            params: vec![Ty::Int],
            result: Ty::Int,
            uses: Vec::new(),
            fails: None,
            refinements: FnRefinements {
                params: vec![refinements(param)],
                result: refinements(result),
            },
        }))
    };
    let a = refined(&["in 0..5", "in 1..9"], &["in 0..3"]);
    let b = refined(&["in 0..5"], &["in 4..8"]);
    assert_eq!(a, b, "`==` ignores refinements");
    for join in [true, false] {
        for (x, y) in [(&a, &b), (&b, &a), (&a, &a)] {
            let new = combine(x, y, join, &kinds);
            assert_eq!(dump(&new), dump(&old_combine(x, y, join, &kinds)));
        }
    }
    let joined = combine(&a, &b, true, &kinds);
    assert_ne!(dump(&joined), dump(&a), "the join changes the refinements");
    assert!(
        combine(&a, &a, true, &kinds).same(&a),
        "a node joined with itself is itself"
    );
}

#[test]
fn an_operation_that_changes_nothing_returns_its_input() {
    // The same top node means nothing of the result was rebuilt. The node counter is
    // process-wide, and the tests of this binary run at once, so the count of nodes made is
    // checked by `tests/scaling.rs`, alone in its binary.
    let element = Shared::new(Ty::Kind(0, vec![Ty::Int, Ty::Text].into()));
    let ty = Ty::List(element).with_label(&Label::symbol("T"));
    let vars = Vars::default();
    for same in [
        vars.resolve_shape(&ty),
        ty.substitute(&[]),
        ty.map_labels(&|l: &Label| l.clone()),
        ty.raise_deep(&Label::public()),
        combine(&ty, &ty, true, &KindLabels::default()),
    ] {
        assert!(same.same(&ty), "{same:?}");
    }
}

#[test]
fn map_labels_calls_its_function_on_the_same_labels_in_the_same_order() {
    // `without_decisions` gathers what its function sees, so the order and the number of the
    // calls are part of what `map_labels` does: the fast paths must not change them.
    use std::cell::RefCell;
    let mut random = Random(0x0de5);
    for case in 0..2000 {
        let mut pool = Vec::new();
        let ty = some_type(&mut random, 5, &mut pool);
        let seen_new = RefCell::new(Vec::new());
        let seen_old = RefCell::new(Vec::new());
        let new = ty.map_labels(&|l: &Label| {
            seen_new.borrow_mut().push(l.clone());
            l.clone()
        });
        let old = old_map_labels(&ty, &|l: &Label| {
            seen_old.borrow_mut().push(l.clone());
            l.clone()
        });
        assert_eq!(dump(&new), dump(&old), "case {case}");
        assert_eq!(
            seen_new.into_inner(),
            seen_old.into_inner(),
            "case {case}: {ty:?}"
        );
    }
}

#[test]
fn a_function_node_with_two_uses_of_one_capability_is_rebuilt_when_met_with_itself() {
    // `union_uses` of such `uses` with themselves merges the second's operations into the
    // first: unreachable from a checked program, but the fast path must not keep the node.
    let uses = |ops: &[&str]| FnUse {
        name: "log".to_owned(),
        capability: None,
        ops: ops.iter().map(|o| (*o).to_owned()).collect(),
    };
    let f = Ty::Fn(Shared::new(FnTy {
        params: Vec::new(),
        result: Ty::Unit,
        uses: vec![uses(&["write"]), uses(&["read"])],
        fails: None,
        refinements: FnRefinements::default(),
    }));
    let kinds = KindLabels::default();
    let met = combine(&f, &f, true, &kinds);
    assert_eq!(dump(&met), dump(&old_combine(&f, &f, true, &kinds)));
    assert!(!met.same(&f));
}

// ----- stage 3a: flags, the erase memo and the resolver -----

/// The flags of `ty` by a walk of the whole type, as the operations see it.
fn walked_flags(ty: &Ty) -> Flags {
    match ty {
        Ty::Var(_) => Flags {
            var: true,
            ..Flags::LEAF
        },
        Ty::List(element) => walked_flags(element),
        Ty::Map(key, value) => walked_flags(key).and(walked_flags(value)),
        Ty::Kind(_, args) => args.iter().fold(Flags::LEAF, |f, a| f.and(walked_flags(a))),
        Ty::Fn(function) => {
            let flags = function
                .params
                .iter()
                .fold(walked_flags(&function.result), |f, p| {
                    f.and(walked_flags(p))
                });
            match &function.fails {
                Some(fails) => flags.and(walked_flags(fails)),
                None => flags,
            }
        }
        Ty::Labeled(label, inner) => Flags {
            var: false,
            label: true,
            var_label: label.symbols.iter().any(|s| s.starts_with('?')),
            normal: !label.is_public()
                && !matches!(**inner, Ty::Labeled(..) | Ty::Error | Ty::Never),
        }
        .and(walked_flags(inner)),
        _ => Flags::LEAF,
    }
}

/// Types built to defeat each flag: what an operation changes, deep below positions that
/// change nothing.
fn adversarial() -> Vec<Ty> {
    let deep = |ty: Ty| {
        (0..6).fold(ty, |ty, i| {
            if i % 2 == 0 {
                Ty::List(Shared::new(ty))
            } else {
                Ty::Kind(0, vec![ty].into())
            }
        })
    };
    let function = |fails: Ty| {
        Ty::Fn(Shared::new(FnTy {
            params: vec![Ty::Int],
            result: Ty::Int,
            uses: Vec::new(),
            fails: Some(fails),
            refinements: FnRefinements::default(),
        }))
    };
    vec![
        // A variable under a label.
        deep(Ty::Var(0).with_label(&Label::symbol("T"))),
        // A label not in normal form, deep down: `public`, and a label under a label.
        deep(Ty::Labeled(Label::public(), Shared::new(Ty::Int))),
        deep(Ty::Labeled(
            Label::symbol("T"),
            Shared::new(Ty::Int.with_label(&Label::symbol("U"))),
        )),
        // A label on `Error` and on `Never`.
        deep(Ty::Labeled(Label::symbol("T"), Shared::new(Ty::Error))),
        // A failure kind with a variable, and with a label (#216).
        deep(function(Ty::Var(1))),
        deep(function(Ty::Text.with_label(&Label::symbol("T")))),
        // A variable's label, deep down.
        deep(Ty::Int.with_label(&Label::symbol("?1"))),
    ]
}

#[test]
fn the_flags_of_a_node_are_those_of_everything_below_it() {
    let mut random = Random(0xf1a9);
    let mut types = adversarial();
    for _ in 0..3000 {
        let mut pool = Vec::new();
        types.push(some_type(&mut random, 5, &mut pool));
    }
    for ty in &types {
        assert_eq!(ty.flags(), walked_flags(ty), "{ty:?}");
    }
}

#[test]
fn erasing_through_the_memo_gives_what_erasing_gives() {
    let vars = Vars::default();
    let mut random = Random(0xe5a5e);
    let mut types = adversarial();
    for _ in 0..3000 {
        let mut pool = Vec::new();
        types.push(some_type(&mut random, 5, &mut pool));
    }
    // Twice, so the second pass reads what the first left in the memo.
    for _ in 0..2 {
        for ty in &types {
            assert_eq!(dump(&vars.erase(ty)), dump(&old_erase(ty)), "{ty:?}");
            assert_eq!(dump(&ty.erase()), dump(&old_erase(ty)), "{ty:?}");
        }
    }
}

#[test]
fn the_erase_memo_keeps_its_keys_so_a_freed_address_cannot_answer_for_another_node() {
    // ADR 0077 memo rule 1: a memo keyed by a node's address holds the node. Temporaries are
    // made, erased and dropped in a loop, so the allocator reuses their addresses; each new
    // node at a reused address must be erased as itself, never as the node that was there.
    let vars = Vars::default();
    for round in 0..2000_usize {
        let label = if round % 2 == 0 {
            Label::symbol("T")
        } else {
            Label::symbol("U")
        };
        let leaf = if round % 3 == 0 { Ty::Int } else { Ty::Text };
        let temporary = Ty::List(Shared::new(leaf.clone().with_label(&label)));
        assert_eq!(dump(&vars.erase(&temporary)), dump(&old_erase(&temporary)));
        drop(temporary);
        let other = Ty::List(Shared::new(Ty::List(Shared::new(leaf.with_label(&label)))));
        assert_eq!(
            dump(&vars.erase(&other)),
            dump(&old_erase(&other)),
            "round {round}"
        );
    }
}

/// `ty` with each solved variable of `values` replaced by its value, recursively.
fn substitute_vars(ty: &Ty, values: &[Option<Ty>]) -> Ty {
    match ty {
        Ty::Var(var) => match values.get(*var) {
            Some(Some(value)) => substitute_vars(value, values),
            _ => ty.clone(),
        },
        Ty::Labeled(label, inner) => {
            Ty::Labeled(label.clone(), Shared::new(substitute_vars(inner, values)))
        }
        Ty::List(element) => Ty::List(Shared::new(substitute_vars(element, values))),
        Ty::Map(key, value) => Ty::Map(
            Shared::new(substitute_vars(key, values)),
            Shared::new(substitute_vars(value, values)),
        ),
        Ty::Kind(id, args) => Ty::Kind(
            *id,
            args.iter().map(|a| substitute_vars(a, values)).collect(),
        ),
        Ty::Fn(function) => Ty::Fn(Shared::new(FnTy {
            params: function
                .params
                .iter()
                .map(|p| substitute_vars(p, values))
                .collect(),
            result: substitute_vars(&function.result, values),
            uses: function.uses.clone(),
            fails: function.fails.as_ref().map(|f| substitute_vars(f, values)),
            refinements: function.refinements.clone(),
        })),
        other => other.clone(),
    }
}

/// `uses` without a second entry for a capability: binding a variable joins function types
/// (`union_uses`), which merges such entries, so a variable's value holds none.
fn distinct(uses: &[FnUse]) -> Vec<FnUse> {
    let mut out: Vec<FnUse> = Vec::new();
    for u in uses {
        if !out
            .iter()
            .any(|o| o.name == u.name && o.capability == u.capability)
        {
            out.push(u.clone());
        }
    }
    out
}

/// `ty` with every variable whose index is not below `limit` made `Int`, and the `uses` of its
/// function types distinct.
fn below(ty: &Ty, limit: usize) -> Ty {
    match ty {
        Ty::Var(var) if *var >= limit => Ty::Int,
        Ty::Labeled(label, inner) => Ty::Labeled(label.clone(), Shared::new(below(inner, limit))),
        Ty::List(element) => Ty::List(Shared::new(below(element, limit))),
        Ty::Map(key, value) => Ty::Map(
            Shared::new(below(key, limit)),
            Shared::new(below(value, limit)),
        ),
        Ty::Kind(id, args) => Ty::Kind(*id, args.iter().map(|a| below(a, limit)).collect()),
        Ty::Fn(function) => Ty::Fn(Shared::new(FnTy {
            params: function.params.iter().map(|p| below(p, limit)).collect(),
            result: below(&function.result, limit),
            uses: distinct(&function.uses),
            fails: function.fails.as_ref().map(|f| below(f, limit)),
            refinements: function.refinements.clone(),
        })),
        other => other.clone(),
    }
}

#[test]
fn resolving_with_solved_variables_and_the_resolver_give_what_rebuilding_gave() {
    let mut random = Random(0x5e77);
    let span = span();
    for case in 0..1500 {
        let mut vars = Vars::default();
        let mut values = Vec::new();
        for index in 0..3 {
            let var = vars.fresh(Origin::Element, span);
            let mut pool = Vec::new();
            // A variable's value names only earlier variables: no cycle. It is solved as it is,
            // without the join `unify` does, so the reference knows what it holds.
            let value = below(&some_type(&mut random, 4, &mut pool), index);
            let Ty::Var(var) = var else {
                panic!("a fresh variable is a variable");
            };
            vars.solve(var, value.clone());
            values.push(Some(value));
        }
        let mut cases = adversarial();
        let mut pool = Vec::new();
        cases.push(below(&some_type(&mut random, 5, &mut pool), 3));
        for ty in &cases {
            let expected = old_resolve_shape(&substitute_vars(ty, &values));
            assert_eq!(
                dump(&vars.resolve_shape(ty)),
                dump(&expected),
                "case {case}: {ty:?}"
            );
            let settled_old = old_map_labels(&vars.resolve_shape(ty), &|l: &Label| {
                vars.settle_label(l, &mut Vec::new(), false)
            });
            assert_eq!(
                dump(&vars.resolve_labeled(ty)),
                dump(&settled_old),
                "case {case}: {ty:?}"
            );
            let resolver = vars.resolver();
            assert_eq!(
                dump(&resolver.resolve_labeled(ty)),
                dump(&vars.resolve_labeled(ty)),
                "case {case}: {ty:?}"
            );
            // Asked again, the resolver answers from its memo.
            assert_eq!(
                dump(&resolver.resolve_labeled(ty)),
                dump(&vars.resolve_labeled(ty)),
                "case {case}: {ty:?}"
            );
        }
    }
}
