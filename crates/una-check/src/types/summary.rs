//! Result summaries of user generic functions (ADR 0067, `spec/07 §7` rule 8): which type
//! parameters' symbolic labels a function's body can put at each position of its declared
//! result, so that a call raises each position only by the labels of those type arguments.

use std::collections::BTreeSet;

use super::authority::Pass;
use super::check::Checker;
use super::env::{FnKind, Step, Summary};
use super::label::Label;
use super::ty::{FnTy, Shared, Ty};
use super::unify::Vars;
use una_project::Edition;

impl Checker<'_, '_, '_> {
    /// The generic function whose result summary the body being checked contributes to: in
    /// the rounds of their least fixpoint, in edition 0.2, a function declared outside the
    /// prelude with type parameters, whose return is `name`.
    fn summarized(&self, name: &str) -> Option<usize> {
        if self.pass != Pass::Collect || self.edition() < Edition::V0_2 {
            return None;
        }
        let id = self.current?;
        let def = self.env.functions.get(id)?;
        (def.module < self.env.prelude
            && def.kind == FnKind::Function
            && !def.type_params.is_empty()
            && def.name == name)
            .then_some(id)
    }

    /// Records the symbolic labels of a value returned by the body of `name`, of type `found`
    /// with its labels, at each position of the declared result.
    pub(crate) fn summarize_return(&mut self, name: &str, found: &Ty) {
        let Some(id) = self.summarized(name) else {
            return;
        };
        let Some(def) = self.env.functions.get(id) else {
            return;
        };
        let (declared, params) = (def.result.base.clone(), def.type_params.clone());
        let found = self.labeled(found);
        let mut summary = Summary::new();
        collect(&found, &declared, &params, &mut Vec::new(), &mut summary);
        self.grow(id, summary);
    }

    /// Records that a condition labeled `label` decides which value the body of `name`
    /// returns: its symbolic labels reach the top of the result (`spec/07 §3`).
    pub(crate) fn summarize_condition(&mut self, name: &str, label: &Label) {
        let Some(id) = self.summarized(name) else {
            return;
        };
        let Some(def) = self.env.functions.get(id) else {
            return;
        };
        let found = own(label, &def.type_params, None);
        if found.is_empty() {
            return;
        }
        let mut summary = Summary::new();
        summary.insert(Vec::new(), found);
        self.grow(id, summary);
    }

    /// Records that the body calls its `of` parameter `index`, or passes it on, under the
    /// decisions `decided` on its own type parameters (`ℓ*_T`), which its callers' arguments
    /// then run under (ADR 0067 amendment, #234).
    pub(crate) fn raise_of(&mut self, index: usize, decided: &Label) {
        if self.pass != Pass::Collect || self.edition() < Edition::V0_2 {
            return;
        }
        let Some(id) = self.current else {
            return;
        };
        let Some(def) = self.env.functions.get(id) else {
            return;
        };
        let indices: BTreeSet<usize> = decided
            .symbols
            .iter()
            .filter_map(|s| s.strip_suffix('*'))
            .filter_map(|name| def.type_params.iter().position(|p| *p == name))
            .collect();
        if indices.is_empty() {
            return;
        }
        let known = self.env.of_raises.entry((id, index)).or_default();
        for param in indices {
            if known.insert(param) {
                self.env.summaries_grew = true;
            }
        }
    }

    /// Joins `summary` into the summary of function `id`, noting whether it grew.
    fn grow(&mut self, id: usize, summary: Summary) {
        let known = self.env.summaries.entry(id).or_default();
        for (path, params) in summary {
            let at = known.entry(path).or_default();
            for param in params {
                if at.insert(param) {
                    self.env.summaries_grew = true;
                }
            }
        }
    }
}

/// The indices, among `params`, of the type parameters whose symbolic labels `label` holds,
/// `ℓ_T` or `ℓ*_T`, leaving out `ℓ_T` of the parameter `inherent`: the label of a value of
/// that parameter at a position of that parameter, which substitution gives.
fn own(label: &Label, params: &[&str], inherent: Option<usize>) -> BTreeSet<usize> {
    label
        .symbols
        .iter()
        .filter_map(|symbol| {
            let name = symbol.trim_end_matches('*');
            let index = params.iter().position(|param| *param == name)?;
            let identity = Some(index) == inherent && !symbol.ends_with('*');
            (!identity).then_some(index)
        })
        .collect()
}

