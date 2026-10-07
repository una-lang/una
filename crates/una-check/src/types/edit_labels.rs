//! Label variables of editable bindings (ADR 0075, `spec/17 §2` rule 4, `§6` rule 1).
//!
//! A `let edit` without an annotation gives each position of its initial value's type that is
//! still an unsolved type variable a label variable as its top label. Pass 1 names the
//! variable with a symbol of its own, records every label that flows into its position, and
//! solves the variables of the declaration when its check ends: the least label above every
//! flow, a variable that flows into another counting its solution. Later checks of the
//! declaration, pass 2 among them, read the solution. Solving is linear in the recorded flows:
//! the strongly connected components of the variables that flow into each other, in reverse
//! topological order.

use std::collections::BTreeMap;

use una_syntax::Span;

use super::authority::Pass;
use super::check::Checker;
use super::label::Label;
use super::ty::Ty;

/// The first character of the symbols that name label variables of editable bindings. No type
/// parameter or unification variable has a name that starts with it.
const MARK: char = '^';

/// The symbol of the label variable at the `index`-th open position of the binding at `span`.
fn symbol(span: Span, index: usize) -> String {
    format!("{MARK}{:?}:{}:{index}", span.file, span.start)
}

/// Whether `symbol` names a label variable of an editable binding.
fn is_variable(symbol: &str) -> bool {
    symbol.starts_with(MARK)
}

