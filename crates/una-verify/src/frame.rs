//! The length frame of a loop (ADR 0071): which lists the loop updates keep their length, so
//! the verifier can assume it in the body and after the loop.
//!
//! The analysis is syntactic, over Core, in one pass over the body. Each name gets the set of
//! state positions whose length at the start of the iteration it shares; a tuple gets one such
//! set per component.

use std::collections::{BTreeMap, BTreeSet};

use una_core::{Atom, BinOp, Bound, Clause, Expr, Name, Path, Rhs, Term, Type};
use una_syntax::Span;

/// The state positions whose starting length a value shares, per component of the value: one
/// component for a list, one per component for a tuple, none for anything else.
type Shape = Vec<BTreeSet<usize>>;

/// The parameters and `ensures` clauses of a callee.
pub(crate) type Callee<'p> = (&'p [(Name, Type)], &'p [Clause]);

/// The length frames of loops, computed once per loop.
pub(crate) struct Frames<'p, 'l> {
    contract: &'l dyn Fn(&Path) -> Option<Callee<'p>>,
    done: &'l mut BTreeMap<Span, Vec<bool>>,
}

impl<'p, 'l> Frames<'p, 'l> {
    /// Frames that look callees up with `contract`, and keep each loop's in `done`.
    pub(crate) fn new(
        contract: &'l dyn Fn(&Path) -> Option<Callee<'p>>,
        done: &'l mut BTreeMap<Span, Vec<bool>>,
    ) -> Frames<'p, 'l> {
        Frames { contract, done }
    }

    /// Whether each state binding of the loop at `span` is length-preserving (ADR 0071 D1):
    /// every path of `body` that completes an iteration gives it a value whose length is its
    /// value's at the start of the iteration.
    pub(crate) fn preserved(
        &mut self,
        span: Span,
        state: &[(Name, Type)],
        body: &Expr,
    ) -> Vec<bool> {
        if let Some(known) = self.done.get(&span) {
            return known.clone();
        }
        let mut env: BTreeMap<Name, Shape> = BTreeMap::new();
        for (index, (name, _)) in state.iter().enumerate() {
            env.insert(name.clone(), vec![BTreeSet::from([index])]);
        }
        let shape = self.expr(body, &mut env);
        let preserved: Vec<bool> = (0..state.len())
            .map(|index| match &shape {
                // No path completes an iteration: the loop fails at its first one, or has none.
                None => true,
                Some(shape) => shape.get(index).is_some_and(|set| set.contains(&index)),
            })
            .collect();
        self.done.insert(span, preserved.clone());
        preserved
    }

    /// The shape of the value of `expr`, joined over the paths that complete; `None` when
    /// none does.
    fn expr(&mut self, expr: &Expr, env: &mut BTreeMap<Name, Shape>) -> Option<Shape> {
        match expr {
            Expr::Atom { atom, .. } => Some(atom_shape(atom, env)),
            Expr::Let {
                name, value, body, ..
            } => {
                let shape = match value.as_ref() {
                    Bound::Rhs(rhs) => self.rhs(rhs, env),
                    Bound::Expr(inner) => self.expr(inner, &mut env.clone())?,
                };
                env.insert(name.clone(), shape);
                self.expr(body, env)
            }
            Expr::LetTuple {
                names, value, body, ..
            } => {
                let shape = atom_shape(value, env);
                for (index, (name, _)) in names.iter().enumerate() {
                    let component = shape.get(index).cloned().unwrap_or_default();
                    env.insert(name.clone(), vec![component]);
                }
                self.expr(body, env)
            }
            Expr::Loop {
                state,
                init,
                body,
                span,
                ..
            } => {
                let preserved = self.preserved(*span, state, body);
                Some(
                    init.iter()
                        .zip(preserved)
                        .map(|(atom, kept)| {
                            if kept {
                                single(atom, env)
                            } else {
                                BTreeSet::new()
                            }
                        })
                        .collect(),
                )
            }
            Expr::If {
                then, otherwise, ..
            } => {
                let then = self.expr(then, &mut env.clone());
                let otherwise = self.expr(otherwise, &mut env.clone());
                join(then, otherwise)
            }
            Expr::Case {
                arms, otherwise, ..
            } => {
                let mut shape = None;
                for arm in arms {
                    let mut scope = env.clone();
                    for name in arm.binds.iter().flatten() {
                        scope.remove(name);
                    }
                    shape = join(shape, self.expr(&arm.body, &mut scope));
                }
                if let Some(otherwise) = otherwise {
                    shape = join(shape, self.expr(otherwise, &mut env.clone()));
                }
                shape
            }
            Expr::Handle {
                body,
                error,
                handler,
            } => {
                let body = self.expr(body, &mut env.clone());
                let mut scope = env.clone();
                scope.remove(error);
                join(body, self.expr(handler, &mut scope))
            }
            Expr::With { body, .. } => self.expr(body, env),
            Expr::Fail { .. } | Expr::Unreachable { .. } => None,
            // The value of `attempt` is a `Result`, and those of `together` say nothing about
            // the state.
            Expr::Attempt { .. } | Expr::Together { .. } | Expr::TogetherFor { .. } => {
                Some(Vec::new())
            }
        }
    }

