//! The length frame of a loop (ADR 0071): which lists the loop updates keep their length, so
//! the verifier can assume it in the body and after the loop. In edition 0.4 it is the shape
//! frame (ADR 0120 V4): an array keeps its length, a matrix its rows and its columns, and a
//! tensor its rank, its element count and its length along each axis.
//!
//! The analysis is syntactic, over Core, in one pass over the body. Each name gets the set of
//! state positions whose length at the start of the iteration it shares; a tuple gets one such
//! set per component.

use std::collections::{BTreeMap, BTreeSet};

use una_core::{Atom, BinOp, Bound, Clause, Expr, Name, Path, Rhs, ShapeFun, Term, Type};
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
    /// For each loop whose body is being read, innermost last, the join of the states its
    /// `skip` and `stop` exits give: paths that complete an iteration too (ADR 0116 V2).
    exits: Vec<Option<Shape>>,
}

impl<'p, 'l> Frames<'p, 'l> {
    /// Frames that look callees up with `contract`, and keep each loop's in `done`.
    pub(crate) fn new(
        contract: &'l dyn Fn(&Path) -> Option<Callee<'p>>,
        done: &'l mut BTreeMap<Span, Vec<bool>>,
    ) -> Frames<'p, 'l> {
        Frames {
            contract,
            done,
            exits: Vec::new(),
        }
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
        self.exits.push(None);
        let shape = self.expr(body, &mut env);
        let exits = self.exits.pop().flatten();
        let shape = join(shape, exits);
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
            // The value of a loop with `on stop` is that block's, which says nothing here.
            Expr::Loop {
                on_stop: Some(_), ..
            } => Some(Vec::new()),
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
            // `e within d` has `e`'s value when it returns in time (`spec/27 §5`).
            Expr::With { body, .. } | Expr::Within { body, .. } => self.expr(body, env),
            // A `tasks` statement has its block's value; its `exit`s cross it (`spec/27 §7`).
            Expr::Tasks { body, .. } => self.expr(body, env),
            // `quiet e` is `e` (S-39).
            Expr::Quiet { body, .. } => self.expr(body, env),
            Expr::Fail { .. } | Expr::Unreachable { .. } => None,
            // `skip` and `stop` complete the iteration with their state, and the path ends.
            Expr::Exit { state, .. } => {
                let shape: Shape = state.iter().map(|atom| single(atom, env)).collect();
                if let Some(exits) = self.exits.last_mut() {
                    *exits = join(exits.take(), Some(shape));
                }
                None
            }
            // The value of `attempt` is a `Result`, and those of `together` and `first of` say
            // nothing about the state.
            Expr::Attempt { .. }
            | Expr::Together { .. }
            | Expr::TogetherFor { .. }
            | Expr::FirstOf { .. }
            | Expr::Start { .. } => Some(Vec::new()),
        }
    }

    /// The shape of a right-hand side.
    fn rhs(&mut self, rhs: &Rhs, env: &BTreeMap<Name, Shape>) -> Shape {
        match rhs {
            Rhs::Atom(atom) => atom_shape(atom, env),
            Rhs::Tuple(atoms) => atoms.iter().map(|atom| single(atom, env)).collect(),
            // ADR 0071 D1 rule 2: the component `j` of the result keeps the length of the
            // argument for `p` when the callee ensures `p'.len == old(p).len`, where `p'` is
            // that component. ADR 0120 V4: a matrix keeps its shape when the callee ensures
            // both `p'.rows == old(p).rows` and `p'.columns == old(p).columns`.
            Rhs::Call { function, args, .. } => {
                let Some((params, ensures)) = (self.contract)(function) else {
                    return Vec::new();
                };
                let mut kept: BTreeMap<(usize, &Name), BTreeSet<Key>> = BTreeMap::new();
                for clause in ensures {
                    if let Some((component, param, fun)) = keeps_length(&clause.term) {
                        kept.entry((component, param)).or_default().insert(fun);
                    }
                }
                let mut shape: Shape = Vec::new();
                for ((component, param), funs) in kept {
                    // A tensor keeps its shape when its rank, its element count and its length
                    // along each of the 8 axes a rank allows are kept.
                    let tensor = funs.contains(&Key::Fun(ShapeFun::Rank))
                        && funs.contains(&Key::Fun(ShapeFun::Count))
                        && (0..8).all(|axis| funs.contains(&Key::Dim(axis)));
                    let whole = funs.contains(&Key::Len)
                        || (funs.contains(&Key::Fun(ShapeFun::Rows))
                            && funs.contains(&Key::Fun(ShapeFun::Columns)))
                        || tensor;
                    if !whole {
                        continue;
                    }
                    let Some(position) = params.iter().position(|(name, _)| name == param) else {
                        continue;
                    };
                    let Some(arg) = args.get(position) else {
                        continue;
                    };
                    if shape.len() <= component {
                        shape.resize(component + 1, BTreeSet::new());
                    }
                    if let Some(set) = shape.get_mut(component) {
                        set.extend(single(arg, env));
                    }
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

/// A shape term that a frame keeps (ADR 0071, ADR 0120 V4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    /// `.len`.
    Len,
    /// `.rows`, `.columns`, `.rank`, a tensor's element count, `.size`.
    Fun(ShapeFun),
    /// `.dim(axis: k)`, with k an integer literal.
    Dim(u64),
}

/// `result.j.len == p.len`, either way round, or the same of another shape term (ADR 0120
/// V4): the component, the parameter it keeps the shape term of, and the term. In `ensures`,
/// an `edit` parameter's final value is `result.j` and its value on entry, `old(p)`, is `p`.
fn keeps_length(term: &Term) -> Option<(usize, &Name, Key)> {
    let Term::Binary {
        op: BinOp::Eq,
        left,
        right,
    } = term
    else {
        return None;
    };
    let component = |term: &Term| {
        let (inner, fun) = shape_function(term)?;
        match inner {
            Term::Proj { value, index } if matches!(value.as_ref(), Term::Result) => {
                Some((*index, fun))
            }
            _ => None,
        }
    };
    let found = match (component(left), param(right)) {
        (Some(found), Some(name)) => Some((found, name)),
        _ => component(right).zip(param(left)),
    };
    match found {
        Some(((index, fun), (name, other))) if fun == other => Some((index, name, fun)),
        _ => None,
    }
}

/// The value a shape term applies to, and the term.
fn shape_function(term: &Term) -> Option<(&Term, Key)> {
    match term {
        Term::Len(inner) => Some((inner.as_ref(), Key::Len)),
        Term::Shape { fun, value } => Some((value.as_ref(), Key::Fun(*fun))),
        Term::Dim { value, axis } => match axis.as_ref() {
            Term::Int(digits) => Some((value.as_ref(), Key::Dim(digits.parse().ok()?))),
            _ => None,
        },
        _ => None,
    }
}

/// `p.len`, `p.rows` or another shape term of a parameter: the parameter, and the term.
fn param(term: &Term) -> Option<(&Name, Key)> {
    let (inner, fun) = shape_function(term)?;
    match inner {
        Term::Var(name) => Some((name, fun)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use una_core::{Base, Label};
    use una_syntax::SourceDb;

    fn span() -> Span {
        let mut db = SourceDb::new();
        Span::at(db.add("t.una", b""), 0)
    }

    fn ty() -> Type {
        Type::Base {
            base: Base::Unknown,
            refinements: Vec::new(),
            label: Label::public(),
        }
    }

    fn var(name: &str) -> Atom {
        Atom::Var(name.to_owned())
    }

    fn let_rhs(name: &str, rhs: Rhs, body: Expr) -> Expr {
        Expr::Let {
            name: name.to_owned(),
            ty: ty(),
            value: Box::new(Bound::Rhs(rhs)),
            body: Box::new(body),
            span: span(),
            written: false,
        }
    }

    fn call(module: &str, name: &str, args: Vec<Atom>) -> Rhs {
        Rhs::Call {
            function: Path {
                module: module.to_owned(),
                name: name.to_owned(),
            },
            type_args: Vec::new(),
            arg_spans: args.iter().map(|_| span()).collect(),
            args,
            span: span(),
        }
    }

    /// `let t = callee(xs, i) in let (a, u) = t in (a)`: the iteration's new state.
    fn through(callee: &str, list: &str) -> Expr {
        let_rhs(
            "t",
            call("m", callee, vec![var(list), var("i")]),
            Expr::LetTuple {
                names: vec![("a".to_owned(), ty()), ("u".to_owned(), ty())],
                value: var("t"),
                body: Box::new(let_rhs(
                    "r",
                    Rhs::Tuple(vec![var("a")]),
                    Expr::Atom {
                        atom: var("r"),
                        span: span(),
                    },
                )),
                span: span(),
            },
        )
    }

    fn len(term: Term) -> Term {
        Term::Len(Box::new(term))
    }

    fn eq(left: Term, right: Term) -> Clause {
        Clause {
            term: Term::Binary {
                op: BinOp::Eq,
                left: Box::new(left),
                right: Box::new(right),
            },
            text: String::new(),
            span: None,
        }
    }

    fn post() -> Term {
        Term::Proj {
            value: Box::new(Term::Result),
            index: 0,
        }
    }

    /// The frame of a loop with one state binding `xs` and body `body`, where `keeps` has the
    /// exact clause, `flipped` has it the other way round, `grows` a weaker one, and any other
    /// callee no contract.
    fn preserved(body: &Expr) -> bool {
        let params = vec![("p".to_owned(), ty()), ("i".to_owned(), ty())];
        let keeps = vec![eq(len(post()), len(Term::Var("p".to_owned())))];
        let flipped = vec![eq(len(Term::Var("p".to_owned())), len(post()))];
        let grows = vec![Clause {
            term: Term::Binary {
                op: BinOp::Ge,
                left: Box::new(len(post())),
                right: Box::new(len(Term::Var("p".to_owned()))),
            },
            text: String::new(),
            span: None,
        }];
        let contract = |path: &Path| match path.name.as_str() {
            "keeps" => Some((params.as_slice(), keeps.as_slice())),
            "flipped" => Some((params.as_slice(), flipped.as_slice())),
            "grows" => Some((params.as_slice(), grows.as_slice())),
            _ => None,
        };
        let mut done = BTreeMap::new();
        Frames::new(&contract, &mut done).preserved(span(), &[("xs".to_owned(), ty())], body)
            == [true]
    }

    #[test]
    fn a_callee_that_ensures_the_length_keeps_it_either_way_round() {
        assert!(preserved(&through("keeps", "xs")));
        assert!(preserved(&through("flipped", "xs")));
    }

    #[test]
    fn a_weaker_contract_or_none_breaks_it() {
        assert!(!preserved(&through("grows", "xs")));
        assert!(!preserved(&through("append", "xs")));
    }

    #[test]
    fn the_argument_must_be_the_state() {
        assert!(!preserved(&through("keeps", "other")));
    }

    #[test]
    fn a_join_keeps_it_only_when_every_completing_branch_does() {
        let fresh = let_rhs(
            "r",
            Rhs::Tuple(vec![var("n")]),
            Expr::Atom {
                atom: var("r"),
                span: span(),
            },
        );
        let fresh = let_rhs("n", Rhs::List(Vec::new()), fresh);
        let branch = |otherwise: Expr| Expr::If {
            cond: Atom::Bool(true),
            then: Box::new(through("keeps", "xs")),
            otherwise: Box::new(otherwise),
        };
        assert!(!preserved(&branch(fresh)));
        assert!(preserved(&branch(through("flipped", "xs"))));
        // A branch that never completes an iteration does not count.
        let stops = Expr::Unreachable {
            span: span(),
            goal: String::new(),
        };
        assert!(preserved(&branch(stops)));
    }
}
