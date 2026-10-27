//! The names an expression of Core IR reads, which the native backend and the interpreter use
//! to find the last use of a binding (#30, #201).

use std::collections::BTreeSet;

use crate::{Atom, Bound, Expr, LoopSource, Rhs, Source};

/// Every binder that `expr` reads, in it and in the lambdas it makes.
pub fn vars(expr: &Expr, out: &mut BTreeSet<String>) {
    let atoms = |list: &[Atom], out: &mut BTreeSet<String>| {
        for atom in list {
            if let Atom::Var(name) = atom {
                out.insert(name.clone());
            }
        }
    };
    match expr {
        Expr::Atom { atom, .. } => atoms(std::slice::from_ref(atom), out),
        Expr::Let { value, body, .. } => {
            match value.as_ref() {
                Bound::Rhs(rhs) => rhs_vars(rhs, out),
                Bound::Expr(inner) => vars(inner, out),
            }
            vars(body, out);
        }
        Expr::If {
            cond,
            then,
            otherwise,
        } => {
            atoms(std::slice::from_ref(cond), out);
            vars(then, out);
            vars(otherwise, out);
        }
        Expr::Case {
            scrutinee,
            arms,
            otherwise,
        } => {
            atoms(std::slice::from_ref(scrutinee), out);
            for arm in arms {
                vars(&arm.body, out);
            }
            if let Some(otherwise) = otherwise {
                vars(otherwise, out);
            }
        }
        Expr::Fail { args, .. } => atoms(args, out),
        Expr::Exit { state, .. } => atoms(state, out),
        Expr::Handle { body, handler, .. } => {
            vars(body, out);
            vars(handler, out);
        }
        Expr::Attempt { body, .. } => vars(body, out),
        Expr::Within { deadline, body, .. } => {
            atoms(std::slice::from_ref(deadline), out);
            vars(body, out);
        }
        Expr::LetTuple { value, body, .. } => {
            atoms(std::slice::from_ref(value), out);
            vars(body, out);
        }
        Expr::Loop {
            source,
            init,
            body,
            on_stop,
            ..
        } => {
            match source.as_ref() {
                LoopSource::List(list) => atoms(std::slice::from_ref(list), out),
                LoopSource::Range { low, high, .. } => {
                    atoms(std::slice::from_ref(low), out);
                    atoms(std::slice::from_ref(high), out);
                }
                LoopSource::Until { cond, .. } => vars(cond, out),
                LoopSource::Ever { within } => {
                    if let Some((deadline, _)) = within {
                        atoms(std::slice::from_ref(deadline), out);
                    }
                }
            }
            atoms(init, out);
            vars(body, out);
            if let Some(on_stop) = on_stop {
                vars(&on_stop.body, out);
            }
        }
        Expr::Together { tasks, .. } => {
            for task in tasks {
                vars(task, out);
            }
        }
        Expr::TogetherFor { source, task, .. } => {
            match source.as_ref() {
                LoopSource::List(list) => atoms(std::slice::from_ref(list), out),
                LoopSource::Range { low, high, .. } => {
                    atoms(std::slice::from_ref(low), out);
                    atoms(std::slice::from_ref(high), out);
                }
                LoopSource::Until { cond, .. } => vars(cond, out),
                LoopSource::Ever { within } => {
                    if let Some((deadline, _)) = within {
                        atoms(std::slice::from_ref(deadline), out);
                    }
                }
            }
            vars(task, out);
        }
        Expr::With { source, body, .. } => {
            if let Source::Handler { args, .. } = source {
                atoms(args, out);
            }
            vars(body, out);
        }
        Expr::Unreachable { .. } => {}
    }
}

/// Every binder that `rhs` reads, in it and in the lambdas and predicates it makes.
pub fn rhs_vars(rhs: &Rhs, out: &mut BTreeSet<String>) {
    let mut add = |atom: &Atom| {
        if let Atom::Var(name) = atom {
            out.insert(name.clone());
        }
    };
    match rhs {
        Rhs::Atom(atom) => add(atom),
        Rhs::Prim { args, .. }
        | Rhs::Call { args, .. }
        | Rhs::Op { args, .. }
        | Rhs::Construct { args, .. }
        | Rhs::List(args)
        | Rhs::Tuple(args) => args.iter().for_each(add),
        Rhs::Apply { callee, args, .. } => {
            add(callee);
            args.iter().for_each(add);
        }
        Rhs::Field { value, .. } => add(value),
        Rhs::Declassify { arg, .. } => add(arg),
        Rhs::Checked {
            value,
            args,
            predicate,
            ..
        } => {
            add(value);
            args.iter().for_each(add);
            vars(predicate, out);
        }
        Rhs::Lambda { body, .. } => vars(body, out),
    }
}

/// Every binder that `expr` introduces, in it and in the lambdas and predicates it makes. Core
/// binders are unique in a declaration, so a name `expr` binds is bound nowhere else: a read of
/// it in `expr` reads that binding.
pub fn binders<'e>(expr: &'e Expr, out: &mut BTreeSet<&'e str>) {
    let rhs_binders = |rhs: &'e Rhs, out: &mut BTreeSet<&'e str>| match rhs {
        Rhs::Lambda { params, body, .. } => {
            out.extend(params.iter().map(|(name, _)| name.as_str()));
            binders(body, out);
        }
        Rhs::Checked { it, predicate, .. } => {
            out.insert(it.as_str());
            binders(predicate, out);
        }
        _ => {}
    };
    match expr {
        Expr::Atom { .. } | Expr::Fail { .. } | Expr::Exit { .. } | Expr::Unreachable { .. } => {}
        Expr::Let {
            name, value, body, ..
        } => {
            out.insert(name.as_str());
            match value.as_ref() {
                Bound::Rhs(rhs) => rhs_binders(rhs, out),
                Bound::Expr(inner) => binders(inner, out),
            }
            binders(body, out);
        }
        Expr::LetTuple { names, body, .. } => {
            out.extend(names.iter().map(|(name, _)| name.as_str()));
            binders(body, out);
        }
        Expr::Loop {
            source,
            var,
            state,
            body,
            on_stop,
            ..
        } => {
            out.insert(var.0.as_str());
            out.extend(state.iter().map(|(name, _)| name.as_str()));
            if let LoopSource::Until { cond, .. } = source.as_ref() {
                binders(cond, out);
            }
            binders(body, out);
            if let Some(on_stop) = on_stop {
                out.extend(on_stop.state.iter().map(|(name, _)| name.as_str()));
                binders(&on_stop.body, out);
            }
        }
        Expr::Together { tasks, .. } => tasks.iter().for_each(|task| binders(task, out)),
        Expr::TogetherFor { var, task, .. } => {
            out.insert(var.0.as_str());
            binders(task, out);
        }
        Expr::If {
            then, otherwise, ..
        } => {
            binders(then, out);
            binders(otherwise, out);
        }
        Expr::Case {
            arms, otherwise, ..
        } => {
            for arm in arms {
                out.extend(arm.binds.iter().flatten().map(String::as_str));
                binders(&arm.body, out);
            }
            if let Some(otherwise) = otherwise {
                binders(otherwise, out);
            }
        }
        Expr::Handle {
            body,
            error,
            handler,
        } => {
            out.insert(error.as_str());
            binders(body, out);
            binders(handler, out);
        }
        Expr::Attempt { body, .. } | Expr::Within { body, .. } => binders(body, out),
        Expr::With { cap, body, .. } => {
            out.insert(cap.as_str());
            binders(body, out);
        }
    }
}
