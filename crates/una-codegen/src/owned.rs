//! Borrow inference for the parameters of the package's functions (ADR 0072 D2, #262).
//!
//! A parameter is **owned** when its function consumes it: the body may move it into an
//! update in place, so the caller gives up its reference when the argument is at its last
//! use, and retains it otherwise. Every other parameter is **borrowed**, with no reference
//! counting at the call, as before. A function consumes a name that reaches:
//! - the list of `append` or `set` of the prelude, or the text a concatenation begins with;
//! - an owned parameter of a function of the package, or any parameter of a function value,
//!   which a lambda owns, or the initial state of `fold`;
//! - the initial state of a loop whose body consumes that state;
//! - a binding that aliases it (`let b = a`, or the text of `a` as text), which is consumed.
//!
//! Owned parameters of mutually recursive functions are a least fixpoint: every parameter
//! starts borrowed, and a pass over the package marks the ones whose functions consume them,
//! until none changes. Each pass is linear in the size of the package, and a pass that
//! changes nothing ends it, after at most one pass per parameter.

use std::collections::{BTreeMap, BTreeSet};

use una_core::{Atom, Bound, Decl, Expr, MapOp, PRELUDE_MODULE, Prim, Program, Rhs};

/// The owned parameters of each function of the package, by qualified name, in parameter
/// order (the type descriptors that follow them are borrowed).
pub(crate) fn owned_params(program: &Program) -> BTreeMap<String, Vec<bool>> {
    let functions: Vec<(String, &una_core::Fun)> = program
        .modules
        .iter()
        .flat_map(|module| {
            module.decls.iter().filter_map(move |decl| match decl {
                Decl::Fun(fun) => Some((format!("{}.{}", module.path, fun.name), fun.as_ref())),
                _ => None,
            })
        })
        .collect();
    let mut owned: BTreeMap<String, Vec<bool>> = functions
        .iter()
        .map(|(name, fun)| (name.clone(), vec![false; fun.params.len()]))
        .collect();
    loop {
        let mut changed = false;
        for (name, fun) in &functions {
            let consumed = consumed(&fun.body, &owned);
            let next: Vec<bool> = fun
                .params
                .iter()
                .map(|(param, _)| consumed.contains(param.as_str()))
                .collect();
            if owned.get(name) != Some(&next) {
                owned.insert(name.clone(), next);
                changed = true;
            }
        }
        if !changed {
            return owned;
        }
    }
}

/// The names `body` consumes, given the owned parameters known so far.
fn consumed<'e>(body: &'e Expr, owned: &BTreeMap<String, Vec<bool>>) -> BTreeSet<&'e str> {
    let mut facts = Facts::default();
    facts.expr(body, owned);
    // A binding is consumed when an alias of it is: follow the aliases back to their sources.
    let mut consumed = facts.consumed;
    let mut pending: Vec<&str> = consumed.iter().copied().collect();
    while let Some(name) = pending.pop() {
        for source in facts.aliases.get(name).into_iter().flatten() {
            if consumed.insert(source) {
                pending.push(source);
            }
        }
    }
    consumed
}

/// What a body says about consumption: the names it consumes directly, and for each binding,
/// the names it aliases.
#[derive(Default)]
struct Facts<'e> {
    consumed: BTreeSet<&'e str>,
    aliases: BTreeMap<&'e str, Vec<&'e str>>,
}

impl<'e> Facts<'e> {
    fn consume(&mut self, atom: Option<&'e Atom>) {
        if let Some(Atom::Var(name)) = atom {
            self.consumed.insert(name);
        }
    }

    fn alias(&mut self, name: &'e str, atom: Option<&'e Atom>) {
        if let Some(Atom::Var(source)) = atom {
            self.aliases.entry(name).or_default().push(source);
        }
    }

