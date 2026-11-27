//! The built-in methods of the `PriorityQueue` of `std.collections` (`spec/28 §8`, ADR 0119
//! rule 13), with the labels of IF4 and IF5: the priorities are the queue's shape, as a set's
//! items are (`set.rs`), and the items are labeled as a sequence's (`seq.rs`).

use una_core::{Coll, PriorityOp};
use una_diag::Code;
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{Arg, Convention, Expr};

use super::check::{Checker, Role};
use super::env::Entity;
use super::records::Res;
use super::ty::{Shared, Ty};

/// A built-in method of a priority queue: its name, the operation, whether it updates the
/// receiver in place (`edit q.push(...)`), the labels of its arguments, and for a removal the
/// read that gives the call's value, read before the removal.
pub(crate) type PriorityMethod = (
    &'static str,
    PriorityOp,
    bool,
    &'static [&'static str],
    Option<PriorityOp>,
);

/// The built-in methods of `PriorityQueue[T, P]`, in the order of its table (`spec/28 §8`).
/// `priority_method` resolves exactly these; `super::builtin_methods` lists them for the
/// standard library reference (#690).
pub(crate) const PRIORITY_METHODS: [PriorityMethod; 6] = [
    ("push", PriorityOp::Push, true, &["item", "priority"], None),
    ("pop", PriorityOp::Pop, true, &[], Some(PriorityOp::Peek)),
    (
        "pop_ranked",
        PriorityOp::Pop,
        true,
        &[],
        Some(PriorityOp::PeekRanked),
    ),
    ("peek", PriorityOp::Peek, false, &[], None),
    ("peek_priority", PriorityOp::PeekPriority, false, &[], None),
    ("items", PriorityOp::Items, false, &[], None),
];

