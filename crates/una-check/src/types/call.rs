//! Calls (`spec/04 §3`): labeled arguments and punning (U2004 to U2006), calls of functions,
//! function values, policies and capability operations, field access and method-call syntax
//!, and constructors (U2009).

use una_diag::{Code, Escape, ObligationKind};
use una_project::Edition;
use una_syntax::Span;
use una_syntax::ast::{Arg, Convention, Expr, ExprKind, Ident};

use super::Report;
use super::check::{Checker, Role, Site};
use super::closest;
use super::crossing::{Granting, Node, Owner};
use super::env::{Entity, FieldDef, FnAst, FnDef, FnKind, ParamDef};
use super::expr::count;
use super::label::Label;
use super::records::Res;
use super::shared::Flagged as _;
use super::ty::{FnTy, FnUse, KindId, KindLabels, Refinement, Shared, Ty};
use super::unify::{Origin, Vars};

/// A checked argument: where it is, its type, the type of its parameter, and the parameter.
pub(crate) type Checked<'a> = (Span, Ty, Ty, &'a str);

impl<'a> Checker<'_, 'a, '_> {
    /// `e.f`: `.len` of `Text`, `List`, `Map` and the collections that have it, a field of `e`'s record, or a call `f(e)` of a
    /// function of one parameter declared in scope (`spec/04 §3`); U1001 when
    /// none applies.
    pub(crate) fn field(&mut self, target: &Expr, name: &Ident, span: Span, site: Site) -> Ty {
        let target_ty = self.expr(target, None);
        let field = self.name(name);
        let resolved = self.resolve(&target_ty);
        // `.len` and fields take the target's top label.
        let labeled = self.labeled(&target_ty);
        let top = labeled.top_label();
        if field == "len" && matches!(resolved, Ty::Text | Ty::List(_)) {
            self.resolved(span, Res::Len);
            return Ty::Int.with_label(&top);
        }
        // A map's size depends on which of its keys are equal, so `.len` takes the labels of
        // its key position too (`spec/21 §3` rule 2).
        if field == "len"
            && let Ty::Map(key, _) = labeled.unlabeled()
        {
            self.resolved(span, Res::Len);
            let kinds = self.env.kind_labels.clone();
            return Ty::Int.with_label(&top.join(&key.every_label(&kinds)));
        }
        // `a.len`, `m.rows` and `m.columns` of a dense value (`spec/29 §17`).
        if let Some(ty) = self.dense_field(&target_ty, field, span) {
            return ty;
        }
        // The length of a `Stack` or a `Queue` takes its top label, as a list's does (ADR 0119
        // IF1).
        if field == "len"
            && let Ty::Coll(una_core::Coll::Stack | una_core::Coll::Queue, _) = labeled.unlabeled()
        {
            self.resolved(span, Res::Len);
            return Ty::Int.with_label(&top);
        }
        // A collection's size depends on its shape, so `.len` takes the labels of the type
        // arguments that are part of it too (ADR 0119 IF2 to IF4). It is in the refinement
        // language (ADR 0119 V5).
        if field == "len"
            && let Ty::Coll(coll, args) = labeled.unlabeled()
            && super::collections::has_len(*coll)
        {
            self.resolved(span, Res::Len);
            let kinds = self.env.kind_labels.clone();
            let label = args
                .iter()
                .enumerate()
                .filter(|(index, _)| coll.is_shape(*index))
                .fold(top, |label, (_, arg)| label.join(&arg.every_label(&kinds)));
            return Ty::Int.with_label(&label);
        }
        // `z.re` and `z.im` of a measured complex value have its unit (ADR 0120 rule 64).
        if let Ty::Measured(unit, magnitude) = labeled.unlabeled()
            && let Some(ty) = self.record_field(magnitude, field)
        {
            if let Some((kind, index)) = self.field_index(magnitude, field) {
                self.resolved(span, Res::Field { kind, index });
            }
            return ty.in_unit(Some(&**unit)).with_label(&top);
        }
        // `b.size` of a bit set, labeled with its shape (`spec/28 §9`, ADR 0119 IF2).
        if let Some(ty) = self.bit_set_size(&target_ty, field, span) {
            return ty;
        }
        if let Some(ty) = self.record_field(labeled.unlabeled(), field) {
            if let Some((kind, index)) = self.field_index(&resolved, field) {
                self.resolved(span, Res::Field { kind, index });
            }
            return ty.with_label(&top);
        }
        match resolved {
            Ty::Error(_) | Ty::Never => return Ty::error(),
            Ty::Var(_) => {
                self.unknown_type(target.span);
                return Ty::error();
            }
            _ => {}
        }
        match self.env.lower(self.module, field) {
            Some(Entity::Function(id)) => {
                let params = self.env.functions.get(id).map_or(0, |f| f.params.len());
                if params == 1 {
                    return self.call_function(id, Some((target.span, target_ty)), &[], span, site);
                }
                self.report.error(
                    Code::U2001,
                    name.span,
                    format!(
                        "`{field}` takes {}, so `.{field}` needs them, as in `.{field}(...)`",
                        count(params, "argument")
                    ),
                );
                Ty::error()
            }
            _ => {
                self.unknown_member(&resolved, name, false);
                Ty::error()
            }
        }
    }

    /// The kind and position of field `field` of a value of record type `ty`.
    fn field_index(&self, ty: &Ty, field: &str) -> Option<(KindId, usize)> {
        let Ty::Kind(id, _) = ty else {
            return None;
        };
        let fields = self.env.kinds.get(*id)?.record.as_ref()?;
        Some((*id, fields.iter().position(|f| f.name == field)?))
    }

    /// The type of field `field` of a value of record type `ty`, when it has one.
    fn record_field(&self, ty: &Ty, field: &str) -> Option<Ty> {
        let Ty::Kind(id, args) = ty else {
            return None;
        };
        let fields = self.env.kinds.get(*id)?.record.as_ref()?;
        let found = fields.iter().find(|f| f.name == field)?;
        Some(found.ty.base.substitute(args))
    }