impl Checker<'_, '_, '_> {
    /// `bound`, the type of an un-annotated `let edit` at `span`, with a label variable at the
    /// top of each position that is an unsolved type variable: in pass 1 its symbol joined with
    /// the solution of earlier checks, in pass 2 the solution alone (ADR 0075 rule 1).
    pub(crate) fn open_label_positions(&mut self, bound: Ty, span: Span) -> Ty {
        let mut index = 0;
        self.open(bound, span, &mut index)
    }

    fn open(&mut self, ty: Ty, span: Span, index: &mut usize) -> Ty {
        match ty {
            Ty::Var(var) => {
                let name = symbol(span, *index);
                *index += 1;
                self.env.edit_label_spans.insert(name.clone(), span);
                self.env.edit_labels_seen = true;
                if self.pass == Pass::Collect {
                    self.edit_open.push((name.clone(), var));
                }
                // Once an earlier check learned what the variable resolves to, every label
                // position of that type has a variable of its own (ADR 0075 rule 1).
                match self.env.edit_shapes.get(&name).cloned() {
                    // ADR 0075 rule 1: a position whose labels cannot all be variables is
                    // rejected, never decided by the order unification sees its uses in (N-45).
                    Some(Ty::Error) => {
                        if self.pass == Pass::Check {
                            let shown = self.report.text(span).to_owned();
                            self.report.error(
                                una_diag::Code::U4001,
                                span,
                                format!(
                                    "the labels of `{shown}` cannot be inferred, since part of its type holds a function; annotate its type, as in `let edit {shown}: List[...] = ...`"
                                ),
                            );
                        }
                        Ty::Var(var).with_label(&self.variable_label(&name))
                    }
                    Some(shape) => {
                        let labeled = self.shaped(&shape, &name);
                        // The variable is still solved by unification, now with this type.
                        if !self.vars.unify(&Ty::Var(var), &labeled) {
                            return Ty::Var(var).with_label(&self.variable_label(&name));
                        }
                        labeled
                    }
                    None => Ty::Var(var).with_label(&self.variable_label(&name)),
                }
            }
            Ty::Labeled(label, inner) => {
                // An unsolved variable under a label keeps that label too.
                let inner = self.open(*inner, span, index);
                inner.with_label(&label)
            }
            Ty::List(element) => Ty::List(Box::new(self.open(*element, span, index))),
            Ty::Map(key, value) => {
                let key = self.open(*key, span, index);
                Ty::Map(Box::new(key), Box::new(self.open(*value, span, index)))
            }
            Ty::Kind(kind, args) => Ty::Kind(
                kind,
                args.into_iter()
                    .map(|arg| self.open(arg, span, index))
                    .collect(),
            ),
            // A function's positions are fixed by its type, and the others hold no variable.
            other => other,
        }
    }

    /// The label of the variable `name`: in pass 1 its symbol joined with the solution of
    /// earlier checks, in pass 2 the solution alone.
    fn variable_label(&mut self, name: &str) -> Label {
        let solved = self
            .env
            .edit_labels
            .get(name)
            .cloned()
            .unwrap_or_else(Label::public);
        if self.pass == Pass::Collect {
            self.edit_bounds
                .entry(name.to_owned())
                .or_insert_with(Label::public);
            Label::symbol(name).join(&solved)
        } else {
            solved
        }
    }

    /// `shape` with the variable `name` as its top label and a variable derived from `name` at
    /// each label position inside it: linear in the size of the shape.
    fn shaped(&mut self, shape: &Ty, name: &str) -> Ty {
        let label = self.variable_label(name);
        let inner = match shape {
            Ty::List(element) => Ty::List(Box::new(self.shaped(element, &derived(name, "e")))),
            Ty::Map(key, value) => Ty::Map(
                Box::new(self.shaped(key, &derived(name, "k"))),
                Box::new(self.shaped(value, &derived(name, "v"))),
            ),
            Ty::Kind(kind, args) => Ty::Kind(
                *kind,
                args.iter()
                    .enumerate()
                    .map(|(i, arg)| self.shaped(arg, &derived(name, &i.to_string())))
                    .collect(),
            ),
            other => other.clone(),
        };
        inner.with_label(&label)
    }

    /// At the end of a pass 1 check, the type each open variable resolved to, without labels,
    /// when it holds no unification variable and no function type: the shape that later checks
    /// give label variables at every position. Returns whether a shape is new.
    pub(crate) fn record_edit_shapes(
        open: &[(String, usize)],
        vars: &super::unify::Vars,
        shapes: &mut BTreeMap<String, Ty>,
    ) -> bool {
        let mut changed = false;
        for (name, var) in open {
            let resolved = vars.resolve(&Ty::Var(*var));
            if shapes.contains_key(name) {
                continue;
            }
            // A variable nothing solved holds no value with labels, as the elements of a list
            // that stays empty. A type with a function or a variable inside has label positions
            // a shape cannot name: it is recorded as such, and pass 2 rejects it (N-45).
            if matches!(resolved, Ty::Var(_)) {
                continue;
            }
            let shape = if closed(&resolved) {
                resolved
            } else {
                Ty::Error
            };
            shapes.insert(name.clone(), shape);
            changed = true;
        }
        changed
    }

    /// In pass 1, records the labels that `found` brings to the label variables of `expected`,
    /// the type of an editable place it flows into (ADR 0075 rule 2).
    pub(crate) fn record_edit_bounds(&mut self, found: &Ty, expected: &Ty) {
        if self.pass != Pass::Collect || self.edit_bounds.is_empty() {
            return;
        }
        let (found, expected) = (self.labeled(found), self.labeled(expected));
        let mut flows = Vec::new();
        pair_labels(&found, &expected, None, &mut flows);
        for (name, label) in flows {
            // A variable derived from one of this declaration's, for a position inside a type
            // an earlier check has not yet seen, is recorded too.
            let base = name.split('/').next().unwrap_or(&name);
            if !self.edit_bounds.contains_key(base) {
                continue;
            }
            let bound = self.edit_bounds.entry(name).or_insert_with(Label::public);
            *bound = bound.join(&label);
        }
    }
}

/// At the end of a declaration's check in pass 1, solves the label variables of `bounds` into
/// `solutions`, for the next check (ADR 0075 rule 2). Returns whether a solution changed.
pub(crate) fn solve_into(
    bounds: &BTreeMap<String, Label>,
    solutions: &mut BTreeMap<String, Label>,
) -> Option<String> {
    let mut changed = None;
    for (name, label) in solve(bounds) {
        if let Some(old) = solutions.get(&name) {
            // Bounds only grow with the solutions and summaries they read, so a solution only
            // rises; the loops that re-solve end on that (ADR 0075 rule 5).
            debug_assert!(old.flows_to(&label), "the solution of {name} fell");
            if *old == label {
                continue;
            }
        }
        changed.get_or_insert_with(|| name.clone());
        solutions.insert(name, label);
    }
    changed
}

