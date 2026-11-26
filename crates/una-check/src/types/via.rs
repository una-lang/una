//! The failure-decision label of a `via` function (ADR 0098): public when its body's only
//! failure points are `Keys.hmac` calls with a literal key name, passed on at the top level of
//! the body, and above every label otherwise.
//!
//! A failure leaves a body only through a `fail` or a `pass`: `fail V`, `checked ... else
//! fail V`, `if error fail V`, `if error pass` and `if none fail V`. There is no implicit
//! propagation (`spec/06`): every failing call, whatever its syntax (method syntax, a function
//! value, a prelude function whose `fails of` lambda fails), is handled by one of those. So a
//! body whose words `fail` and `pass` are exactly the handling clauses of the allowed calls has
//! no other failure point. The count is over the body's text, so a word in a text literal or a
//! comment only makes the result `⊤`: the check fails closed.
//!
//! The `Keys` that the calls reach must be the host's (rule 2, from the review of ADR 0098):
//! a handler of `Keys` declared in the package is installed only by the `given` lines of
//! examples, which are not runs of the program, and the body has no `with`.
//!
//! At this edition `with` and `given` are the only ways to install a handler (ADR 0098 rule
//! 2): a handler is not a value (U2001) and a capability is second-class (U3005). Any new
//! construct that installs or rebinds a handler must extend this check; ADR 0093's narrowed
//! rebinding `@y[ops]` installs nothing, so it needs no extension. The test `install_forms_are_reviewed` fails to compile
//! when the syntax gains a statement, an expression, a declaration or a binding form, so that
//! each new form is reviewed here.

use std::collections::{BTreeMap, BTreeSet};

use una_syntax::Span;

use una_syntax::ast::{
    Arg, Body, DeclKind, ExprKind, Handled, HandlingKind, LetValue, Literal, Stmt, TextPart,
};

use super::Report;
use super::env::{Env, FnAst};

/// The `via` functions of the package whose failure reveals nothing (ADR 0098 rule 2), by
/// function id, and those of the shape that a `Keys` handler named elsewhere keeps from it,
/// with that handler and where it is named. Linear in the size of the package's text.
pub(crate) fn plain_vias(env: &Env<'_>, report: &Report<'_>) -> PlainVias {
    // A handler of `Keys` installed by the program's code could decide a failure on anything
    // it holds, so no `via` function is lowered then. Handlers are installed only by `with` or
    // `given`, so a handler whose name appears in no declaration but its own and examples is
    // never installed in a run of the program: every `Keys` binding a run reaches is the
    // host's.
    let installed = env.handlers.iter().find_map(|handler| {
        // A handler whose capability does not resolve, as one written `for` an alias, may be
        // one of `Keys`: it counts, so the check fails closed.
        let keys = handler
            .capability
            .and_then(|c| env.capabilities.get(c))
            .is_none_or(|c| c.name == "Keys" && c.module == env.prelude);
        if !keys {
            return None;
        }
        let name = env.text(handler.module, handler.ast.name.name);
        let mut own = None;
        let mut mentions = Vec::new();
        for module in env.modules.iter().take(env.prelude) {
            let decls = &module.tree.decls;
            for decl in decls {
                match &decl.kind {
                    DeclKind::Example(_) => continue,
                    DeclKind::Handler(ast) if std::ptr::eq(ast, handler.ast) => {
                        own = Some(decl.span)
                    }
                    _ => {}
                }
                mentions.extend(named_at(report, decl.span, name));
            }
            // A comment outside every declaration, a doc comment included, is not in a
            // declaration's span: it counts too, so a mention there fails closed.
            for comment in &module.tree.comments {
                let inside = decls.iter().any(|decl| {
                    decl.span.file == comment.span.file
                        && decl.span.start <= comment.span.start
                        && comment.span.end <= decl.span.end
                });
                if !inside {
                    mentions.extend(named_at(report, comment.span, name));
                }
            }
        }
        // Its own declaration names it once.
        (mentions.len() != 1).then(|| {
            let within = |span: &Span, outer: Option<Span>| {
                outer.is_some_and(|o| {
                    o.file == span.file && o.start <= span.start && span.end <= o.end
                })
            };
            mentions.sort_by_key(|span| (span.file, span.start));
            let stray = mentions.into_iter().find(|span| !within(span, own)).or(own);
            (name.to_owned(), stray)
        })
    });
    let shaped = env
        .vias
        .keys()
        .copied()
        .filter(|&id| plain(env, report, id));
    match installed {
        None => PlainVias {
            plain: shaped.collect(),
            blocked: BTreeMap::new(),
        },
        Some((handler, at)) => PlainVias {
            plain: BTreeSet::new(),
            blocked: shaped
                .filter_map(|id| at.map(|at| (id, (handler.clone(), at))))
                .collect(),
        },
    }
}