    /// U1001 for `e.f` or `e.f(args)` when `e`'s type has no field `f` and no function `f`
    /// applies.
    fn unknown_member(&mut self, ty: &Ty, name: &Ident, call: bool) {
        let member = self.name(name);
        let shown = self.show(ty);
        let fields: Vec<&str> = match ty {
            Ty::Kind(id, _) => self
                .env
                .kinds
                .get(*id)
                .and_then(|k| k.record.as_ref())
                .map(|fields| fields.iter().map(|f| f.name).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let what = if call { "field or function" } else { "field" };
        let mut diagnostic = self.report.make(
            Code::U1001,
            name.span,
            format!(
                "`{shown}` has no {what} `{member}`, and no function `{member}` in scope applies"
            ),
        );
        if let Some(close) = closest(member, fields) {
            diagnostic.repairs.push(
                self.report
                    .edit(format!("write `{close}`"), name.span, close),
            );
        }
        // ADR 0139 rule 7: `Map.update` moved to edition 0.5 with no diagnostic of its own, so
        // edition 0.4 gives the ordinary U1001, with the advice to raise the edition (#1743).
        if self.edition() == Edition::V0_4
            && call
            && member == "update"
            && matches!(ty, Ty::Map(..))
        {
            diagnostic.repairs.push(Report::advice(
                "`Map.update` exists from edition 0.5 (ADR 0139): write `edition 0.5` in ita.una to use it",
            ));
        }
        self.report.push(diagnostic);
    }

    /// Whether every argument of a call that may hold a function, by its parameter's type or
    /// its own, is a lambda written at the call for a parameter whose declared type is a
    /// function type, and every argument was typed against a parameter (ADR 0116 IF5 item 4,
    /// #1023). A lambda at a parameter of another type, such as `fold`'s `initial: U`, is not
    /// known: the callee may return it, so it can run after the call. The receiver is never
    /// one, as it is no lambda.
    fn known_function_arguments(
        &self,
        params: &[ParamDef<'_>],
        args: &[Arg],
        has_receiver: bool,
        checked: &[Checked<'_>],
    ) -> bool {
        let applied = |name: &str| {
            params
                .iter()
                .any(|p| p.name == name && matches!(p.ty.base.unlabeled(), Ty::Fn(_)))
        };
        checked.len() == args.len() + usize::from(has_receiver)
            && checked.iter().all(|(at, found, expected, name)| {
                !(self.holds_function(expected) || self.holds_function(found))
                    || (applied(name)
                        && args.iter().any(|arg| {
                            arg.value.span == *at
                                && matches!(arg.value.kind, ExprKind::Lambda { .. })
                        }))
            })
    }

    /// `e.f(args)`: a call of field `f` when it has a function type, otherwise `f(e, args)` with
    /// `f` declared in scope (`spec/04 §3`).
    // The expected type reaches the built-in unit methods (ADR 0104 rule 4).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn method(
        &mut self,
        target: &Expr,
        name: &Ident,
        args: &[Arg],
        span: Span,
        site: Site,
        marker: Option<Convention>,
        expected: Option<&Ty>,
    ) -> Ty {
        // `edit total.record(e)`: the receiver is a place rooted at an editable binding
        // (`spec/17 §3` rule 2).
        let place = match marker {
            Some(Convention::Edit) => {
                self.editable_place(target, "the receiver of `edit`", false, None)
            }
            _ => None,
        };
        let target_ty = match &place {
            Some(ty) => ty.clone(),
            None => self.expr(target, None),
        };
        let method = self.name(name);
        let resolved = self.resolve(&target_ty);
        let labeled = self.labeled(&target_ty);
        if let Some(field_ty) = self
            .record_field(labeled.unlabeled(), method)
            .map(|ty| ty.with_label(&labeled.top_label()))
            && matches!(self.resolve(&field_ty), Ty::Fn(_))
        {
            self.outside_rl(span, || format!("call of field `{method}`"));
            let ty = self.call_value(
                &field_ty,
                args,
                span,
                &format!("field `{method}`"),
                site,
                None,
                None,
            );
            if let Some((kind, index)) = self.field_index(&resolved, method) {
                self.resolved(span, Res::FieldFn { kind, index });
                if let Some(current) = self.current {
                    self.findings.value_callers.insert(current);
                }
                // `call_value` recorded the call's edge to `Λ` (ADR 0116 P1), once per site.
                self.timing_value_call();
                // S-39: a function value may run any construct.
                self.quiet_site(span);
            }
            return ty;
        }
        if let Some(ty) = self.decimal_method(&target_ty, method, args, span) {
            return ty;
        }
        if let Some(ty) = self.complex_method(&target_ty, method, args, span) {
            return ty;
        }
        if let Some(ty) = self.float_method(&target_ty, method, args, span) {
            return ty;
        }
        if let Some(ty) = self.dense_method(target, &target_ty, marker, method, args, span, site) {
            return ty;
        }
        if let Some(ty) = self.int_method(&target_ty, method, args, span, expected) {
            return ty;
        }
        if let Some(ty) = self.map_method(
            target,
            &target_ty,
            place.as_ref(),
            marker,
            method,
            args,
            span,
        ) {
            return ty;
        }
        if let Some(ty) = self.seq_method(
            target,
            &target_ty,
            place.as_ref(),
            marker,
            method,
            args,
            span,
        ) {
            return ty;
        }
        if let Some(ty) = self.set_method(
            target,
            &target_ty,
            place.as_ref(),
            marker,
            method,
            args,
            span,
        ) {
            return ty;
        }
        if let Some(ty) = self.hash_map_method(
            target,
            &target_ty,
            place.as_ref(),
            marker,
            method,
            args,
            span,
        ) {
            return ty;
        }
        if let Some(ty) = self.hash_set_method(
            target,
            &target_ty,
            place.as_ref(),
            marker,
            method,
            args,
            span,
        ) {
            return ty;
        }
        if let Some(ty) = self.priority_method(
            target,
            &target_ty,
            place.as_ref(),
            marker,
            method,
            args,
            span,
        ) {
            return ty;
        }
        if let Some(ty) = self.bit_set_method(
            target,
            &target_ty,
            place.as_ref(),
            marker,
            method,
            args,
            span,
        ) {
            return ty;
        }
        // Edition 0.4: `edit m.update(key:, initial:, change:)` on a map is a built-in method
        // (ADR 0119 rule 18), declared where no name reaches it, so it resolves ahead of any
        // function `update` in scope, as every built-in method does.
        if method == "update"
            && let Some(id) = self.env.map_update
            && matches!(resolved, Ty::Map(..))
        {
            // ADR 0121 rule 8: elaboration lowers the call, so the report counts it from here.
            self.findings
                .preview_sites
                .insert((una_project::ledger::Construct::MapUpdate, span));
            self.outside_rl(span, || format!("the method `{method}`"));
            self.receiver_marker = Some((marker, target.clone()));
            let ty = self.call_function(id, Some((target.span, target_ty)), args, span, site);
            self.receiver_marker = None;
            return ty;
        }
        // Edition 0.4: `update` on a hash map, the hidden function of `std.collections` (ADR
        // 0119 rule 12), reached the same way.
        if method == "update"
            && let Some(id) = self.env.hash_map_update
            && matches!(resolved, Ty::Coll(una_core::Coll::HashMap, _))
        {
            self.outside_rl(span, || format!("the method `{method}`"));
            self.receiver_marker = Some((marker, target.clone()));
            let ty = self.call_function(id, Some((target.span, target_ty)), args, span, site);
            self.receiver_marker = None;
            return ty;
        }
        match self.env.lower(self.module, method) {
            Some(Entity::Function(id)) => {
                // `x.complex(im: y)` (ADR 0120 rule 22).
                if let Some(ty) =
                    self.complex_call(id, Some((target, &target_ty)), args, span, expected)
                {
                    return ty;
                }
                self.receiver_marker = Some((marker, target.clone()));
                let ty = self.call_function(id, Some((target.span, target_ty)), args, span, site);
                self.receiver_marker = None;
                self.decimal_call(id, Some(target.span), args, span, ty)
            }
            _ => {
                match resolved {
                    Ty::Error(_) | Ty::Never => {}
                    Ty::Var(_) => self.unknown_type(target.span),
                    _ => self.unknown_member(&resolved, name, true),
                }
                self.unchecked(args);
                Ty::error()
            }
        }
    }

    /// Checks arguments that belong to no parameter, for the diagnostics inside them.
    fn unchecked(&mut self, args: &[Arg]) {
        for arg in args {
            self.expr(&arg.value, None);
        }
    }

    /// The arguments of a call of a name that does not resolve: U1001 is reported there, or
    /// the name is one a broken declaration would have declared. A lambda among them reports
    /// nothing about its parameters, which only the missing callee could type (ADR 0096
    /// rule 8). Every other unchecked call keeps U2010.
    fn unchecked_unknown(&mut self, args: &[Arg]) {
        self.vars.mark_unknown();
        for arg in args {
            self.unknown_callee_argument = matches!(arg.value.kind, ExprKind::Lambda { .. });
            self.expr(&arg.value, None);
            self.unknown_callee_argument = false;
        }
    }

    /// `callee(args)`.
    pub(crate) fn call(
        &mut self,
        callee: &Expr,
        args: &[Arg],
        span: Span,
        site: Site,
        expected: Option<&Ty>,
    ) -> Ty {
        match &callee.kind {
            ExprKind::Name(ident) => {
                let name = self.name(ident);
                if let Some(ty) = self.local(name) {
                    self.outside_rl(span, || format!("call of local function `{name}`"));
                    self.captured_value(name, ident.span);
                    // A parameter that an `of` clause names runs the effects of its argument
                    // (ADR 0062).
                    let own = self.labeled(&ty).top_label();
                    // ADR 0123 rule 5: a `uses of` parameter called in a branch (U3033).
                    if self.of_param(name).is_some() {
                        self.branch_server_of_param(name, ident.span);
                    }
                    let of = self.of_param(name).map(|index| {
                        self.run_of_param(name, index, ident.span, &Label::public(), &own)
                    });
                    let index = self.of_param(name);
                    // A `uses of` parameter's argument may run any binding (ADR 0092 rule 3, as
                    // amended; #615).
                    if self.uses_of_param(name) {
                        self.every_condition(span);
                    }
                    return self.call_value(&ty, args, span, &format!("`{name}`"), site, of, index);
                }
                match self.env.lower(self.module, name) {
                    Some(Entity::Function(id)) => {
                        // A tensor's shape selects the function of its rank (`spec/29 §17`).
                        if let Some(ty) = self.tensor_construct(id, args, span, site) {
                            return ty;
                        }
                        // `complex(re: x, im: y)` (ADR 0120 rule 22).
                        if let Some(ty) = self.complex_call(id, None, args, span, expected) {
                            return ty;
                        }
                        let ty = self.call_function(id, None, args, span, site);
                        self.decimal_call(id, None, args, span, ty)
                    }
                    Some(Entity::Handler(_)) => {
                        self.report.error(
                            Code::U2001,
                            ident.span,
                            format!(
                                "handler `{name}` is not a function; install it with `with` or `given`"
                            ),
                        );
                        self.unchecked(args);
                        Ty::error()
                    }
                    // An unknown name is U1001, reported by M3, or a poisoned name.
                    _ => {
                        self.unchecked_unknown(args);
                        Ty::unknown()
                    }
                }
            }
            ExprKind::UpperName(ident) => self.constructor_call(ident, args, span, expected),
            _ => {
                self.outside_rl(span, || "call of a function value".to_owned());
                let ty = self.expr(callee, None);
                self.call_value(&ty, args, span, "this function", site, None, None)
            }
        }
    }

    /// A call of a function value: positional arguments only.
    /// `of`: for a parameter that an `of` clause names, the effects and failure of its
    /// argument, which the call performs (ADR 0062).
    #[allow(clippy::too_many_arguments)]
    fn call_value(
        &mut self,
        ty: &Ty,
        args: &[Arg],
        span: Span,
        what: &str,
        site: Site,
        of: Option<super::effects::OfFound>,
        of_index: Option<usize>,
    ) -> Ty {
        let value_label = self.labeled(ty).top_label();
        // The function type is taken before its labels are settled: the result of a generic
        // function value carries the labels of type arguments that the arguments of this call
        // may still solve (ADR 0038 D3.2, D-137).
        let head = self.vars.head(ty);
        let function = match head.unlabeled().clone() {
            Ty::Fn(function) => (*function).clone(),
            Ty::Labeled(..) | Ty::Refined(..) => {
                self.unchecked(args);
                return Ty::error();
            }
            Ty::Error(_) | Ty::Never => {
                self.unchecked(args);
                return Ty::error();
            }
            Ty::Var(_) => {
                self.unknown_type(span);
                self.unchecked(args);
                return Ty::error();
            }
            other => {
                let shown = self.show(&other);
                self.report.error(
                    Code::U2001,
                    span,
                    format!("{what} is not a function; it has type `{shown}`"),
                );
                self.unchecked(args);
                return Ty::error();
            }
        };
        for arg in args {
            if let Some(label) = &arg.label {
                let value = self.report.text(arg.value.span);
                let repair = self.report.edit("remove the label", arg.span, value);
                let mut diagnostic = self.report.make(
                    Code::U2005,
                    label.span,
                    format!("{what} is a function value, which takes its arguments without labels"),
                );
                diagnostic.repairs.push(repair);
                self.report.push(diagnostic);
            }
        }
        if args.len() != function.params.len() {
            self.report.error(
                Code::U2001,
                span,
                format!(
                    "{what} takes {}, but {} given",
                    count(function.params.len(), "argument"),
                    given(args.len())
                ),
            );
            self.unchecked(args);
            return function.result;
        }
        self.resolved(span, Res::Value);
        if let Some(current) = self.current {
            self.findings.value_callers.insert(current);
        }
        self.graph_value_call(span);
        self.timing_value_call();
        self.ordered(span, (0..args.len()).map(Some).collect());
        let mut flows = Vec::new();
        let mut decided = Label::public();
        for (arg, param) in args.iter().zip(&function.params) {
            self.ctx.argument = true;
            let found = self.expr_as(
                &arg.value,
                Some(param),
                Role::Argument {
                    param: "",
                    callee: what,
                },
            );
            self.fn_flow(arg.value.span, param, &found);
            // ADR 0037 rule 7: the refinements of the value's parameter are proven at the
            // argument.
            if !function.refinements.param(flows.len()).is_empty() {
                self.obligation(ObligationKind::Refinement, arg.value.span, None);
            }
            // ADR 0067 amendment: an argument decided by comparing values of the enclosing
            // function's type parameters carries `ℓ*_T`. A function value cannot be checked
            // against that, so the decision raises the call's result and program counter
            // instead, as the value's own label does, and the argument flows without it.
            let stripped = self.without_decisions(&found, &mut decided);
            // Edition 0.1 keeps the decision in the flow as well (#253, #356): a function value
            // whose parameter is `ℓ_T` rejects it (U4001), where edition 0.2 lets the raise
            // below carry it. Stripping it here would accept programs that edition 0.1 rejects.
            let found = if self.edition() >= Edition::V0_2 {
                stripped
            } else {
                found
            };
            flows.push((arg.value.span, found, param.clone()));
        }
        for (arg_span, found, param) in flows {
            self.instantiated_by(arg_span, &param, &found);
            self.flow(arg_span, &found, &param, &format!("the argument of {what}"));
        }
        // A parameter that an `of` clause names runs the effects its caller chose, so a
        // decision on its argument would reveal itself through them (#234): its callers run
        // their argument under the decision, as the prelude's `first_where` does.
        if let Some(index) = of_index {
            self.raise_of(index, &decided);
        }
        let value_label = value_label.join(&decided);
        // S-39: a function value may run any construct. Which one runs depends on the value, so
        // the call stands under `pc ⊔ l_f` with the decisions of its arguments (T-App below),
        // and is quiet when that is above `public`, at a `public` program counter too (round 4).
        self.quiet_site_under(span, &value_label);
        // `spec/27 §8` rule 5: which code runs, and so how many check points, `l_f` decides.
        self.switch_site(span, &value_label, || {
            format!("which code this call of {what} runs, and whether it runs,")
        });
        // Calling a function value needs the capabilities of its type (`spec/10 §2`, S-Fun).
        let level = self.frames.len();
        self.needs_site(what, &function.uses, span, level);
        // ADR 0092 rule 3: the value performs its type's `uses` through the caller's bindings
        // (#615).
        self.condition_needs(&function.uses, span, level);
        // T-App (ADR 0038 D1): which function runs depends on the value, so its label raises
        // the program counter of the call and its result.
        self.call_under_pc(
            span,
            what,
            &function.uses,
            function.fails.is_some(),
            false,
            &value_label,
        );
        let of = of.unwrap_or_default();
        let fails = function.fails.clone().or(of.fails);
        self.position_site(
            what,
            span,
            !function.uses.is_empty() || of.effects,
            fails.as_ref(),
            site.top,
            site.handled,
        );
        if let Some(fails) = &fails {
            self.call_fails.insert(span, fails.clone());
        }
        // The result of a function value carries the value's label.
        function.result.with_label(&value_label)
    }

    /// `found` without the deep symbolic labels `ℓ*_T` of the enclosing function's type
    /// parameters, which are joined into `decided` (ADR 0067 amendment, and #253 in edition
    /// 0.1).
    fn without_decisions(&self, found: &Ty, decided: &mut Label) -> Ty {
        // Both editions: in edition 0.1 under D-137 as well, since a decided argument given to
        // a function value would otherwise lose its decision (#253).
        if self.type_params.is_empty() {
            return found.clone();
        }
        let names = self.type_params.clone();
        let deep = |symbol: &str| {
            symbol
                .strip_suffix('*')
                .is_some_and(|name| names.contains(&name))
        };
        let labeled = self.labeled(found);
        let seen = std::cell::RefCell::new(Label::public());
        let stripped = labeled.map_labels(&|label| {
            let mut kept = label.clone();
            kept.symbols.retain(|s| !deep(s));
            let mut gathered = Label::public();
            gathered.symbols = label.symbols.iter().filter(|s| deep(s)).cloned().collect();
            let joined = seen.borrow().join(&gathered);
            *seen.borrow_mut() = joined;
            kept
        });
        *decided = decided.join(&seen.into_inner());
        stripped
    }

    /// S-Fun at a flow of a value of type `found` into the declared type `expected`
    /// (`spec/10 §2`, `spec/05 §9` rule 7, ADR 0038 D1): every function type in
    /// the value's type, directly or under `List`, `Option`, `Result` and kind arguments, has no
    /// `uses` that the expected function type at its position lacks (U3003) and fails only with
    /// its error kind (U3009); function parameters are compared the other way round. One flow
    /// has at most one such diagnostic.
    pub(crate) fn fn_flow(&mut self, span: Span, expected: &Ty, found: &Ty) {
        self.fn_flow_as(span, expected, found, false);
    }

    /// [`Checker::fn_flow`] for the written type of a lambda parameter, `expected`, which the
    /// values of `found`, from the expected function type, flow into.
    pub(crate) fn fn_flow_param(&mut self, span: Span, expected: &Ty, found: &Ty) {
        self.fn_flow_as(span, expected, found, true);
    }

    fn fn_flow_as(&mut self, span: Span, expected: &Ty, found: &Ty, param: bool) {
        if self.pass != super::authority::Pass::Check {
            return;
        }
        let (expected, found) = (self.resolve(expected), self.resolve(found));
        // ADR 0037 rule 2: a refinement of a function type that the flow does not carry over
        // is a pending `refinement` obligation at the value. A lambda's written parameter types
        // are the lambda's own flow, recorded by [`Checker::lambda`].
        if !param && refinement_flow(&expected, &found) {
            self.obligation(ObligationKind::Refinement, span, None);
        }
        let direct = matches!(expected, Ty::Fn(_));
        let Some((problem, given)) = fn_problem(&expected, &found, false) else {
            return;
        };
        let (code, what) = match &problem {
            FnProblem::Uses(need) => (Code::U3003, format!("uses `{}`", self.show_need(need))),
            FnProblem::Fails(kind) => (Code::U3009, format!("can fail with `{}`", self.show(kind))),
        };
        let message = if param {
            format!(
                "the expected function type gives this lambda parameter a function that {what}, which its written type does not allow"
            )
        } else if given {
            format!(
                "the expected type gives this function a function that {what}, which its own type does not allow"
            )
        } else if direct {
            format!("this function {what}, which the expected function type does not allow")
        } else {
            format!(
                "this value holds a function that {what}, which the expected type does not allow"
            )
        };
        self.report.error(code, span, message);
    }

    /// A call of a declared function, predicate or policy, with `receiver` as its first
    /// argument in method-call syntax.
    pub(crate) fn call_function(
        &mut self,
        id: usize,
        receiver: Option<(Span, Ty)>,
        args: &[Arg],
        span: Span,
        site: Site,
    ) -> Ty {
        let Some(def) = self.env.functions.get(id).cloned() else {
            return Ty::error();
        };
        let callee = match def.kind {
            // A method of a dense value is named as the method (`spec/29 §17`).
            FnKind::Function if def.module == self.env.prelude => {
                format!(
                    "`{}`",
                    super::dense::dense_display(def.name).unwrap_or(def.name)
                )
            }
            FnKind::Function => format!("`{}`", def.name),
            FnKind::Predicate { .. } => format!("predicate `{}`", def.name),
            FnKind::Policy => format!("policy `{}`", def.name),
        };
        // `nan()` and `infinity()` of the prelude are constants of the refinement language
        // (`spec/29 §8`, ADR 0114 rule 8.4).
        let float_constant = def.module == self.env.prelude
            && matches!(def.name, "nan" | "infinity")
            && self.edition() >= una_project::Edition::V0_3;
        if !matches!(def.kind, FnKind::Predicate { .. }) && !float_constant {
            // Contracts call only predicates (`spec/03 §3`).
            let word = if def.kind == FnKind::Policy {
                "policy"
            } else {
                "function"
            };
            self.outside_rl_hint(
                span,
                || format!("the call of {word} `{}`", def.name),
                "; the refinement language calls only predicates",
            );
        }
        // ADR 0042: an opaque predicate is called only as the predicate of `checked` and in
        // opaque bodies. D-149: also in the `where` clause of a kind and in the
        // clauses of a capability operation, which name the fact without establishing it.
        if def.kind == (FnKind::Predicate { reflect: false })
            && !self.ctx.opaque
            && !self.ctx.opaque_calls
            && self.pass == super::authority::Pass::Check
        {
            let mut diagnostic = self.report.make(
                Code::U2016,
                span,
                format!(
                    "opaque predicate `{}` is called outside `checked`, so its fact would bypass the escape budget",
                    def.name
                ),
            );
            // ADR 0095: a test that establishes no fact keeps the escape budget; `checked`,
            // an escape, comes last.
            if self.plain_test_route(id) {
                diagnostic.repairs.push(super::Report::advice(format!(
                    "if the code needs only the test and not the fact, test with a function, not a predicate, with the body of `{}`",
                    def.name
                )));
            }
            diagnostic.repairs.push(super::Report::escape(
                Escape::Checked,
                format!(
                    "check the value with `checked(value, {}(it)) else fail <Variant>`",
                    def.name
                ),
            ));
            self.report.push(diagnostic);
        }
        // A call with an `edit` or `take` argument changes a binding, so it is the whole
        // expression of a statement or `let` (`spec/17 §3` rule 5).
        let marked = args.iter().any(|arg| arg.marker.is_some())
            || self
                .receiver_marker
                .as_ref()
                .is_some_and(|(marker, _)| marker.is_some());
        let statement = self.statement_calls.contains(&span);
        if marked && !statement && self.pass == super::authority::Pass::Check {
            // #837: the advice of the place where the call stands, a `do` line included.
            let advice = self.nested_advice(site.handled, true);
            // Read by `position_site` later in this function, which must run after this
            // report: it skips the U3013 of the call's effects at this span (#2213).
            self.marked_nested = Some(span);
            self.report.error(
                Code::U3013,
                span,
                format!(
                    "{callee} is called with `edit` or `take`, so the call must be the whole expression of a statement or `let`; {advice}"
                ),
            );
        }
        // Declared effects and failures in an opaque predicate.
        if !def.uses.is_empty() {
            self.impure(span, || {
                format!("call of {callee}, which uses capabilities,")
            });
        }
        if def.fails.is_some() && !site.handled {
            self.impure(span, || format!("call of {callee}, which can fail,"));
        }
        self.edge(id, span);
        self.call_targets.insert(span, id);
        if self.pass == super::authority::Pass::Check {
            self.findings.named_calls.insert(span);
        }
        if self.pass == super::authority::Pass::Check {
            // In a body no label rule checks, the program counter is unknown: the call is
            // entered at the cap, `⊤` here, which `file_state` caps.
            let entry = self.entry();
            let pc = if self.labels() {
                self.ctx.pc.clone()
            } else {
                Label::top(&self.env.categories)
            };
            self.env.file_calls.push((entry, id, pc));
        }
        self.testing_site(def.module, def.name, span);
        let needs: Vec<FnUse> = def.uses.iter().map(|need| need.usage.clone()).collect();
        let level = self.frames.len();
        // ADR 0093 rules 3 and 12: a call into a boundary takes nothing by name. A policy call
        // is a call of its `via` too, and crosses when either is in a boundary.
        let via_module = match def.ast {
            FnAst::Policy(policy) => {
                let via = self.env.text(def.module, policy.via.name);
                match self.env.lower(def.module, via) {
                    Some(Entity::Function(via)) => self.env.functions.get(via).map(|f| f.module),
                    _ => None,
                }
            }
            _ => None,
        };
        let crossing = self
            .crossing(def.module)
            .or_else(|| via_module.and_then(|module| self.crossing(module)));
        let granted = match crossing {
            Some(boundary) => self.crossing_needs(
                boundary,
                &callee,
                &needs,
                &def.uses,
                self.env.in_package(def.module),
                span,
                level,
                super::crossing::GrantSite::Call,
            ),
            None => needs.clone(),
        };
        self.needs_site(&callee, &granted, span, level);
        // ADR 0092 rule 3: the callee's bindings receive the caller's (#615).
        self.condition_needs(&granted, span, level);
        // ADR 0092, amendment of 2026-10-14: the edit that writes the caller's condition on the
        // callee. A call into a boundary grants nothing by name, so it gets none.
        if crossing.is_none() {
            self.condition_edits(
                super::conditions::EditTarget::Function(id),
                args,
                receiver.is_some(),
                span,
                level,
            );
        }
        // ADR 0093 rule 15: what the callee grants on by name, from its own bindings, reaches
        // the bindings its `uses` resolve to here.
        self.summary_needs(Owner::Node(Node::Function(id)), &granted, level);
        self.reach(Node::Function(id));
        // S-39: under a raised program counter, the events that the runtime appends while such
        // a call runs are left out of the trace when the callee can run a construct that
        // appends them.
        self.quiet_call_site(span, id);
        // ADR 0116 rule 8: a call of a function that says `diverges` may run forever.
        if def.diverges
            && let Some(reason) = self.divergence_refused()
        {
            self.report.error(
                Code::U3027,
                span,
                format!(
                    "`{}` says `diverges`, so this call may run forever: {reason}",
                    def.name
                ),
            );
        }
        self.call_fact(id, span);
        self.via_site(id, span);
        self.call_under_pc(
            span,
            &callee,
            &needs,
            def.fails.is_some(),
            false,
            &Label::public(),
        );
        // A callee with `of` clauses performs the effects of its arguments too, so its
        // position is checked once they are known (ADR 0062).
        let of_clauses = !def.uses_of.is_empty() || def.fails_of.is_some();
        // Order matters (#837, #2213): the U3013 of a call with `edit` or `take` above sets
        // `marked_nested` and is reported before this check, which then skips the U3013 of its
        // effects at the same span. Moving this check before it, or rolling back the report
        // between the two (speculative checking), would drop both diagnostics silently.
        if !of_clauses {
            self.position_site(
                &callee,
                span,
                !needs.is_empty(),
                def.fails.as_ref(),
                site.top,
                site.handled,
            );
        }
        if def.kind == FnKind::Policy {
            self.resolved(
                span,
                Res::Function {
                    id,
                    type_args: Vec::new(),
                },
            );
            // A policy call is a release (`spec/07 §5`); into a boundary it is U3024 itself,
            // unless its policy is declared in a dependency's module that the root trusts by
            // a trust line (ADR 0122 rule 53.1).
            let trusted_policy = self.env.trusted_dependencies.contains(&def.module);
            if !trusted_policy {
                self.release_site(span);
            }
            // The call runs the policy's `via` function (`spec/07 §5`): an edge of the graph
            // of ADR 0116 P1 at the call site, and, for the timing label's search over the call
            // edges and the quiet calls of S-39, the policy's own edge to it (#1673).
            if let Some(via) = self.env.via_of(id) {
                self.graph_call(via, span);
                self.findings.policy_vias.insert((id, via));
            }
            let result = self.call_policy(id, &def, &callee, receiver, args, span);
            if let Some(boundary) = crossing {
                let granting = Granting {
                    release: (!trusted_policy).then_some(span),
                    ..Granting::default()
                };
                self.cross(
                    boundary,
                    &format!("the call of {callee}"),
                    Node::Function(id),
                    &granted,
                    level,
                    granting,
                    span,
                );
            }
            return result;
        }
        let subst = self.instantiate(&def.type_params, def.name, span);
        if !subst.is_empty() {
            self.instances.push((id, subst.clone(), span));
        }
        self.resolved(
            span,
            Res::Function {
                id,
                type_args: subst.clone(),
            },
        );
        if of_clauses {
            self.of_callee = Some(super::effects::OfCallee {
                uses_of: def.uses_of.clone(),
                fails_of: def.fails_of,
                own_fails: def.fails.clone(),
                // The `update` of a map and of a hash map runs its argument as `spec/07 §7`
                // rule 9 describes for the prelude's functions (ADR 0119 IF7).
                prelude: (def.module == self.env.prelude
                    || Some(id) == self.env.map_update
                    || Some(id) == self.env.hash_map_update)
                    .then(|| def.name.to_owned()),
                raises: self
                    .env
                    .of_raises
                    .range((id, 0)..=(id, usize::MAX))
                    .map(|((_, param), indices)| {
                        let args: Vec<Ty> = indices
                            .iter()
                            .filter_map(|i| subst.get(*i).cloned())
                            .collect();
                        (*param, Vars::label_of(&args))
                    })
                    .collect(),
            });
            self.of_found.push(super::effects::OfFound::default());
        }
        // ADR 0093 rule 5: what the arguments of a call into a boundary grant it.
        if crossing.is_some() {
            let granting = self.granting_here(span);
            self.granting.push(granting);
        }
        let has_receiver = receiver.is_some();
        let checked = self.arguments(
            &callee,
            &def.params,
            &subst,
            receiver,
            args,
            span,
            def.module,
        );
        // ADR 0116 IF5 item 4 (#1023): a prelude function that applies a function argument
        // runs code known at the call only when every argument that may hold a function is a
        // lambda written at the call for a parameter of function type, which the timing label
        // of a site counts by its captures and its body. Any other one, the receiver and a
        // lambda given to a parameter such as `fold`'s `initial` included, keeps `⊤`.
        if super::timing::applies_function(self.env, &def)
            && !self.known_function_arguments(&def.params, args, has_receiver, &checked)
        {
            self.timing_opaque_apply(id);
        }
        // ADR 0092 rule 3: the function values the callee may run through the caller's
        // bindings, charged once the arguments are typed (#615).
        self.passed_conditions(id, has_receiver, args, span, level);
        if let Some(boundary) = crossing {
            let granting = self.granting.pop().unwrap_or_default();
            self.cross(
                boundary,
                &format!("the call of {callee}"),
                Node::Function(id),
                &granted,
                level,
                granting,
                span,
            );
        }
        let mut call_fails = def.fails.clone();
        if of_clauses {
            let found = self.of_found.pop().unwrap_or_default();
            // The effects of its arguments are the call's, and so of the lambda around it.
            if found.effects {
                self.note_effect();
            }
            call_fails = call_fails.or(found.fails);
            // After the marked U3013 above, as the check without `of` clauses (#2213).
            self.position_site(
                &callee,
                span,
                !needs.is_empty() || found.effects,
                call_fails.as_ref(),
                site.top,
                site.handled,
            );
        }
        let values = declared_args(&def.params, &checked);
        let result = def.result.base.substitute(&subst);
        // ADR 0103 rule 5: a type argument at a position of the result that is not covariant,
        // inside a function type or under a user kind's argument, keeps no refinement, so an
        // inferred refinement never meets a contravariant or invariant flow.
        // ADR 0120: a trusted function that computes new elements keeps no refinement of its
        // type arguments in its result (`super::dense::SYNTHESIZING`).
        let synthesizes = super::dense::synthesizes(self.env.prelude, def.module, def.name);
        if synthesizes {
            self.synthesized(span);
        }
        let result = if subst.is_empty()
            || (covariant_in(&def.result.base, &self.env.kind_labels) && !synthesizes)
        {
            result
        } else {
            self.refined_vars_at_base(&result)
        };
        let decided = self.decided_callbacks(&callee, span, &needs, call_fails.is_some(), &checked);
        // S-39: the callee runs under the decision (T-Call), so the call is quiet under it, as
        // under a raised program counter. Only edition 0.1 raises a call so, and no construct
        // that appends events of its own exists there, so this is defensive (round 4).
        self.quiet_call_site_under(span, id, &decided);
        // `spec/27 §8` rule 5: a check point of the schedule of started tasks (#1684).
        self.switch_call(span, id, &def, &checked);
        let result = if prelude_labeled(self.env.prelude, &def)
            || collections_labeled(self.env.collections, &def)
        {
            self.prelude_result(&callee, def.name, checked, result)
        } else {
            self.argument_flows(&callee, checked);
            self.generic_result(id, &def, &subst, result)
        };
        let result = if decided.is_public() {
            result
        } else {
            result.raise_deep(&decided)
        };
        if !subst.is_empty() {
            self.instantiation(def.name, &def.type_params, subst, values, span);
        }
        if def.requires {
            self.obligation(ObligationKind::Precondition, span, None);
        }
        if def.module == self.env.prelude {
            self.dense_result(&result, span);
            // ADR 0109 rule 3: `sum` and `abs` compute an `Int` by arithmetic.
            self.prelude_overflow_site(def.name, span);
        }
        if let Some(fails) = &call_fails {
            self.call_fails.insert(span, fails.clone());
            // A prelude function that declares its own failure and whose result carries its
            // arguments' labels fails depending on their values too, as `parse_json` does: its
            // failure is labeled like its result (`spec/23 §5`, `spec/22 §5` rule 2). The
            // failure of a function value given for `fails of` follows ADR 0067 instead.
            if def.fails.is_some() && prelude_labeled(self.env.prelude, &def) {
                let label = self.labeled(&result).top_label();
                // The payload too, as `ParseError`'s position: it is labeled itself, not only
                // read under the raised program counter.
                self.call_fails.insert(span, fails.raise_deep(&label));
                self.decided_failures.insert(span, label);
            }
            // Edition 0.1 (#350): whether the call fails, and how, may reveal a decision given
            // to its function values, as an operation's failure reveals its arguments.
            if !decided.is_public() {
                let payload = self.call_fails.get(&span).cloned().unwrap_or(fails.clone());
                self.call_fails.insert(span, payload.raise_deep(&decided));
                let label = self
                    .decided_failures
                    .get(&span)
                    .map_or_else(|| decided.clone(), |label| label.join(&decided));
                self.decided_failures.insert(span, label);
            }
        }
        result
    }

    /// The result of a call of a generic function declared outside the prelude: the value its
    /// body returns may carry the symbolic labels of its type parameters, which the call
    /// replaces by `ℓ*(σ)`, the join of every label of each type argument (`spec/10 §5`,
    /// generic instantiation, rule 3).
    /// - Edition 0.2 (ADR 0067): each position is raised by the type arguments of its summary.
    /// - Edition 0.1, D-137: every position that holds data is raised by the join of
    ///   `ℓ*(σ)` over all the type arguments.
    pub(crate) fn generic_result(
        &self,
        id: usize,
        def: &FnDef<'_>,
        subst: &[Ty],
        result: Ty,
    ) -> Ty {
        if subst.is_empty() || def.module == self.env.prelude {
            return result;
        }
        if self.edition() >= Edition::V0_2 {
            let summary = self.env.summaries.get(&id).cloned().unwrap_or_default();
            return super::summary::raise(&def.result.base, result, &summary, subst);
        }
        result.raise_deep(&Vars::label_of(subst))
    }

    /// The result type of a generic function used as a value with type arguments `args`, which
    /// the value's type records lazily, since they are known only once the value flows
    /// somewhere (`spec/10 §5`, generic instantiation): raised as a call raises it.
    pub(crate) fn generic_value_result(&self, id: usize, args: &[Ty]) -> Ty {
        let Some(def) = self.env.functions.get(id) else {
            return Ty::error();
        };
        let result = def.result.base.substitute(args);
        self.generic_result(id, def, args, result)
    }

    /// The result of a call of a prelude function of `spec/14 §1` "Result labels": its
    /// parameters that are not functions accept arguments of any label (D-136). The arguments
    /// of function type flow into their parameters as usual, so a lambda's result stays below
    /// the label its parameter type declares.
    /// - The structural functions (ADR 0051) label each position of the result with what it
    ///   holds: the element and accumulator positions carry the labels of their type
    ///   variables, which every argument they come from joins (`spec/07 §7` rule 1), and the
    ///   top label is the label of what decides the shape: [`structural_inputs`].
    /// - The others label their result with the join of every label of every argument,
    ///   element types included (ADR 0038 D3.4); `sort` raises every position by it.
    fn prelude_result(
        &mut self,
        callee: &str,
        name: &str,
        checked: Vec<Checked<'a>>,
        result: Ty,
    ) -> Ty {
        let structural = structural_inputs(name);
        let mut label = Label::public();
        let mut functions = Vec::new();
        for (span, found, expected, param) in checked {
            match structural {
                Some(inputs) => {
                    if let Some((_, deep)) = inputs.iter().find(|(input, _)| *input == param) {
                        let found = self.labeled(&found);
                        label = label.join(&if *deep {
                            self.deep(&found)
                        } else {
                            found.top_label()
                        });
                    }
                    // A predicate's result decides which elements are kept; its flow into the
                    // declared `Bool` bounds it, and that bound labels the result.
                    if let Ty::Fn(function) = self.labeled(&expected).unlabeled()
                        && param == "keep"
                    {
                        label = label.join(&function.result.top_label());
                    }
                }
                None => label = label.join(&self.deep(&found)),
            }
            if matches!(self.resolve(&expected), Ty::Fn(_)) {
                functions.push((span, found, expected, param));
            }
        }
        self.argument_flows(callee, functions);
        // Where an element of `sort` ends up depends on every element, so every position of
        // its result carries the labels of the list and of all its elements (`spec/20 §4`
        // rule 2).
        // Likewise for the kernels whose result is a dense value: each of its elements depends
        // on many input elements and on the inputs' shapes (ADR 0120 IF6).
        // A factorization and a solve choose their pivots from the values, so the top of the
        // `Option` they return, and every position of an `Lu`, are raised the same way (IF6).
        if matches!(
            name,
            "sort"
                | "dense_matrix_multiply"
                | "dense_matrix_multiply_array"
                | "dense_matrix_row_sums"
                | "dense_matrix_column_sums"
                | "dense_matrix_lu"
                | "dense_matrix_solve"
                | "dense_matrix_solve_array"
                | "dense_matrix_cholesky"
                | "dense_matrix_solve_lower"
                | "dense_matrix_solve_upper"
                | "dense_lu_solve"
                | "dense_lu_pivots"
                | "dense_tensor_sum_axis"
                | "dense_tensor_contract"
        ) {
            return result.raise_deep(&label);
        }
        result.with_label(&label)
    }

    /// Edition 0.1 (#346, #350): a call that gives function values data decided by values of
    /// the enclosing function's type parameters. The arguments carry `ℓ*_T`; a function value
    /// whose parameters lack it was checked against `ℓ_T` at each call, as a named function
    /// value or a lambda whose parameter is annotated (the body assumes the annotation) is.
    /// Whoever calls it with that data, the prelude or a generic function, may reveal the
    /// decision through what it returns, through its effects and through its failures, its
    /// own or the function value's. So the result is raised at every position, the callee's
    /// effects and the function values' run under the raised program counter (T-Call, T-App,
    /// as #253 does for a call written in the body), and its failure is labeled by it, as
    /// a host operation's is. A lambda whose parameter types are inferred takes
    /// `ℓ*_T` in them, and its own flows carry it. Edition 0.2 tells `ℓ*_T` from `ℓ_T` at
    /// the function argument's flow (S-Fun) instead. S-Fun does not subsume this rule in
    /// edition 0.1 (#356): when the decided argument goes to a parameter that no function value
    /// sees, no flow carries the decision, and without summaries this raise is the only bound
    /// on the callee's effects, as the `hof_rule_350__effect__two_params_unused` and
    /// `hof_boundary__effect__*` conformance cases show. Returns the label of the decision.
    fn decided_callbacks(
        &mut self,
        callee: &str,
        span: Span,
        needs: &[FnUse],
        fails: bool,
        checked: &[Checked<'a>],
    ) -> Label {
        let mut raised = Label::public();
        if self.edition() >= Edition::V0_2 || self.type_params.is_empty() {
            return raised;
        }
        // Every decision the arguments carry, at any position, beside callbacks too.
        let mut decided = Label::public();
        for (_, found, _, _) in checked {
            self.without_decisions(found, &mut decided);
        }
        if decided.is_public() {
            return raised;
        }
        let mut uses: Vec<FnUse> = needs.to_vec();
        let mut fails = fails;
        for (arg_span, found, expected, _) in checked {
            let mut inner = Vec::new();
            self.function_types(&self.labeled(found), &mut inner, &mut Vec::new());
            let mut declared = Vec::new();
            self.function_types(&self.labeled(expected), &mut declared, &mut Vec::new());
            let (annotated, opaque) = self.annotated_in(*arg_span);
            // Only an inline lambda written as the argument, none of whose parameters is
            // annotated, takes its parameter types from the data, as its body assumes them
            // (#350, H-3, H-4, H-5): it accepts the decisions that every parameter's type
            // carries. A named function value was checked against `ℓ_T`, an annotated
            // parameter's body assumes `ℓ_T`, and a wrapped, let-bound or field function value's
            // type does not show what its body assumed: they accept nothing.
            let inline = annotated
                .as_ref()
                .filter(|flags| !opaque && !flags.contains(&true));
            for (index, (function, field)) in inner.into_iter().enumerate() {
                let mut accepted = Label::public();
                if index == 0 && !field && inline.is_some() && !function.params.is_empty() {
                    let mut every: Option<super::label::Names> = None;
                    for input in &function.params {
                        let mut carried = Label::public();
                        self.without_decisions(input, &mut carried);
                        every = Some(match every {
                            None => carried.symbols,
                            Some(so_far) => so_far.intersection(&carried.symbols),
                        });
                    }
                    accepted.symbols = every.unwrap_or_default();
                }
                let mut missing = decided.clone();
                missing.symbols.retain(|s| !accepted.symbols.contains(s));
                if missing.is_public() {
                    continue;
                }
                raised = raised.join(&missing);
                fails = fails || function.fails.is_some();
                uses.extend(function.uses.iter().cloned());
                if let Some((declared, _)) = declared.get(index) {
                    uses.extend(declared.uses.iter().cloned());
                }
            }
        }
        if !raised.is_public() {
            // T-Call: the callee runs under the decision, so its effects are bounded by it and
            // it may not fail (`spec/07 §7` rule 4).
            self.call_under_pc(span, callee, &uses, fails, false, &raised);
        }
        raised
    }

    /// The lambdas checked inside the argument at `arg` (#350, H-4): the parameter
    /// annotations of the lambda that is the argument itself, inside parentheses or not, and
    /// whether another lambda with an annotated parameter lies anywhere in it. The body of an
    /// annotated lambda assumes its annotation, whatever its type unifies with, and through a
    /// join (`if`, `when`), a block or a call the argument's type no longer shows which
    /// function value assumes what: such an argument's function values accept nothing.
    fn annotated_in(&self, arg: Span) -> (Option<Vec<bool>>, bool) {
        let from = Span {
            file: arg.file,
            start: arg.start,
            end: 0,
        };
        let to = Span {
            file: arg.file,
            start: arg.end,
            end: usize::MAX,
        };
        let mut direct = None;
        let mut opaque = false;
        for (span, flags) in self.annotated_lambdas.range(from..=to) {
            if span.end > arg.end {
                continue;
            }
            let before = self.report.text(Span {
                file: arg.file,
                start: arg.start,
                end: span.start,
            });
            let after = self.report.text(Span {
                file: arg.file,
                start: span.end,
                end: arg.end,
            });
            let wraps =
                |text: &str, paren: char| text.chars().all(|c| c == paren || c.is_whitespace());
            if direct.is_none() && wraps(before, '(') && wraps(after, ')') {
                direct = Some(flags.clone());
            } else if flags.contains(&true) {
                opaque = true;
            }
        }
        (direct, opaque)
    }

    /// The function types at the positions of `ty` that hold values: its top, list elements,
    /// the arguments of `Option`, `Result` and kinds, and the fields of kinds, instantiated,
    /// recursively (#350). `seen` stops at a kind already walked.
    /// Each comes with whether it is a kind's field: its type is the field's, instantiated with
    /// the kind's arguments as they unified, not the type of the function value it holds.
    fn function_types(&self, ty: &Ty, out: &mut Vec<(FnTy, bool)>, seen: &mut Vec<KindId>) {
        self.function_types_in(ty, false, out, seen);
    }

    fn function_types_in(
        &self,
        ty: &Ty,
        field: bool,
        out: &mut Vec<(FnTy, bool)>,
        seen: &mut Vec<KindId>,
    ) {
        match ty.unlabeled() {
            Ty::Fn(function) => out.push(((**function).clone(), field)),
            Ty::List(element) | Ty::Dense(_, element) => {
                self.function_types_in(element, field, out, seen);
            }
            Ty::Coll(_, args) => {
                for arg in args.iter() {
                    self.function_types_in(&self.labeled(arg), field, out, seen);
                }
            }
            Ty::Kind(id, args) => {
                for arg in args {
                    self.function_types_in(&self.labeled(arg), field, out, seen);
                }
                if seen.contains(id) {
                    return;
                }
                seen.push(*id);
                if let Some(kind) = self.env.kinds.get(*id) {
                    let fields = kind
                        .record
                        .iter()
                        .flatten()
                        .chain(kind.variants.iter().flat_map(|v| v.fields.iter()));
                    for field in fields {
                        let field = field.ty.base.substitute(args);
                        self.function_types_in(&self.labeled(&field), true, out, seen);
                    }
                }
            }
            _ => {}
        }
    }

    /// U4001 for the arguments of a call, once the whole call is typed so that type variables
    /// have their final labels.
    pub(crate) fn argument_flows(&mut self, callee: &str, checked: Vec<Checked<'a>>) {
        for (span, found, expected, param) in checked {
            self.flow(
                span,
                &found,
                &expected,
                &format!("parameter `{param}` of {callee}"),
            );
        }
    }

    /// `p(v)`, a declassification (`spec/10 §7`): one argument, without a label, that flows
    /// into the policy's `from` type.
    fn call_policy(
        &mut self,
        id: usize,
        def: &FnDef<'a>,
        callee: &str,
        receiver: Option<(Span, Ty)>,
        args: &[Arg],
        span: Span,
    ) -> Ty {
        let from = def.params.first().map(|p| p.ty.clone());
        let expected = from.as_ref().map_or(Ty::error(), |from| from.base.clone());
        // The one argument, or the receiver, is the `from` value.
        self.ordered(span, vec![Some(0)]);
        let origin = def
            .params
            .first()
            .and_then(|p| self.origin(def.module, p.span));
        let mut found = None;
        let value = match (receiver, args) {
            (Some((receiver_span, receiver_ty)), []) => {
                self.unify_at(
                    receiver_span,
                    &expected,
                    &receiver_ty,
                    Role::Argument { param: "", callee },
                );
                found = Some(receiver_ty);
                Some(receiver_span)
            }
            (None, [arg]) => {
                if let Some(label) = &arg.label {
                    let text = self.report.text(arg.value.span);
                    let repair = self.report.edit("remove the label", arg.span, text);
                    let mut diagnostic = self.report.make(
                        Code::U2005,
                        label.span,
                        format!("{callee} takes one argument without a label"),
                    );
                    diagnostic.repairs.push(repair);
                    self.report.push(diagnostic);
                }
                self.ctx.argument = true;
                found = Some(self.expr_as(
                    &arg.value,
                    Some(&expected),
                    Role::Argument { param: "", callee },
                ));
                Some(arg.value.span)
            }
            _ => {
                self.report
                    .error(Code::U2001, span, format!("{callee} takes one argument"));
                self.unchecked(args);
                None
            }
        };
        if let (Some(value), Some(from)) = (value, &from)
            && from.is_refined()
        {
            self.obligation(ObligationKind::Refinement, value, origin);
        }
        // `spec/27 §8` rule 5: the call runs the policy's `via`, whose code the label rules do
        // not check, so the deep label of the `from` value decides its check points, as for
        // any call of unchecked code (#2214).
        if let (Some(value), Some(found)) = (value, &found) {
            let checked = [(value, found.clone(), expected.clone(), "")];
            self.switch_call(span, id, def, &checked);
        }
        if let (Some(value), Some(from), Some(found)) = (value, &from, found) {
            self.flow(
                value,
                &found,
                &from.base,
                &format!("the `from` type of {callee}"),
            );
        }
        // T-Declassify: `pc ⊑ ℓ(to)` (`spec/07 §5` rule 2).
        let to = def.result.base.top_label();
        self.declassify_under_pc(span, callee, &to);
        if let Some(fails) = &def.fails {
            // A failure of the `via` function is released as its value is: it is decided by
            // the `from` value inside the trusted body, so it is visible only where the release
            // is, at `pc ⊔ ℓ(to)` (S-18, ADR 0090). A public `to` keeps a public failure.
            // ADR 0098: when the `via` function's failure reveals nothing (its failure-decision
            // label is public, which is below every `to`), the failure is the call's own, at
            // `pc`. The label is one of the two sound ones, never their meet.
            let target = match def.ast {
                FnAst::Policy(policy) => {
                    let via = self.env.text(def.module, policy.via.name);
                    match self.env.lower(def.module, via) {
                        Some(Entity::Function(target)) => Some((target, via)),
                        _ => None,
                    }
                }
                _ => None,
            };
            let plain = target.is_some_and(|(id, _)| self.env.plain_vias.contains(&id));
            if let Some((id, via)) = target
                && let Some((handler, at)) = self.env.blocked_vias.get(&id)
            {
                self.via_notes.insert(
                    span,
                    (
                        *at,
                        format!(
                            "the `via` `{via}` would have a public failure (ADR 0098), but the `Keys` handler `{handler}` is named here, outside its declaration and {}",
                            self.edition().tests_word()
                        ),
                    ),
                );
            }
            let decided = if plain {
                self.ctx.pc.clone()
            } else {
                self.ctx.pc.join(&to)
            };
            if decided.is_public() {
                self.call_fails.insert(span, fails.clone());
            } else {
                self.call_fails.insert(span, fails.raise_deep(&decided));
                self.decided_failures.insert(span, decided);
            }
        }
        def.result.base.clone()
    }

    /// `@c.op(args)`, typed with the operation of the capability bound to `@c`; an unbound
    /// `@c` is U3001 (M5).
    pub(crate) fn cap_op(
        &mut self,
        cap: &Ident,
        op: &Ident,
        args: &[Arg],
        span: Span,
        site: Site,
    ) -> Ty {
        let cap_name = self.name(cap);
        let op_name = self.name(op);
        self.outside_rl(span, || format!("operation `@{cap_name}.{op_name}`"));
        self.impure(span, || format!("operation `@{cap_name}.{op_name}`"));
        // ADR 0094 rule 2: the label the binding gives the operation's answers, through the
        // rebindings the reference leads through; a handler installed in this body answers at
        // `public` here (rule 4).
        let gives = match self.resolve_cap(cap_name) {
            super::authority::Resolved::Cap { gives, .. } => super::ty::given(&gives, op_name),
            _ => Label::public(),
        };
        if !gives.is_public() && self.pass == super::authority::Pass::Check {
            self.note_answer(super::flow::Answer {
                span,
                binding: cap_name.to_owned(),
                op: op_name.to_owned(),
                label: gives.clone(),
                limited: false,
            });
        }
        let resolved = self.op_site(cap, op);
        // ADR 0123: no operation of `HttpServer` in a branch (U3033, rule 5), and each one is a
        // hand-off site of the serve threads (`L_srv`).
        self.branch_server_op(resolved, cap_name, op_name, span);
        self.server_site(resolved, span);
        // S-39, S-42: the operation's event, and those of a handler that serves it, would show
        // that it ran.
        self.quiet_op_site(span);
        self.switch_site(span, &Label::public(), || {
            format!("whether this operation `@{cap_name}.{op_name}` runs")
        });
        if let Some(id) = resolved {
            self.timing_op(id, op_name);
            // The edge of ADR 0116 P1 from the node whose code holds the operation, a lambda's
            // included, to the operation: every operation call, whatever `uses` names (#1148).
            self.graph_op(id, op_name, span);
        }
        let capability = resolved
            .and_then(|id| self.env.capabilities.get(id))
            .cloned();
        let Some(capability) = capability else {
            // In pass 1, inference decides the capability of an inferred reference; its
            // failure is resolved then.
            if self.base == super::authority::Base::Inferring
                && let super::authority::Resolved::Missing { name, .. } = self.resolve_cap(cap_name)
            {
                self.pending_ops.insert(span, (name, op_name.to_owned()));
            }
            // The binding names a capability that does not resolve (U1001), or no binding
            // holds `@cap` (U3001): either way the package is rejected, and what this unknown
            // operation leaves unsolved is not reported (#581).
            self.unchecked_unknown(args);
            return Ty::unknown();
        };
        // An operation of the host's `Http`, whose timing `L_lim` covers (ADR 0117 IF4).
        if capability.module == self.env.prelude && capability.name == "Http" {
            self.http_site(span);
        }
        let Some(def) = capability.ops.iter().find(|o| o.name == op_name).cloned() else {
            let names: Vec<&str> = capability.ops.iter().map(|o| o.name).collect();
            let mut diagnostic = self.report.make(
                Code::U1001,
                op.span,
                format!(
                    "capability `{}` has no operation `{op_name}`",
                    self.env.cap_def_name(&capability)
                ),
            );
            if let Some(close) = closest(op_name, names) {
                diagnostic.repairs.push(self.report.edit(
                    format!("write `{close}`"),
                    op.span,
                    close,
                ));
            }
            self.report.push(diagnostic);
            self.unchecked(args);
            return Ty::error();
        };
        if let Some((capability_id, _)) = self.op_index(cap_name, op_name) {
            self.resolved(
                span,
                Res::Op {
                    capability: capability_id,
                },
            );
        }
        // ADR 0092 rule 3: the operation runs where its binding's condition holds (#615).
        let level = self.frames.len();
        self.condition_site(cap_name, &[op_name.to_owned()], span, level);
        let callee = format!(
            "operation `{}.{op_name}`",
            self.env.cap_def_name(&capability)
        );
        let checked = self.arguments(
            &callee,
            &def.params,
            &[],
            None,
            args,
            span,
            capability.module,
        );
        // An operation's arguments are sinks bounded by `accepts`, except
        // the carried ones, which accept every label and raise the result (ADR 0039).
        let mut carried = Label::public();
        // The join of the labels of the arguments that decide whether a host operation fails,
        // and how.
        let mut decided = Label::public();
        let host = capability.module == self.env.prelude;
        // S-14 (ADR 0085): what a `Files.write` writes, and where, decides the outcome of every
        // later `Files` operation, in this run or a later one.
        let file_write = host && capability.name == "Files" && op_name == "write";
        if file_write && self.pass == super::authority::Pass::Collect {
            self.env.files_written = true;
        }
        let mut written = Label::public();
        let mut sinks: Vec<(Span, Ty)> = Vec::new();
        for (arg_span, found, expected, param) in checked {
            if host && decides_failure(capability.name, op_name, param) {
                // Capped at what the operation accepts: an argument above it is U4003 already,
                // so `Keys.hmac` and `Files.read`, which accept only `public`, are unchanged.
                // Every argument of `Http` counts, the values of its headers included, since
                // the server that receives them may decide the failure (S-15, ADR 0085).
                let accepted = def.accepts.clone().unwrap_or_else(Label::public);
                decided = decided.join(&self.deep(&found).meet(&accepted));
            }
            // Where a write lands decides which files exist; what it writes decides nothing
            // later operations observe but resource consumption, outside the model
            // (`spec/07 §1`), and a later read returns it `internal` anyway.
            if file_write && param == "path" {
                written = written.join(&self.deep(&found));
            }
            // ADR 0050: the argument flows into the declared parameter type at its
            // contravariant positions and invariant kind arguments, which a handler body
            // assumes; `accepts` bounds its data.
            // The program cannot raise the label of a host operation's parameter (#404).
            let advice = host.then(|| {
                format!(
                    "a parameter of {callee}, an operation of a host capability, keeps its declared label: lower this value through a `policy`"
                )
            });
            self.op_argument_flow(
                arg_span,
                &found,
                &expected,
                &format!("parameter `{param}` of {callee}"),
                advice,
            );
            let is_carried = def
                .carried
                .iter()
                .any(|&i| def.params.get(i).is_some_and(|p| p.name == param));
            if is_carried {
                carried = carried.join(&self.deep(&found));
            } else {
                sinks.push((arg_span, found));
            }
        }
        self.sink(
            span,
            &format!("`@{cap_name}.{op_name}`"),
            &def.accepts,
            &sinks,
        );
        if def.requires {
            self.obligation(ObligationKind::Precondition, span, None);
        }
        if host && capability.name == "Files" {
            if file_write && self.pass == super::authority::Pass::Check {
                // The path and the program counter; the highest program counter the code can be
                // entered under joins them once every call is known (`file_state`). In a body no
                // label rule checks, a policy's `via` function, the labels are unknown: the write
                // counts at the cap.
                let cap = def.accepts.clone().unwrap_or_else(Label::public);
                self.env.file_cap = cap.clone();
                let entry = self.entry();
                let site = if self.labels() {
                    written.join(&self.ctx.pc)
                } else {
                    cap
                };
                self.env.file_writes.push((entry, site, span));
            }
            decided = decided.join(&self.env.file_state);
        }
        // ADR 0094 rule 2: whether the operation fails, and its payload, carry the label the
        // binding gives it, as the value does below; the decision on the outcome then runs
        // under the label (`spec/07 §3` rule 4).
        decided = decided.join(&gives);
        // S-46 (#1563), `spec/27 §10` rule 4 (ADR 0117 IF5): `Limited` is labeled `internal`.
        // Whether `acquire` of `std.limit` finds a token depends on the timing of all the code
        // that shares the instance, hidden code included, so whether it fails, and how, is
        // `internal` for the program, whatever handler serves it. O(1) per operation site.
        if self.is_limit(&capability) {
            let limited = Label {
                level: una_syntax::ast::Level::Internal,
                ..Label::public()
            };
            decided = decided.join(&limited);
            if self.pass == super::authority::Pass::Check {
                self.note_answer(super::flow::Answer {
                    span,
                    binding: cap_name.to_owned(),
                    op: op_name.to_owned(),
                    label: limited,
                    limited: true,
                });
            }
        }
        // ADR 0123 IF3 (`spec/31 §1`): whether `reply` fails also depends on which reply rights
        // the task holds, which the program's order of `next`, `start` and `reply` decides, so
        // its failure is labeled `internal` at least, as `Http`'s are. O(1) per operation site.
        if host && capability.name == "HttpServer" && op_name == "reply" {
            decided = decided.join(&Label {
                level: una_syntax::ast::Level::Internal,
                ..Label::public()
            });
        }
        if let Some(fails) = &def.fails {
            // Whether a host operation fails, and how, is labeled like the arguments that
            // decide it, as a prelude function's failure is (`spec/07 §7`): its
            // payload too, and a handling clause branches on that label.
            if decided.is_public() {
                self.call_fails.insert(span, fails.clone());
            } else {
                self.call_fails.insert(span, fails.raise_deep(&decided));
                self.decided_failures.insert(span, decided.clone());
            }
        }
        let what = format!("`@{cap_name}.{op_name}`");
        self.position_site(
            &what,
            span,
            true,
            def.fails.as_ref(),
            site.top,
            site.handled,
        );
        let result = def.result.base.with_label(&carried);
        // ADR 0094 rule 2: the answer carries the label at every position of its type.
        let result = if gives.is_public() {
            result
        } else {
            result.raise_deep(&gives)
        };
        // ADR 0116 amendment of 2026-11-09 (#898), C1: in edition 0.4 a reading of the host's
        // `Clock` carries `L_clk`, the timing label of the code that ran before it, which the
        // probe of pass 2 computes over the whole package; a site absent there is `public`
        // (the first probe), and the label is confirmed after pass 2. O(log n) per site.
        let result = if host
            && capability.name == "Clock"
            && op_name == "now"
            && self.edition() >= Edition::V0_4
        {
            self.clock_site(span);
            match self.env.clock_timing.get(&span) {
                Some(label) if !label.is_public() => result.raise_deep(label),
                _ => result,
            }
        } else {
            result
        };
        // ADR 0123 IF6 (#1674): the answer is data the code holds, bound or not.
        let answer = self.deep(&result);
        self.reach_label(&answer);
        if self.pass == super::authority::Pass::Check {
            self.outcome_site(span, &capability, &decided, def.fails.as_ref(), &result);
        }
        result
    }

    /// S-45 (#1552, `spec/11 §5`): records the label that the outcome of the operation at
    /// `span` is rendered at in a run's trace, `L_out`, for elaboration to carry into Core IR:
    /// the label of the decision whether it fails and of its payload, `decided`, joined with
    /// the labels of its failure type and of its result. `acquire` of `std.limit` adds
    /// `internal`, the label of `Limited` (`spec/27 §10` rule 4), and a body that no label rule
    /// checks counts as `⊤`. The parts are kept as types, so the labels of unification
    /// variables resolve with the declaration's records. O(1) per operation site.
    fn outcome_site(
        &mut self,
        span: Span,
        capability: &super::env::CapDef<'_>,
        decided: &Label,
        fails: Option<&Ty>,
        result: &Ty,
    ) {
        let mut seed = decided.clone();
        // Since S-46 (#1563) `decided` already holds `internal` for `acquire`; this join stays
        // as the trace's fail-closed backstop, independent of the program's label rule.
        let limit = self
            .env
            .limiter
            .and_then(|id| self.env.handlers.get(id))
            .is_some_and(|limiter| limiter.module == capability.module);
        if limit {
            seed = seed.join(&Label {
                level: una_syntax::ast::Level::Internal,
                ..Label::public()
            });
        }
        if !self.ctx.labels {
            seed = Label::top(&self.env.categories);
        }
        let mut parts = vec![Ty::Unit.with_label(&seed), result.clone()];
        if let Some(fails) = fails {
            parts.push(fails.raise_deep(decided));
        }
        self.records.outcomes.entry(span).or_default().extend(parts);
    }

    /// Whether `capability` is `Limit` of `std.limit` (`spec/27 §10`): the module that declares
    /// it also holds the handler `limiter`, which is in scope only when a package of edition
    /// 0.4 imports the module.
    fn is_limit(&self, capability: &super::env::CapDef<'_>) -> bool {
        capability.name == "Limit"
            && self
                .env
                .limiter
                .and_then(|id| self.env.handlers.get(id))
                .is_some_and(|limiter| limiter.module == capability.module)
    }

    /// The capability bound to `@cap` and the position of its operation `op`.
    fn op_index(&self, cap: &str, op: &str) -> Option<(usize, usize)> {
        let capability = match self.resolve_cap(cap) {
            super::authority::Resolved::Cap { capability, .. }
            | super::authority::Resolved::Handler { capability, .. } => capability?,
            super::authority::Resolved::Missing { .. } => return None,
        };
        let index = self
            .env
            .capabilities
            .get(capability)?
            .ops
            .iter()
            .position(|o| o.name == op)?;
        Some((capability, index))
    }

    /// The parameter an unlabeled argument names by punning (`spec/04 §3` rule 3): a bare name
    /// equal to a parameter's name counts as labeled with it. In a call, it applies from the
    /// second argument on (ADR 0049).
    pub(crate) fn punned(&self, arg: &Arg, names: &[&str]) -> Option<usize> {
        if arg.label.is_some() {
            return None;
        }
        let ExprKind::Name(ident) = &arg.value.kind else {
            return None;
        };
        let name = self.name(ident);
        names.iter().position(|candidate| *candidate == name)
    }

    /// `keep`'s expected type `fn(T) -> Bool` with the result a fresh variable bound to
    /// `Bool`, whose label rises with the predicate's result (ADR 0067 L4).
    fn labeled_predicate(&mut self, expected: Ty, span: Span) -> Ty {
        let Ty::Fn(function) = expected else {
            return expected;
        };
        let result = self.vars.fresh(
            Origin::Instance {
                param: "Bool".to_owned(),
                of: "keep".to_owned(),
            },
            span,
        );
        self.vars.unify(&result, &function.result);
        let mut function = (*function).clone();
        function.result = result;
        Ty::Fn(Shared::new(function))
    }

    /// The arguments of a call of a function, operation or handler (`spec/04 §3`): the first
    /// may go unlabeled, the others are labeled or punned (U2004); an unknown label is U2005,
    /// a repeated parameter U2006, and a wrong number of arguments U2001.
    /// Each argument is checked against its parameter; a refined parameter records a pending
    /// obligation.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn arguments(
        &mut self,
        callee: &str,
        params: &[ParamDef<'a>],
        subst: &[Ty],
        receiver: Option<(Span, Ty)>,
        args: &[Arg],
        span: Span,
        module: usize,
    ) -> Vec<Checked<'a>> {
        let mut checked: Vec<Checked<'a>> = Vec::new();
        let of_callee = self.of_callee.take();
        // The elements of a list literal passed as arguments bind consecutive parameters
        // (`spec/29 §17`): the position of an argument among them, and its parameter.
        let spread = self.spread.take();
        let spread_target = |index: usize| {
            let (first, count, param) = spread?;
            let k = index.checked_sub(first).filter(|k| *k < count)?;
            Some(param + k)
        };
        // A parameter of the prelude written `with_`, `and_` or `from_` takes the label without
        // the underscore: the keywords `with`, `and` and `from` label the arguments of the dense
        // methods (`spec/29 §17`, ADR 0120), and cannot name a parameter.
        let names: Vec<&str> = params
            .iter()
            .map(|p| match p.name {
                "with_" if module == self.env.prelude => "with",
                "and_" if module == self.env.prelude => "and",
                "from_" if module == self.env.prelude => "from",
                name => name,
            })
            .collect();
        let offset = usize::from(receiver.is_some());
        let given_count = args.len() + offset;
        if given_count > params.len() {
            self.report.error(
                Code::U2001,
                span,
                format!(
                    "{callee} takes {}, but {} given",
                    count(params.len(), "argument"),
                    given(given_count)
                ),
            );
            self.unchecked(args);
            return checked;
        }
        let receiver_info = self.receiver_marker.take().filter(|_| receiver.is_some());
        // The `edit` places get their new values once every argument is typed (#186).
        let mut flow_backs = Vec::new();
        if let (Some((receiver_span, receiver_ty)), Some(first)) = (&receiver, params.first()) {
            let expected = first.ty.base.substitute(subst);
            let role = Role::Argument {
                param: first.name,
                callee,
            };
            self.unify_at(*receiver_span, &expected, receiver_ty, role);
            // The receiver's marker against the first parameter's convention (`spec/17 §3`).
            if let Some((marker, place)) = &receiver_info {
                match (first.convention, *marker) {
                    (None, None) => {}
                    (Some(Convention::Edit), Some(Convention::Edit)) => {
                        flow_backs.push(super::memory::FlowBack {
                            place: place.clone(),
                            place_ty: receiver_ty.clone(),
                            value: expected.clone(),
                            span: *receiver_span,
                        });
                    }
                    (Some(convention), None) => self.report.error(
                        Code::U2020,
                        *receiver_span,
                        format!(
                            "parameter `{}` of {callee} is declared `{}`, so the call is written `{} {}.{}`",
                            first.name,
                            convention.as_str(),
                            convention.as_str(),
                            self.report.text(*receiver_span),
                            callee.trim_matches('`')
                        ),
                    ),
                    (_, Some(marker)) => self.report.error(
                        Code::U2020,
                        *receiver_span,
                        format!(
                            "parameter `{}` of {callee} does not take its argument by `{}`",
                            first.name,
                            marker.as_str()
                        ),
                    ),
                }
            }
            self.fn_flow(*receiver_span, &expected, receiver_ty);
            // A function value as the receiver of a call into a boundary: fail closed (ADR 0093
            // rule 8), as for any argument whose code is not known here.
            if self.crossing_arguments(span)
                && (self.holds_function(&expected) || self.holds_function(receiver_ty))
            {
                self.effectful_value(*receiver_span, &[&expected, receiver_ty]);
                self.opaque_value(*receiver_span, None);
            }
            checked.push((*receiver_span, receiver_ty.clone(), expected, first.name));
            if first.ty.is_refined() {
                let origin = self.origin(module, first.span);
                self.obligation(ObligationKind::Refinement, *receiver_span, origin);
            }
        }
        let mut taken: Vec<bool> = params.iter().map(|_| false).collect();
        if let Some(first) = taken.first_mut() {
            *first = receiver.is_some();
        }
        // R-030: a repair of an unknown label suggests only a parameter that no other
        // argument gives, so applying it cannot make the argument a duplicate (U2006).
        let mut given_params = taken.clone();
        for (index, arg) in args.iter().enumerate() {
            let target = match &arg.label {
                _ if spread_target(index).is_some() => spread_target(index),
                Some(label) => names.iter().position(|name| *name == self.name(label)),
                // ADR 0049: an unlabeled first argument binds the first parameter, and
                // punning applies only from the second argument on.
                None if index + offset == 0 => Some(0),
                None => self.punned(arg, &names),
            };
            if let Some(slot) = target.and_then(|target| given_params.get_mut(target)) {
                *slot = true;
            }
        }
        let open: Vec<&str> = names
            .iter()
            .zip(&given_params)
            .filter(|(_, given)| !**given)
            .map(|(name, _)| *name)
            .collect();
        let mut targets: Vec<Option<usize>> = Vec::new();
        let mut problem = false;
        for (index, arg) in args.iter().enumerate() {
            let position = index + offset;
            let target = if let Some(target) = spread_target(index) {
                Some(target)
            } else if let Some(label) = &arg.label {
                let label_name = self.name(label);
                match names.iter().position(|name| *name == label_name) {
                    Some(target) => Some(target),
                    None => {
                        self.unknown_label(callee, &open, label);
                        None
                    }
                }
            } else if position == 0 {
                // ADR 0049 (`spec/04 §3` rule 2): the first argument is positional.
                Some(0)
            } else if let Some(target) = self.punned(arg, &names) {
                Some(target)
            } else {
                self.missing_label(callee, &names, arg, position);
                None
            };
            let target = match target {
                Some(target) if taken.get(target) == Some(&true) => {
                    let name = names.get(target).copied().unwrap_or("?");
                    self.report.error(
                        Code::U2006,
                        arg.span,
                        format!("parameter `{name}` of {callee} is given twice"),
                    );
                    None
                }
                other => other,
            };
            if let Some(target) = target
                && let Some(slot) = taken.get_mut(target)
            {
                *slot = true;
            }
            problem |= target.is_none();
            targets.push(target);
        }
        // The parameter of each argument, the receiver first.
        let order: Vec<Option<usize>> = receiver
            .as_ref()
            .map(|_| Some(0))
            .into_iter()
            .chain(targets.iter().copied())
            .collect();
        self.ordered(span, order);
        // Lambdas are checked last, so the other arguments fix the types their parameters
        // take (`spec/03 §7`).
        for (arg, target) in lambdas_last(args, &targets) {
            let Some(param) = target.and_then(|target| params.get(target)) else {
                self.expr(&arg.value, None);
                continue;
            };
            let expected = param.ty.base.substitute(subst);
            // ADR 0067 L4: in edition 0.2, whose prelude gives `filter` and `first_where` an
            // `of` clause, the predicate's result is a variable bound to `Bool` that takes the
            // predicate's labels, which then raise the result's top label (`spec/14 §1`).
            let expected = if param.name == "keep"
                && matches!(
                    of_callee.as_ref().and_then(|of| of.prelude.as_deref()),
                    Some("filter" | "first_where")
                ) {
                self.labeled_predicate(expected, span)
            } else {
                expected
            };
            let role = Role::Argument {
                param: names
                    .get(target.unwrap_or(0))
                    .copied()
                    .unwrap_or(param.name),
                callee,
            };
            self.ctx.argument = true;
            self.prelude_argument = module == self.env.prelude;
            let slot = target.and_then(|target| {
                of_callee
                    .as_ref()
                    .and_then(|of| self.of_slot(of, target, &checked, params))
            });
            let of_kind = slot.as_ref().and_then(|slot| slot.kind.clone());
            let of_uses = slot.as_ref().is_some_and(|slot| slot.uses);
            let of_fails = slot.as_ref().is_some_and(|slot| slot.fails);
            let raise = slot.as_ref().map(|slot| slot.raise.clone());
            let by_result = slot.as_ref().is_some_and(|slot| slot.by_result);
            let is_slot = slot.is_some();
            self.of_argument = slot;
            // ADR 0093 rules 5 and 8: an argument of a call into a boundary that is a named
            // function grants the boundary what the function uses and runs; a lambda grants
            // what its body does, collected as it is checked.
            let crossing = self.crossing_arguments(span);
            let named = crossing && self.named_function(&arg.value);
            self.granting_direct = named;
            let mut found = self.expr_as(&arg.value, Some(&expected), role);
            self.prelude_argument = false;
            if is_slot {
                // Not consumed by a lambda or a parameter passed on: a function value, whose
                // effects and failures the callee performs at this argument.
                if self.of_argument.take().is_some() {
                    let named_value = self.named_function(&arg.value);
                    found = self.of_function_value(
                        arg.value.span,
                        &found,
                        named_value,
                        of_uses,
                        of_fails,
                        of_kind.as_ref(),
                        &raise.unwrap_or_else(Label::public),
                        by_result,
                    );
                }
            }
            self.granting_direct = false;
            // Any other function value given to the boundary, from a parameter, a local or a
            // computed value, has code that is not known here: fail closed (rule 8).
            if crossing
                && !named
                && !matches!(arg.value.kind, ExprKind::Lambda { .. })
                && (self.holds_function(&expected) || self.holds_function(&found))
            {
                let of_param = match &arg.value.kind {
                    ExprKind::Name(ident) => {
                        let name = self.name(ident);
                        self.of_param(name).map(|_| name.to_owned())
                    }
                    _ => None,
                };
                if of_param.is_none() {
                    self.effectful_value(arg.value.span, &[&expected, &found]);
                }
                self.opaque_value(arg.value.span, of_param);
            }
            flow_backs.extend(self.convention_arg(callee, param, arg, &found, &expected));
            self.fn_flow(arg.value.span, &expected, &found);
            if param.ty.is_refined() {
                let origin = self.origin(module, param.span);
                self.obligation(ObligationKind::Refinement, arg.value.span, origin);
            }
            checked.push((arg.value.span, found, expected, param.name));
        }
        // Every argument is typed, so the parameters' types have their final labels: a later
        // argument may raise the label of a type variable that an `edit` place takes back.
        self.flow_back(flow_backs);
        // Exclusivity (`spec/17 §3` rule 4): an `edit` argument and any other use of its
        // binding in the same call, the receiver included.
        let mut edits: Vec<(usize, &Expr)> = args
            .iter()
            .enumerate()
            .filter(|(_, arg)| arg.marker == Some(Convention::Edit))
            .map(|(index, arg)| (index + 1, &arg.value))
            .collect();
        let mut uses: Vec<(usize, &Expr)> = args
            .iter()
            .enumerate()
            .map(|(index, arg)| (index + 1, &arg.value))
            .collect();
        if let Some((marker, place)) = &receiver_info {
            if *marker == Some(Convention::Edit) {
                edits.push((0, place));
            }
            uses.push((0, place));
        }
        if !edits.is_empty() {
            self.exclusive(&edits, &uses);
        }
        if !problem {
            let missing: Vec<String> = params
                .iter()
                .zip(&taken)
                .filter(|(_, taken)| !**taken)
                .map(|(param, _)| format!("`{}`", param.name))
                .collect();
            if !missing.is_empty() {
                let what = if missing.len() == 1 {
                    "argument"
                } else {
                    "arguments"
                };
                self.report.error(
                    Code::U2001,
                    span,
                    format!("{callee} is missing {what} {}", missing.join(", ")),
                );
            }
        }
        checked
    }

    /// U2005: a label that names no parameter; `names` are the parameters that no other
    /// argument gives, which the repair may suggest.
    pub(crate) fn unknown_label(&mut self, callee: &str, names: &[&str], label: &Ident) {
        let label_name = self.name(label);
        let mut diagnostic = self.report.make(
            Code::U2005,
            label.span,
            format!("{callee} has no parameter `{label_name}`"),
        );
        match closest(label_name, names.iter().copied()) {
            Some(close) => diagnostic.repairs.push(self.report.edit(
                format!("write `{close}`"),
                label.span,
                close,
            )),
            None if names.is_empty() => diagnostic.repairs.push(Report::advice(
                "every parameter is given by another argument; remove this one",
            )),
            None => {
                let listed: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
                diagnostic.repairs.push(Report::advice(format!(
                    "the parameters that no other argument gives are {}",
                    listed.join(", ")
                )));
            }
        }
        self.report.push(diagnostic);
    }

    /// U2004: an argument after the first without a label, with an edit that labels it with
    /// the parameter at its position.
    pub(crate) fn missing_label(
        &mut self,
        callee: &str,
        names: &[&str],
        arg: &Arg,
        position: usize,
    ) {
        let mut diagnostic = self.report.make(
            Code::U2004,
            arg.span,
            format!(
                "argument {} of {callee} needs a label; only the first argument may go without one",
                position + 1
            ),
        );
        if let Some(name) = names.get(position) {
            let text = self.report.text(arg.span);
            diagnostic.repairs.push(self.report.edit(
                format!("label it `{name}`"),
                arg.span,
                format!("{name}: {text}"),
            ));
        }
        self.report.push(diagnostic);
    }

    /// `Name(args)`: a variant or record constructor.
    fn constructor_call(
        &mut self,
        ident: &Ident,
        args: &[Arg],
        span: Span,
        expected: Option<&Ty>,
    ) -> Ty {
        let name = self.name(ident);
        self.outside_rl(span, || format!("constructor `{name}`"));
        // ADR 0120 rules 3 and 38: `Matrix`, `Tensor` and `Lu` are types without a
        // constructor; an `Lu[T]` in particular comes only from `lu()`, so none is forged.
        if name != "Array"
            && let Some(dense) = self.env.dense_name(self.module, name)
        {
            let made_by = match dense {
                super::ty::Dense::Lu => "a factorization comes from `m.lu()`",
                super::ty::Dense::Tensor => {
                    "a tensor is made by `tensor(...)` or `tensor_filled(...)`"
                }
                _ => {
                    "a matrix is made by `matrix(...)`, `matrix_filled(...)` or `matrix_diagonal(...)`"
                }
            };
            self.report.error(
                Code::U2001,
                ident.span,
                format!("`{name}` is a built-in type with no constructor: {made_by}"),
            );
            self.unchecked(args);
            return Ty::error();
        }
        // ADR 0120 rule 3: `Array(...)` where an array is expected is the variant of `Json`.
        if let Some(repair) = self.json_array_repair(expected, name) {
            let mut diagnostic = self.report.make(
                Code::U2001,
                span,
                "`Array(...)` makes the variant `Json.Array`, and an `Array[T]` is expected here",
            );
            diagnostic.repairs.push(super::Report::advice(repair));
            self.report.push(diagnostic);
            self.unchecked(args);
            return Ty::error();
        }
        if self.type_params.contains(&name) {
            self.report.error(
                Code::U2001,
                ident.span,
                format!("type parameter `{name}` is not a constructor"),
            );
            self.unchecked(args);
            return Ty::error();
        }
        // ADR 0120 rule 3: `Decimal(...)` is the `Json` variant, never the decimal type.
        if name == "Decimal"
            && let Some(scale) = expected.and_then(|e| self.decimal_scale(e))
        {
            let mut diagnostic = self.report.make(
                Code::U2001,
                span,
                format!(
                    "`Decimal(...)` is the `Json` variant, not a `Decimal[{scale}]`; a decimal is written as a literal, such as `1.50`, or read with `parse_decimal(t, scale: {scale})`"
                ),
            );
            diagnostic.repairs.push(super::Report::advice(format!(
                "write the value as a decimal literal, or `parse_decimal(t, scale: {scale})` for a text"
            )));
            self.report.push(diagnostic);
            self.unchecked(args);
            return Ty::Decimal(scale);
        }
        let problem = match self.choose_upper(ident, expected) {
            Some(Entity::Variant(kind, index)) => {
                let (params, kind_name) = self.kind_params(kind);
                let targs = self.instantiate(&params, kind_name, ident.span);
                self.expected_invariant(kind, &targs, expected);
                self.resolved(
                    span,
                    Res::Ctor {
                        kind,
                        variant: Some(index),
                    },
                );
                let hint = self.expected_args(kind, expected);
                let label = self.construct(
                    kind,
                    Some(index),
                    &targs,
                    hint.as_deref(),
                    args,
                    ident.span,
                    span,
                    true,
                );
                let targs = self.invariant_base(kind, targs, hint.is_some());
                return Ty::Kind(kind, targs.into()).with_label(&label);
            }
            Some(Entity::Kind(id)) if self.env.kinds.get(id).is_some_and(|k| !k.is_sum()) => {
                let (params, kind_name) = self.kind_params(id);
                let targs = self.instantiate(&params, kind_name, ident.span);
                self.expected_invariant(id, &targs, expected);
                self.resolved(
                    span,
                    Res::Ctor {
                        kind: id,
                        variant: None,
                    },
                );
                let hint = self.expected_args(id, expected);
                let label = self.construct(
                    id,
                    None,
                    &targs,
                    hint.as_deref(),
                    args,
                    ident.span,
                    span,
                    true,
                );
                let targs = self.invariant_base(id, targs, hint.is_some());
                return Ty::Kind(id, targs.into()).with_label(&label);
            }
            Some(Entity::Kind(_)) => {
                format!("`{name}` is a sum kind; construct one of its variants")
            }
            Some(Entity::Alias(_) | Entity::Builtin(_)) => {
                format!("`{name}` is a type, not a constructor")
            }
            Some(Entity::Capability(_)) => format!("capability `{name}` is not a value"),
            Some(Entity::Unit(_)) => {
                format!("`{name}` is a unit, not a constructor; a value takes it with `measured()`")
            }
            Some(Entity::Constant { .. } | Entity::Const(_)) => {
                format!("`{name}` is a constant, not a constructor")
            }
            Some(Entity::Function(_) | Entity::Handler(_)) => {
                self.unchecked(args);
                return Ty::error();
            }
            // An unknown or ambiguous name: U1001 (#581).
            None => {
                self.unchecked_unknown(args);
                return Ty::unknown();
            }
        };
        self.report.error(Code::U2001, ident.span, problem);
        self.unchecked(args);
        Ty::error()
    }

    /// `ty` with each variable whose value now holds a refinement replaced by that value's
    /// base (ADR 0103 rule 5). Every other variable stays a variable, so the labels that later
    /// unifications join into it still reach this type; a type without a refinement, in any
    /// edition before `0.3` always, is returned as it is.
    fn refined_vars_at_base(&self, ty: &Ty) -> Ty {
        if self.edition() < Edition::V0_3 || !self.vars.resolve_shape(ty).flags().refined {
            return ty.clone();
        }
        match ty {
            Ty::Var(_) => self.vars.resolve_shape(ty).without_refinements(),
            Ty::Refined(_, inner) => self.refined_vars_at_base(inner),
            Ty::Labeled(label, inner) => self.refined_vars_at_base(inner).with_label(label),
            Ty::List(element) => Ty::List(Shared::new(self.refined_vars_at_base(element))),
            Ty::Dense(dense, element) => {
                Ty::Dense(*dense, Shared::new(self.refined_vars_at_base(element)))
            }
            Ty::Map(key, value) => Ty::Map(
                Shared::new(self.refined_vars_at_base(key)),
                Shared::new(self.refined_vars_at_base(value)),
            ),
            Ty::Kind(id, args) => Ty::Kind(
                *id,
                args.iter()
                    .map(|arg| self.refined_vars_at_base(arg))
                    .collect(),
            ),
            Ty::Coll(coll, args) => Ty::Coll(
                *coll,
                args.iter()
                    .map(|arg| self.refined_vars_at_base(arg))
                    .collect(),
            ),
            Ty::Fn(function) => Ty::Fn(Shared::new(FnTy {
                params: function
                    .params
                    .iter()
                    .map(|param| self.refined_vars_at_base(param))
                    .collect(),
                result: self.refined_vars_at_base(&function.result),
                ..(**function).clone()
            })),
            other => other.without_refinements(),
        }
    }

    /// ADR 0103 rule 5: without an expected type, a type argument of `kind` at an invariant
    /// parameter (ADR 0046) keeps no inferred refinement, so it never meets an invariant flow.
    fn invariant_base(&self, kind: KindId, targs: Vec<Ty>, expected: bool) -> Vec<Ty> {
        if expected {
            return targs;
        }
        targs
            .into_iter()
            .enumerate()
            .map(|(index, arg)| {
                if self.env.kind_labels.is_invariant(kind, index) {
                    self.refined_vars_at_base(&arg)
                } else {
                    arg
                }
            })
            .collect()
    }

    /// The type arguments `targs` of a constructor of `kind` take the arguments of the
    /// expected type at the kind's invariant parameters, labels included, before the fields
    /// are checked, so that `Feed(get: fn() -> 2)` is a `Feed[secret Int]` where one is
    /// expected (ADR 0046). An expected argument that still holds a type variable is left to
    /// the flow that follows the constructor, since the variable may take labels later.
    /// D-163: ADR 0046 fixes the flow, not how a constructor infers its
    /// arguments; this uses the expected type only where it is already known.
    fn expected_invariant(&mut self, kind: KindId, targs: &[Ty], expected: Option<&Ty>) {
        let Some(expected) = expected else {
            return;
        };
        let Ty::Kind(id, args) = self.labeled(expected).unlabeled().clone() else {
            return;
        };
        if id != kind {
            return;
        }
        let kinds = self.env.kind_labels.clone();
        for (index, (targ, arg)) in targs.iter().zip(&args).enumerate() {
            if kinds.is_invariant(kind, index) && !holds_var(arg) {
                self.vars.unify(targ, arg);
            }
        }
    }

    /// Whether `expr` is an integer literal, possibly negated, a constructor application or a
    /// list literal, whose type an expected type decides.
    fn literal_or_constructor(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Paren(inner) => Self::literal_or_constructor(inner),
            ExprKind::Literal(_) | ExprKind::Unary { .. } => {
                super::fixed::literal_value(expr).is_some()
            }
            ExprKind::Call { callee, .. } => matches!(callee.kind, ExprKind::UpperName(_)),
            // ADR 0103 rules 3 and 5: `Box(v: [5])` where a `Box[List[Pos]]` is expected is a
            // literal at `List[Pos]`, one obligation per element (#1657).
            ExprKind::List(_) => true,
            _ => false,
        }
    }

    /// The type arguments of `expected` when it is a type of `kind` (#187).
    fn expected_args(&self, kind: KindId, expected: Option<&Ty>) -> Option<Vec<Ty>> {
        match self.labeled(expected?).unlabeled() {
            Ty::Kind(id, args) if *id == kind => Some(args.to_vec()),
            _ => None,
        }
    }

    /// A constructor application, also after `fail` (`spec/04 §3` rule 4): with two or more
    /// fields every argument is labeled or punned, in any order, and one field may go without
    /// a label. Missing and unknown fields are U2009, a repeated field U2006, and an unlabeled
    /// argument where labels are required U2004. `span` covers the name and
    /// the arguments, `name` only the name.
    ///
    /// Returns the label of the constructed value's top position: `public`, except for a
    /// variant of the prelude kind `Json` in edition `0.4` and later when `labeled` is true,
    /// whose arguments may carry any label and whose result carries the join of every label of
    /// every argument (ADR 0073, accepted amendment of 2026-11-09 to J5, #917).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn construct(
        &mut self,
        kind: KindId,
        index: Option<usize>,
        targs: &[Ty],
        hint: Option<&[Ty]>,
        args: &[Arg],
        name: Span,
        span: Span,
        labeled: bool,
    ) -> Label {
        let Some(def) = self.env.kinds.get(kind) else {
            return Label::public();
        };
        let module = def.module;
        let labeled_json = labeled
            && self.edition() >= Edition::V0_4
            && module == self.env.prelude
            && def.name == "Json";
        let (fields, constructor): (Vec<FieldDef<'a>>, &'a str) = match index {
            Some(index) => match def.variants.get(index) {
                Some(variant) => (variant.fields.clone(), variant.name),
                None => return Label::public(),
            },
            None => (def.record.clone().unwrap_or_default(), def.name),
        };
        if fields.is_empty() {
            if span != name {
                let mut diagnostic = self.report.make(
                    Code::U2009,
                    name,
                    format!("variant `{constructor}` has no fields; write `{constructor}`"),
                );
                diagnostic.repairs.push(self.report.edit(
                    format!("write `{constructor}`"),
                    span,
                    constructor,
                ));
                self.report.push(diagnostic);
            }
            self.unchecked(args);
            return Label::public();
        }
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        let mut taken: Vec<bool> = fields.iter().map(|_| false).collect();
        // R-030: the repair of an unknown field suggests only a field that no other argument
        // gives, so applying it cannot make the argument a duplicate (U2006; D-157).
        let mut given_fields = taken.clone();
        for (position, arg) in args.iter().enumerate() {
            let target = match &arg.label {
                Some(label) => names.iter().position(|n| *n == self.name(label)),
                None => self
                    .punned(arg, &names)
                    .or((fields.len() == 1 && position == 0).then_some(0)),
            };
            if let Some(slot) = target.and_then(|target| given_fields.get_mut(target)) {
                *slot = true;
            }
        }
        let open: Vec<&str> = names
            .iter()
            .zip(&given_fields)
            .filter(|(_, given)| !**given)
            .map(|(name, _)| *name)
            .collect();
        let mut targets: Vec<Option<usize>> = Vec::new();
        let mut problem = false;
        for (position, arg) in args.iter().enumerate() {
            let target = if let Some(label) = &arg.label {
                let label_name = self.name(label);
                let found = names.iter().position(|n| *n == label_name);
                if found.is_none() {
                    let mut diagnostic = self.report.make(
                        Code::U2009,
                        label.span,
                        format!("`{constructor}` has no field `{label_name}`"),
                    );
                    if let Some(close) = closest(label_name, open.iter().copied()) {
                        diagnostic.repairs.push(self.report.edit(
                            format!("write `{close}`"),
                            label.span,
                            close,
                        ));
                    } else if open.is_empty() {
                        diagnostic.repairs.push(Report::advice(
                            "every field is given by another argument; remove this one",
                        ));
                    } else {
                        // D-157, as for U2005.
                        let listed: Vec<String> = open.iter().map(|n| format!("`{n}`")).collect();
                        diagnostic.repairs.push(Report::advice(format!(
                            "the fields that no other argument gives are {}",
                            listed.join(", ")
                        )));
                    }
                    self.report.push(diagnostic);
                }
                found
            } else if let Some(target) = self.punned(arg, &names) {
                Some(target)
            } else if fields.len() == 1 && position == 0 {
                Some(0)
            } else if fields.len() == 1 {
                self.report.error(
                    Code::U2009,
                    arg.span,
                    format!(
                        "`{constructor}` has 1 field, but {} given",
                        given(args.len())
                    ),
                );
                None
            } else {
                let mut diagnostic = self.report.make(
                    Code::U2004,
                    arg.span,
                    format!(
                        "`{constructor}` has {} fields, so every argument needs its field's label",
                        fields.len()
                    ),
                );
                if let Some(field) = names.get(position) {
                    let text = self.report.text(arg.span);
                    diagnostic.repairs.push(self.report.edit(
                        format!("label it `{field}`"),
                        arg.span,
                        format!("{field}: {text}"),
                    ));
                }
                self.report.push(diagnostic);
                None
            };
            let target = match target {
                Some(target) if taken.get(target) == Some(&true) => {
                    let field = names.get(target).copied().unwrap_or("?");
                    self.report.error(
                        Code::U2006,
                        arg.span,
                        format!("field `{field}` of `{constructor}` is given twice"),
                    );
                    None
                }
                other => other,
            };
            if let Some(target) = target
                && let Some(slot) = taken.get_mut(target)
            {
                *slot = true;
            }
            problem |= target.is_none();
            targets.push(target);
        }
        self.ordered(span, targets.clone());
        let mut flows = Vec::new();
        let mut values = Vec::new();
        for (arg, target) in lambdas_last(args, &targets) {
            let Some(field) = target.and_then(|target| fields.get(target)) else {
                self.expr(&arg.value, None);
                continue;
            };
            let expected = field.ty.base.substitute(targs);
            let role = Role::Field {
                field: field.name,
                kind: constructor,
            };
            // A literal, a constructor or a list literal, given for a field whose type holds a
            // type parameter that the expected type fixes, takes the field's type at those
            // arguments, as `Some(3)` where an `Option[U8]` is expected (`spec/18 §2.1`, #187),
            // and `Box(v: [5])` or `Bag(items: [5])` where a `Box[List[Pos]]` or a `Bag[Pos]`
            // is (ADR 0103 rule 5, #1657). The parameters then take the value's type. A field
            // of a parameter takes the argument without its top label and refinement, which
            // the field's own flow and obligation check; a field that holds a parameter at a
            // nested position keeps the refinement there, since only the literal's elements
            // can prove it.
            let hinted = match hint {
                Some(hint) if Self::literal_or_constructor(&arg.value) => match &field.ty.base {
                    Ty::Param(param) => hint
                        .get(*param)
                        .filter(|ty| !holds_var(ty))
                        .map(|ty| ty.unlabeled().clone()),
                    base if holds_param(base) => {
                        let hint: Vec<Ty> = hint.iter().map(without_top_label).collect();
                        Some(base.substitute(&hint)).filter(|ty| !holds_var(ty))
                    }
                    _ => None,
                },
                _ => None,
            };
            let found = match hinted {
                Some(hinted) => {
                    let found = self.expr_as(&arg.value, Some(&hinted), role);
                    self.unify_at(arg.value.span, &expected, &found, role);
                    found
                }
                None => self.expr_as(&arg.value, Some(&expected), role),
            };
            self.fn_flow(arg.value.span, &expected, &found);
            if field.ty.is_refined() {
                let origin = self.origin(module, field.span);
                self.obligation(ObligationKind::Refinement, arg.value.span, origin);
            }
            values.push((arg.value.span, field.ty.base.clone(), found.clone()));
            flows.push((arg.value.span, found, expected, field.name));
        }
        if !targs.is_empty() {
            let (params, kind_name) = self.kind_params(kind);
            self.instantiation(kind_name, &params, targs.to_vec(), values, span);
        }
        // Fields flow once the whole constructor is typed. The program cannot raise the label
        // of a field of a kind of the standard library (#404).
        let standard = self
            .env
            .kinds
            .get(kind)
            .is_some_and(|k| !self.env.in_package(k.module));
        let mut result_label = Label::public();
        for (arg_span, found, expected, field) in flows {
            let into = format!("field `{field}` of `{constructor}`");
            if labeled_json {
                // ADR 0073 J5 as amended (accepted, 2026-11-10): the argument enters with its
                // labels removed, as a prelude function's argument does, and every label it
                // carries, at every position that holds data (`spec/07 §7` rule 3), labels the
                // result's top. Reading the value back is raised by that top label (rule 3),
                // and `to_json` carries it, so nothing that entered leaves lower.
                let resolved = self.labeled(&found);
                let label = self.deep(&resolved);
                if !label.is_public() && self.pass == super::authority::Pass::Check {
                    self.note_json_source(super::flow::JsonSource {
                        span: arg_span,
                        constructor: constructor.to_owned(),
                        label: label.clone(),
                    });
                }
                result_label = result_label.join(&label);
                let stripped = resolved.map_labels(&|_| Label::public());
                self.flow(arg_span, &stripped, &expected, &into);
            } else if standard {
                let kind_name = self.env.kind_name(kind);
                let mut advice = format!(
                    "a field of `{kind_name}`, a kind of the standard library, keeps its declared label: lower this value through a `policy`"
                );
                if kind_name == "Header" {
                    advice.push_str(", or give a token by the name of its key, as `key_name` of the request, rather than in a header");
                }
                self.flow_fixed(arg_span, &found, &expected, &into, advice);
            } else {
                self.flow(arg_span, &found, &expected, &into);
            }
        }
        if !problem {
            let missing: Vec<String> = fields
                .iter()
                .zip(&taken)
                .filter(|(_, taken)| !**taken)
                .map(|(field, _)| format!("`{}`", field.name))
                .collect();
            if !missing.is_empty() {
                let what = if missing.len() == 1 {
                    "field"
                } else {
                    "fields"
                };
                self.report.error(
                    Code::U2009,
                    name,
                    format!("`{constructor}` is missing {what} {}", missing.join(", ")),
                );
            }
        }
        result_label
    }
}

/// The prelude functions whose results carry the labels of their arguments, as `spec/14 §1`
/// "Result labels" lists them (ADR 0038 D3.4): their native implementations declare these
/// labels, which the prelude's Una signatures cannot write.
const PRELUDE_LABELED: &[&str] = &[
    "abs",
    "append",
    "at",
    "bytes",
    "chars",
    "contains",
    "contains_text",
    "ends_with",
    "filter",
    "find",
    "first",
    "first_where",
    "fold",
    "is_digits",
    "is_empty",
    "is_ok",
    "is_some",
    "join",
    "map",
    "max",
    "min",
    "or_else",
    "parse_int",
    // JSON (`spec/23 §5`): a parsed value and its failure, and a written or quoted text,
    // carry the labels of their argument.
    "parse_json",
    "quote_json",
    "replace",
    "slice",
    "sort",
    "split",
    "starts_with",
    "sum",
    "to_int",
    "to_json",
    "to_lower",
    "to_upper",
    "trim",
    "trim_end",
    "trim_start",
    // `F64` (`spec/29 §7`, `§10`, ADR 0114 rules 7.3 and 10.5): a parsed or converted value,
    // and the failure of `parse_f64`, carry the labels of their argument.
    "parse_f64",
    "f64_from_bits",
    "as_f64",
    "json_number",
    // `Decimal` (`spec/29 §16`, ADR 0120 IF1): a parsed or converted decimal, and the failure
    // of `parse_decimal`, carry the labels of their arguments.
    "parse_decimal",
    "as_decimal",
    "json_decimal",
    // Dense values (`spec/29 §17`, ADR 0120, IF2 to IF6): the constructors and the methods
    // that read, give a view of or compute from a dense value. The methods that update their
    // receiver are not here: their arguments keep their parameters' labels.
    "array",
    "array_filled",
    "matrix",
    "matrix_filled",
    "matrix_diagonal",
    "dense_array_get",
    "dense_array_at",
    "dense_array_slice",
    "dense_array_compact",
    "dense_array_to_list",
    "dense_array_map",
    "dense_array_zip_map",
    "dense_array_fold",
    "dense_array_multiply_elements",
    "dense_array_divide_elements",
    "dense_array_sum",
    "dense_array_dot",
    "dense_array_norm2",
    "dense_array_all_finite",
    "dense_array_sum_exact",
    "dense_array_dot_exact",
    "dense_matrix_get",
    "dense_matrix_at",
    "dense_matrix_row",
    "dense_matrix_column",
    "dense_matrix_block",
    "dense_matrix_transpose",
    "dense_matrix_compact",
    "dense_matrix_map",
    "dense_matrix_zip_map",
    "dense_matrix_fold",
    "dense_matrix_multiply_elements",
    "dense_matrix_divide_elements",
    "dense_matrix_sum",
    "dense_matrix_row_sums",
    "dense_matrix_column_sums",
    "dense_matrix_all_finite",
    "dense_matrix_sum_exact",
    "dense_matrix_multiply",
    "dense_matrix_multiply_array",
    "dense_matrix_to_tensor",
    "dense_matrix_lu",
    "dense_matrix_solve",
    "dense_matrix_solve_array",
    "dense_matrix_cholesky",
    "dense_matrix_solve_lower",
    "dense_matrix_solve_upper",
    "dense_lu_solve",
    "dense_lu_factors",
    "dense_lu_pivots",
    "dense_array_to_tensor",
    "tensor",
    "tensor_filled",
    "dense_tensor_dim",
    "dense_tensor_compact",
    "dense_tensor_to_array",
    "dense_tensor_to_matrix",
    "dense_tensor_map",
    "dense_tensor_zip_map",
    "dense_tensor_fold",
    "dense_tensor_multiply_elements",
    "dense_tensor_divide_elements",
    "dense_tensor_sum",
    "dense_tensor_sum_axis",
    "dense_tensor_all_finite",
    "dense_tensor_sum_exact",
    "dense_tensor_contract",
    // `F32` (ADR 0120 rule 5): the same for `parse_f32` and `f32_from_bits`.
    "parse_f32",
    "f32_from_bits",
];

/// Whether `name` is `prefix` followed by a rank from 1 to 8: a function of the prelude of one
/// rank of a tensor (`spec/29 §17`).
fn ranked(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|rank| rank.parse::<usize>().ok())
        .is_some_and(|rank| (1..=8).contains(&rank))
}

