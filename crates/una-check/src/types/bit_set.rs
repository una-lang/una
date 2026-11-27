//! The built-in methods of the `BitSet` of `std.collections` (`spec/28 §9`, ADR 0119 rule 14),
//! with the labels of IF2 and IF5, modeled on those of sets (`set.rs`). A bit set has no type
//! argument: its items are `Int`, and its members and its size are its shape, which its top
//! label covers.

use una_core::{BitSetOp, Coll, Label};
use una_diag::{Code, ObligationKind};
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{Arg, Convention, Expr};

use super::check::{Checker, Role};
use super::records::Res;
use super::ty::{Shared, Ty};

/// The built-in methods of bit sets (`spec/28 §9`), in the order of its table: each name, the
/// operation, whether it updates the receiver in place (`edit b.add(...)`), and the labels of
/// its arguments. `bit_set_method` resolves exactly these; `super::builtin_methods` lists them
/// for the standard library reference.
pub(crate) const BIT_SET_METHODS: [(&str, BitSetOp, bool, &[&str]); 12] = [
    ("has", BitSetOp::Has, false, &["item"]),
    ("add", BitSetOp::With, true, &["item"]),
    ("remove", BitSetOp::Without, true, &["item"]),
    ("with", BitSetOp::With, false, &["item"]),
    ("without", BitSetOp::Without, false, &["item"]),
    ("union", BitSetOp::Union, false, &["other"]),
    ("intersection", BitSetOp::Intersection, false, &["other"]),
    ("difference", BitSetOp::Difference, false, &["other"]),
    ("is_subset", BitSetOp::IsSubset, false, &["of"]),
    ("first", BitSetOp::First, false, &[]),
    ("last", BitSetOp::Last, false, &[]),
    ("items", BitSetOp::Items, false, &[]),
];

/// `BitSet`.
fn bit_set() -> Ty {
    Ty::Coll(Coll::BitSet, Vec::new().into())
}