/// Walks the value's type `found` and the declared result type together, recording at each
/// position the type parameters among `params` whose symbolic labels `found` has there.
/// At a position that is a type parameter `T`, the label `ℓ_T` of a value of `T` passed
/// through is the type argument's own, which substitution puts there, so it is left out;
/// `ℓ*_T`, from a decision on `T` values, stands for every label of the type argument and is
/// recorded (ADR 0067 amendment). The walk stops there, since the type argument's inner
/// positions come from substitution.
fn collect(found: &Ty, declared: &Ty, params: &[&str], path: &mut Vec<Step>, out: &mut Summary) {
    let inherent = match declared.unlabeled() {
        Ty::Param(index) => Some(*index),
        _ => None,
    };
    // INVARIANT: a plain symbolic label `ℓ_T` labels only a position of type `T`; everywhere
    // else a symbol of the function's type parameters is starred, `ℓ*_T`. `own` drops `ℓ_T` at
    // its own position, which would lose a decision if a rule ever put `ℓ_T` elsewhere (the
    // formal model's `LTy.InPlace`, #440).
    debug_assert!(
        found.top_label().symbols.iter().all(|symbol| {
            symbol.starts_with('?')
                || symbol.ends_with('*')
                || !params.contains(&symbol.as_ref())
                || inherent.is_some_and(|index| params.get(index) == Some(&symbol.as_ref()))
        }),
        "a plain symbolic label off its own type parameter (#440)"
    );
    let at = own(&found.top_label(), params, inherent);
    let param = inherent.is_some();
    if !at.is_empty() {
        out.entry(path.clone()).or_default().extend(at);
    }
    if param {
        return;
    }
    let mut step = |step: Step, found: &Ty, declared: &Ty, out: &mut Summary| {
        path.push(step);
        collect(found, declared, params, path, out);
        path.pop();
    };
    match (found.unlabeled(), declared.unlabeled()) {
        (Ty::List(found), Ty::List(declared)) | (Ty::Dense(_, found), Ty::Dense(_, declared)) => {
            step(Step::Element, found, declared, out);
        }
        (Ty::Map(found_key, found_value), Ty::Map(declared_key, declared_value)) => {
            step(Step::Arg(0), found_key, declared_key, out);
            step(Step::Arg(1), found_value, declared_value, out);
        }
        (Ty::Kind(_, found), Ty::Kind(_, declared)) => {
            for (index, (found, declared)) in found.iter().zip(declared).enumerate() {
                step(Step::Arg(index), found, declared, out);
            }
        }
        (Ty::Coll(_, found), Ty::Coll(_, declared)) => {
            for (index, (found, declared)) in found.iter().zip(declared.iter()).enumerate() {
                step(Step::Arg(index), found, declared, out);
            }
        }
        (Ty::Fn(found), Ty::Fn(declared)) => {
            step(Step::Result, &found.result, &declared.result, out);
        }
        _ => {}
    }
}

/// The result `actual` of a call of a generic function whose declared result is `declared`,
/// instantiated with `subst`: each position raised by the labels of the type arguments that
/// its summary names, as they will be (ADR 0067, `spec/10 §5`).
pub(crate) fn raise(declared: &Ty, actual: Ty, summary: &Summary, subst: &[Ty]) -> Ty {
    raise_at(declared, actual, summary, subst, &mut Vec::new())
}

fn raise_at(
    declared: &Ty,
    actual: Ty,
    summary: &Summary,
    subst: &[Ty],
    path: &mut Vec<Step>,
) -> Ty {
    let args: Vec<Ty> = summary
        .get(path.as_slice())
        .map(|params| {
            params
                .iter()
                .filter_map(|index| subst.get(*index).cloned())
                .collect()
        })
        .unwrap_or_default();
    let label = Vars::label_of(&args);
    let top = actual.top_label().join(&label);
    let mut inner = |step: Step, declared: &Ty, actual: Ty| {
        path.push(step);
        let raised = raise_at(declared, actual, summary, subst, path);
        path.pop();
        raised
    };
    let raised = match (declared.unlabeled(), actual.unlabeled().clone()) {
        (Ty::Param(_), other) => other,
        (Ty::List(declared), Ty::List(actual)) => Ty::List(Shared::new(inner(
            Step::Element,
            declared,
            (*actual).clone(),
        ))),
        (Ty::Dense(_, declared), Ty::Dense(dense, actual)) => Ty::Dense(
            dense,
            Shared::new(inner(Step::Element, declared, (*actual).clone())),
        ),
        (Ty::Map(declared_key, declared_value), Ty::Map(actual_key, actual_value)) => Ty::Map(
            Shared::new(inner(Step::Arg(0), declared_key, (*actual_key).clone())),
            Shared::new(inner(Step::Arg(1), declared_value, (*actual_value).clone())),
        ),
        (Ty::Kind(_, declared), Ty::Kind(id, actual)) => Ty::Kind(
            id,
            actual
                .iter()
                .enumerate()
                .map(|(index, actual)| match declared.get(index) {
                    Some(declared) => inner(Step::Arg(index), declared, actual.clone()),
                    None => actual.clone(),
                })
                .collect(),
        ),
        (Ty::Coll(_, declared), Ty::Coll(coll, actual)) => Ty::Coll(
            coll,
            actual
                .iter()
                .enumerate()
                .map(|(index, actual)| match declared.get(index) {
                    Some(declared) => inner(Step::Arg(index), declared, actual.clone()),
                    None => actual.clone(),
                })
                .collect(),
        ),
        (Ty::Fn(declared), Ty::Fn(actual)) => {
            let actual = (*actual).clone();
            Ty::Fn(Shared::new(FnTy {
                result: inner(Step::Result, &declared.result, actual.result.clone()),
                ..actual
            }))
        }
        (_, other) => other,
    };
    raised.with_label(&top)
}