/// The labels at the positions of `found` and `expected` that correspond, for each label
/// variable in `expected`: `(variable, label of found there)`. Under a position with a variable,
/// a position without one has the variable derived from it (`outer`), as later checks name it.
fn pair_labels(found: &Ty, expected: &Ty, outer: Option<&str>, out: &mut Vec<(String, Label)>) {
    let (found_label, found) = strip(found);
    let (expected_label, expected) = strip(expected);
    let own: Vec<&String> = expected_label
        .symbols
        .iter()
        .filter(|s| is_variable(s))
        .collect();
    for name in &own {
        out.push(((*name).clone(), found_label.clone()));
    }
    let here = own.first().map(|name| name.as_str()).or(outer);
    if own.is_empty()
        && let Some(name) = outer
    {
        out.push((name.to_owned(), found_label.clone()));
    }
    let child = |slot: &str| here.map(|name| derived(name, slot));
    match (found, expected) {
        (Ty::List(a), Ty::List(b)) => pair_labels(a, b, child("e").as_deref(), out),
        (Ty::Map(ka, va), Ty::Map(kb, vb)) => {
            pair_labels(ka, kb, child("k").as_deref(), out);
            pair_labels(va, vb, child("v").as_deref(), out);
        }
        (Ty::Kind(k1, a1), Ty::Kind(k2, a2)) if k1 == k2 => {
            for (i, (a, b)) in a1.iter().zip(a2).enumerate() {
                pair_labels(a, b, child(&i.to_string()).as_deref(), out);
            }
        }
        _ => {}
    }
}

/// The variable at the position `slot` inside the position of the variable `name`.
fn derived(name: &str, slot: &str) -> String {
    format!("{name}/{slot}")
}

/// Whether `ty` holds no unification variable and no function type, so it can be a shape.
fn closed(ty: &Ty) -> bool {
    match ty {
        Ty::Var(_) | Ty::Fn(_) | Ty::Error | Ty::Never => false,
        Ty::Labeled(_, inner) | Ty::List(inner) => closed(inner),
        Ty::Map(key, value) => closed(key) && closed(value),
        Ty::Kind(_, args) => args.iter().all(closed),
        _ => true,
    }
}

/// The join of the labels at the top of `ty`, and the type under them.
fn strip(ty: &Ty) -> (Label, &Ty) {
    let mut label = Label::public();
    let mut ty = ty;
    while let Ty::Labeled(top, inner) = ty {
        label = label.join(top);
        ty = inner;
    }
    (label, ty)
}

/// The least solution of `bounds`, where each variable is above its bound and a variable named
/// in a bound stands for its own solution: Tarjan's strongly connected components, each solved
/// after the components it flows from, so every bound is joined once.
fn solve(bounds: &BTreeMap<String, Label>) -> BTreeMap<String, Label> {
    let names: Vec<&String> = bounds.keys().collect();
    let index_of: BTreeMap<&String, usize> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (*name, i))
        .collect();
    // The constant part of each bound, and the variables it names.
    let mut constant = Vec::with_capacity(names.len());
    let mut edges: Vec<Vec<usize>> = Vec::with_capacity(names.len());
    for name in &names {
        let bound = bounds.get(*name).cloned().unwrap_or_else(Label::public);
        let mut label = bound.clone();
        label.symbols.retain(|s| !is_variable(s));
        constant.push(label);
        edges.push(
            bound
                .symbols
                .iter()
                .filter(|s| is_variable(s))
                .filter_map(|s| index_of.get(s).copied())
                .collect(),
        );
    }
    let components = tarjan(&edges);
    // The component of each variable, so an edge inside a component is told in O(1).
    let mut component_of = vec![usize::MAX; names.len()];
    for (id, component) in components.iter().enumerate() {
        for &member in component {
            if let Some(slot) = component_of.get_mut(member) {
                *slot = id;
            }
        }
    }
    let mut solution: Vec<Label> = vec![Label::public(); names.len()];
    // Tarjan emits a component after every component it reaches, so those are solved first.
    for (id, component) in components.into_iter().enumerate() {
        let mut label = Label::public();
        for &member in &component {
            if let Some(own) = constant.get(member) {
                label = label.join(own);
            }
            for &next in edges.get(member).map_or(&[][..], Vec::as_slice) {
                if component_of.get(next) != Some(&id)
                    && let Some(solved) = solution.get(next)
                {
                    label = label.join(solved);
                }
            }
        }
        for &member in &component {
            if let Some(slot) = solution.get_mut(member) {
                *slot = label.clone();
            }
        }
    }
    names.into_iter().cloned().zip(solution).collect()
}