/// For a structural prelude function (ADR 0051, `spec/14 §1`), the parameters that decide the
/// shape of its result, whose labels raise its top label: by their top label, or by every label
/// (`true`) when their value decides it. `None` for the other functions.
fn structural_inputs(name: &str) -> Option<&'static [(&'static str, bool)]> {
    Some(match name {
        "append" | "first" | "is_empty" => &[("xs", false)],
        "at" => &[("xs", false), ("index", true)],
        "is_some" => &[("o", false)],
        "is_ok" => &[("r", false)],
        "map" => &[("xs", false), ("each", false)],
        "fold" => &[("xs", false), ("step", false)],
        "filter" | "first_where" => &[("xs", false), ("keep", false)],
        // Dense values (`spec/29 §17`, IF3, IF5): the top label is the label of what decides
        // the shape, and of the indices a read or a view takes; each element position carries
        // the labels of its type variable.
        "array" => &[("xs", false)],
        "array_filled" => &[("length", true)],
        // `std.collections` (ADR 0119 IF1, IF2): a bit set's size is its shape, so the bit set
        // takes every label of `size`; a conversion takes the list's top label, and its
        // positions keep the labels of their type variables.
        "empty_bit_set" => &[("size", true)],
        "to_stack" | "to_queue" | "to_set" | "to_hash_set" => &[("xs", false)],
        "to_hash_map" => &[("es", false)],
        "matrix" => &[("rows", true), ("columns", true), ("elements", false)],
        "matrix_filled" => &[("rows", true), ("columns", true)],
        "matrix_diagonal" => &[("values", false)],
        "dense_array_get" | "dense_array_at" => &[("a", false), ("index", true)],
        "dense_matrix_get" | "dense_matrix_at" => &[("m", false), ("row", true), ("column", true)],
        "dense_array_slice" => &[("a", false), ("from_", true), ("until", true)],
        "dense_matrix_row" | "dense_matrix_column" => &[("m", false), ("index", true)],
        "dense_matrix_block" => &[
            ("m", false),
            ("row_from", true),
            ("row_until", true),
            ("column_from", true),
            ("column_until", true),
        ],
        "dense_array_compact" | "dense_array_to_list" => &[("a", false)],
        "dense_matrix_transpose" | "dense_matrix_compact" => &[("m", false)],
        "dense_array_map" => &[("a", false), ("each", false)],
        "dense_matrix_map" => &[("m", false), ("each", false)],
        "dense_array_zip_map" => &[("a", false), ("with_", false), ("each", false)],
        "dense_matrix_zip_map" => &[("m", false), ("with_", false), ("each", false)],
        "dense_array_fold" => &[("a", false), ("step", false)],
        "dense_matrix_fold" => &[("m", false), ("step", false)],
        "dense_array_multiply_elements" | "dense_array_divide_elements" => {
            &[("a", false), ("by", false)]
        }
        "dense_matrix_multiply_elements" | "dense_matrix_divide_elements" => {
            &[("m", false), ("by", false)]
        }
        "dense_array_to_tensor" => &[("a", false)],
        "dense_matrix_to_tensor" => &[("m", false)],
        "dense_lu_factors" => &[("f", false)],
        "dense_tensor_dim" => &[("t", false), ("axis", true)],
        "dense_tensor_compact" | "dense_tensor_to_array" | "dense_tensor_to_matrix" => {
            &[("t", false)]
        }
        "dense_tensor_map" => &[("t", false), ("each", false)],
        "dense_tensor_zip_map" => &[("t", false), ("with_", false), ("each", false)],
        "dense_tensor_fold" => &[("t", false), ("step", false)],
        "dense_tensor_multiply_elements" | "dense_tensor_divide_elements" => {
            &[("t", false), ("by", false)]
        }
        // The functions of each rank (`spec/29 §17`): a shape's lengths and an index decide the
        // top label; the elements carry their type variable's labels.
        name if ranked(name, "dense_tensor_make_") => &[
            ("shape_0", true),
            ("shape_1", true),
            ("shape_2", true),
            ("shape_3", true),
            ("shape_4", true),
            ("shape_5", true),
            ("shape_6", true),
            ("shape_7", true),
            ("elements", false),
        ],
        name if ranked(name, "dense_tensor_filled_") => &[
            ("shape_0", true),
            ("shape_1", true),
            ("shape_2", true),
            ("shape_3", true),
            ("shape_4", true),
            ("shape_5", true),
            ("shape_6", true),
            ("shape_7", true),
        ],
        name if ranked(name, "dense_tensor_get_") || ranked(name, "dense_tensor_at_") => &[
            ("t", false),
            ("at_0", true),
            ("at_1", true),
            ("at_2", true),
            ("at_3", true),
            ("at_4", true),
            ("at_5", true),
            ("at_6", true),
            ("at_7", true),
        ],
        name if ranked(name, "dense_tensor_permute_") => &[("t", false)],
        name if ranked(name, "dense_tensor_reshape_") => &[
            ("t", false),
            ("shape_0", true),
            ("shape_1", true),
            ("shape_2", true),
            ("shape_3", true),
            ("shape_4", true),
            ("shape_5", true),
            ("shape_6", true),
            ("shape_7", true),
        ],
        _ => return None,
    })
}

