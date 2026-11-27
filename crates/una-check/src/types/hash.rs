//! The built-in methods of the `HashMap` and the `HashSet` of `std.collections` (`spec/28 §7`,
//! ADR 0119 rules 8 to 12), with the labels of IF2, IF3, IF5 and IF8, modeled on those of maps
//! (`map.rs`) and of sets (`set.rs`). `update` on a hash map is a hidden function of
//! `std.collections`, reached by method syntax as a map's is (`call.rs`).

use una_core::{Coll, HashMapOp, HashSetOp, Label};
use una_diag::Code;
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{Arg, Convention, Expr};

use super::check::{Checker, Role};
use super::env::Constraint;
use super::records::Res;
use super::ty::{Shared, Ty};

/// The built-in methods of hash maps (`spec/28 §7`), in the order of its table: each name, the
/// operation, whether it updates the receiver in place (`edit m.set(...)`), and the labels of
/// its arguments. `hash_map_method` resolves exactly these, and `update` resolves to the hidden
/// function of `std.collections`; `super::builtin_methods` lists them for the standard library
/// reference.
pub(crate) const HASH_MAP_METHODS: [(&str, HashMapOp, bool, &[&str]); 10] = [
    ("has", HashMapOp::Has, false, &["key"]),
    ("get", HashMapOp::Get, false, &["key"]),
    ("set", HashMapOp::With, true, &["key", "value"]),
    ("remove", HashMapOp::Without, true, &["key"]),
    ("with", HashMapOp::With, false, &["key", "value"]),
    ("without", HashMapOp::Without, false, &["key"]),
    ("keys", HashMapOp::Keys, false, &[]),
    ("values", HashMapOp::Values, false, &[]),
    ("entries", HashMapOp::Entries, false, &[]),
    ("same_entries", HashMapOp::SameEntries, false, &["as"]),
];

/// The built-in methods of hash sets (`spec/28 §7`), in the order of its table, as
/// [`HASH_MAP_METHODS`] lists those of hash maps.
pub(crate) const HASH_SET_METHODS: [(&str, HashSetOp, bool, &[&str]); 11] = [
    ("has", HashSetOp::Has, false, &["item"]),
    ("add", HashSetOp::With, true, &["item"]),
    ("remove", HashSetOp::Without, true, &["item"]),
    ("with", HashSetOp::With, false, &["item"]),
    ("without", HashSetOp::Without, false, &["item"]),
    ("items", HashSetOp::Items, false, &[]),
    ("union", HashSetOp::Union, false, &["other"]),
    ("intersection", HashSetOp::Intersection, false, &["other"]),
    ("difference", HashSetOp::Difference, false, &["other"]),
    ("is_subset", HashSetOp::IsSubset, false, &["of"]),
    ("same_items", HashSetOp::SameItems, false, &["as"]),
];

/// `HashMap[key, value]`.
fn hash_map_of(key: Ty, value: Ty) -> Ty {
    Ty::Coll(Coll::HashMap, vec![key, value].into())
}

/// `HashSet[item]`.
fn hash_set_of(item: Ty) -> Ty {
    Ty::Coll(Coll::HashSet, vec![item].into())
}