    /// The shape of a right-hand side.
    fn rhs(&mut self, rhs: &Rhs, env: &BTreeMap<Name, Shape>) -> Shape {
        match rhs {
            Rhs::Atom(atom) => atom_shape(atom, env),
            Rhs::Tuple(atoms) => atoms.iter().map(|atom| single(atom, env)).collect(),
            // ADR 0071 D1 rule 2: the component `j` of the result keeps the length of the
            // argument for `p` when the callee ensures `p'.len == old(p).len`, where `p'` is
            // that component.
            Rhs::Call { function, args, .. } => {
                let Some((params, ensures)) = (self.contract)(function) else {
                    return Vec::new();
                };
                let mut shape: Shape = Vec::new();
                for clause in ensures {
                    let Some((component, param)) = keeps_length(&clause.term) else {
                        continue;
                    };
                    let Some(position) = params.iter().position(|(name, _)| name == param) else {
                        continue;
                    };
                    let Some(arg) = args.get(position) else {
                        continue;
                    };
                    if shape.len() <= component {
                        shape.resize(component + 1, BTreeSet::new());
                    }
                    shape[component].extend(single(arg, env));
                }
                shape
            }
            _ => Vec::new(),
        }
    }
}

/// The shape of an atom.
fn atom_shape(atom: &Atom, env: &BTreeMap<Name, Shape>) -> Shape {
    match atom {
        Atom::Var(name) => env.get(name).cloned().unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The positions whose length a list atom shares.
fn single(atom: &Atom, env: &BTreeMap<Name, Shape>) -> BTreeSet<usize> {
    match atom_shape(atom, env).as_slice() {
        [set] => set.clone(),
        _ => BTreeSet::new(),
    }
}

/// The join of two paths' shapes: what both keep.
fn join(a: Option<Shape>, b: Option<Shape>) -> Option<Shape> {
    match (a, b) {
        (None, other) | (other, None) => other,
        (Some(a), Some(b)) => Some(
            a.iter()
                .zip(&b)
                .map(|(x, y)| x.intersection(y).copied().collect())
                .collect(),
        ),
    }
}

/// `result.j.len == p.len`, either way round: the component and the parameter it keeps the
/// length of. In `ensures`, an `edit` parameter's final value is `result.j` and its value on
/// entry, `old(p)`, is `p`.
fn keeps_length(term: &Term) -> Option<(usize, &Name)> {
    let Term::Binary {
        op: BinOp::Eq,
        left,
        right,
    } = term
    else {
        return None;
    };
    let component = |term: &Term| match term {
        Term::Len(inner) => match inner.as_ref() {
            Term::Proj { value, index } if matches!(value.as_ref(), Term::Result) => Some(*index),
            _ => None,
        },
        _ => None,
    };
    match (component(left), param(right)) {
        (Some(index), Some(name)) => Some((index, name)),
        _ => component(right).zip(param(left)),
    }
}


/// `p.len`: the parameter whose length a term is.
fn param(term: &Term) -> Option<&Name> {
    match term {
        Term::Len(inner) => match inner.as_ref() {
            Term::Var(name) => Some(name),
            _ => None,
        },
        _ => None,
    }
}
