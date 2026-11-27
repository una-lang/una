//! The nesting depth of an expression of Core IR (#2264): the bound that keeps every pass
//! over Core IR, which recurses once per nested expression, within its thread's stack.
//!
//! Elaboration turns each statement of a block and each temporary of an expression into one
//! `let` whose body is the rest, so a body that a parser of bounded nesting reads as flat, a
//! list literal of many records or a block of many lines, is a Core expression as deep as it
//! is long. The checker refuses a body past [`MAX_DEPTH`], so that no later pass (the
//! verifier, the interpreter, the native backend, `una core` and the Core export, and the
//! derived `Clone`, `Drop` and `PartialEq` of the tree) can exhaust its stack on it.

use una_syntax::Span;

use crate::{Bound, Expr, LoopSource, Rhs};

/// The deepest nesting of Core expressions a body may have. Measured in unoptimized builds on
/// the 8 MiB main thread of `una`, every pass handles twice as deep (16,000) and the first to
/// exhaust its stack does so at about 16,000 (`una test`'s copy of the program); the checker's
/// own thread has 64 MiB. Twice the cap of the records, variants and lists that the constants
/// one body names may substitute (ADR 0126, 4,096), so that cap is reached first.
pub const MAX_DEPTH: usize = 8_192;

/// The first position, in a depth-first walk, at which `expr` nests deeper than `limit` Core
/// expressions, or `None` when it is no deeper. Each `let`, branch, handled body, loop body,
/// task and lambda body is one level. Iterative, with an explicit stack: O(size of `expr`)
/// time and O(its depth) memory on the heap, never on the native stack. `fallback` is the
/// position reported when the expression that passes the limit has none of its own.
pub fn deeper_than(expr: &Expr, limit: usize, fallback: Span) -> Option<Span> {
    let mut stack: Vec<(&Expr, usize, Span)> = vec![(expr, 1, fallback)];
    while let Some((expr, depth, outer)) = stack.pop() {
        let here = span_of(expr).unwrap_or(outer);
        if depth > limit {
            return Some(here);
        }
        let next = depth + 1;
        let mut push = |child| stack.push((child, next, here));
        match expr {
            Expr::Atom { .. }
            | Expr::Exit { .. }
            | Expr::Fail { .. }
            | Expr::Unreachable { .. } => {}
            Expr::Let { value, body, .. } => {
                match value.as_ref() {
                    Bound::Expr(inner) => push(inner),
                    Bound::Rhs(rhs) => match rhs {
                        Rhs::Lambda { body, .. } => push(body),
                        Rhs::Checked { predicate, .. } => push(predicate),
                        _ => {}
                    },
                }
                push(body);
            }
            Expr::LetTuple { body, .. }
            | Expr::Tasks { body, .. }
            | Expr::Quiet { body, .. }
            | Expr::Attempt { body, .. }
            | Expr::Within { body, .. }
            | Expr::With { body, .. } => push(body),
            Expr::Start { task, .. } => push(task),
            Expr::Loop {
                source,
                body,
                on_stop,
                ..
            } => {
                if let LoopSource::Until { cond, .. } = source.as_ref() {
                    push(cond);
                }
                push(body);
                if let Some(on_stop) = on_stop {
                    push(&on_stop.body);
                }
            }
            Expr::TogetherFor { source, task, .. } => {
                if let LoopSource::Until { cond, .. } = source.as_ref() {
                    push(cond);
                }
                push(task);
            }
            Expr::Together { tasks, .. } | Expr::FirstOf { tasks, .. } => {
                for task in tasks {
                    push(task);
                }
            }
            Expr::If {
                then, otherwise, ..
            } => {
                push(then);
                push(otherwise);
            }
            Expr::Case {
                arms, otherwise, ..
            } => {
                for arm in arms {
                    push(&arm.body);
                }
                if let Some(otherwise) = otherwise {
                    push(otherwise);
                }
            }
            Expr::Handle { body, handler, .. } => {
                push(body);
                push(handler);
            }
        }
    }
    None
}

/// The position of `expr` in the source, when it has one of its own.
fn span_of(expr: &Expr) -> Option<Span> {
    match expr {
        Expr::Atom { span, .. }
        | Expr::Let { span, .. }
        | Expr::LetTuple { span, .. }
        | Expr::Loop { span, .. }
        | Expr::Exit { span, .. }
        | Expr::Together { span, .. }
        | Expr::TogetherFor { span, .. }
        | Expr::FirstOf { span, .. }
        | Expr::Tasks { span, .. }
        | Expr::Start { span, .. }
        | Expr::Quiet { span, .. }
        | Expr::Fail { span, .. }
        | Expr::Attempt { span, .. }
        | Expr::Within { span, .. }
        | Expr::Unreachable { span, .. } => Some(*span),
        Expr::If { .. } | Expr::Case { .. } | Expr::Handle { .. } | Expr::With { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Atom, Base, Label, Type};

    fn file() -> una_syntax::FileId {
        una_syntax::SourceDb::new().add("m.una", b"")
    }

    fn chain(n: usize) -> Expr {
        let file = file();
        let span = Span::new(file, 0, 0);
        let mut expr = Expr::Atom {
            atom: Atom::Unit,
            span,
        };
        for i in 0..n {
            expr = Expr::Let {
                name: format!("x{i}"),
                ty: Type::Base {
                    base: Base::Unit,
                    refinements: Vec::new(),
                    label: Label::public(),
                },
                value: Box::new(Bound::Rhs(Rhs::Atom(Atom::Unit))),
                body: Box::new(expr),
                span: Span::new(file, i, i + 1),
                written: false,
            };
        }
        expr
    }

    #[test]
    fn a_chain_at_the_limit_passes_and_one_more_does_not() {
        let fallback = Span::new(file(), 0, 0);
        // `n` lets and the atom at the end: `n + 1` levels.
        let at = chain(MAX_DEPTH - 1);
        assert_eq!(deeper_than(&at, MAX_DEPTH, fallback), None);
        let past = chain(MAX_DEPTH);
        assert!(deeper_than(&past, MAX_DEPTH, fallback).is_some());
        // The walk itself never recurses: a chain far past the limit, on a small stack.
        let far = chain(200_000);
        let found = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                let found = deeper_than(&far, MAX_DEPTH, fallback);
                // The derived `Drop` recurses, which is why the checker bounds the depth: the
                // chain is not dropped on this small stack.
                std::mem::forget(far);
                found
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(found.is_some());
    }
}