impl Checker<'_, '_, '_> {
    /// `q.op(args)` when `q` is a `PriorityQueue` in edition 0.5 and `op` is one of its
    /// built-in methods (`spec/28 §8`); `None` otherwise, so the call resolves as before.
    /// `place` is the type of the receiver as a place, when it is written `edit q.push(...)`.
    /// Each method is checked in O(1) besides its arguments.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn priority_method(
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
        let Ty::Coll(Coll::PriorityQueue, queue_args) = labeled.unlabeled().clone() else {
            return None;
        };
        let (op, update, params, read) = PRIORITY_METHODS
            .iter()
            .find(|(name, ..)| *name == method)
            .map(|(_, op, update, params, read)| (*op, *update, *params, *read))?;
        let item = queue_args.first().cloned().unwrap_or_else(Ty::error);
        let priority = queue_args.get(1).cloned().unwrap_or_else(Ty::error);
        // The refinement language has `.len` of a collection (ADR 0119 V5), and none of a
        // priority queue's methods: each is outside it.
        self.outside_rl(span, || format!("the method `{method}`"));
        let receiver = self.report.text(target.span).to_owned();
        // `push`, `pop` and `pop_ranked` update the queue in place, as an `edit` receiver (ADR
        // 0119 rule 1, `spec/17 §3`); the other methods read it.
        match (update, marker) {
            (true, Some(Convention::Edit)) | (false, None) => {}
            (true, _) => self.report.error(
                Code::U2020,
                target.span,
                format!(
                    "`{method}` updates the priority queue, so the call is written `edit {receiver}.{method}(...)`"
                ),
            ),
            (false, Some(marker)) => self.report.error(
                Code::U2020,
                target.span,
                format!(
                    "`{method}` reads the priority queue and does not take it by `{}`",
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
                .map(|param| format!("{param}: {}", param.get(..1).unwrap_or_default()))
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
            // ADR 0119 rule 1 and ADR 0103 rule 3: a push stores its item at `T` and its
            // priority at `P`, refinements included, each a flow site the verifier checks.
            let expected = match *param {
                "priority" => priority.clone(),
                _ => item.clone(),
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
        // The priorities decide which item comes out: every label of the priority position is
        // below the top label (ADR 0119 IF4), and every read carries it.
        let top = labeled.top_label().join(&priority.every_label(&kinds));
        let option = |checker: &Self, payload: Ty| {
            checker
                .prelude_kind_of("Option", vec![payload])
                .with_label(&top)
        };
        let result = match (op, read) {
            (PriorityOp::Push, _) => Ty::Unit,
            (PriorityOp::Peek, _) | (PriorityOp::Pop, Some(PriorityOp::Peek)) => {
                option(self, item.clone())
            }
            (PriorityOp::PeekPriority, _) => option(self, priority.clone()),
            (PriorityOp::PeekRanked, _) | (PriorityOp::Pop, _) => {
                let ranked = self.ranked_of(item.clone(), priority.clone());
                option(self, ranked)
            }
            (PriorityOp::Items, _) => Ty::List(Shared::new(item.clone())).with_label(&top),
        };
        if !update {
            self.resolved(span, Res::PriorityMethod(op));
            return Some(result);
        }
        self.resolved(span, Res::PriorityUpdate { op, read });
        if let Some(place_ty) = place {
            // Elaboration rebinds the place to the new queue, of the place's type.
            self.typed(target.span, place_ty);
            let place = self.labeled(place_ty);
            let (place_item, place_priority) = match place.unlabeled() {
                Ty::Coll(Coll::PriorityQueue, place_args) => (
                    place_args.first().cloned().unwrap_or_else(Ty::error),
                    place_args.get(1).cloned().unwrap_or_else(Ty::error),
                ),
                _ => (item.clone(), priority.clone()),
            };
            // An editable binding keeps its declared labels (`spec/17 §6` rule 1): a priority
            // above the queue's top label, with the labels of its priority position, is U4001
            // at the priority, since the order of the pops would reveal it (IF4). Label
            // premises only where labels are checked: not in a `via` body (`spec/07 §6` rule 6,
            // #384).
            let place_top = place.top_label().join(&place_priority.every_label(&kinds));
            if let Some((priority_span, priority_ty)) = found.get(1)
                && self.labels()
            {
                let priority_label = self.deep(priority_ty);
                if !priority_label.flows_to(&place_top) {
                    self.flow_error(
                        Code::U4001,
                        *priority_span,
                        format!(
                            "this priority is labeled `{priority_label}`, above the label `{place_top}` of the priority queue `{receiver}`, whose order of pops would reveal it; declare the priority type or the queue with a `{priority_label}` label"
                        ),
                        super::flow::Carrier::Value("this priority"),
                        &priority_label,
                        &place_top,
                    );
                }
            }
            // The new queue: a push's item position takes the item's labels, so an item above
            // the binding's item labels is U4001 at the item; its priority position stays the
            // place's, checked above. A removal keeps the place's type. Either way the update
            // runs under the program counter, which stays below the binding's top label
            // (`spec/17 §6` rule 1, IF1).
            let new = match found.first() {
                Some((_, pushed)) => {
                    let pushed = self.labeled(pushed);
                    let joined = self.vars.join(&place_item, &pushed);
                    Ty::Coll(Coll::PriorityQueue, vec![joined, place_priority].into())
                        .with_label(&labeled.top_label())
                }
                None => place_ty.clone(),
            };
            let at = found.first().map_or(span, |(item_span, _)| *item_span);
            self.rebind(target, place_ty, &new, at);
        }
        Some(result)
    }

    /// `Ranked[item, priority]`, the kind of `std.collections` that `pop_ranked` gives (ADR
    /// 0119 rule 13); `Error` when the module is not loaded, which no queue type can be without.
    fn ranked_of(&self, item: Ty, priority: Ty) -> Ty {
        let id = self
            .env
            .modules
            .iter()
            .find(|module| module.path == una_core::COLLECTIONS_MODULE)
            .and_then(|module| match module.upper.get("Ranked") {
                Some(Entity::Kind(id)) => Some(*id),
                _ => None,
            });
        match id {
            Some(id) => Ty::Kind(id, vec![item, priority].into()),
            None => Ty::error(),
        }
    }
}
