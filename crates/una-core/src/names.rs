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
        Expr::Handle { body, handler, .. } => {
            vars(body, out);
            vars(handler, out);
        }
        Expr::Attempt { body, .. } => vars(body, out),
        Expr::LetTuple { value, body, .. } => {
            atoms(std::slice::from_ref(value), out);
            vars(body, out);
        }
        Expr::Loop {
            source, init, body, ..
        } => {
            match source.as_ref() {
                LoopSource::List(list) => atoms(std::slice::from_ref(list), out),
                LoopSource::Range { low, high, .. } => {
                    atoms(std::slice::from_ref(low), out);
                    atoms(std::slice::from_ref(high), out);
                }
            }
            atoms(init, out);
            vars(body, out);
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
