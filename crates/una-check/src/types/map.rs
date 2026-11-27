//! The built-in methods of maps (`spec/21 §2`), with the labels of `§3`.

use una_core::{Label, MapOp};
use una_diag::Code;
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{Arg, Convention, Expr};

use super::check::{Checker, Role};
use super::records::Res;
use super::ty::{Shared, Ty};

/// The built-in methods of maps (`spec/21 §2`), in the order of its table: each name, the
/// operation, whether it updates the receiver in place (`edit m.set(...)`), and the labels of
/// its arguments. `map_method` resolves exactly these; `super::builtin_methods` lists them for
/// the standard library reference (#690).
pub(crate) const MAP_METHODS: [(&str, MapOp, bool, &[&str]); 9] = [
    ("has", MapOp::Has, false, &["key"]),
    ("get", MapOp::Get, false, &["key"]),
    ("set", MapOp::With, true, &["key", "value"]),
    ("remove", MapOp::Without, true, &["key"]),
    ("with", MapOp::With, false, &["key", "value"]),
    ("without", MapOp::Without, false, &["key"]),
    ("keys", MapOp::Keys, false, &[]),
    ("values", MapOp::Values, false, &[]),
    ("entries", MapOp::Entries, false, &[]),
];

impl Checker<'_, '_, '_> {
    /// `m.op(args)` when `m` is a map in edition 0.2 and `op` is one of its built-in methods
    /// (`spec/21 §2`); `None` otherwise, so the call resolves as before. `place` is the type
    /// of the receiver as a place, when it is written `edit m.set(...)` or `edit m.remove(...)`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn map_method(
        &mut self,
        target: &Expr,
        target_ty: &Ty,
        place: Option<&Ty>,
        marker: Option<Convention>,
        method: &str,
        args: &[Arg],
        span: Span,
    ) -> Option<Ty> {
        if self.edition() < Edition::V0_2 {
            return None;
        }
        let labeled = self.labeled(target_ty);
        let Ty::Map(key, value) = labeled.unlabeled().clone() else {
            return None;
        };
        let (op, update, params) = MAP_METHODS
            .iter()
            .find(|(name, ..)| *name == method)
            .map(|(_, op, update, params)| (*op, *update, *params))?;
        // `m.has(key: k)` is in the refinement language, as `m.len` is (`spec/21 §4` rule 4).
        if op != MapOp::Has {
            self.outside_rl(span, || format!("the method `{method}`"));
        }
        let receiver = self.report.text(target.span).to_owned();
        // `set` and `remove` update the map in place, as an `edit` receiver (`spec/21 §2`
        // rule 1, `spec/17 §3`); the other methods read it.
        match (update, marker) {
            (true, Some(Convention::Edit)) | (false, None) => {}
            (true, _) => self.report.error(
                Code::U2020,
                target.span,
                format!(
                    "`{method}` updates the map, so the call is written `edit {receiver}.{method}(...)`"
                ),
            ),
            (false, Some(marker)) => self.report.error(
                Code::U2020,
                target.span,
                format!(
                    "`{method}` reads the map and does not take it by `{}`",
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
            let expected = if *param == "key" && op != MapOp::With {
                // ADR 0103 rule 5: a lookup takes `key` at the base of `K`.
                key.without_refinements()
            } else if *param == "key" {
                key.as_ref().clone()
            } else {
                value.as_ref().clone()
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
        // The keys are part of the shape: a read takes the labels of the key position with
        // the top label (`spec/21 §3` rule 1).
        let top = labeled.top_label().join(&key.every_label(&kinds));
        let key_label = found
            .first()
            .map_or_else(Label::public, |(_, ty)| self.deep(ty));
        let result = match op {
            MapOp::Has => Ty::Bool.with_label(&top.join(&key_label)),
            MapOp::Get => self
                .prelude_kind_of("Option", vec![value.as_ref().clone()])
                .with_label(&top.join(&key_label)),
            MapOp::With | MapOp::Without => {
                // The value position takes the value's labels (rule 4).
                let value = match found.get(1) {
                    Some((_, found)) => {
                        let found = self.labeled(found);
                        self.vars.join(&value, &found)
                    }
                    None => value.as_ref().clone(),
                };
                Ty::Map(key.clone(), Shared::new(value)).with_label(&labeled.top_label())
            }
            MapOp::Keys => Ty::List(key.clone()).with_label(&top),
            MapOp::Values => Ty::List(value.clone()).with_label(&top),
            MapOp::Entries => Ty::List(Shared::new(
                self.prelude_kind_of("Entry", vec![key.as_ref().clone(), value.as_ref().clone()]),
            ))
            .with_label(&top),
        };
        if !update {
            self.resolved(span, Res::MapMethod(op));
            // `with` and `without` raise the top label by the key's labels (rule 4).
            return Some(match op {
                MapOp::With | MapOp::Without => result.with_label(&key_label),
                _ => result,
            });
        }
        self.resolved(span, Res::MapUpdate(op));
        if let Some(place_ty) = place {
            // Elaboration rebinds the place to the new map, of the place's type.
            self.typed(target.span, place_ty);
            // An editable binding keeps its declared labels (`spec/17 §6` rule 1): a key above
            // the map's top label is U4001 at the key (`spec/21 §3` rule 4).
            let place = self.labeled(place_ty);
            let place_top = match place.unlabeled() {
                Ty::Map(place_key, _) => place.top_label().join(&place_key.every_label(&kinds)),
                _ => place.top_label(),
            };
            // Label premises only where labels are checked: not in a `via` body (`spec/07 §6`
            // rule 6, #384).
            if let Some((key_span, _)) = found.first()
                && self.labels()
                && !key_label.flows_to(&place_top)
            {
                self.flow_error(
                    Code::U4001,
                    *key_span,
                    format!(
                        "this key is labeled `{key_label}`, above the label `{place_top}` of the map `{receiver}`, whose size and keys would reveal it; declare the map with a `{key_label}` top label"
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

    /// The prelude kind `name` at `args`, or `Error` when the prelude lacks it.
    pub(crate) fn prelude_kind_of(&self, name: &str, args: Vec<Ty>) -> Ty {
        match self.env.prelude_kind(name) {
            Some(id) => Ty::Kind(id, args.into()),
            None => Ty::error(),
        }
    }
}
