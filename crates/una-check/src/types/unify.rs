//! Unification variables, for generic instantiations, empty lists and lambda parameters
//! (`spec/03 §7`).

use std::rc::Rc;

use una_syntax::Span;

use super::label::Label;
use super::ty::{FnTy, FnUse, KindLabels, Ty};

/// Why a unification variable exists, for the diagnostic when inference leaves it unknown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The element type of an empty list.
    Element,
    /// A type parameter of a generic function or kind, instantiated at a use.
    Instance { param: String, of: String },
}

#[derive(Clone, Debug)]
struct Slot {
    value: Option<Ty>,
    origin: Origin,
    span: Span,
}

/// The unification variables of one declaration.
#[derive(Clone, Debug, Default)]
pub(crate) struct Vars {
    slots: Vec<Slot>,
    /// The labels of the kinds' fields, for the deep labels of the variables' values.
    kinds: Rc<KindLabels>,
}

impl Vars {
    /// No variables yet, over the kinds described by `kinds`.
    pub(crate) fn new(kinds: Rc<KindLabels>) -> Vars {
        Vars {
            slots: Vec::new(),
            kinds,
        }
    }

    /// A new variable.
    pub(crate) fn fresh(&mut self, origin: Origin, span: Span) -> Ty {
        self.slots.push(Slot {
            value: None,
            origin,
            span,
        });
        Ty::Var(self.slots.len() - 1)
    }

    /// `ty` with its outermost variables replaced by their values.
    fn shallow(&self, ty: &Ty) -> Ty {
        let mut current = ty.clone();
        while let Ty::Var(var) = current {
            match self.slots.get(var).and_then(|slot| slot.value.clone()) {
                Some(value) => current = value,
                None => break,
            }
        }
        current
    }

    /// `ty` with every solved variable replaced by its value, and without labels: what type
    /// checking compares.
    pub(crate) fn resolve(&self, ty: &Ty) -> Ty {
        self.resolve_shape(ty).erase()
    }

    /// `ty` with every solved variable replaced by its value, labels included,
    /// and the label of each variable that a label names (see [`Vars::label_of`]) replaced by
    /// the join of every label of the variable's value, as it is now.
    pub(crate) fn resolve_labeled(&self, ty: &Ty) -> Ty {
        self.settle(&self.resolve_shape(ty), &[], false)
    }

    /// [`Vars::resolve_labeled`] for a local bound by `let`: the labels of solved variables are
    /// settled as they are now, and the label of a variable still unsolved is kept, so that
    /// the call that later solves it, like the call of a generic function bound with `let`,
    /// counts its labels (ADR 0038 D3.2, DECISION D-137, re-review N-003).
    /// D-162: a variable left unsolved by the `let` keeps its label, and the local
    /// is not generalized: every call of it solves the same variable, whose labels join.
    pub(crate) fn resolve_bound(&self, ty: &Ty) -> Ty {
        self.settle(&self.resolve_shape(ty), &[], true)
    }

    /// The symbolic label that stands for `ℓ*(σ)` of the variables among `tys`, the join of every
    /// label of their values, resolved lazily by [`Vars::resolve_labeled`] so that labels the
    /// variables take later count (ADR 0038 D3, generic instantiation).
    pub(crate) fn label_of(tys: &[Ty]) -> Label {
        let mut label = Label::public();
        for ty in tys {
            if let Ty::Var(var) = ty {
                label.symbols.insert(format!("?{var}"));
            }
        }
        label
    }

    /// `ty` with every solved variable replaced by its value, labels included, and the labels
    /// of variables left symbolic.
    fn resolve_shape(&self, ty: &Ty) -> Ty {
        match self.shallow(ty) {
            Ty::Labeled(label, inner) => self.resolve_shape(&inner).with_label(&label),
            Ty::List(element) => Ty::List(Box::new(self.resolve_shape(&element))),
            Ty::Kind(id, args) => {
                Ty::Kind(id, args.iter().map(|arg| self.resolve_shape(arg)).collect())
            }
            Ty::Fn(function) => Ty::Fn(Box::new(FnTy {
                params: function
                    .params
                    .iter()
                    .map(|p| self.resolve_shape(p))
                    .collect(),
                result: self.resolve_shape(&function.result),
                uses: function.uses.clone(),
                fails: function.fails.as_ref().map(|f| self.resolve(f)),
                refinements: function.refinements.clone(),
            })),
            other => other,
        }
    }