impl Checker<'_, '_, '_> {
    /// The checks every method of a hash table shares: `edit` exactly on an update (U2020), an
    /// update as the whole expression of its statement (U3013), and the number of arguments
    /// (U2001). `false` when the number is wrong, after which the arguments are checked alone.
    #[allow(clippy::too_many_arguments)]
    fn table_call_shape(
        &mut self,
        what: &str,
        target: &Expr,
        marker: Option<Convention>,
        method: &str,
        update: bool,
        params: &[&str],
        args: &[Arg],
        span: Span,
    ) -> bool {
        let receiver = self.report.text(target.span).to_owned();
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
                    "`{method}` is called with `edit`, so the call must be the whole expression of a statement; bind other values with `let` first"
                ),
            );
        }
        if args.len() == params.len() {
            return true;
        }
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
        false
    }

    /// U2025 at `span` when `value`, the value type of a hash map, does not satisfy `Eq`, which
    /// `same_entries` needs to compare the values (ADR 0119 rule 11); a type parameter's need
    /// joins the function's constraints (`spec/20 §3`).
    fn same_entries_values(&mut self, value: &Ty, span: Span) {
        let resolved = self.resolve(value);
        if self.facts.has_function(&resolved) || self.facts.unequal(&resolved).is_some() {
            let shown = self.show(&resolved);
            self.report.error(
                Code::U2025,
                span,
                format!(
                    "`same_entries` compares the values with `==`, so a hash map's value type must satisfy `Eq`, and `{shown}` does not"
                ),
            );
        }
        self.needs.push((value.clone(), Constraint::Eq, span));
    }

    /// `m.op(args)` when `m` is a `HashMap` in edition 0.5 and `op` is one of its built-in
    /// methods (`spec/28 §7`); `None` otherwise, so the call resolves as before. `place` is the
    /// type of the receiver as a place, when it is written `edit m.set(...)` or
    /// `edit m.remove(...)`. O(1) besides its arguments.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn hash_map_method(
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
        let Ty::Coll(Coll::HashMap, kv) = labeled.unlabeled().clone() else {
            return None;
        };
        let (op, update, params) = HASH_MAP_METHODS
            .iter()
            .find(|(name, ..)| *name == method)
            .map(|(_, op, update, params)| (*op, *update, *params))?;
        let key = kv.first().cloned().unwrap_or_else(Ty::error);
        let value = kv.get(1).cloned().unwrap_or_else(Ty::error);
        // `m.has(key: k)` is in the refinement language, as `m.len` is (ADR 0119 V5); every
        // other method of a hash map is outside it.
        if op != HashMapOp::Has {
            self.outside_rl(span, || format!("the method `{method}`"));
        }
        let receiver = self.report.text(target.span).to_owned();
        if !self.table_call_shape(
            "hash map", target, marker, method, update, params, args, span,
        ) {
            return Some(Ty::error());
        }
        let callee = format!("`{method}`");
        let mut found = Vec::new();
        for (arg, param) in args.iter().zip(params) {
            self.method_label(method, param, arg);
            // ADR 0103 rules 3 and 5: `set` and `with` store the key and the value at `K` and
            // `V`; a lookup stores nothing, so it takes its key at the base of `K`, and the
            // other map of `same_entries` is only read.
            let expected = match (*param, op) {
                ("key", HashMapOp::With) => key.clone(),
                ("key", _) => key.without_refinements(),
                ("as", _) => hash_map_of(key.without_refinements(), value.without_refinements()),
                _ => value.clone(),
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
        // The keys are the shape: every label of the key position, the labels a kind's fields
        // declare included, is below the top label (ADR 0119 IF3).
        let top = labeled.top_label().join(&key.every_label(&kinds));
        let key_label = match op {
            HashMapOp::Has | HashMapOp::Get | HashMapOp::With | HashMapOp::Without => found
                .first()
                .map_or_else(Label::public, |(_, ty)| self.deep(ty)),
            _ => Label::public(),
        };
        let result = match op {
            HashMapOp::Has => Ty::Bool.with_label(&top.join(&key_label)),
            HashMapOp::Get => self
                .prelude_kind_of("Option", vec![value.clone()])
                .with_label(&top.join(&key_label)),
            HashMapOp::With | HashMapOp::Without => {
                // The value position takes the stored value's labels, as a map's does.
                let value = match found.get(1) {
                    Some((_, found)) => {
                        let found = self.labeled(found);
                        self.vars.join(&value, &found)
                    }
                    None => value.clone(),
                };
                hash_map_of(key.clone(), value).with_label(&labeled.top_label())
            }
            HashMapOp::Keys => Ty::List(Shared::new(key.clone())).with_label(&top),
            HashMapOp::Values => Ty::List(Shared::new(value.clone())).with_label(&top),
            HashMapOp::Entries => Ty::List(Shared::new(
                self.prelude_kind_of("Entry", vec![key.clone(), value.clone()]),
            ))
            .with_label(&top),
            // Every key and value of both maps is read (ADR 0119 IF8).
            HashMapOp::SameEntries => {
                if let Some((at, _)) = found.first() {
                    self.same_entries_values(&value, *at);
                }
                let other = found
                    .first()
                    .map_or_else(Label::public, |(_, ty)| self.deep(ty));
                Ty::Bool.with_label(&self.deep(target_ty).join(&other))
            }
        };
        if !update {
            self.resolved(span, Res::HashMapMethod(op));
            // `with` and `without` raise the top label by the key's labels (IF3).
            return Some(match op {
                HashMapOp::With | HashMapOp::Without => result.with_label(&key_label),
                _ => result,
            });
        }
        self.resolved(span, Res::HashMapUpdate(op));
        if let Some(place_ty) = place {
            // Elaboration rebinds the place to the new map, of the place's type.
            self.typed(target.span, place_ty);
            // An editable binding keeps its declared labels (`spec/17 §6` rule 1): a key above
            // the map's top label, with its key position's, is U4001 at the key (IF3).
            let place = self.labeled(place_ty);
            let place_top = match place.unlabeled() {
                Ty::Coll(Coll::HashMap, place_kv) => place_kv.first().map_or_else(
                    || place.top_label(),
                    |k| place.top_label().join(&k.every_label(&kinds)),
                ),
                _ => place.top_label(),
            };
            if let Some((key_span, _)) = found.first()
                && self.labels()
                && !key_label.flows_to(&place_top)
            {
                self.flow_error(
                    Code::U4001,
                    *key_span,
                    format!(
                        "this key is labeled `{key_label}`, above the label `{place_top}` of the hash map `{receiver}`, whose size and keys would reveal it; declare the map with a `{key_label}` top label"
                    ),
                    super::flow::Carrier::Value("this key"),
                    &key_label,
                    &place_top,
                );
            }
            let span = found.get(1).map_or(span, |(value_span, _)| *value_span);
            self.rebind(target, place_ty, &result, span);
        }
        Some(Ty::Unit)
    }

    /// `s.op(args)` when `s` is a `HashSet` in edition 0.5 and `op` is one of its built-in
    /// methods (`spec/28 §7`); `None` otherwise, so the call resolves as before. `place` is the
    /// type of the receiver as a place, when it is written `edit s.add(...)` or
    /// `edit s.remove(...)`. O(1) besides its arguments.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn hash_set_method(
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
        let Ty::Coll(Coll::HashSet, set_args) = labeled.unlabeled().clone() else {
            return None;
        };
        let (op, update, params) = HASH_SET_METHODS
            .iter()
            .find(|(name, ..)| *name == method)
            .map(|(_, op, update, params)| (*op, *update, *params))?;
        let item = set_args.first().cloned().unwrap_or_else(Ty::error);
        // `s.has(item: x)` is in the refinement language, as `s.len` is (ADR 0119 V5); every
        // other method of a hash set is outside it.
        if op != HashSetOp::Has {
            self.outside_rl(span, || format!("the method `{method}`"));
        }
        let receiver = self.report.text(target.span).to_owned();
        if !self.table_call_shape(
            "hash set", target, marker, method, update, params, args, span,
        ) {
            return Some(Ty::error());
        }
        let callee = format!("`{method}`");
        let mut found = Vec::new();
        for (arg, param) in args.iter().zip(params) {
            self.method_label(method, param, arg);
            // ADR 0103 rules 3 and 5: `add`, `with` and `union` store their items at `T`; a
            // lookup stores nothing, so it takes its argument at the base of `T`.
            let stores = matches!(op, HashSetOp::With | HashSetOp::Union);
            let at = if stores {
                item.clone()
            } else {
                item.without_refinements()
            };
            let expected = match *param {
                "other" | "of" | "as" => hash_set_of(at),
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
        // (ADR 0119 IF2), every field of a record item included, since the hash reads it.
        let top = labeled.top_label().join(&item.every_label(&kinds));
        let item_label = match op {
            HashSetOp::Has | HashSetOp::With | HashSetOp::Without => found
                .iter()
                .fold(Label::public(), |label, (_, ty)| label.join(&self.deep(ty))),
            _ => Label::public(),
        };
        // The other set of a set operation: its item type and its top label with its shape's
        // (IF2, IF5).
        let other = found.first().and_then(|(_, ty)| {
            let other = self.labeled(ty);
            match other.unlabeled() {
                Ty::Coll(Coll::HashSet, args) => {
                    let other_item = args.first().cloned().unwrap_or_else(Ty::error);
                    let other_top = other.top_label().join(&other_item.every_label(&kinds));
                    Some((other_item, other_top))
                }
                _ => None,
            }
        });
        let other_top = match op {
            HashSetOp::Union
            | HashSetOp::Intersection
            | HashSetOp::Difference
            | HashSetOp::IsSubset => other
                .as_ref()
                .map_or_else(Label::public, |(_, top)| top.clone()),
            _ => Label::public(),
        };
        let result = match op {
            HashSetOp::Has => Ty::Bool.with_label(&top.join(&item_label)),
            // The binding's labels for an update; a value form raises them by the item's
            // below.
            HashSetOp::With | HashSetOp::Without => {
                hash_set_of(item.clone()).with_label(&labeled.top_label())
            }
            HashSetOp::Items => Ty::List(Shared::new(item.clone())).with_label(&top),
            // The union's items come from both sets: its item type is the join of theirs.
            HashSetOp::Union => {
                let joined = match &other {
                    Some((other_item, _)) => {
                        let other_item = self.labeled(other_item);
                        self.vars.join(&item, &other_item)
                    }
                    None => item.clone(),
                };
                hash_set_of(joined).with_label(&labeled.top_label().join(&other_top))
            }
            HashSetOp::Intersection | HashSetOp::Difference => {
                hash_set_of(item.clone()).with_label(&labeled.top_label().join(&other_top))
            }
            HashSetOp::IsSubset => Ty::Bool.with_label(&top.join(&other_top)),
            // Every item of both sets is read (ADR 0119 IF8).
            HashSetOp::SameItems => {
                let other = found
                    .first()
                    .map_or_else(Label::public, |(_, ty)| self.deep(ty));
                Ty::Bool.with_label(&self.deep(target_ty).join(&other))
            }
        };
        if !update {
            self.resolved(span, Res::HashSetMethod(op));
            // `with` and `without` raise the top label by the item's labels (IF2).
            return Some(match op {
                HashSetOp::With | HashSetOp::Without => result.with_label(&item_label),
                _ => result,
            });
        }
        self.resolved(span, Res::HashSetUpdate(op));
        if let Some(place_ty) = place {
            // Elaboration rebinds the place to the new set, of the place's type.
            self.typed(target.span, place_ty);
            // An editable binding keeps its declared labels (`spec/17 §6` rule 1): an item
            // above the set's top label is U4001 at the item (IF2).
            let place = self.labeled(place_ty);
            let place_top = match place.unlabeled() {
                Ty::Coll(Coll::HashSet, place_args) => {
                    place_args.iter().fold(place.top_label(), |label, arg| {
                        label.join(&arg.every_label(&kinds))
                    })
                }
                _ => place.top_label(),
            };
            if let Some((item_span, _)) = found.first()
                && self.labels()
                && !item_label.flows_to(&place_top)
            {
                self.flow_error(
                    Code::U4001,
                    *item_span,
                    format!(
                        "this item is labeled `{item_label}`, above the label `{place_top}` of the hash set `{receiver}`, whose size and items would reveal it; declare the set with a `{item_label}` top label"
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