    fn rhs(&mut self, name: &'e str, rhs: &'e Rhs, owned: &BTreeMap<String, Vec<bool>>) {
        match rhs {
            Rhs::Atom(atom) => self.alias(name, Some(atom)),
            Rhs::Prim {
                prim: Prim::ToText,
                args,
                ..
            } => self.alias(name, args.first()),
            Rhs::Prim {
                prim: Prim::Concat,
                args,
                ..
            } => self.consume(args.first()),
            // A map's `set` and `remove` update it in place (`spec/21 §5` rule 2).
            Rhs::Prim {
                prim: Prim::Map(MapOp::With | MapOp::Without),
                args,
                ..
            } => self.consume(args.first()),
            // So do the updates of the collections of `std.collections` (`spec/28`, ADR 0119
            // rule 1).
            Rhs::Prim {
                prim: Prim::Seq(op),
                args,
                ..
            } if op.rebuilds() => self.consume(args.first()),
            Rhs::Prim {
                prim: Prim::Set(op),
                args,
                ..
            } if op.rebuilds() => self.consume(args.first()),
            Rhs::Prim {
                prim: Prim::HashMap(op),
                args,
                ..
            } if op.rebuilds() => self.consume(args.first()),
            Rhs::Prim {
                prim: Prim::HashSet(op),
                args,
                ..
            } if op.rebuilds() => self.consume(args.first()),
            Rhs::Prim {
                prim: Prim::Priority(op),
                args,
                ..
            } if op.rebuilds() => self.consume(args.first()),
            Rhs::Prim {
                prim: Prim::BitSet(op),
                args,
                ..
            } if op.rebuilds() => self.consume(args.first()),
            Rhs::Call { function, args, .. } => {
                if function.module == PRELUDE_MODULE {
                    if matches!(function.name.as_str(), "append" | "set") {
                        self.consume(args.first());
                    }
                    // `fold` hands its initial state to the step as owned.
                    if function.name == "fold" {
                        self.consume(args.get(1));
                    }
                } else if let Some(params) =
                    owned.get(&format!("{}.{}", function.module, function.name))
                {
                    for (arg, &owned) in args.iter().zip(params) {
                        if owned {
                            self.consume(Some(arg));
                        }
                    }
                }
            }
            // A lambda owns every parameter, so a call through a function value consumes
            // every argument.
            Rhs::Apply { args, .. } => args.iter().for_each(|arg| self.consume(Some(arg))),
            Rhs::Lambda { body, .. } => self.expr(body, owned),
            Rhs::Checked { predicate, .. } => self.expr(predicate, owned),
            _ => {}
        }
    }

    fn expr(&mut self, expr: &'e Expr, owned: &BTreeMap<String, Vec<bool>>) {
        match expr {
            Expr::Atom { .. }
            | Expr::Fail { .. }
            | Expr::Exit { .. }
            | Expr::Unreachable { .. } => {}
            Expr::Let {
                name, value, body, ..
            } => {
                match value.as_ref() {
                    Bound::Rhs(rhs) => self.rhs(name, rhs, owned),
                    Bound::Expr(inner) => self.expr(inner, owned),
                }
                self.expr(body, owned);
            }
            Expr::LetTuple { body, .. } => self.expr(body, owned),
            Expr::Loop {
                state, init, body, ..
            } => {
                // A state's initial value is consumed when the state is: `state` aliases it.
                for ((name, _), atom) in state.iter().zip(init) {
                    self.alias(name, Some(atom));
                }
                self.expr(body, owned);
            }
            Expr::Together { tasks, .. } => tasks.iter().for_each(|task| self.expr(task, owned)),
            Expr::TogetherFor { task, .. } => self.expr(task, owned),
            Expr::If {
                then, otherwise, ..
            } => {
                self.expr(then, owned);
                self.expr(otherwise, owned);
            }
            Expr::Case {
                arms, otherwise, ..
            } => {
                for arm in arms {
                    self.expr(&arm.body, owned);
                }
                if let Some(otherwise) = otherwise {
                    self.expr(otherwise, owned);
                }
            }
            Expr::Handle { body, handler, .. } => {
                self.expr(body, owned);
                self.expr(handler, owned);
            }
            Expr::Attempt { body, .. } | Expr::With { body, .. } => self.expr(body, owned),
        }
    }
}