    /// `ty`, already resolved, with the labels of variables replaced; `visiting` holds the
    /// variables being replaced, whose labels are not counted twice.
    /// `keep` keeps the labels of unsolved variables (see [`Vars::resolve_bound`]).
    fn settle(&self, ty: &Ty, visiting: &[usize], keep: bool) -> Ty {
        ty.map_labels(&|label| self.settle_label(label, &mut visiting.to_vec(), keep))
    }

    fn settle_label(&self, label: &Label, visiting: &mut Vec<usize>, keep: bool) -> Label {
        if !label.symbols.iter().any(|s| s.starts_with('?')) {
            return label.clone();
        }
        let mut settled = label.clone();
        settled.symbols.retain(|s| !s.starts_with('?'));
        for symbol in &label.symbols {
            let Some(var) = symbol
                .strip_prefix('?')
                .and_then(|n| n.parse::<usize>().ok())
            else {
                continue;
            };
            if visiting.contains(&var) {
                continue;
            }
            let value = self.resolve_shape(&Ty::Var(var));
            if keep && let Ty::Var(unsolved) = value {
                settled.symbols.insert(format!("?{unsolved}"));
                continue;
            }
            visiting.push(var);
            let value = self.settle(&value, visiting, keep);
            settled = settled.join(&value.deep_label(&self.kinds));
            visiting.pop();
        }
        settled
    }

    /// `ty` with the variables and labels at its top removed: its shape at the top.
    fn bare(&self, ty: &Ty) -> Ty {
        let mut current = self.shallow(ty);
        while let Ty::Labeled(_, inner) = current {
            current = self.shallow(&inner);
        }
        current
    }

    /// `ty` with the variables at its top replaced by their values, labels kept; its
    /// components stay as they are, so later unifications still reach their variables.
    pub(crate) fn head(&self, ty: &Ty) -> Ty {
        match self.shallow(ty) {
            Ty::Labeled(label, inner) => self.head(&inner).with_label(&label),
            other => other,
        }
    }

    /// The labels of `a` and `b` joined position by position; they have one shape, as after
    /// unification. Function parameters meet, since they are contravariant.
    pub(crate) fn join(&self, a: &Ty, b: &Ty) -> Ty {
        combine(
            &self.resolve_shape(a),
            &self.resolve_shape(b),
            true,
            &self.kinds,
        )
    }

    fn occurs(&self, var: usize, ty: &Ty) -> bool {
        match self.shallow(ty) {
            Ty::Labeled(_, inner) => self.occurs(var, &inner),
            Ty::Var(other) => other == var,
            Ty::List(element) => self.occurs(var, &element),
            Ty::Kind(_, args) => args.iter().any(|arg| self.occurs(var, arg)),
            Ty::Fn(function) => {
                function.params.iter().any(|p| self.occurs(var, p))
                    || self.occurs(var, &function.result)
            }
            _ => false,
        }
    }

    fn bind(&mut self, var: usize, ty: Ty) {
        if let Some(slot) = self.slots.get_mut(var) {
            slot.value = Some(ty);
        }
    }

    /// The solved variable that `ty` is, through labels and links between variables.
    fn solved_head(&self, ty: &Ty) -> Option<usize> {
        let mut current = ty.clone();
        loop {
            match current {
                Ty::Labeled(_, inner) => current = *inner,
                Ty::Var(var) => match self.slots.get(var).and_then(|slot| slot.value.clone()) {
                    Some(Ty::Var(next)) => current = Ty::Var(next),
                    Some(Ty::Labeled(label, inner)) if matches!(*inner, Ty::Var(_)) => {
                        current = Ty::Labeled(label, inner);
                    }
                    Some(_) => return Some(var),
                    None => return None,
                },
                _ => return None,
            }
        }
    }

    /// Makes `a` and `b` equal, solving variables; false when they cannot be. The error type
    /// and `Never` agree with every type (ADR 0024 rule 2); a variable meeting `Never` stays
    /// unsolved. Labels do not take part in equality: a variable takes the labels of the type
    /// it is solved with, and a solved variable keeps the join of the labels of every type it
    /// meets.
    pub(crate) fn unify(&mut self, a: &Ty, b: &Ty) -> bool {
        self.unify_in(a, b, false)
    }