/// Whether the parameter `param` of the host operation `capability.op` decides whether it
/// fails, and how (`spec/07 §7`, S-15): every argument of `Http`, since the server
/// receives them all;
/// the path of `Files`, not the content, besides the file-state label of every `Files` operation
/// (ADR 0085); the key name of `Keys`, not the data; both arguments of `HttpServer.reply` (ADR
/// 0123 IF3).
fn decides_failure(capability: &str, op: &str, param: &str) -> bool {
    match (capability, op) {
        ("Http", _) => true,
        // ADR 0123 IF3: `NotHeld` and `BadReply` are decided by the reply right and the reply.
        ("HttpServer", "reply") => true,
        ("Files", _) => param == "path",
        ("Keys", "hmac") => param == "key_name",
        _ => false,
    }
}

/// Whether `def` is a prelude function whose result carries the labels of its arguments.
/// D-136: its parameters that are not functions accept arguments of any label,
/// since the result carries all of them, at the positions that hold them for the structural
/// functions (ADR 0051); the other prelude functions keep the labels of their parameter types.
fn prelude_labeled(prelude: usize, def: &FnDef<'_>) -> bool {
    def.module == prelude
        && def.kind == FnKind::Function
        && (PRELUDE_LABELED.contains(&def.name)
            || RANKED_LABELED.iter().any(|prefix| ranked(def.name, prefix)))
}