/// How much the summary and the `of` raises of `function` hold: they only grow, so a change
/// of size is a change.
pub(crate) fn size(env: &super::env::Env<'_>, function: usize) -> usize {
    let summary: usize = env
        .summaries
        .get(&function)
        .map_or(0, |summary| summary.values().map(BTreeSet::len).sum());
    let raises: usize = env
        .of_raises
        .range((function, 0)..=(function, usize::MAX))
        .map(|(_, params)| params.len())
        .sum();
    summary + raises
}

/// The strongly connected components of the call graph among the functions of `nodes`, from
/// `edges` (caller, callee), callees first: a component comes after every component it calls
/// (Kosaraju, with explicit stacks, so long chains do not recurse).
pub(crate) fn components(nodes: &BTreeSet<usize>, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    use std::collections::BTreeMap;
    let mut forward: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut backward: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &(caller, callee) in edges {
        if nodes.contains(&caller) && nodes.contains(&callee) {
            forward.entry(caller).or_default().push(callee);
            backward.entry(callee).or_default().push(caller);
        }
    }
    // First pass: the order in which a depth-first search over calls finishes each function.
    let mut finished = Vec::new();
    let mut seen = BTreeSet::new();
    for &start in nodes {
        if !seen.insert(start) {
            continue;
        }
        let mut stack = vec![(start, 0usize)];
        while let Some((node, next)) = stack.pop() {
            let callees = forward.get(&node).map_or(&[][..], Vec::as_slice);
            if let Some(&callee) = callees.get(next) {
                stack.push((node, next + 1));
                if seen.insert(callee) {
                    stack.push((callee, 0));
                }
            } else {
                finished.push(node);
            }
        }
    }
    // Second pass, over callers, in reverse finishing order: each search finds a component,
    // and the components come out callers first, so they are reversed at the end.
    let order: BTreeMap<usize, usize> = finished
        .iter()
        .enumerate()
        .map(|(position, node)| (*node, position))
        .collect();
    let mut assigned = BTreeSet::new();
    let mut out = Vec::new();
    for &start in finished.iter().rev() {
        if !assigned.insert(start) {
            continue;
        }
        let mut component = vec![start];
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            for &caller in backward.get(&node).map_or(&[][..], Vec::as_slice) {
                if assigned.insert(caller) {
                    component.push(caller);
                    stack.push(caller);
                }
            }
        }
        // Within a component, in the order the search over calls finished them: callees
        // first, as far as a cycle allows.
        component.sort_by_key(|node| order.get(node).copied().unwrap_or(usize::MAX));
        out.push(component);
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_come_callees_first() {
        // 0 -> 1 -> 2 <-> 3, and 4 alone.
        let nodes: BTreeSet<usize> = (0..5).collect();
        let edges = [(0, 1), (1, 2), (2, 3), (3, 2)];
        let found = components(&nodes, &edges);
        let position = |n: usize| found.iter().position(|c| c.contains(&n)).unwrap_or(99);
        assert!(
            found
                .iter()
                .any(|c| c.len() == 2 && c.contains(&2) && c.contains(&3))
        );
        assert!(position(2) < position(1) && position(1) < position(0));
        assert_eq!(found.iter().map(Vec::len).sum::<usize>(), 5);
        // A chain of 10,000 functions does not overflow the stack.
        let nodes: BTreeSet<usize> = (0..10_000).collect();
        let edges: Vec<(usize, usize)> = (0..9_999).map(|n| (n, n + 1)).collect();
        let found = components(&nodes, &edges);
        assert_eq!(found.len(), 10_000);
        assert_eq!(found.first(), Some(&vec![9_999]));
    }
}