    /// [`Vars::unify`]; under an invariant parameter of a kind (`fixed`), a solved variable
    /// keeps its labels, since values of the kind at one label do not flow into it at another
    /// (ADR 0046 rule 2): the flow that follows compares them.
    /// D-161: ADR 0046 fixes the flow, not unification; a join here would raise
    /// the argument after the flows that fixed it were checked.
    fn unify_in(&mut self, a: &Ty, b: &Ty, fixed: bool) -> bool {
        if !self.unify_shapes(a, b, fixed) {
            return false;
        }
        if fixed {
            return true;
        }
        for (target, other) in [(a, b), (b, a)] {
            if let Some(var) = self.solved_head(target)
                && let Some(value) = self.slots.get(var).and_then(|slot| slot.value.clone())
            {
                let joined = self.join(&value, other);
                self.bind(var, joined);
            }
        }
        true
    }

    fn unify_shapes(&mut self, a: &Ty, b: &Ty, fixed: bool) -> bool {
        let a = self.shallow(a);
        let b = self.shallow(b);
        // Top labels do not take part in equality; an unsolved variable takes the other side
        // with its labels.
        let (a_bare, b_bare) = (self.bare(&a), self.bare(&b));
        match (&a_bare, &b_bare) {
            (Ty::Var(x), Ty::Var(y)) if x == y => return true,
            (Ty::Var(_), Ty::Never) | (Ty::Never, Ty::Var(_)) => return true,
            (Ty::Var(x), Ty::Var(y)) => {
                // The later variable points to the earlier one, whose origin is reported.
                let (early, late) = if x < y { (*x, *y) } else { (*y, *x) };
                self.bind(late, Ty::Var(early));
                return true;
            }
            (Ty::Var(var), _) => {
                if self.occurs(*var, &b) {
                    return false;
                }
                self.bind(*var, b.clone());
                return true;
            }
            (_, Ty::Var(var)) => {
                if self.occurs(*var, &a) {
                    return false;
                }
                self.bind(*var, a.clone());
                return true;
            }
            _ => {}
        }
        let (a, b) = (a_bare, b_bare);
        match (&a, &b) {
            (Ty::Never, _) | (_, Ty::Never) => true,
            (Ty::Error, _) | (_, Ty::Error) => true,
            (Ty::Fixed(x), Ty::Fixed(y)) => x == y,
            (Ty::Int, Ty::Int)
            | (Ty::Bool, Ty::Bool)
            | (Ty::Text, Ty::Text)
            | (Ty::Unit, Ty::Unit)
            | (Ty::Hashed, Ty::Hashed) => true,
            (Ty::List(x), Ty::List(y)) => self.unify_in(x, y, fixed),
            (Ty::Kind(i, xs), Ty::Kind(j, ys)) => {
                let kinds = self.kinds.clone();
                i == j
                    && xs.len() == ys.len()
                    && xs
                        .iter()
                        .zip(ys)
                        .enumerate()
                        .fold(true, |ok, (index, (x, y))| {
                            let fixed = fixed || kinds.is_invariant(*i, index);
                            self.unify_in(x, y, fixed) && ok
                        })
            }
            (Ty::Param(i), Ty::Param(j)) => i == j,
            (Ty::Fn(f), Ty::Fn(g)) => {
                f.params.len() == g.params.len()
                    && f.params
                        .iter()
                        .zip(&g.params)
                        .fold(true, |ok, (x, y)| self.unify_in(x, y, fixed) && ok)
                    && self.unify_in(&f.result, &g.result, fixed)
            }
            _ => false,
        }
    }

    /// The variables still unsolved, each with its origin and span, in creation order.
    pub(crate) fn unsolved(&self) -> Vec<(Origin, Span)> {
        self.slots
            .iter()
            .filter(|slot| slot.value.is_none())
            .map(|slot| (slot.origin.clone(), slot.span))
            .collect()
    }
}

