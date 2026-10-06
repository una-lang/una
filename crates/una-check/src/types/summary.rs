//! Result summaries of user generic functions (ADR 0067, `spec/07 §7` rule 8): which type
//! parameters' symbolic labels a function's body can put at each position of its declared
//! result, so that a call raises each position only by the labels of those type arguments.

use std::collections::BTreeSet;

use super::authority::Pass;
use super::check::Checker;
use super::env::{FnKind, Step, Summary};
use super::label::Label;
use super::ty::{FnTy, Ty};
use super::unify::Vars;
use una_project::Edition;

impl Checker<'_, '_, '_> {
    /// The generic function whose result summary the body being checked contributes to: in
    /// the rounds of their least fixpoint, in edition 0.2, a function declared outside the
    /// prelude with type parameters, whose return is `name`.
    fn summarized(&self, name: &str) -> Option<usize> {
        if self.pass != Pass::Collect || self.env.edition < Edition::V0_2 {
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
        let mut found = own(label, &def.type_params);
        if let Ty::Param(index) = def.result.base.unlabeled() {
            found.remove(index);
        }
        if found.is_empty() {
            return;
        }
        let mut summary = Summary::new();
        summary.insert(Vec::new(), found);
        self.grow(id, summary);
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

/// The indices, among `params`, of the type parameters whose symbolic labels `label` holds.
fn own(label: &Label, params: &[&str]) -> BTreeSet<usize> {
    label
        .symbols
        .iter()
        .filter_map(|symbol| params.iter().position(|param| param == symbol))
        .collect()
}

/// Walks the value's type `found` and the declared result type together, recording at each
/// position the type parameters among `params` whose symbolic labels `found` has there. At
/// a position that is a type parameter, the value is a value of that parameter, whose labels
/// its type argument gives by substitution, so only the other parameters count there, and
/// the walk stops.
fn collect(found: &Ty, declared: &Ty, params: &[&str], path: &mut Vec<Step>, out: &mut Summary) {
    let mut at = own(&found.top_label(), params);
    let param = match declared.unlabeled() {
        Ty::Param(index) => {
            at.remove(index);
            true
        }
        _ => false,
    };
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
        (Ty::List(found), Ty::List(declared)) => step(Step::Element, found, declared, out),
        (Ty::Kind(_, found), Ty::Kind(_, declared)) => {
            for (index, (found, declared)) in found.iter().zip(declared).enumerate() {
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
        (Ty::List(declared), Ty::List(actual)) => {
            Ty::List(Box::new(inner(Step::Element, declared, *actual)))
        }
        (Ty::Kind(_, declared), Ty::Kind(id, actual)) => Ty::Kind(
            id,
            actual
                .into_iter()
                .enumerate()
                .map(|(index, actual)| match declared.get(index) {
                    Some(declared) => inner(Step::Arg(index), declared, actual),
                    None => actual,
                })
                .collect(),
        ),
        (Ty::Fn(declared), Ty::Fn(actual)) => {
            let actual = *actual;
            Ty::Fn(Box::new(FnTy {
                result: inner(Step::Result, &declared.result, actual.result.clone()),
                ..actual
            }))
        }
        (_, other) => other,
    };
    raised.with_label(&top)
}