/// The prelude functions of each rank of a tensor whose results carry their arguments' labels
/// as [`PRELUDE_LABELED`] lists them (`spec/29 §17`): all of them but `set`, whose arguments
/// keep their parameters' labels, as for the other dense methods that update their receiver.
const RANKED_LABELED: [&str; 6] = [
    "dense_tensor_make_",
    "dense_tensor_filled_",
    "dense_tensor_get_",
    "dense_tensor_at_",
    "dense_tensor_permute_",
    "dense_tensor_reshape_",
];

/// The functions of `std.collections` whose results carry the labels of their arguments, as
/// the prelude's of `PRELUDE_LABELED` do: `empty_bit_set`, whose size is part of the bit set's
/// shape (ADR 0119 IF2), and the conversions of rule 16, whose results hold the list's items in
/// its order. Their bodies are placeholders that both engines replace, so no summary of a body
/// could label them.
const COLLECTIONS_LABELED: [&str; 6] = [
    "empty_bit_set",
    "to_stack",
    "to_queue",
    "to_set",
    "to_hash_set",
    "to_hash_map",
];

/// Whether `def` is a function of `std.collections` whose result is labeled by its arguments
/// (`COLLECTIONS_LABELED`), with `structural_inputs`.
fn collections_labeled(collections: Option<usize>, def: &FnDef<'_>) -> bool {
    Some(def.module) == collections
        && def.kind == FnKind::Function
        && COLLECTIONS_LABELED.contains(&def.name)
}