/// `a` and `b`, of one shape, with their labels joined (`join`) or met (not `join`) position
/// by position; function parameters take the other operation. An argument of an invariant
/// parameter of a kind keeps the labels of `a` (ADR 0046 rule 2): a value of `b` flows into
/// it only at equal labels, which the flow that follows checks (D-160).
fn combine(a: &Ty, b: &Ty, join: bool, kinds: &KindLabels) -> Ty {
    let (la, lb) = (a.top_label(), b.top_label());
    let label = if join { la.join(&lb) } else { la.meet(&lb) };
    let base = match (a.unlabeled(), b.unlabeled()) {
        (Ty::Never, other) | (other, Ty::Never) => return other.clone().with_label(&label),
        (Ty::Error, _) | (_, Ty::Error) => return Ty::Error,
        (Ty::List(x), Ty::List(y)) => Ty::List(Box::new(combine(x, y, join, kinds))),
        (Ty::Kind(i, xs), Ty::Kind(j, ys)) if i == j && xs.len() == ys.len() => Ty::Kind(
            *i,
            xs.iter()
                .zip(ys)
                .enumerate()
                .map(|(index, (x, y))| {
                    if kinds.is_invariant(*i, index) {
                        x.clone()
                    } else {
                        combine(x, y, join, kinds)
                    }
                })
                .collect(),
        ),
        (Ty::Fn(f), Ty::Fn(g)) if f.params.len() == g.params.len() => Ty::Fn(Box::new(FnTy {
            params: f
                .params
                .iter()
                .zip(&g.params)
                .map(|(x, y)| combine(x, y, !join, kinds))
                .collect(),
            result: combine(&f.result, &g.result, join, kinds),
            // ADR 0038 D1: a joined function type keeps the `uses` and `fails` of both.
            uses: union_uses(&f.uses, &g.uses),
            fails: f.fails.clone().or_else(|| g.fails.clone()),
            // ADR 0037 rule 1: the refinements of a join, position by position.
            refinements: f.refinements.combine(&g.refinements, join),
        })),
        (Ty::Var(_), other) if !matches!(other, Ty::Var(_)) => other.clone(),
        (same, _) => same.clone(),
    };
    base.with_label(&label)
}

/// The capabilities of `a` and those of `b`: a reference both name keeps the operations of
/// both, in the order of `a`, then `b`.
fn union_uses(a: &[FnUse], b: &[FnUse]) -> Vec<FnUse> {
    let mut uses = a.to_vec();
    for need in b {
        match uses
            .iter_mut()
            .find(|u| u.name == need.name && u.capability == need.capability)
        {
            Some(known) => {
                for op in &need.ops {
                    if !known.ops.contains(op) {
                        known.ops.push(op.clone());
                    }
                }
            }
            None => uses.push(need.clone()),
        }
    }
    uses
}

#[cfg(test)]
mod tests {
    use una_syntax::{FileId, SourceDb};

    use super::*;

    fn span() -> Span {
        let mut db = SourceDb::new();
        let file: FileId = db.add("a.una", b"x");
        Span::at(file, 0)
    }

    #[test]
    fn variables_are_solved_and_resolved() {
        let mut vars = Vars::default();
        let a = vars.fresh(Origin::Element, span());
        assert!(vars.unify(&Ty::List(Box::new(a.clone())), &Ty::List(Box::new(Ty::Int))));
        assert_eq!(vars.resolve(&a), Ty::Int);
        assert!(!vars.unify(&a, &Ty::Text));
        assert!(vars.unsolved().is_empty());
    }

    #[test]
    fn never_and_error_agree_with_everything() {
        let mut vars = Vars::default();
        let a = vars.fresh(Origin::Element, span());
        assert!(vars.unify(&a, &Ty::Never));
        assert_eq!(vars.unsolved().len(), 1);
        assert!(vars.unify(&Ty::Error, &Ty::Text));
        assert!(vars.unify(&a, &Ty::Error));
        assert!(vars.unsolved().is_empty());
    }

    #[test]
    fn variables_keep_the_join_of_the_labels_they_meet() {
        use una_core::{Label, Level};
        let secret = Label {
            level: Level::Secret,
            categories: std::collections::BTreeSet::new(),
            symbols: std::collections::BTreeSet::new(),
        };
        let mut vars = Vars::default();
        let t = vars.fresh(Origin::Element, span());
        assert!(vars.unify(&Ty::List(Box::new(t.clone())), &Ty::List(Box::new(Ty::Int))));
        assert!(vars.unify(&t, &Ty::Int.with_label(&secret)));
        assert_eq!(vars.resolve_labeled(&t), Ty::Int.with_label(&secret));
        assert_eq!(vars.resolve(&t), Ty::Int);
        // Labels never make types unequal.
        assert!(vars.unify(&Ty::Text.with_label(&secret), &Ty::Text));
        assert!(!vars.unify(&Ty::Text.with_label(&secret), &Ty::Int));
    }

    #[test]
    fn the_occurs_check_rejects_infinite_types() {
        let mut vars = Vars::default();
        let a = vars.fresh(Origin::Element, span());
        assert!(!vars.unify(&a, &Ty::List(Box::new(a.clone()))));
    }

    #[test]
    fn two_variables_keep_the_earlier_origin() {
        let mut vars = Vars::default();
        let a = vars.fresh(Origin::Element, span());
        let b = vars.fresh(
            Origin::Instance {
                param: "T".to_owned(),
                of: "first".to_owned(),
            },
            span(),
        );
        assert!(vars.unify(&b, &a));
        assert_eq!(vars.unsolved(), vec![(Origin::Element, span())]);
    }
}