/// The `via` functions of ADR 0098's shape, as [`plain_vias`] finds them.
#[derive(Default)]
pub(crate) struct PlainVias {
    /// Those whose failure reveals nothing, by function id: a call of their policy fails at the
    /// program counter.
    pub(crate) plain: BTreeSet<usize>,
    /// Those of the shape that a `Keys` handler named outside its declaration and examples keeps
    /// from it, by function id: the handler's name and the first place it is named.
    pub(crate) blocked: BTreeMap<usize, (String, Span)>,
}

/// The spans in `span`'s text where the word `name` occurs.
fn named_at(report: &Report<'_>, span: Span, name: &str) -> Vec<Span> {
    let text = report.text(span);
    let mut found = Vec::new();
    let mut start = None;
    for (offset, c) in text.char_indices().chain([(text.len(), ' ')]) {
        let word = c.is_alphanumeric() || c == '_';
        match (word, start) {
            (true, None) => start = Some(offset),
            (false, Some(from)) => {
                if text.get(from..offset) == Some(name) {
                    found.push(Span {
                        file: span.file,
                        start: span.start + from,
                        end: span.start + offset,
                    });
                }
                start = None;
            }
            _ => {}
        }
    }
    found
}

/// Whether the `via` function `id` has the shape of ADR 0098 rule 2.
fn plain(env: &Env<'_>, report: &Report<'_>, id: usize) -> bool {
    let Some(def) = env.functions.get(id) else {
        return false;
    };
    let FnAst::Function(function) = def.ast else {
        return false;
    };
    // The bindings of `Keys` from the prelude that the header declares.
    let keys: BTreeSet<&str> = def
        .uses
        .iter()
        .filter(|need| {
            need.usage
                .capability
                .and_then(|c| env.capabilities.get(c))
                .is_some_and(|c| c.name == "Keys" && c.module == env.prelude)
        })
        .map(|need| need.usage.name.as_str())
        .collect();
    let (stmts, span) = match &function.body {
        Body::Block(block) => (block.stmts.as_slice(), block.span),
        Body::Line(expr) => (&[][..], expr.span),
    };
    let allowed = stmts
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::Handled(handled) => Some(handled),
            Stmt::Let(binding) => match &binding.value {
                LetValue::Handled(handled) => Some(handled),
                LetValue::BlockExpr(_) => None,
            },
            _ => None,
        })
        .filter(|handled| allowed_hmac(env, report, def.module, &keys, handled))
        .count();
    let text = report.text(span);
    // `with` installs a handler; `within` makes a failure that the timing of the code it
    // bounds decides (ADR 0117 IF2), so neither may stand in the body, as a word anywhere; nor
    // may `first of`, whose winner timing decides (ADR 0117 IF3).
    let found: Vec<&str> = words(text).filter(|word| !word.is_empty()).collect();
    let race = found.windows(2).any(|pair| pair == ["first", "of"]);
    exits(text) == allowed
        && !race
        && !found
            .iter()
            .any(|word| *word == "with" || *word == "within")
}

/// Whether `handled` is `@k.hmac(key_name: "literal", data: e)` with one clause, `if error pass`
/// or `if error fail V`, where `@k` is a `Keys` binding of the header.
fn allowed_hmac(
    env: &Env<'_>,
    report: &Report<'_>,
    module: usize,
    keys: &BTreeSet<&str>,
    handled: &Handled,
) -> bool {
    let [clause] = handled.handlings.as_slice() else {
        return false;
    };
    // A variant with a payload could carry what the unchecked body computes (rule 2).
    let fixed = match &clause.kind {
        HandlingKind::ErrorPass => true,
        HandlingKind::ErrorFail(variant) => variant.args.is_empty(),
        _ => false,
    };
    // A `with` installs a handler; a `within` adds a failure decided by timing, of the `secret`
    // data `hmac` reads (ADR 0117 IF2), which a plain `via` must not reveal.
    if !fixed || !handled.with.is_empty() || handled.within.is_some() {
        return false;
    }
    if let ExprKind::Attempt {
        within: Some(_), ..
    } = &handled.expr.kind
    {
        return false;
    }
    let ExprKind::CapOp { cap, op, args } = &handled.expr.kind else {
        return false;
    };
    keys.contains(env.text(module, cap.name))
        && env.text(module, op.name) == "hmac"
        && args
            .iter()
            .any(|arg| literal_key_name(env, report, module, arg))
}

/// Whether `arg` is `key_name: "..."`, a plain text literal: one text part, and as written,
/// no interpolation and no escape (rule 2, read on the token, not on its value).
fn literal_key_name(env: &Env<'_>, report: &Report<'_>, module: usize, arg: &Arg) -> bool {
    arg.label
        .as_ref()
        .is_some_and(|label| env.text(module, label.name) == "key_name")
        && match &arg.value.kind {
            ExprKind::Literal(Literal::Text(text)) => {
                let written = report.text(text.span);
                matches!(text.parts.as_slice(), [TextPart::Text(_)] | [])
                    && !written.contains('\\')
                    && !written.contains('{')
            }
            _ => false,
        }
}