/// Whether every type parameter of `ty` is at a covariant position (ADR 0103 rule 5): never
/// inside a function type, nor in an argument of a kind's parameter that is invariant (ADR
/// 0046), as one that a field holds in a function type is.
fn covariant_in(ty: &Ty, kinds: &KindLabels) -> bool {
    let no_params = |ty: &Ty| {
        let mut params = Vec::new();
        ty.params(&mut params);
        params.is_empty()
    };
    match ty {
        Ty::Labeled(_, inner) | Ty::Refined(_, inner) | Ty::List(inner) | Ty::Dense(_, inner) => {
            covariant_in(inner, kinds)
        }
        Ty::Map(key, value) => covariant_in(key, kinds) && covariant_in(value, kinds),
        Ty::Coll(_, args) => args.iter().all(|arg| covariant_in(arg, kinds)),
        Ty::Kind(id, args) => args.iter().enumerate().all(|(index, arg)| {
            if kinds.is_invariant(*id, index) {
                no_params(arg)
            } else {
                covariant_in(arg, kinds)
            }
        }),
        Ty::Fn(_) => no_params(ty),
        _ => true,
    }
}

/// `ty` without the label at its top, its refinements kept.
fn without_top_label(ty: &Ty) -> Ty {
    match ty {
        Ty::Labeled(_, inner) => (**inner).clone(),
        other => other.clone(),
    }
}

