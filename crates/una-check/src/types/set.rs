//! The built-in methods of the `Set` of `std.collections` (`spec/28 §6`, ADR 0119 rule 7), with
//! the labels of IF2 and IF5, modeled on those of maps (`map.rs`).

use una_core::{Coll, Label, SetOp};
use una_diag::Code;
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{Arg, Convention, Expr};

use super::check::{Checker, Role};
use super::records::Res;
use super::ty::{Shared, Ty};

/// The built-in methods of sets (`spec/28 §6`), in the order of its table: each name, the
/// operation, whether it updates the receiver in place (`edit s.add(...)`), and the labels of
/// its arguments. `set_method` resolves exactly these; `super::builtin_methods` lists them for
/// the standard library reference.
pub(crate) const SET_METHODS: [(&str, SetOp, bool, &[&str]); 13] = [
    ("has", SetOp::Has, false, &["item"]),
    ("add", SetOp::With, true, &["item"]),
    ("remove", SetOp::Without, true, &["item"]),
    ("with", SetOp::With, false, &["item"]),
    ("without", SetOp::Without, false, &["item"]),
    ("first", SetOp::First, false, &[]),
    ("last", SetOp::Last, false, &[]),
    ("between", SetOp::Between, false, &["low", "high"]),
    ("items", SetOp::Items, false, &[]),
    ("union", SetOp::Union, false, &["other"]),
    ("intersection", SetOp::Intersection, false, &["other"]),
    ("difference", SetOp::Difference, false, &["other"]),
    ("is_subset", SetOp::IsSubset, false, &["of"]),
];

/// `Set[item]`.
fn set_of(item: Ty) -> Ty {
    Ty::Coll(Coll::Set, vec![item].into())
}

impl Checker<'_, '_, '_> {
    /// `s.op(args)` when `s` is a `Set` in edition 0.5 and `op` is one of its built-in methods
    /// (`spec/28 §6`); `None` otherwise, so the call resolves as before. `place` is the type of
    /// the receiver as a place, when it is written `edit s.add(...)` or `edit s.remove(...)`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_method(
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
        let Ty::Coll(Coll::Set, set_args) = labeled.unlabeled().clone() else {
            return None;
        };
        let (op, update, params) = SET_METHODS
            .iter()
            .find(|(name, ..)| *name == method)
            .map(|(_, op, update, params)| (*op, *update, *params))?;
        let item = set_args.first().cloned().unwrap_or_else(Ty::error);
        // `s.has(item: x)` is in the refinement language, as `s.len` is (ADR 0119 V5); every
        // other method of a set is outside it.
        if op != SetOp::Has {
            self.outside_rl(span, || format!("the method `{method}`"));
        }
        let receiver = self.report.text(target.span).to_owned();
        // `add` and `remove` update the set in place, as an `edit` receiver (ADR 0119 rule 1,
        // `spec/17 §3`); the other methods read it.
        match (update, marker) {
            (true, Some(Convention::Edit)) | (false, None) => {}
            (true, _) => self.report.error(
                Code::U2020,
                target.span,
                format!(
                    "`{method}` updates the set, so the call is written `edit {receiver}.{method}(...)`"
                ),
            ),
            (false, Some(marker)) => self.report.error(
                Code::U2020,
                target.span,
                format!(
                    "`{method}` reads the set and does not take it by `{}`",
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
            // ADR 0103 rules 3 and 5: `add`, `with` and `union` store their items at `T`, a
            // flow with its `refinement` obligations; a lookup stores nothing, so it takes its
            // argument at the base of `T`.
            let stores = matches!(op, SetOp::With | SetOp::Union);
            let at = if stores {
                item.clone()
            } else {
                item.without_refinements()
            };
            let expected = match *param {
                "other" | "of" => set_of(at),
                _ => at,
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
        let kinds = self.env.kind_labels.clone();
        // The items are the shape: every label of the item position is below the top label
        // (ADR 0119 IF2).
        let top = labeled.top_label().join(&item.every_label(&kinds));
        // The labels of the arguments that are items: `has`, `add`, `remove`, `with`,
        // `without` and `between` (IF2).
        let item_label = match op {
            SetOp::Has | SetOp::With | SetOp::Without | SetOp::Between => found
                .iter()
                .fold(Label::public(), |label, (_, ty)| label.join(&self.deep(ty))),
            _ => Label::public(),
        };
        // The other set of a set operation: its item type and its top label with its shape's
        // (IF2, IF5).
        let other = found.first().and_then(|(_, ty)| {
            let other = self.labeled(ty);
            match other.unlabeled() {
                Ty::Coll(Coll::Set, args) => {
                    let other_item = args.first().cloned().unwrap_or_else(Ty::error);
                    let other_top = other.top_label().join(&other_item.every_label(&kinds));
                    Some((other_item, other_top))
                }
                _ => None,
            }
        });
        let other_top = match op {
            SetOp::Union | SetOp::Intersection | SetOp::Difference | SetOp::IsSubset => other
                .as_ref()
                .map_or_else(Label::public, |(_, top)| top.clone()),
            _ => Label::public(),
        };
        let result = match op {
            SetOp::Has => Ty::Bool.with_label(&top.join(&item_label)),
            // The binding's labels for an update; a value form raises them by the item's
            // below.
            SetOp::With | SetOp::Without => set_of(item.clone()).with_label(&labeled.top_label()),
            SetOp::First | SetOp::Last => self
                .prelude_kind_of("Option", vec![item.clone()])
                .with_label(&top),
            SetOp::Between => {
                Ty::List(Shared::new(item.clone())).with_label(&top.join(&item_label))
            }
            SetOp::Items => Ty::List(Shared::new(item.clone())).with_label(&top),
            // The union's items come from both sets: its item type is the join of theirs.
            SetOp::Union => {
                let joined = match &other {
                    Some((other_item, _)) => {
                        let other_item = self.labeled(other_item);
                        self.vars.join(&item, &other_item)
                    }
                    None => item.clone(),
                };
                set_of(joined).with_label(&labeled.top_label().join(&other_top))
            }
            SetOp::Intersection | SetOp::Difference => {
                set_of(item.clone()).with_label(&labeled.top_label().join(&other_top))
            }
            SetOp::IsSubset => Ty::Bool.with_label(&top.join(&other_top)),
        };
        if !update {
            self.resolved(span, Res::SetMethod(op));
            // `with` and `without` raise the top label by the item's labels (IF2).
            return Some(match op {
                SetOp::With | SetOp::Without => result.with_label(&item_label),
                _ => result,
            });
        }
        self.resolved(span, Res::SetUpdate(op));
        if let Some(place_ty) = place {
            // Elaboration rebinds the place to the new set, of the place's type.
            self.typed(target.span, place_ty);
            // An editable binding keeps its declared labels (`spec/17 §6` rule 1): an item
            // above the set's top label is U4001 at the item (IF2, as `spec/21 §3` rule 4).
            let place = self.labeled(place_ty);
            let place_top = match place.unlabeled() {
                Ty::Coll(Coll::Set, place_args) => {
                    place_args.iter().fold(place.top_label(), |label, arg| {
                        label.join(&arg.every_label(&kinds))
                    })
                }
                _ => place.top_label(),
            };
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
                        "this item is labeled `{item_label}`, above the label `{place_top}` of the set `{receiver}`, whose size and items would reveal it; declare the set with a `{item_label}` top label"
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