/// The words of `text`.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
}

/// The number of words `fail` and `pass` in `text`.
fn exits(text: &str) -> usize {
    words(text)
        .filter(|word| *word == "fail" || *word == "pass")
        .count()
}

#[cfg(test)]
mod tests {
    use super::exits;

    #[test]
    fn exits_count_whole_words_only() {
        assert_eq!(exits("if error pass"), 1);
        assert_eq!(
            exits("checked(x, it > 0) else fail Bad\nif none fail Missing"),
            2
        );
        assert_eq!(exits("let failed = passes + fail_count"), 0);
        // A word in a text literal counts too, so the check only fails closed.
        assert_eq!(exits("\"fail\""), 1);
    }

    /// The forms that may install or rebind a handler, matched without a wildcard: a new
    /// variant of any of these fails to compile here, and its author must decide whether it
    /// installs a handler, and extend `plain_vias` if it does (ADR 0098 rule 2).
    #[allow(dead_code, clippy::match_same_arms)]
    fn install_forms_are_reviewed(
        stmt: &una_syntax::ast::Stmt,
        expr: &una_syntax::ast::ExprKind,
        decl: &una_syntax::ast::DeclKind,
        binding: &una_syntax::ast::CapBindingValue,
    ) -> [bool; 4] {
        use una_syntax::ast::{CapBindingValue, DeclKind, ExprKind, Stmt};
        // Whether the form installs a handler, at this edition.
        let stmt = match stmt {
            // A `with` clause of the handled line: counted by `words`. A `start` with one is
            // U2035, and counted all the same, so the check fails closed.
            Stmt::Handled(_) | Stmt::Let(_) | Stmt::Assign(_) | Stmt::Start(_) => true,
            Stmt::Tasks(_)
            | Stmt::IfThen(_)
            | Stmt::For(_)
            | Stmt::Together(_)
            | Stmt::BlockExpr(_)
            | Stmt::Stop(_)
            | Stmt::Skip(_)
            | Stmt::Until(_)
            | Stmt::Ever(_)
            | Stmt::OnStop(_) => false,
        };
        let expr = match expr {
            ExprKind::IfLine { .. }
            | ExprKind::Lambda { .. }
            | ExprKind::Checked { .. }
            | ExprKind::Attempt { .. }
            | ExprKind::Fail(_)
            | ExprKind::Binary { .. }
            | ExprKind::Unary { .. }
            | ExprKind::Is { .. }
            | ExprKind::In { .. }
            | ExprKind::Field { .. }
            | ExprKind::Old(_)
            | ExprKind::MethodCall { .. }
            | ExprKind::Call { .. }
            | ExprKind::Name(_)
            | ExprKind::UpperName(_)
            | ExprKind::CapOp { .. }
            | ExprKind::CapRef(_)
            | ExprKind::Paren(_)
            | ExprKind::List(_)
            | ExprKind::Result
            | ExprKind::It
            | ExprKind::Error
            | ExprKind::Literal(_) => false,
        };
        let decl = match decl {
            // `given` lines, in examples only.
            DeclKind::Example(_) => true,
            DeclKind::Kind(_)
            | DeclKind::Predicate(_)
            | DeclKind::Function(_)
            | DeclKind::Capability(_)
            | DeclKind::Handler(_)
            | DeclKind::Policy(_)
            | DeclKind::Requirement(_)
            | DeclKind::Measure(_)
            | DeclKind::Const(_) => false,
        };
        let binding = match binding {
            CapBindingValue::Handler(_) => true,
            // A narrowed rebinding (ADR 0093 rule 4) restricts an existing binding to some of
            // its operations and installs nothing: the handler behind it stays the one already
            // bound, so the host's `Keys` stays the host's.
            CapBindingValue::Cap(_) | CapBindingValue::Narrowed { .. } => false,
        };
        [stmt, expr, decl, binding]
    }

    /// The parts of a handled line and of `attempt`, destructured without `..`: a new part
    /// fails to compile here, and its author must decide whether it changes what the line's
    /// failure depends on, and extend `allowed_hmac` if it does (ADR 0098 rule 2; ADR 0117 IF2
    /// for `within`).
    #[allow(dead_code)]
    fn handled_parts_are_reviewed(
        handled: &una_syntax::ast::Handled,
        expr: &una_syntax::ast::ExprKind,
    ) -> usize {
        let una_syntax::ast::Handled {
            expr: _,
            with,
            within,
            handlings,
            span: _,
        } = handled;
        let attempt = match expr {
            una_syntax::ast::ExprKind::Attempt { operand: _, within } => {
                usize::from(within.is_some())
            }
            _ => 0,
        };
        with.len() + usize::from(within.is_some()) + handlings.len() + attempt
    }
}
