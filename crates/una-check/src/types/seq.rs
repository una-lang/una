//! The built-in methods of `Stack` and `Queue` (`spec/28 §5`, ADR 0119 rules 5 and 6), with
//! the labels of ADR 0119 IF1: they are labeled as a list is.

use una_core::{Coll, SeqOp};
use una_diag::Code;
use una_syntax::Span;
use una_syntax::ast::{Arg, Convention, Expr};

use super::check::{Checker, Role};
use super::records::Res;
use super::ty::{Shared, Ty};

/// A built-in method of a sequence: its name, the operation, whether it updates the receiver in
/// place (`edit q.push(...)`), and the labels of its arguments.
pub(crate) type SeqMethod = (&'static str, SeqOp, bool, &'static [&'static str]);

/// The built-in methods of `Stack[T]`, in the order of its table (`spec/28 §5` rule 1). The
/// top is the front of the sequence, so `push` adds at the front and `pop` removes it.
/// `seq_method` resolves exactly these; `super::builtin_methods` lists them for the standard
/// library reference (#690).
pub(crate) const STACK_METHODS: [SeqMethod; 4] = [
    ("push", SeqOp::PushFront, true, &["item"]),
    ("pop", SeqOp::PopFront, true, &[]),
    ("peek", SeqOp::PeekFront, false, &[]),
    ("items", SeqOp::Items, false, &[]),
];

/// The built-in methods of `Queue[T]`, in the order of its table (`spec/28 §5` rule 2): `push`
/// adds at the back, and `pop` removes the front.
pub(crate) const QUEUE_METHODS: [SeqMethod; 7] = [
    ("push", SeqOp::PushBack, true, &["item"]),
    ("pop", SeqOp::PopFront, true, &[]),
    ("peek", SeqOp::PeekFront, false, &[]),
    ("push_front", SeqOp::PushFront, true, &["item"]),
    ("pop_back", SeqOp::PopBack, true, &[]),
    ("peek_back", SeqOp::PeekBack, false, &[]),
    ("items", SeqOp::Items, false, &[]),
];

impl Checker<'_, '_, '_> {
    /// `q.op(args)` when `q` is a `Stack` or a `Queue` and `op` is one of its built-in methods
    /// (`spec/28 §5`); `None` otherwise, so the call resolves as before. `place` is the type of
    /// the receiver as a place, when it is written `edit q.push(...)`. Each method is checked in
    /// O(1) besides its arguments.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn seq_method(
        &mut self,
        target: &Expr,
        target_ty: &Ty,
        place: Option<&Ty>,
        marker: Option<Convention>,
        method: &str,
        args: &[Arg],
        span: Span,
    ) -> Option<Ty> {
        let labeled = self.labeled(target_ty);
        let Ty::Coll(coll @ (Coll::Stack | Coll::Queue), elements) = labeled.unlabeled().clone()
        else {
            return None;
        };
        let table: &[SeqMethod] = match coll {
            Coll::Stack => &STACK_METHODS,
            _ => &QUEUE_METHODS,
        };
        let (op, update, params) = table
            .iter()
            .find(|(name, ..)| *name == method)
            .map(|(_, op, update, params)| (*op, *update, *params))?;
        let element = elements.first().cloned().unwrap_or_else(Ty::error);
        self.outside_rl(span, || format!("the method `{method}`"));
        let receiver = self.report.text(target.span).to_owned();
        let what = match coll {
            Coll::Stack => "stack",
            _ => "queue",
        };
        // `push`, `pop`, `push_front` and `pop_back` update the sequence in place, as an `edit`
        // receiver (ADR 0119 rule 1, `spec/17 §3`); the other methods read it.
        match (update, marker) {
            (true, Some(Convention::Edit)) | (false, None) => {}
            (true, _) => self.report.error(
                Code::U2020,
                target.span,
                format!(
                    "`{method}` updates the {what}, so the call is written `edit {receiver}.{method}(...)`"
                ),
            ),
            (false, Some(marker)) => self.report.error(
                Code::U2020,
                target.span,
                format!(
                    "`{method}` reads the {what} and does not take it by `{}`",
                    marker.as_str()
                ),
            ),
        }
        if update
            && !self.statement_calls.contains(&span)
            && self.pass == super::authority::Pass::Check
        {
            self.report.error(
                Code::U3013,
                span,
                format!(
                    "`{method}` is called with `edit`, so the call must be the whole expression of a statement or `let`; bind other values with `let` first"
                ),
            );
        }
        if args.len() != params.len() {
            let shape = params
                .iter()
                .map(|param| format!("{param}: {}", &param[..1]))
                .collect::<Vec<_>>()
                .join(", ");
            let takes = match params.len() {
                0 => "takes no arguments".to_owned(),
                1 => "takes one argument".to_owned(),
                n => format!("takes {n} arguments"),
            };
            self.report.error(
                Code::U2001,
                span,
                format!("`{method}` {takes}, as in `{receiver}.{method}({shape})`"),
            );
            self.unchecked_args(args);
            return Some(Ty::error());
        }
        let callee = format!("`{method}`");
        let mut found = Vec::new();
        for (arg, param) in args.iter().zip(params) {
            self.method_label(method, param, arg);
            // ADR 0119 rule 1 and ADR 0103 rule 3: a push stores its item at the element
            // type, refinements included, a flow site the verifier checks.
            let ty = self.expr_as(
                &arg.value,
                Some(&element),
                Role::Argument {
                    param,
                    callee: &callee,
                },
            );
            found.push((arg.value.span, ty));
        }
        // IF1: the top label covers the length and the positions; what a read gives back
        // carries it, and its elements keep the element position's labels.
        let top = labeled.top_label();
        let result = match op {
            SeqOp::PeekFront | SeqOp::PeekBack | SeqOp::PopFront | SeqOp::PopBack => self
                .prelude_kind_of("Option", vec![element.clone()])
                .with_label(&top),
            SeqOp::Items => Ty::List(Shared::new(element.clone())).with_label(&top),
            SeqOp::PushFront | SeqOp::PushBack => Ty::Unit,
        };
        if !update {
            self.resolved(span, Res::SeqMethod(op));
            return Some(result);
        }
        self.resolved(span, Res::SeqUpdate(op));
        if let Some(place_ty) = place {
            // Elaboration rebinds the place to the new sequence, of the place's type.
            self.typed(target.span, place_ty);
            // The new sequence: a push's element position takes the item's labels, so an item
            // above the binding's element labels is U4001 at the item; a removal keeps the
            // place's type. Either way the update runs under the program counter, which stays
            // below the binding's top label (`spec/17 §6` rule 1, IF1).
            let new = match found.first() {
                Some((_, item)) => {
                    let item = self.labeled(item);
                    let element = self.vars.join(&element, &item);
                    Ty::Coll(coll, vec![element].into()).with_label(&top)
                }
                None => place_ty.clone(),
            };
            let at = found.first().map_or(span, |(item_span, _)| *item_span);
            self.rebind(target, place_ty, &new, at);
        }
        Some(result)
    }
}