/// The strongly connected components of a graph, each emitted after the components it reaches,
/// without recursion on the native stack.
fn tarjan(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    const UNSEEN: usize = usize::MAX;
    let n = edges.len();
    let mut index = vec![UNSEEN; n];
    let mut low = vec![0; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut components = Vec::new();
    let mut next = 0;
    let at = |v: &[usize], i: usize| v.get(i).copied().unwrap_or(UNSEEN);
    for root in 0..n {
        if at(&index, root) != UNSEEN {
            continue;
        }
        // Frames of the depth-first search: a node and the position of its next edge.
        let mut frames: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some(&(node, edge)) = frames.last() {
            if edge == 0 && at(&index, node) == UNSEEN {
                if let Some(slot) = index.get_mut(node) {
                    *slot = next;
                }
                if let Some(slot) = low.get_mut(node) {
                    *slot = next;
                }
                next += 1;
                stack.push(node);
                if let Some(slot) = on_stack.get_mut(node) {
                    *slot = true;
                }
            }
            let targets = edges.get(node).map_or(&[][..], Vec::as_slice);
            if let Some(&target) = targets.get(edge) {
                if let Some(frame) = frames.last_mut() {
                    frame.1 += 1;
                }
                if at(&index, target) == UNSEEN {
                    frames.push((target, 0));
                } else if on_stack.get(target) == Some(&true) {
                    let reached = at(&index, target);
                    if let Some(slot) = low.get_mut(node) {
                        *slot = (*slot).min(reached);
                    }
                }
                continue;
            }
            frames.pop();
            let node_low = at(&low, node);
            if let Some(&(parent, _)) = frames.last()
                && let Some(slot) = low.get_mut(parent)
            {
                *slot = (*slot).min(node_low);
            }
            if at(&index, node) == node_low {
                let mut component = Vec::new();
                while let Some(member) = stack.pop() {
                    if let Some(slot) = on_stack.get_mut(member) {
                        *slot = false;
                    }
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                components.push(component);
            }
        }
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(entries: &[(&str, Label)]) -> BTreeMap<String, Label> {
        entries
            .iter()
            .map(|(name, label)| ((*name).to_owned(), label.clone()))
            .collect()
    }

    #[test]
    fn a_variable_takes_what_flows_into_it_and_into_the_variables_it_names() {
        let t = Label::symbol("T");
        let solved = solve(&bounds(&[
            ("^a", t.clone()),
            ("^b", Label::symbol("^a")),
            ("^c", Label::public()),
        ]));
        assert_eq!(solved.get("^a"), Some(&t));
        assert_eq!(solved.get("^b"), Some(&t));
        assert_eq!(solved.get("^c"), Some(&Label::public()));
    }

    #[test]
    fn a_cycle_of_variables_takes_the_join_of_its_bounds() {
        let (t, u) = (Label::symbol("T"), Label::symbol("U"));
        let solved = solve(&bounds(&[
            ("^a", Label::symbol("^b").join(&t)),
            ("^b", Label::symbol("^a")),
            ("^c", Label::symbol("^b").join(&u)),
        ]));
        assert_eq!(solved.get("^a"), Some(&t));
        assert_eq!(solved.get("^b"), Some(&t));
        assert_eq!(solved.get("^c"), Some(&t.join(&u)));
    }

    #[test]
    fn solving_a_long_cycle_is_linear() {
        // ^i <- ^(i+1 mod n), and T into ^0: one component of n variables, each above T.
        let n = 200_000;
        let mut entries: BTreeMap<String, Label> = BTreeMap::new();
        for i in 0..n {
            entries.insert(format!("^{i}"), Label::symbol(format!("^{}", (i + 1) % n)));
        }
        entries.insert(
            "^0".to_owned(),
            Label::symbol("^1").join(&Label::symbol("T")),
        );
        let solved = solve(&entries);
        assert!(solved.values().all(|label| *label == Label::symbol("T")));
    }

    #[test]
    fn solving_a_long_chain_is_linear_and_does_not_recurse() {
        // ^0 <- ^1 <- ... <- ^n, with T at the end: each variable is joined once.
        let n = 200_000;
        let mut entries: BTreeMap<String, Label> = BTreeMap::new();
        for i in 0..n {
            entries.insert(format!("^{i}"), Label::symbol(format!("^{}", i + 1)));
        }
        entries.insert(format!("^{n}"), Label::symbol("T"));
        let solved = solve(&entries);
        assert_eq!(solved.get("^0"), Some(&Label::symbol("T")));
    }
}