impl Checker<'_, '_, '_> {
    /// `b.size` when `b` is a `BitSet` in edition 0.5 (`spec/28 §9`): an `Int` with the bit set's
    /// top label, since the size is part of its shape (ADR 0119 IF2); `None` otherwise. In the
    /// refinement language (ADR 0119 V5).
    pub(crate) fn bit_set_size(&mut self, target_ty: &Ty, field: &str, span: Span) -> Option<Ty> {
        if field != "size" || self.edition() < Edition::V0_5 {
            return None;
        }
        let labeled = self.labeled(target_ty);
        let Ty::Coll(Coll::BitSet, _) = labeled.unlabeled() else {
            return None;
        };
        self.resolved(span, Res::BitSetMethod(BitSetOp::Size));
        Some(Ty::Int.with_label(&labeled.top_label()))
    }

    /// `b.op(args)` when `b` is a `BitSet` in edition 0.5 and `op` is one of its built-in methods
    /// (`spec/28 §9`); `None` otherwise, so the call resolves as before. `place` is the type of
    /// the receiver as a place, when it is written `edit b.add(...)` or `edit b.remove(...)`.
    /// Each method is checked in O(1) besides its arguments.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn bit_set_method(
        &mut self,
        target: &Expr,
        target_ty: &Ty,
        place: Option<&Ty>,
        marker: Option<Convention>,
        method: &str,
        args: &[Arg],
        span: Span,
    ) -> Option<Ty> {
        if self.edition() < Edition::V0_5 {
            return None;
        }
        let labeled = self.labeled(target_ty);
        let Ty::Coll(Coll::BitSet, _) = labeled.unlabeled() else {
            return None;
        };
        let (op, update, params) = BIT_SET_METHODS
            .iter()
            .find(|(name, ..)| *name == method)
            .map(|(_, op, update, params)| (*op, *update, *params))?;
        // `b.has(item: i)` is in the refinement language, as `b.len` and `b.size` are (ADR 0119
        // V5); every other method of a bit set is outside it.
        if op != BitSetOp::Has {
            self.outside_rl(span, || format!("the method `{method}`"));
        }
        let receiver = self.report.text(target.span).to_owned();
        // `add` and `remove` update the bit set in place, as an `edit` receiver (ADR 0119 rule 1,
        // `spec/17 §3`); the other methods read it.
        match (update, marker) {
            (true, Some(Convention::Edit)) | (false, None) => {}
            (true, _) => self.report.error(
                Code::U2020,
                target.span,
                format!(
                    "`{method}` updates the bit set, so the call is written `edit {receiver}.{method}(...)`"
                ),
            ),
            (false, Some(marker)) => self.report.error(
                Code::U2020,
                target.span,
                format!(
                    "`{method}` reads the bit set and does not take it by `{}`",
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
                    "`{method}` is called with `edit`, so the call must be the whole expression of a statement; bind other values with `let` first"
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
            let expected = match *param {
                "other" | "of" => bit_set(),
                _ => Ty::Int,
            };
            let ty = self.expr_as(
                &arg.value,
                Some(&expected),
                Role::Argument {
                    param,
                    callee: &callee,
                },
            );
            found.push((arg.value.span, ty));
        }
        // ADR 0119 rule 14: `add` and `with` require their item in the range, and the set
        // operations an operand of the receiver's size, each a `precondition` obligation at the
        // argument, which tier 1 generates (`una-verify`).
        if (op == BitSetOp::With || op.pairs())
            && let Some((at, _)) = found.first()
        {
            self.obligation(ObligationKind::Precondition, *at, None);
        }
        // The members and the size are the shape: the top label covers both (ADR 0119 IF2).
        let top = labeled.top_label();
        // The labels of the item argument of `has`, `add`, `remove`, `with` and `without` (IF2).
        let item_label = match op {
            BitSetOp::Has | BitSetOp::With | BitSetOp::Without => found
                .iter()
                .fold(Label::public(), |label, (_, ty)| label.join(&self.deep(ty))),
            _ => Label::public(),
        };
        // The other bit set of a set operation: its top label (IF2, IF5).
        let other_top = if op.pairs() {
            found
                .first()
                .map_or_else(Label::public, |(_, ty)| self.labeled(ty).top_label())
        } else {
            Label::public()
        };
        let result = match op {
            BitSetOp::Has => Ty::Bool.with_label(&top.join(&item_label)),
            // The binding's labels for an update; a value form raises them by the item's below.
            BitSetOp::With | BitSetOp::Without => bit_set().with_label(&top),
            BitSetOp::First | BitSetOp::Last => self
                .prelude_kind_of("Option", vec![Ty::Int])
                .with_label(&top),
            BitSetOp::Items => Ty::List(Shared::new(Ty::Int)).with_label(&top),
            BitSetOp::Union | BitSetOp::Intersection | BitSetOp::Difference => {
                bit_set().with_label(&top.join(&other_top))
            }
            BitSetOp::IsSubset => Ty::Bool.with_label(&top.join(&other_top)),
            // `size` is a property, resolved by `bit_set_size`, never a method.
            BitSetOp::Size => Ty::Int.with_label(&top),
        };
        if !update {
            self.resolved(span, Res::BitSetMethod(op));
            // `with` and `without` raise the top label by the item's labels (IF2).
            return Some(match op {
                BitSetOp::With | BitSetOp::Without => result.with_label(&item_label),
                _ => result,
            });
        }
        self.resolved(span, Res::BitSetUpdate(op));
        if let Some(place_ty) = place {
            // Elaboration rebinds the place to the new bit set, of the place's type.
            self.typed(target.span, place_ty);
            // An editable binding keeps its declared labels (`spec/17 §6` rule 1): an item
            // above the bit set's top label is U4001 at the item (IF2, as `spec/21 §3` rule 4).
            let place_top = self.labeled(place_ty).top_label();
            // Label premises only where labels are checked: not in a `via` body (`spec/07 §6`
            // rule 6, #384).
            if let Some((item_span, _)) = found.first()
                && self.labels()
                && !item_label.flows_to(&place_top)
            {
                self.flow_error(
                    Code::U4001,
                    *item_span,
                    format!(
                        "this item is labeled `{item_label}`, above the label `{place_top}` of the bit set `{receiver}`, whose size and members would reveal it; declare the bit set with a `{item_label}` top label"
                    ),
                    super::flow::Carrier::Value("this item"),
                    &item_label,
                    &place_top,
                );
            }
            let span = found.first().map_or(span, |(item_span, _)| *item_span);
            self.rebind(target, place_ty, &result, span);
        }
        Some(Ty::Unit)
    }
}
