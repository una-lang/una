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

/// The deepest nesting of Core expressions a body may have: 2^17, eight times the 16,000
/// statements that the project's scaling tests check and run in one body. Every thread that
/// walks Core IR by recursion has [`STACK_BYTES`] of stack, sized from this bound.
pub const MAX_DEPTH: usize = 131_072;

/// The stack that one level of Core IR may take in the deepest recursive pass, with a margin
/// of two (#2339). Measured in an unoptimized build (opt-level 0, the build of the release
/// gates) on aarch64, as the smallest stack each command needs at 63,000 and at 126,000
/// levels: `una core`'s printer takes about 4 KiB per level (244 and 487 MiB), the copy of the
/// program `una test` makes about 2.2 KiB (142 and 280 MiB), `una report` about 1.2 KiB,
/// `una build` and `una core --json` about 0.5 KiB, and the checker, the verifier and the
/// evaluator less than 0.1 KiB. The frames of these passes on x86-64 are within 5% of those on
/// aarch64, at opt-level 0 and 1; opt-level 1 takes about half. A pass whose frame would take
/// more walks its chains of `let`s in a loop, as `una core --json` does.
pub const STACK_PER_LEVEL: usize = 8 * 1024;

/// The stack below the passes over Core IR: the passes over the syntax tree, whose nesting the
/// parser bounds at 256 levels of up to about 40 KiB each in an unoptimized build, and the
/// frames of the command around them.
pub const STACK_BASE: usize = 64 * 1024 * 1024;

/// The stack of every thread of `una` that walks Core IR by recursion, at the deepest Core IR
/// the checker accepts ([`MAX_DEPTH`]): the command's own thread, the checker's, the verifier's,
/// the copy of the program for `una test` and the tier 1 check of `una lsp` (#2264, #2339).
/// 1,088 MiB, reserved, not committed: a thread touches only the pages its recursion reaches.
pub const STACK_BYTES: usize = MAX_DEPTH * STACK_PER_LEVEL + STACK_BASE;

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
        // The derived `Drop` recurses, which is why the checker bounds the depth: no chain of
        // this test is dropped on the test's stack.
        let at = chain(MAX_DEPTH - 1);
        assert_eq!(deeper_than(&at, MAX_DEPTH, fallback), None);
        std::mem::forget(at);
        let past = chain(MAX_DEPTH);
        assert!(deeper_than(&past, MAX_DEPTH, fallback).is_some());
        std::mem::forget(past);
        // The walk itself never recurses: a chain far past the limit, on a small stack.
        let far = chain(200_000);
        let found = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                let found = deeper_than(&far, MAX_DEPTH, fallback);
                std::mem::forget(far);
                found
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(found.is_some());
    }
}