/// Whether `ty` names a type parameter at some position. O(size of `ty`).
fn holds_param(ty: &Ty) -> bool {
    match ty {
        Ty::Param(_) => true,
        Ty::Labeled(_, inner) | Ty::Refined(_, inner) | Ty::List(inner) | Ty::Dense(_, inner) => {
            holds_param(inner)
        }
        Ty::Map(key, value) => holds_param(key) || holds_param(value),
        Ty::Kind(_, args) | Ty::Coll(_, args) => args.iter().any(holds_param),
        Ty::Fn(function) => {
            function.params.iter().any(holds_param)
                || holds_param(&function.result)
                || function.fails.as_ref().is_some_and(holds_param)
        }
        _ => false,
    }
}

/// Whether `ty` holds a type variable at any position.
fn holds_var(ty: &Ty) -> bool {
    match ty {
        Ty::Var(_) => true,
        Ty::Labeled(_, inner) | Ty::Refined(_, inner) | Ty::List(inner) | Ty::Dense(_, inner) => {
            holds_var(inner)
        }
        Ty::Map(key, value) => holds_var(key) || holds_var(value),
        Ty::Kind(_, args) | Ty::Coll(_, args) => args.iter().any(holds_var),
        Ty::Fn(function) => {
            function.params.iter().any(holds_var)
                || holds_var(&function.result)
                || function.fails.as_ref().is_some_and(holds_var)
        }
        _ => false,
    }
}

/// The declared type of the parameter of each checked argument, with where the argument is
/// and its own type.
fn declared_args(params: &[ParamDef<'_>], checked: &[Checked<'_>]) -> Vec<(Span, Ty, Ty)> {
    checked
        .iter()
        .filter_map(|(span, found, _, name)| {
            params
                .iter()
                .find(|p| p.name == *name)
                .map(|p| (*span, p.ty.base.clone(), found.clone()))
        })
        .collect()
}

/// What a function type in a value's type has beyond the expected function type at its
/// position.
enum FnProblem {
    Uses(FnUse),
    Fails(Ty),
}

/// The first function type in `found` that exceeds the one at its position in `expected`,
/// both resolved (S-Fun, ADR 0038 D1), and whether that position is a function parameter's,
/// where the expected type gives the value its functions (`flipped` says whether the current
/// position is one).
fn fn_problem(expected: &Ty, found: &Ty, flipped: bool) -> Option<(FnProblem, bool)> {
    match (expected, found) {
        (Ty::Fn(e), Ty::Fn(f)) => {
            let extra = f.uses.iter().find(|need| {
                !e.uses.iter().any(|allowed| {
                    allowed.name == need.name
                        && allowed.capability == need.capability
                        && need.ops.iter().all(|op| allowed.ops.contains(op))
                })
            });
            if let Some(need) = extra {
                return Some((FnProblem::Uses(need.clone()), flipped));
            }
            if let Some(kind) = &f.fails
                && *kind != Ty::error()
                && e.fails
                    .as_ref()
                    .is_none_or(|allowed| allowed != kind && *allowed != Ty::error())
            {
                return Some((FnProblem::Fails(kind.clone()), flipped));
            }
            // Parameters are contravariant: the expected type's arguments flow into the value.
            e.params
                .iter()
                .zip(&f.params)
                .find_map(|(e, f)| fn_problem(f, e, !flipped))
                .or_else(|| fn_problem(&e.result, &f.result, flipped))
        }
        (Ty::List(e), Ty::List(f)) | (Ty::Dense(_, e), Ty::Dense(_, f)) => {
            fn_problem(e, f, flipped)
        }
        (Ty::Map(ek, ev), Ty::Map(fk, fv)) => {
            fn_problem(ek, fk, flipped).or_else(|| fn_problem(ev, fv, flipped))
        }
        (Ty::Kind(i, es), Ty::Kind(j, fs)) if i == j => es
            .iter()
            .zip(fs)
            .find_map(|(e, f)| fn_problem(e, f, flipped)),
        (Ty::Coll(i, es), Ty::Coll(j, fs)) if i == j => es
            .iter()
            .zip(fs.iter())
            .find_map(|(e, f)| fn_problem(e, f, flipped)),
        _ => None,
    }
}

/// Whether a value of type `found` flowing into `expected` is a `refinement` obligation of
/// ADR 0037 rule 2: a function type at the same position, directly or under type arguments,
/// whose parameter carries a refinement that the expected parameter does not (contravariant),
/// or whose result lacks a refinement of the expected result (covariant). Both are resolved.
pub(crate) fn refinement_flow(expected: &Ty, found: &Ty) -> bool {
    match (expected.unlabeled(), found.unlabeled()) {
        (Ty::Fn(e), Ty::Fn(f)) if e.params.len() == f.params.len() => {
            let lacks = |target: &[Refinement], source: &[Refinement]| {
                source.iter().any(|refinement| !target.contains(refinement))
            };
            let params = (0..e.params.len()).any(|index| {
                lacks(e.refinements.param(index), f.refinements.param(index))
                    || matches!(
                        (f.params.get(index), e.params.get(index)),
                        (Some(target), Some(source)) if refinement_flow(target, source)
                    )
            });
            params
                || lacks(&f.refinements.result, &e.refinements.result)
                || refinement_flow(&e.result, &f.result)
        }
        (Ty::List(e), Ty::List(f)) | (Ty::Dense(_, e), Ty::Dense(_, f)) => refinement_flow(e, f),
        (Ty::Map(ek, ev), Ty::Map(fk, fv)) => refinement_flow(ek, fk) || refinement_flow(ev, fv),
        (Ty::Kind(i, es), Ty::Kind(j, fs)) if i == j && es.len() == fs.len() => {
            es.iter().zip(fs).any(|(e, f)| refinement_flow(e, f))
        }
        (Ty::Coll(i, es), Ty::Coll(j, fs)) if i == j && es.len() == fs.len() => {
            es.iter().zip(fs.iter()).any(|(e, f)| refinement_flow(e, f))
        }
        _ => false,
    }
}

/// The arguments with their targets, lambdas after the other arguments, each group in source
/// order.
fn lambdas_last<'x>(
    args: &'x [Arg],
    targets: &'x [Option<usize>],
) -> Vec<(&'x Arg, Option<usize>)> {
    let is_lambda = |arg: &Arg| matches!(arg.value.kind, ExprKind::Lambda { .. });
    let pairs = args.iter().zip(targets.iter().copied());
    let (lambdas, others): (Vec<_>, Vec<_>) = pairs.partition(|(arg, _)| is_lambda(arg));
    others.into_iter().chain(lambdas).collect()
}

/// "1 is given", "2 are given".
fn given(n: usize) -> String {
    if n == 1 {
        "1 is".to_owned()
    } else {
        format!("{n} are")
    }
}
